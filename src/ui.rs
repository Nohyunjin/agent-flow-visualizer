use crate::{
    app::{App, Pane},
    model::*,
    timing::{elapsed, format_duration},
};
use chrono::{DateTime, Local, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;
fn color(outcome: Outcome) -> Color {
    match outcome {
        Outcome::Pending => Color::Yellow,
        Outcome::Error => Color::Red,
        Outcome::Returned => Color::Green,
        Outcome::Info => MUTED,
    }
}
fn status_color(status: &str) -> Color {
    match status {
        "WORKING" => Color::Yellow,
        "READY" => Color::Green,
        _ => MUTED,
    }
}
fn block(title: String, focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(if focused { ACCENT } else { MUTED }))
}

fn activity_age(last_activity: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let elapsed = now
        .signed_duration_since(last_activity)
        .num_seconds()
        .max(0);
    if last_activity == DateTime::UNIX_EPOCH {
        "?".into()
    } else if elapsed < 60 {
        format!("{elapsed}s")
    } else if elapsed < 3600 {
        format!("{}m", elapsed / 60)
    } else if elapsed < 86400 {
        format!("{}h", elapsed / 3600)
    } else {
        format!("{}d", elapsed / 86400)
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width < 42 || area.height < 12 {
        frame.render_widget(
            Paragraph::new("Agent Flow\nResize to at least 42 × 12.\nq: quit")
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(6),
        Constraint::Length(2),
    ])
    .split(area);
    header(frame, app, sections[0]);
    let body = sections[1];
    if app.dashboard.visible {
        dashboard(frame, app, body);
    } else if app.parallel.visible {
        parallel(frame, app, body);
    } else if area.width >= 145 {
        let cols = Layout::horizontal([
            Constraint::Percentage(25),
            Constraint::Percentage(39),
            Constraint::Percentage(36),
        ])
        .split(body);
        agents(frame, app, cols[0]);
        flow(frame, app, cols[1]);
        detail(frame, app, cols[2]);
    } else if area.width >= 95 {
        let cols = Layout::horizontal([Constraint::Length(32), Constraint::Min(40)]).split(body);
        let rows = Layout::vertical([Constraint::Percentage(53), Constraint::Percentage(47)])
            .split(cols[1]);
        agents(frame, app, cols[0]);
        flow(frame, app, rows[0]);
        detail(frame, app, rows[1]);
    } else {
        match app.pane {
            Pane::Agents => agents(frame, app, body),
            Pane::Flow => flow(frame, app, body),
            Pane::Detail => detail(frame, app, body),
        }
    }
    footer(frame, app, sections[2]);
    if app.help {
        help(frame, app, area);
    }
}

fn header(frame: &mut Frame, app: &App, area: Rect) {
    let now = Utc::now();
    let active = app
        .snapshot
        .sessions
        .iter()
        .filter(|s| s.status(now) == "WORKING")
        .count();
    let pending = app
        .snapshot
        .sessions
        .iter()
        .flat_map(|s| &s.events)
        .filter(|e| e.outcome == Outcome::Pending)
        .count();
    let mode = if app.demo {
        " DEMO "
    } else if app.paused {
        " PAUSED "
    } else {
        " LIVE "
    };
    let provider = app.provider.map(Provider::label).unwrap_or("All providers");
    let title = Line::from(vec![
        Span::styled(" AGENT FLOW ", Style::default().fg(ACCENT).bold()),
        Span::styled(mode, Style::default().reversed()),
        Span::raw(format!(
            "  {active} working · {} loaded · {pending} awaiting result",
            app.snapshot.sessions.len()
        )),
    ]);
    if app.dashboard.visible {
        frame.render_widget(
            Paragraph::new(vec![
                title,
                Line::from(vec![
                    Span::styled(" DASHBOARD ", Style::default().fg(ACCENT).bold()),
                    Span::raw(format!(
                        " w Activity: {} · o Sort: {} · {provider}{}",
                        app.dashboard.window.label(),
                        app.dashboard.sort.label(),
                        if app.active_only {
                            " · recent activity"
                        } else {
                            ""
                        }
                    )),
                ]),
                Line::styled(
                    " Retained events · each agent separately · open age does not prove liveness",
                    Style::default().fg(MUTED),
                ),
            ]),
            area,
        );
        return;
    }
    if app.parallel.visible {
        let root = app
            .parallel
            .root
            .as_ref()
            .and_then(|key| app.snapshot.sessions.iter().find(|s| &s.key == key));
        let label = root
            .map(|s| {
                if s.title.is_empty() {
                    s.label()
                } else {
                    one_line(&s.title, 55)
                }
            })
            .unwrap_or_else(|| "Parent transcript not loaded".into());
        frame.render_widget(
            Paragraph::new(vec![
                title,
                Line::from(vec![
                    Span::styled(" PARALLEL ", Style::default().fg(ACCENT).bold()),
                    Span::raw(format!(
                        "{}/{} · {label}",
                        if app.parallel.lanes.is_empty() {
                            0
                        } else {
                            app.parallel.focus + 1
                        },
                        app.parallel.lanes.len()
                    )),
                ]),
                Line::styled(
                    format!(
                        " {}{}{}{}",
                        if area.width < 65 {
                            "Rows are not time-aligned."
                        } else {
                            "Independent lists; rows are not time-aligned."
                        },
                        if app.tools_only { "  tools" } else { "" },
                        if app.errors_only { "  errors" } else { "" },
                        if app.event_query.is_empty() {
                            String::new()
                        } else {
                            format!("  /{}", app.event_query)
                        }
                    ),
                    Style::default().fg(MUTED),
                ),
            ]),
            area,
        );
        return;
    }
    if !app.subtree
        && let Some(session) = app.selected_session()
    {
        let mut trail = vec![session.label()];
        let mut parent = session.parent.as_ref();
        let mut seen = std::collections::HashSet::from([session.key.as_str()]);
        while let Some(key) = parent {
            if !seen.insert(key) {
                break;
            }
            if let Some(s) = app.snapshot.sessions.iter().find(|s| &s.key == key) {
                trail.push(s.label());
                parent = s.parent.as_ref();
            } else {
                trail.push("parent not loaded".into());
                break;
            }
        }
        trail.reverse();
        let children = app
            .snapshot
            .sessions
            .iter()
            .filter(|s| s.parent.as_ref() == Some(&session.key))
            .count();
        frame.render_widget(
            Paragraph::new(vec![
                title,
                Line::from(vec![
                    Span::styled(" AGENT DETAIL ", Style::default().fg(ACCENT).bold()),
                    Span::raw(format!(
                        "{} · {} {} · {children} children · follow {}{}{}",
                        trail.join(" › "),
                        session.provider.label(),
                        session.status(now),
                        if app.follow { "ON" } else { "OFF" },
                        if app.tools_only { " · tools" } else { "" },
                        if app.errors_only { " · errors" } else { "" }
                    )),
                ]),
                Line::styled(
                    format!(
                        " Task: {}",
                        if session.title.is_empty() {
                            "No task recorded"
                        } else {
                            &session.title
                        }
                    ),
                    Style::default().fg(MUTED),
                ),
            ]),
            area,
        );
        return;
    }
    let scope = Line::from(vec![
        Span::raw(format!(
            " {provider}  │  {}  │  {}  │  {}",
            if app.active_only {
                "recent activity"
            } else {
                "all sessions"
            },
            if app.subtree {
                "COMBINED events"
            } else {
                "selected agent"
            },
            if app.follow {
                "follow ON"
            } else {
                "follow OFF"
            }
        )),
        Span::styled(
            format!(
                "{}{}",
                if app.tools_only { "  tools" } else { "" },
                if app.errors_only { "  errors" } else { "" }
            ),
            Style::default().fg(Color::Yellow),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(vec![
            title,
            scope,
            Line::styled(
                " d Dashboard · v Parallel · s Agent detail · Status is transcript evidence.  ? help",
                Style::default().fg(MUTED),
            ),
        ]),
        area,
    );
}

fn dashboard(frame: &mut Frame, app: &mut App, area: Rect) {
    let note_height = if area.height >= 16 { 3 } else { 1 };
    let parts = Layout::vertical([Constraint::Min(4), Constraint::Length(note_height)]).split(area);
    if area.width >= 125 {
        let cols = Layout::horizontal([Constraint::Percentage(56), Constraint::Percentage(44)])
            .split(parts[0]);
        dashboard_sessions(frame, app, cols[0]);
        dashboard_turns(frame, app, cols[1]);
    } else if area.width >= 95 && area.height >= 20 {
        let rows = Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(parts[0]);
        dashboard_sessions(frame, app, rows[0]);
        dashboard_turns(frame, app, rows[1]);
    } else if app.dashboard.focus_turns {
        dashboard_turns(frame, app, parts[0]);
    } else {
        dashboard_sessions(frame, app, parts[0]);
    }
    let selected = app
        .dashboard
        .selected()
        .map(|r| &app.snapshot.sessions[r.session]);
    let notes = vec![
        Line::styled(
            if area.width < 65 {
                " * Partial · — Unknown · Tab panels"
            } else {
                " * Partial history / timing gaps. — Unknown. Tab: sessions ↔ turns"
            },
            Style::default().fg(MUTED),
        ),
        Line::raw(
            " Total = ended turns, including interruptions. Open turns excluded. Tools are call → result.",
        ),
        Line::styled(
            selected
                .map(|s| {
                    format!(
                        " {} · {} retained / {} evicted events",
                        s.cwd,
                        s.events.len(),
                        s.dropped
                    )
                })
                .unwrap_or_default(),
            Style::default().fg(MUTED),
        ),
    ];
    frame.render_widget(Paragraph::new(notes), parts[1]);
}

fn dashboard_sessions(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = format!(
        " 1 SESSIONS {} · {}{} ",
        app.dashboard.rows.len(),
        app.dashboard.sort.label(),
        if app.agent_query.is_empty() {
            String::new()
        } else {
            format!(" · /{}", app.agent_query)
        }
    );
    let border = block(title, !app.dashboard.focus_turns);
    if app.dashboard.rows.is_empty() {
        let message = if app.snapshot.scanned_at == chrono::DateTime::UNIX_EPOCH {
            "Reading local transcripts…"
        } else if app.snapshot.sessions.is_empty() {
            "No local sessions found. Try --demo or check log paths with ?."
        } else {
            "No sessions match. w: 24h / 7d / All; Esc: clear search; p: provider; a: activity."
        };
        frame.render_widget(
            Paragraph::new(message)
                .wrap(Wrap { trim: false })
                .block(border),
            area,
        );
        return;
    }
    let items: Vec<_> = app
        .dashboard
        .rows
        .iter()
        .map(|row| {
            let session = &app.snapshot.sessions[row.session];
            let t = &row.timing;
            let status = session.status(app.snapshot.scanned_at);
            let longest = t.longest_turn.and_then(|i| t.turns[i].duration_ms);
            let tool = t.longest_tool.as_ref();
            let identity = Line::from(vec![
                Span::styled(
                    if t.partial { "* " } else { "" },
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    if session.parent.is_none() && !session.title.is_empty() {
                        one_line(&session.title, 70)
                    } else {
                        session.label()
                    },
                    Style::default().bold(),
                ),
                Span::styled(
                    format!(
                        "  {} {}",
                        session.provider.label(),
                        if session.parent.is_some() {
                            "Sub"
                        } else {
                            "Main"
                        }
                    ),
                    Style::default().fg(MUTED),
                ),
                Span::styled(
                    format!("  {status}"),
                    Style::default().fg(if t.partial {
                        Color::Yellow
                    } else {
                        status_color(status)
                    }),
                ),
            ]);
            let metrics = if area.width >= 65 {
                format!(
                    "Total {:>8}   Turn {:>8}   Tool {:>8}",
                    format_duration(t.total_ms),
                    format_duration(longest),
                    format_duration(tool.map(|t| t.duration_ms))
                )
            } else {
                format!(
                    "Total {} · Turn {}",
                    format_duration(t.total_ms),
                    format_duration(longest)
                )
            };
            let tail = format!(
                "{}{}",
                tool.map(|t| format!("{} {}", t.name, format_duration(Some(t.duration_ms))))
                    .unwrap_or_else(|| "No timed tool call".into()),
                if session.turn_open {
                    format!(" · Open {}", format_duration(t.open_ms))
                } else {
                    String::new()
                }
            );
            let last_activity = if session.last_activity == DateTime::UNIX_EPOCH {
                "Last activity unknown".into()
            } else {
                format!(
                    "Last {} ago",
                    activity_age(session.last_activity, app.snapshot.scanned_at)
                )
            };
            ListItem::new(vec![
                identity,
                Line::raw(metrics),
                Line::styled(tail, Style::default().fg(MUTED)),
                Line::styled(last_activity, Style::default().fg(MUTED)),
            ])
        })
        .collect();
    frame.render_stateful_widget(
        List::new(items)
            .block(border)
            .highlight_symbol("▸ ")
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        area,
        &mut app.dashboard.sessions,
    );
}

fn dashboard_turns(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = format!(
        " 2 TURNS {} · ended by duration ",
        app.dashboard.turn_order.len()
    );
    let border = block(title, app.dashboard.focus_turns);
    let Some(row) = app.dashboard.selected() else {
        frame.render_widget(
            Paragraph::new("Select a session to inspect its turns.").block(border),
            area,
        );
        return;
    };
    if row.timing.turns.is_empty() {
        frame.render_widget(Paragraph::new("No retained turn boundaries. Use x from Sessions to open the slowest timed tool, or d to explore Flow.").wrap(Wrap { trim: false }).block(border), area);
        return;
    }
    let items: Vec<_> = app
        .dashboard
        .turn_order
        .iter()
        .map(|i| {
            let turn = &row.timing.turns[*i];
            let (status, shade) = if turn.interrupted {
                ("INTERRUPTED", Color::Red)
            } else if turn.open {
                ("OPEN", Color::Yellow)
            } else if turn.ended {
                ("ENDED", Color::Green)
            } else {
                ("MISSING END", Color::Yellow)
            };
            let duration = if turn.open {
                row.timing.open_ms
            } else {
                turn.duration_ms
            };
            let source = if turn.open {
                "age"
            } else if turn.duration_ms.is_none() {
                "unknown"
            } else if turn.reported {
                "reported"
            } else {
                "timestamps"
            };
            let tool = turn
                .longest_tool
                .as_ref()
                .map(|t| format!("x {} {}", t.name, format_duration(Some(t.duration_ms))))
                .unwrap_or_else(|| "No timed tool call in this turn".into());
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(
                        format!("#{:02}  {}  ", i + 1, format_duration(duration)),
                        Style::default().bold(),
                    ),
                    Span::styled(status, Style::default().fg(shade)),
                    Span::styled(format!(" · {source}"), Style::default().fg(MUTED)),
                ]),
                Line::raw(turn.label().to_owned()),
                Line::styled(tool, Style::default().fg(MUTED)),
                Line::raw(""),
            ])
        })
        .collect();
    frame.render_stateful_widget(
        List::new(items)
            .block(border)
            .highlight_symbol("▸ ")
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        area,
        &mut app.dashboard.turns,
    );
}

fn agents(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = format!(
        " 1 AGENTS {}/{} · z fold {} ",
        app.agents.len(),
        app.snapshot.discovered,
        if app.agent_query.is_empty() {
            String::new()
        } else {
            format!("/{}", app.agent_query)
        }
    );
    let b = block(title, app.pane == Pane::Agents);
    if app.agents.is_empty() {
        let text = if app.snapshot.scanned_at == chrono::DateTime::UNIX_EPOCH {
            "Discovering local sessions…\n\nClaude Code + Codex\nNo configuration changes needed."
        } else if app.snapshot.sessions.is_empty() {
            "No session transcripts found.\n\nStart Claude Code or Codex, or set --claude-dir / --codex-dir.\n\nTry --demo to explore the UI.\n\n? shows source diagnostics."
        } else {
            "No agents match the filter.\n\nEsc clears search\na toggles recent activity\np cycles providers"
        };
        frame.render_widget(
            Paragraph::new(text).wrap(Wrap { trim: false }).block(b),
            area,
        );
        return;
    }
    let now = Utc::now();
    let items: Vec<ListItem> = app
        .agents
        .iter()
        .map(|row| {
            let s = &app.snapshot.sessions[row.index];
            let status = s.status(now);
            let working = row.working_descendants(&app.snapshot, now);
            let activity_style = Style::default().fg(status_color("WORKING")).bold();
            let indent = if row.depth > 0 {
                format!("{}└─", "│ ".repeat(row.depth.min(8) - 1))
            } else {
                String::new()
            };
            let branch = if !row.descendants.is_empty() {
                format!(
                    "{indent}{} {} sub · ",
                    if row.expanded { "▾" } else { "▸" },
                    row.descendants.len()
                )
            } else if row.depth > 0 {
                format!("{indent} ")
            } else {
                "· ".into()
            };
            let label = if row.depth == 0 && !s.title.is_empty() {
                one_line(&s.title, 70)
            } else {
                s.label()
            };
            let age = activity_age(s.last_activity, now);
            let event_count = compact_count(s.events.len() as u64);
            let (indent, gap) = if area.width >= 38 {
                ("  ", "  ")
            } else {
                ("", " ")
            };
            let role = if s.parent.is_some() { "Sub" } else { "Main" };
            let context_color = match s.context.free_percent() {
                Some(p) if p <= 10.0 => Color::Red,
                Some(p) if p <= 25.0 => Color::Yellow,
                _ => ACCENT,
            };
            let mut activity = vec![Span::raw("  ")];
            if working > 0 {
                activity.push(Span::styled(
                    format!("● {working} sub working · "),
                    activity_style,
                ));
            }
            activity.push(Span::styled(
                format!(
                    "{event_count} ev · {}",
                    if row.depth > 0 {
                        one_line(&s.title, 80)
                    } else {
                        s.cwd.rsplit('/').next().unwrap_or("").to_owned()
                    }
                ),
                Style::default().fg(MUTED),
            ));
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(
                        branch,
                        if working > 0 {
                            activity_style
                        } else {
                            Style::default().fg(MUTED)
                        },
                    ),
                    Span::styled(
                        label,
                        if working > 0 {
                            activity_style
                        } else {
                            Style::default()
                        },
                    ),
                ]),
                Line::from(vec![
                    Span::raw(format!("{indent}{} ", s.provider.label())),
                    Span::styled(status, Style::default().fg(status_color(status))),
                    Span::styled(format!("{gap}{age}"), Style::default().fg(MUTED)),
                ]),
                Line::styled(
                    format!("{indent}{role} {}", context_summary(s.context)),
                    Style::default().fg(context_color),
                ),
                Line::from(activity),
            ])
        })
        .collect();
    frame.render_stateful_widget(
        List::new(items)
            .block(b)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("› "),
        area,
        &mut app.agent_state,
    );
}

fn flow(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = format!(
        " 2 FLOW · {} · {} events {} ",
        if app.subtree {
            "combined".to_owned()
        } else {
            app.selected_session()
                .map(Session::label)
                .unwrap_or_default()
        },
        app.flow.len(),
        if app.event_query.is_empty() {
            String::new()
        } else {
            format!("/{}", app.event_query)
        }
    );
    let b = block(title, app.pane == Pane::Flow);
    if app.flow.is_empty() {
        frame.render_widget(Paragraph::new("Select an agent to explore its flow.\n\nNew events appear automatically.\nUse t / e to clear tool or error filters.").wrap(Wrap{trim:false}).block(b),area);
        return;
    }
    // Only format visible rows: a large family may retain thousands of events.
    let count = usize::from(area.height.saturating_sub(2) / 3).max(1);
    let selected = app
        .flow_state
        .selected()
        .unwrap_or(0)
        .min(app.flow.len() - 1);
    let mut offset = app.flow_state.offset().min(selected);
    if selected >= offset + count {
        offset = selected + 1 - count;
    }
    offset = offset.min(app.flow.len().saturating_sub(count));
    *app.flow_state.offset_mut() = offset;
    let end = (offset + count).min(app.flow.len());
    let items: Vec<ListItem> = app.flow[offset..end]
        .iter()
        .map(|(si, ei)| {
            event_row(
                &app.snapshot,
                &app.snapshot.sessions[*si],
                &app.snapshot.sessions[*si].events[*ei],
                app.subtree,
            )
        })
        .collect();
    let mut state = ratatui::widgets::ListState::default().with_selected(Some(selected - offset));
    frame.render_stateful_widget(
        List::new(items)
            .block(b)
            .highlight_style(Style::default().reversed())
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}

fn event_row(
    snapshot: &Snapshot,
    session: &Session,
    e: &FlowEvent,
    show_actor: bool,
) -> ListItem<'static> {
    let icon = match e.kind {
        Kind::User => "◆",
        Kind::Assistant => "◇",
        Kind::Tool => "├",
        Kind::Spawn => "╞",
        Kind::Message => "↔",
        Kind::Turn => "●",
        Kind::Notice => "!",
    };
    let name = if matches!(e.kind, Kind::Spawn | Kind::Message) && !e.name.contains('→') {
        let target = e.target.as_ref().map(|target| {
            crate::source::resolve_target(snapshot, session, target)
                .and_then(|key| {
                    snapshot
                        .sessions
                        .iter()
                        .find(|s| s.key == key)
                        .map(|s| s.label())
                })
                .unwrap_or_else(|| target.clone())
        });
        target
            .map(|target| format!("{} → {target}", e.name))
            .unwrap_or_else(|| e.name.clone())
    } else {
        e.name.clone()
    };
    let summary = if matches!(e.kind, Kind::Spawn | Kind::Message) {
        // The instruction explains the handoff; a generic delivery receipt does not.
        serde_json::from_str::<serde_json::Value>(&e.input)
            .ok()
            .and_then(|value| {
                ["message", "prompt", "content", "description"]
                    .iter()
                    .find_map(|key| {
                        value
                            .get(key)
                            .and_then(|v| v.as_str())
                            .map(|s| one_line(s, 180))
                    })
            })
            .unwrap_or_else(|| one_line(&e.input, 180))
    } else {
        e.summary()
    };
    let time = if e.time == chrono::DateTime::UNIX_EPOCH {
        "--:--:--".into()
    } else {
        e.time.with_timezone(&Local).format("%H:%M:%S").to_string()
    };
    ListItem::new(vec![
        Line::from(vec![
            Span::styled(format!("{time}  "), Style::default().fg(MUTED)),
            Span::styled(
                if show_actor {
                    one_line(&session.label(), 32)
                } else {
                    e.completed_at
                        .and_then(|end| elapsed(e.time, end))
                        .map(|ms| format_duration(Some(ms)))
                        .unwrap_or_default()
                },
                Style::default().fg(if show_actor { ACCENT } else { MUTED }),
            ),
        ]),
        Line::from(vec![
            Span::styled(format!("{icon} "), Style::default().fg(color(e.outcome))),
            Span::raw(name),
            Span::styled(
                format!("  {}", e.outcome.label()),
                Style::default().fg(color(e.outcome)),
            ),
        ]),
        Line::styled(format!("│ {summary}"), Style::default().fg(MUTED)),
    ])
}

fn parallel(frame: &mut Frame, app: &mut App, area: Rect) {
    let count = app.parallel.lanes.len();
    if count == 0 {
        frame.render_widget(
            Paragraph::new("No retained agents in this family. v returns to agent detail.")
                .wrap(Wrap { trim: false })
                .block(block(" PARALLEL ".into(), true)),
            area,
        );
        return;
    }
    let visible = (usize::from(area.width) / 44).clamp(1, 3).min(count);
    let start = (app.parallel.focus / visible * visible).min(count.saturating_sub(visible));
    let cols = Layout::horizontal(vec![Constraint::Ratio(1, visible as u32); visible]).split(area);
    for (column, i) in (start..start + visible).enumerate() {
        let lane = &mut app.parallel.lanes[i];
        let session = &app.snapshot.sessions[lane.session];
        let focused = i == app.parallel.focus;
        let border = block(
            format!(
                " {} {} · {} ",
                i + 1,
                session.label(),
                if session.parent.is_some() {
                    "Sub"
                } else {
                    "Main"
                }
            ),
            focused,
        );
        let inner = border.inner(cols[column]);
        frame.render_widget(border, cols[column]);
        let sections = Layout::vertical([
            Constraint::Length(if inner.height >= 10 { 4 } else { 2 }),
            Constraint::Min(3),
        ])
        .split(inner);
        let parent = session
            .parent
            .as_ref()
            .map(|key| {
                app.snapshot
                    .sessions
                    .iter()
                    .find(|s| &s.key == key)
                    .map(|s| s.label())
                    .unwrap_or_else(|| "not loaded".into())
            })
            .unwrap_or_else(|| "root".into());
        let status = session.status(app.snapshot.scanned_at);
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    if session.title.is_empty() {
                        "No task recorded".to_owned()
                    } else {
                        one_line(&session.title, 100)
                    },
                    Style::default().bold(),
                ),
                Line::from(vec![
                    Span::styled(
                        format!("{} {status}", session.provider.label()),
                        Style::default().fg(status_color(status)),
                    ),
                    Span::raw(format!(
                        " · {}/{} ev · {}",
                        lane.events.len(),
                        session.events.len(),
                        if lane.follow { "follow" } else { "hold" }
                    )),
                ]),
                Line::styled(
                    format!(
                        "{}{}",
                        if session.parent.is_some() {
                            format!("Parent: {parent}")
                        } else {
                            "Root agent".into()
                        },
                        if session.dropped > 0 {
                            " · partial history"
                        } else {
                            ""
                        }
                    ),
                    Style::default().fg(MUTED),
                ),
                Line::styled(context_summary(session.context), Style::default().fg(MUTED)),
            ]),
            sections[0],
        );
        if lane.events.is_empty() {
            frame.render_widget(
                Paragraph::new(
                    "No matching events.\nClear /, t or e filters.\nEnter opens agent detail.",
                )
                .wrap(Wrap { trim: false }),
                sections[1],
            );
            continue;
        }
        let rows = usize::from(sections[1].height / 3).max(1);
        let selected = lane
            .state
            .selected()
            .unwrap_or(0)
            .min(lane.events.len() - 1);
        let mut offset = lane.state.offset().min(selected);
        if selected >= offset + rows {
            offset = selected + 1 - rows;
        }
        offset = offset.min(lane.events.len().saturating_sub(rows));
        *lane.state.offset_mut() = offset;
        let items: Vec<_> = lane.events[offset..(offset + rows).min(lane.events.len())]
            .iter()
            .map(|ei| event_row(&app.snapshot, session, &session.events[*ei], false))
            .collect();
        let mut state =
            ratatui::widgets::ListState::default().with_selected(Some(selected - offset));
        frame.render_stateful_widget(
            List::new(items)
                .highlight_symbol(if focused { "› " } else { "  " })
                .highlight_style(if focused {
                    Style::default().reversed()
                } else {
                    Style::default()
                }),
            sections[1],
            &mut state,
        );
    }
}

fn detail(frame: &mut Frame, app: &mut App, area: Rect) {
    let mut lines = Vec::new();
    if let Some((s, e)) = app.selected_event() {
        lines.push(Line::styled(
            e.name.clone(),
            Style::default().bold().fg(ACCENT),
        ));
        lines.push(Line::from(vec![
            Span::raw(format!("{} · {} · ", s.provider.label(), s.label())),
            Span::styled(e.outcome.label(), Style::default().fg(color(e.outcome))),
        ]));
        lines.push(Line::styled(
            e.time
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S%.3f %:z")
                .to_string(),
            Style::default().fg(MUTED),
        ));
        if let Some(end) = e.completed_at {
            lines.push(Line::raw(format!(
                "Call → result: {} ms",
                end.signed_duration_since(e.time).num_milliseconds().max(0)
            )));
        }
        if let Some(target) = &e.target {
            lines.push(Line::raw(format!("Linked agent: {target}  [Enter]")));
        }
        lines.extend(usage_lines(s, e));
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            if e.is_tool() || e.kind == Kind::Message {
                "INPUT"
            } else {
                "CONTENT"
            },
            Style::default().bold(),
        ));
        if e.input.is_empty() {
            lines.push(Line::styled(
                "(no input recorded)",
                Style::default().fg(MUTED),
            ));
        } else {
            lines.extend(e.input.lines().map(|s| Line::raw(s.to_owned())));
        }
        if e.is_tool() || e.output.is_some() || e.outcome == Outcome::Pending {
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "RESULT",
                Style::default().bold().fg(color(e.outcome)),
            ));
            match &e.output {
                Some(result) if result.is_empty() => lines.push(Line::raw("(empty result)")),
                Some(result) => lines.extend(result.lines().map(|s| Line::raw(s.to_owned()))),
                None => lines.push(Line::styled(
                    "No result recorded yet.",
                    Style::default().fg(Color::Yellow),
                )),
            }
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled("SOURCE", Style::default().bold()));
        lines.push(Line::raw(format!("Session: {}", s.id)));
        lines.push(Line::raw(format!("Event: {}", e.id)));
        lines.push(Line::raw(format!("Model: {}", s.model)));
        lines.push(Line::raw(format!("Workspace: {}", s.cwd)));
        lines.push(Line::raw(s.path.display().to_string()));
        if s.dropped > 0 {
            lines.push(Line::styled(
                format!("{} earlier events omitted by retention limit", s.dropped),
                Style::default().fg(Color::Yellow),
            ));
        }
        if s.inherited_skipped > 0 {
            lines.push(Line::styled(
                format!("{} inherited parent records excluded", s.inherited_skipped),
                Style::default().fg(MUTED),
            ));
        }
        if s.malformed > 0 {
            lines.push(Line::styled(
                format!("{} malformed / oversized records skipped", s.malformed),
                Style::default().fg(Color::Yellow),
            ));
        }
    } else {
        lines.push(Line::raw(
            "Select an event to inspect its input and result.",
        ));
        lines.push(Line::raw(""));
        lines.push(Line::raw(
            "Tab switches panels. Enter follows linked agents.",
        ));
    }
    let paragraph = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
    let content_height = paragraph.line_count(area.width.saturating_sub(2));
    app.detail_max_scroll = content_height
        .saturating_sub(area.height.saturating_sub(2) as usize)
        .min(u16::MAX as usize) as u16;
    app.detail_scroll = app.detail_scroll.min(app.detail_max_scroll);
    let b = block(
        format!(
            " 3 INSPECT {}/{} ",
            app.detail_scroll + 1,
            app.detail_max_scroll.saturating_add(1)
        ),
        app.pane == Pane::Detail,
    );
    frame.render_widget(paragraph.scroll((app.detail_scroll, 0)).block(b), area);
}

fn context_summary(context: ContextUsage) -> String {
    match (context.used(), context.limit_tokens, context.free_percent()) {
        (Some(used), Some(limit), Some(free)) => format!(
            "~{}/{} {:.0}% free",
            compact_count(used),
            compact_count(limit),
            free.floor()
        ),
        (Some(used), _, _) => format!("~{} / ? ctx", compact_count(used)),
        (None, Some(limit), _) => format!("— / {} ctx", compact_count(limit)),
        _ => "— ctx".into(),
    }
}

fn usage_lines(session: &Session, event: &FlowEvent) -> Vec<Line<'static>> {
    let role = if session.parent.is_some() {
        "SUBAGENT"
    } else {
        "MAIN"
    };
    let mut lines = vec![
        Line::raw(""),
        Line::styled(
            format!("CONTEXT AT EVENT · {role}"),
            Style::default().bold().fg(ACCENT),
        ),
    ];
    if let Some(context) = event.context_at_event {
        lines.push(Line::raw(context_summary(context)));
        if let Some(used) = context.used() {
            lines.push(Line::raw(format!("Used ~{used} tokens (last request)")));
        }
        if let Some(remaining) = context.remaining() {
            lines.push(Line::raw(format!("Remaining ~{remaining} tokens")));
        }
        if let Some(input) = context.input_tokens {
            lines.push(Line::raw(format!(
                "Input {input} · Output {}",
                context.output_tokens
            )));
        }
        let source = match context.limit_source {
            Some(ContextLimitSource::Recorded) => "Window: recorded in session log",
            Some(ContextLimitSource::ModelTag) => "Window: recorded model [1m] tag",
            Some(ContextLimitSource::ModelDefault) => {
                "Window: model default; settings may override"
            }
            None => "Window unknown; headroom unavailable",
        };
        lines.push(Line::styled(source, Style::default().fg(MUTED)));
        lines.push(Line::styled(
            "Own context only · excludes subagents",
            Style::default().fg(MUTED),
        ));
        lines.push(Line::styled(
            "~ estimate; excludes later messages/tool output",
            Style::default().fg(MUTED),
        ));
        if event.is_tool() {
            lines.push(Line::styled(
                "Snapshot at call start",
                Style::default().fg(MUTED),
            ));
        }
        if let Some(at) = event.context_recorded_at
            && at != chrono::DateTime::UNIX_EPOCH
        {
            lines.push(Line::styled(
                format!(
                    "Context recorded {}",
                    at.with_timezone(&Local).format("%H:%M:%S")
                ),
                Style::default().fg(MUTED),
            ));
        }
    } else {
        lines.push(Line::styled(
            "— No context usage recorded by this event",
            Style::default().fg(MUTED),
        ));
    }
    lines
}

fn footer(frame: &mut Frame, app: &App, area: Rect) {
    let line = if let Some(pane) = app.search {
        format!(
            " /{}  [Enter apply · Esc clear]",
            if pane == Pane::Agents {
                &app.agent_query
            } else {
                &app.event_query
            }
        )
    } else if app.dashboard.visible {
        if area.width < 65 {
            " d Flow  Tab panel  Enter turn  x tool".into()
        } else if area.width < 95 {
            " d Flow  Tab panel  Enter turn  x tool  o sort  w range".into()
        } else {
            " d Flow  v Parallel  Tab panels  j/k move  Enter turn  x slowest tool  o sort  w range  / search  p provider  ? help  q quit".into()
        }
    } else if app.parallel.visible {
        if area.width < 95 {
            " h/l agents  j/k events  Enter detail".into()
        } else {
            " h/l or Tab agents  j/k events  Enter agent detail  v return  / search  t/e filters  f follow lane  ? help  q quit".into()
        }
    } else if app.pane == Pane::Agents {
        if area.width < 65 {
            " z fold  Z fold all  ←/→ tree  Enter Flow".into()
        } else {
            " z fold  Z fold all  h/l or ←/→ tree  Enter Flow  Tab panels  / search  ? help  q quit"
                .into()
        }
    } else if area.width < 65 {
        " v Parallel  Tab pane  Enter inspect".into()
    } else {
        " v Parallel  d Dashboard  Tab panels  Enter inspect/jump  / search  f follow  ? help  q quit"
            .into()
    };
    let issues: usize = app.snapshot.sessions.iter().map(|s| s.malformed).sum();
    let loading = app
        .snapshot
        .sessions
        .iter()
        .filter(|s| s.bytes_read < s.file_size)
        .count();
    let status = if !app.notice.is_empty() {
        app.notice.clone()
    } else if !app.snapshot.warnings.is_empty() {
        format!(
            "{} source warning(s): {}  [? details]",
            app.snapshot.warnings.len(),
            app.snapshot.warnings[0]
        )
    } else if app.dashboard.visible && area.width < 65 {
        "w range  o sort  v Parallel  ? help  q quit".into()
    } else if app.parallel.visible && area.width < 95 {
        "v return  / search  f follow  ? help  q".into()
    } else if !app.dashboard.visible && area.width < 65 {
        "d Dashboard  / search  ? help  q quit".into()
    } else {
        format!(
            "Read-only · {} transcripts discovered · {loading} loading · {issues} skipped records · last scan {}",
            app.snapshot.discovered,
            app.snapshot
                .scanned_at
                .with_timezone(&Local)
                .format("%H:%M:%S")
        )
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(line, Style::default().fg(ACCENT)),
            Line::styled(format!(" {status}"), Style::default().fg(MUTED)),
        ]),
        area,
    );
}

fn help(frame: &mut Frame, app: &mut App, area: Rect) {
    let width = area.width.min(94);
    let height = area.height.min(35);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let mut lines = vec![
        "DASHBOARD",
        "d                          Switch Dashboard / Flow",
        "Tab / 1 2                  Sessions / turns (narrow screens: focused panel)",
        "o                          Sort by slowest turn / total / tool / open age",
        "w                          Last activity: 24 hours → 7 days → all loaded sessions",
        "Enter                      Open slowest turn, or selected turn in Turns",
        "x                          Open slowest timed tool in the focused selection",
        "Totals cover ended turns in retained events, each agent separately.",
        "Activity range filters sessions, not their retained turns. Flow is unchanged.",
        "Last activity uses recorded timestamps; unknown dates appear in All only.",
        "* means partial history or timing gaps. — means unknown, never zero.",
        "Open age is not process liveness. Unrecorded gaps are not LLM time.",
        "",
        "AGENT DETAIL & PARALLEL",
        "Agent detail shows only the selected agent's own events by default.",
        "v                          Open family lanes / return to agent detail",
        "h l / ← → / Tab            Previous / next agent lane",
        "j k / ↑ ↓                  Move within the focused lane",
        "Enter                      Open the lane's selected event in agent detail",
        "f                          Follow latest events in the focused lane only",
        "Each lane scrolls independently. Rows are not synchronized in time.",
        "Family lanes include parents and siblings, regardless of agent filters.",
        "Event search and t/e filters apply to every lane. s toggles combined Flow.",
        "",
        "NAVIGATION",
        "Tab / Shift-Tab / 1 2 3    Switch agents, flow, inspector",
        "Agents: z / Z              Fold/unfold selected branch / fold all",
        "Agents: h l / ← →          Fold or parent / unfold or first child",
        "Branches start folded; sub counts include all loaded descendants.",
        "Yellow titles + ● N sub working reveal activity inside folded branches.",
        "Sub activity includes nested agents; the parent's own status stays separate.",
        "Agent search/filter changes and linked-agent jumps reveal matching paths.",
        "j k / ↑ ↓                  Move / scroll inspector",
        "g G / Home End             First / last",
        "PageUp PageDown            Move ten rows",
        "[ ]                        Previous / next event from any panel",
        "Enter                      Open flow, inspect, or follow linked agent",
        "b / Backspace              Go to parent agent",
        "",
        "FILTERS & LIVE VIEW",
        "/                          Search current panel (input and results too)",
        "Esc                        Clear search",
        "p                          All → Codex → Claude",
        "a                          All sessions / recent working sessions",
        "s                          Selected agent / include descendants",
        "t / e                      Tools only / errors only",
        "f                          Follow latest event",
        "Space                      Pause / resume displayed snapshots",
        "r                          Rescan now",
        "q / Ctrl-C                 Quit",
        "",
        "READING THE FLOW",
        "Agents show each agent's latest context: ~used / window · percent free.",
        "~ is an estimate from the last request, not cumulative session spend.",
        "Claude windows may use model defaults; settings can override them.",
        "Inspector context is frozen at the selected event (tools: call start).",
        "╞ spawn → child   ├ tool → result   ↔ agent message   ◆ user   ◇ assistant",
        "WORKING: open turn, activity within 120s. QUIET: open turn, older activity.",
        "READY: explicit turn completion. UNKNOWN: no lifecycle evidence.",
        "RETURN means a tool returned, not that its semantic task succeeded.",
        "Pending results may reflect interrupted or incomplete logs.",
        "Internal transcript formats can change. Hidden reasoning is not shown.",
        "",
        "j/k or PgUp/PgDn scroll. Any other key closes help.",
    ]
    .into_iter()
    .map(|s| Line::raw(s.to_owned()))
    .collect::<Vec<_>>();
    if !app.snapshot.warnings.is_empty() {
        lines.push(Line::raw("SOURCE WARNINGS"));
        lines.extend(
            app.snapshot
                .warnings
                .iter()
                .take(3)
                .map(|s| Line::styled(s.clone(), Style::default().fg(Color::Yellow))),
        );
    }
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let max_scroll = paragraph
        .line_count(width.saturating_sub(2))
        .saturating_sub(height.saturating_sub(2) as usize)
        .min(u16::MAX as usize) as u16;
    app.help_scroll = app.help_scroll.min(max_scroll);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        paragraph
            .scroll((app.help_scroll, 0))
            .block(block(" AGENT FLOW / KEYMAP ".into(), true)),
        popup,
    );
}

pub fn render_text(
    app: &mut App,
    width: u16,
    height: u16,
) -> Result<String, std::convert::Infallible> {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend)?;
    terminal.draw(|f| draw(f, app))?;
    let buffer = terminal.backend().buffer();
    let mut lines = Vec::new();
    for y in 0..height {
        let mut row = String::new();
        let mut x = 0;
        while x < width {
            let cell = &buffer[(x, y)];
            row.push_str(cell.symbol());
            x += unicode_width::UnicodeWidthStr::width(cell.symbol()).max(1) as u16;
        }
        lines.push(row.trim_end().to_owned());
    }
    Ok(lines.join("\n"))
}
