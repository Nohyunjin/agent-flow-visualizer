use crate::{
    model::{Provider, Snapshot},
    timing::{SessionTiming, TurnTiming, summarize},
};
use chrono::{DateTime, Duration, Utc};
use ratatui::widgets::ListState;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActivityWindow {
    #[default]
    Day,
    Week,
    All,
}

impl ActivityWindow {
    pub fn label(self) -> &'static str {
        match self {
            Self::Day => "24h",
            Self::Week => "7d",
            Self::All => "All",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::Day => Self::Week,
            Self::Week => Self::All,
            Self::All => Self::Day,
        }
    }
    fn includes(self, last_activity: DateTime<Utc>, now: DateTime<Utc>) -> bool {
        let duration = match self {
            Self::Day => Duration::hours(24),
            Self::Week => Duration::days(7),
            Self::All => return true,
        };
        last_activity != DateTime::UNIX_EPOCH
            && now.signed_duration_since(last_activity) <= duration
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    LongestTurn,
    Total,
    LongestTool,
    Open,
}

impl Sort {
    pub fn label(self) -> &'static str {
        match self {
            Self::LongestTurn => "slowest turn",
            Self::Total => "total ended turns",
            Self::LongestTool => "slowest tool",
            Self::Open => "open turn age",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::LongestTurn => Self::Total,
            Self::Total => Self::LongestTool,
            Self::LongestTool => Self::Open,
            Self::Open => Self::LongestTurn,
        }
    }
    fn value(self, timing: &SessionTiming) -> Option<u64> {
        match self {
            Self::LongestTurn => timing
                .longest_turn
                .and_then(|i| timing.turns[i].duration_ms),
            Self::Total => timing.total_ms,
            Self::LongestTool => timing.longest_tool.as_ref().map(|t| t.duration_ms),
            Self::Open => timing.open_ms,
        }
    }
}

pub struct Row {
    pub key: String,
    pub session: usize,
    pub timing: SessionTiming,
}

pub struct Dashboard {
    pub visible: bool,
    pub rows: Vec<Row>,
    pub sessions: ListState,
    pub turns: ListState,
    pub turn_order: Vec<usize>,
    pub focus_turns: bool,
    pub sort: Sort,
    pub window: ActivityWindow,
}

impl Default for Dashboard {
    fn default() -> Self {
        Self {
            visible: true,
            rows: vec![],
            sessions: ListState::default(),
            turns: ListState::default(),
            turn_order: vec![],
            focus_turns: false,
            sort: Sort::default(),
            window: ActivityWindow::default(),
        }
    }
}

impl Dashboard {
    pub fn selected(&self) -> Option<&Row> {
        self.rows.get(self.sessions.selected()?)
    }
    pub fn selected_turn(&self) -> Option<&TurnTiming> {
        let i = *self.turn_order.get(self.turns.selected()?)?;
        self.selected()?.timing.turns.get(i)
    }
    pub fn rebuild(
        &mut self,
        snapshot: &Snapshot,
        provider: Option<Provider>,
        query: &str,
        active: bool,
    ) {
        let selected_key = self.selected().map(|r| r.key.clone());
        let selected_turn = self.selected_turn().map(|t| t.event_id.clone());
        let query = query.to_lowercase();
        self.rows = snapshot
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                provider.is_none_or(|p| p == s.provider)
                    && self.window.includes(s.last_activity, snapshot.scanned_at)
                    && (!active || s.status(snapshot.scanned_at) == "WORKING")
                    && (query.is_empty()
                        || format!("{} {} {} {}", s.title, s.cwd, s.id, s.agent_path)
                            .to_lowercase()
                            .contains(&query))
            })
            .map(|(i, s)| Row {
                key: s.key.clone(),
                session: i,
                timing: summarize(s, snapshot.scanned_at),
            })
            .collect();
        self.rows.sort_by(|a, b| {
            self.sort
                .value(&b.timing)
                .cmp(&self.sort.value(&a.timing))
                .then(a.key.cmp(&b.key))
        });
        let index = self
            .rows
            .iter()
            .position(|r| Some(&r.key) == selected_key.as_ref())
            .or_else(|| (!self.rows.is_empty()).then_some(0));
        self.sessions.select(index);
        let same = self
            .selected()
            .is_some_and(|r| Some(&r.key) == selected_key.as_ref());
        self.rebuild_turns(if same { selected_turn.as_deref() } else { None });
    }
    fn rebuild_turns(&mut self, selected: Option<&str>) {
        let mut order: Vec<_> = self
            .selected()
            .map(|r| (0..r.timing.turns.len()).collect())
            .unwrap_or_default();
        if let Some(row) = self.selected() {
            order.sort_by(|a, b| {
                row.timing.turns[*b]
                    .duration_ms
                    .cmp(&row.timing.turns[*a].duration_ms)
                    .then(a.cmp(b))
            });
        }
        self.turn_order = order;
        let index = self
            .selected()
            .and_then(|r| {
                self.turn_order
                    .iter()
                    .position(|i| Some(r.timing.turns[*i].event_id.as_str()) == selected)
            })
            .or_else(|| (!self.turn_order.is_empty()).then_some(0));
        self.turns.select(index);
    }
    pub fn movement(&mut self, delta: isize) {
        if self.focus_turns {
            move_selection(&mut self.turns, self.turn_order.len(), delta);
        } else {
            let previous = self.sessions.selected();
            move_selection(&mut self.sessions, self.rows.len(), delta);
            if previous != self.sessions.selected() {
                self.rebuild_turns(None);
            }
        }
    }
    pub fn target(&self, tool: bool) -> Option<(String, String)> {
        let row = self.selected()?;
        let event = if tool {
            if self.focus_turns {
                self.selected_turn()?.longest_tool.as_ref()
            } else {
                row.timing.longest_tool.as_ref()
            }?
            .event_id
            .clone()
        } else if self.focus_turns {
            self.selected_turn()?.event_id.clone()
        } else if self.sort == Sort::Open {
            row.timing.turns.iter().find(|t| t.open)?.event_id.clone()
        } else {
            let turn = row
                .timing
                .longest_turn
                .map(|i| &row.timing.turns[i])
                .or_else(|| row.timing.turns.iter().find(|t| t.open))
                .or_else(|| row.timing.turns.first())?;
            turn.event_id.clone()
        };
        Some((row.key.clone(), event))
    }
}

fn move_selection(state: &mut ListState, len: usize, delta: isize) {
    if len > 0 {
        state.select(Some(
            state
                .selected()
                .unwrap_or(0)
                .saturating_add_signed(delta)
                .min(len - 1),
        ));
    }
}
