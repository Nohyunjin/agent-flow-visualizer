//! Timing is limited to retained transcript evidence, separately for each agent.
//! Tool spans are never added to turn elapsed time or to their parent's time.
use crate::model::{FlowEvent, Kind, Provider, Session, TurnBoundary, one_line};
use chrono::{DateTime, Utc};

#[derive(Clone, Debug)]
pub struct ToolTiming {
    pub event_id: String,
    pub name: String,
    pub duration_ms: u64,
}

#[derive(Clone, Debug)]
pub struct TurnTiming {
    pub event_id: String,
    pub title: String,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub reported: bool,
    pub interrupted: bool,
    pub ended: bool,
    pub open: bool,
    pub longest_tool: Option<ToolTiming>,
    explicit_start: bool,
    has_work: bool,
}

impl TurnTiming {
    fn new(e: &FlowEvent, explicit_start: bool) -> Self {
        Self {
            event_id: e.id.clone(),
            title: if e.kind == Kind::User {
                one_line(&e.input, 120)
            } else {
                String::new()
            },
            start: valid_time(e.time),
            end: None,
            duration_ms: None,
            reported: false,
            interrupted: false,
            ended: false,
            open: false,
            longest_tool: None,
            explicit_start,
            has_work: false,
        }
    }

    fn finish(&mut self, at: DateTime<Utc>, reported_ms: Option<u64>, interrupted: bool) {
        self.end = valid_time(at);
        self.ended = true;
        self.interrupted = interrupted;
        self.duration_ms = self.start.zip(self.end).and_then(|(a, b)| elapsed(a, b));
        // A provider duration can recover a turn whose start is outside retention.
        // Invalid/reversed timestamp pairs remain unknown instead of becoming zero.
        if self.start.is_none() {
            self.duration_ms = reported_ms;
            self.reported = reported_ms.is_some();
        }
    }

    pub fn label(&self) -> &str {
        if self.title.is_empty() {
            "Turn (request not retained)"
        } else {
            &self.title
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SessionTiming {
    pub turns: Vec<TurnTiming>,
    pub total_ms: Option<u64>,
    pub longest_turn: Option<usize>,
    pub longest_tool: Option<ToolTiming>,
    pub open_ms: Option<u64>,
    pub partial: bool,
}

fn valid_time(at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    (at != DateTime::UNIX_EPOCH).then_some(at)
}

pub fn elapsed(start: DateTime<Utc>, end: DateTime<Utc>) -> Option<u64> {
    valid_time(start)?;
    valid_time(end)?;
    if end < start {
        return None;
    }
    u64::try_from(end.signed_duration_since(start).num_milliseconds()).ok()
}

fn keep_longest(current: &mut Option<ToolTiming>, candidate: &ToolTiming) {
    if current
        .as_ref()
        .is_none_or(|t| candidate.duration_ms > t.duration_ms)
    {
        *current = Some(candidate.clone());
    }
}

pub fn summarize(session: &Session, now: DateTime<Utc>) -> SessionTiming {
    let mut summary = SessionTiming {
        partial: session.dropped > 0
            || session.malformed > 0
            || session.bytes_read < session.file_size,
        ..SessionTiming::default()
    };
    let mut current: Option<TurnTiming> = None;
    let mut work_since_end = false;
    for e in &session.events {
        match &e.turn_boundary {
            Some(TurnBoundary::Start) => {
                // Some transcripts put the prompt immediately before the start marker.
                if let Some(turn) = current
                    .as_mut()
                    .filter(|t| !t.explicit_start && !t.has_work)
                {
                    turn.explicit_start = true;
                    turn.start = valid_time(e.time);
                } else {
                    if let Some(turn) = current.take() {
                        summary.partial = true;
                        summary.turns.push(turn);
                    }
                    current = Some(TurnTiming::new(e, true));
                }
                work_since_end = true;
            }
            Some(TurnBoundary::Completed { at, reported_ms }) => {
                if let Some(mut turn) = current.take() {
                    turn.finish(*at, *reported_ms, false);
                    summary.turns.push(turn);
                } else if session.provider == Provider::Claude
                    && e.kind == Kind::Turn
                    && !work_since_end
                    && summary
                        .turns
                        .last()
                        .is_some_and(|t| t.ended && !t.interrupted)
                {
                    // Claude may record both end_turn and a subsequent turn_duration.
                    let last = summary.turns.last_mut().unwrap();
                    if last.duration_ms.is_none() && last.start.is_none() && reported_ms.is_some() {
                        last.finish(*at, *reported_ms, false);
                    }
                } else {
                    let mut turn = TurnTiming::new(e, false);
                    turn.start = None;
                    turn.finish(*at, *reported_ms, false);
                    summary.turns.push(turn);
                    summary.partial = true;
                }
                work_since_end = false;
            }
            Some(TurnBoundary::Interrupted) => {
                let mut turn = current.take().unwrap_or_else(|| {
                    let mut turn = TurnTiming::new(e, false);
                    turn.start = None;
                    summary.partial = true;
                    turn
                });
                turn.finish(e.time, None, true);
                summary.turns.push(turn);
                work_since_end = false;
            }
            None => {
                if e.kind == Kind::User {
                    let turn = current.get_or_insert_with(|| TurnTiming::new(e, false));
                    if turn.title.is_empty() {
                        turn.title = one_line(&e.input, 120);
                    }
                }
                if matches!(
                    e.kind,
                    Kind::User | Kind::Assistant | Kind::Tool | Kind::Spawn | Kind::Message
                ) {
                    work_since_end = true;
                }
            }
        }
        if let Some(turn) = &mut current {
            turn.has_work |= matches!(e.kind, Kind::Assistant | Kind::Tool | Kind::Spawn);
        }
        if e.is_tool() {
            if let Some(ms) = e.completed_at.and_then(|end| elapsed(e.time, end)) {
                let tool = ToolTiming {
                    event_id: e.id.clone(),
                    name: e.name.clone(),
                    duration_ms: ms,
                };
                keep_longest(&mut summary.longest_tool, &tool);
                if let Some(turn) = &mut current {
                    keep_longest(&mut turn.longest_tool, &tool);
                }
            } else if e.completed_at.is_some() {
                summary.partial = true;
            }
        }
    }
    if let Some(mut turn) = current {
        turn.open = session.turn_open;
        if turn.open {
            summary.open_ms = turn.start.and_then(|start| elapsed(start, now));
            summary.partial |= summary.open_ms.is_none();
        } else {
            summary.partial = true;
        }
        summary.turns.push(turn);
    }
    summary.partial |= session.turn_open && summary.open_ms.is_none();
    let mut intervals = Vec::new();
    let mut unplaced_ms = 0u64;
    for (i, turn) in summary.turns.iter().enumerate() {
        if let Some(ms) = turn.duration_ms {
            if summary
                .longest_turn
                .is_none_or(|j| Some(ms) > summary.turns[j].duration_ms)
            {
                summary.longest_turn = Some(i);
            }
            // Union observed intervals: overlapping records cannot inflate elapsed time.
            let start = if turn.reported {
                turn.end.and_then(|end| {
                    let delta = chrono::Duration::try_milliseconds(i64::try_from(ms).ok()?)?;
                    end.checked_sub_signed(delta)
                })
            } else {
                turn.start
            };
            if let Some((start, end)) = start.zip(turn.end) {
                intervals.push((start, end));
            } else {
                unplaced_ms = unplaced_ms.saturating_add(ms);
            }
        } else if !turn.open {
            summary.partial = true;
        }
    }
    if summary.longest_turn.is_some() {
        intervals.sort_unstable();
        let mut merged: Option<(DateTime<Utc>, DateTime<Utc>)> = None;
        let mut total = unplaced_ms;
        for (start, end) in intervals {
            match &mut merged {
                Some((_, last_end)) if start <= *last_end => *last_end = (*last_end).max(end),
                _ => {
                    if let Some((a, b)) = merged {
                        total = total.saturating_add(elapsed(a, b).unwrap_or(0));
                    }
                    merged = Some((start, end));
                }
            }
        }
        if let Some((a, b)) = merged {
            total = total.saturating_add(elapsed(a, b).unwrap_or(0));
        }
        summary.total_ms = Some(total);
    }
    summary
}

pub fn format_duration(ms: Option<u64>) -> String {
    match ms {
        None => "—".into(),
        Some(ms) if ms < 1000 => format!("{ms}ms"),
        Some(ms) if ms < 60_000 => format!("{:.1}s", ms as f64 / 1000.0),
        Some(ms) if ms < 3_600_000 => format!("{}m {:02}s", ms / 60_000, ms / 1000 % 60),
        Some(ms) => format!("{}h {:02}m", ms / 3_600_000, ms / 60_000 % 60),
    }
}
