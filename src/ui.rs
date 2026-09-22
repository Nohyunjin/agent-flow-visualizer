use crate::{
    app::{App, Pane},
    model::*,
};
use chrono::{Local, Utc};
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
    if area.width >= 145 {
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
    let scope = Line::from(vec![
        Span::raw(format!(
            " {provider}  │  {}  │  {}  │  {}",
            if app.active_only {
                "recent activity"
            } else {
                "all sessions"
            },
            if app.subtree {
                "agent + descendants"
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
                " Status reflects transcript events, not process liveness.  ? help",
                Style::default().fg(MUTED),
            ),
        ]),
        area,
    );
}

fn agents(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = format!(
        " 1 AGENTS {}/{} · CONTEXT {} ",
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
    let items: Vec<ListItem> = app
        .agents
        .iter()
        .map(|row| {
            let s = &app.snapshot.sessions[row.index];
            let status = s.status(Utc::now());
            let branch = if row.depth > 0 {
                format!("{}└─ ", "│ ".repeat(row.depth.min(8) - 1))
            } else {
                "▸ ".into()
            };
            let label = if row.depth == 0 && !s.title.is_empty() {
                one_line(&s.title, 70)
            } else {
                s.label()
            };
            let elapsed = Utc::now()
                .signed_duration_since(s.last_activity)
                .num_seconds()
                .max(0);
            let age = if s.last_activity == chrono::DateTime::UNIX_EPOCH {
                "?".into()
            } else if elapsed < 60 {
                format!("{elapsed}s")
            } else if elapsed < 3600 {
                format!("{}m", elapsed / 60)
            } else if elapsed < 86400 {
                format!("{}h", elapsed / 3600)
            } else {
                format!("{}d", elapsed / 86400)
            };
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
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(branch, Style::default().fg(MUTED)),
                    Span::raw(label),
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
                Line::styled(
                    format!(
                        "  {event_count} ev · {}",
                        if row.depth > 0 {
                            one_line(&s.title, 80)
                        } else {
                            s.cwd.rsplit('/').next().unwrap_or("").to_owned()
                        }
                    ),
                    Style::default().fg(MUTED),
                ),
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
        " 2 FLOW {} events {} ",
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
            let session = &app.snapshot.sessions[*si];
            let e = &session.events[*ei];
            let icon = match e.kind {
                Kind::User => "◆",
                Kind::Assistant => "◇",
                Kind::Tool => "├",
                Kind::Spawn => "╞",
                Kind::Message => "↔",
                Kind::Turn => "●",
                Kind::Notice => "!",
            };
            let name = match e.kind {
                Kind::Spawn => format!("{} → {}", e.name, e.target.as_deref().unwrap_or("agent")),
                _ => e.name.clone(),
            };
            let time = if e.time == chrono::DateTime::UNIX_EPOCH {
                "--:--:--".into()
            } else {
                e.time.with_timezone(&Local).format("%H:%M:%S").to_string()
            };
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(format!("{time}  "), Style::default().fg(MUTED)),
                    Span::styled(one_line(&session.label(), 32), Style::default().fg(ACCENT)),
                ]),
                Line::from(vec![
                    Span::styled(format!("{icon} "), Style::default().fg(color(e.outcome))),
                    Span::raw(name),
                    Span::styled(
                        format!("  {}", e.outcome.label()),
                        Style::default().fg(color(e.outcome)),
                    ),
                ]),
                Line::styled(format!("│ {}", e.summary()), Style::default().fg(MUTED)),
            ])
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
    } else {
        " Tab panels  j/k move  Enter inspect/jump  / search  f follow  Space pause  ? help  q quit"
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
        "NAVIGATION",
        "Tab / Shift-Tab / 1 2 3    Switch agents, flow, inspector",
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
