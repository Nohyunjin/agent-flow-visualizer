use agent_flow::{
    app::{App, Pane},
    dashboard::Sort,
    demo,
    model::*,
    parser::parse_line,
    timing::summarize,
    ui,
};
use chrono::{DateTime, Duration, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-09-22T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
        + Duration::seconds(seconds)
}
fn add(session: &mut Session, mut value: Value, seconds: i64, sequence: u64, limit: usize) {
    value["timestamp"] = json!(at(seconds).to_rfc3339());
    parse_line(
        session,
        &serde_json::to_vec(&value).unwrap(),
        sequence,
        limit,
    );
}
fn codex(records: &[(i64, Value)]) -> Session {
    let mut s = Session::new(Provider::Codex, PathBuf::from("fixture.jsonl"));
    for (i, (time, value)) in records.iter().enumerate() {
        add(&mut s, value.clone(), *time, i as u64, 100);
    }
    s
}
fn start() -> Value {
    json!({"type":"event_msg","payload":{"type":"task_started"}})
}
fn end() -> Value {
    json!({"type":"event_msg","payload":{"type":"task_complete"}})
}
fn call(id: &str) -> Value {
    json!({"type":"response_item","payload":{"type":"function_call","call_id":id,"name":"exec_command","arguments":"{}"}})
}
fn result(id: &str) -> Value {
    json!({"type":"response_item","payload":{"type":"function_call_output","call_id":id,"output":"done"}})
}
fn key(app: &mut App, code: KeyCode) {
    assert!(!app.key(KeyEvent::new(code, KeyModifiers::NONE)));
}

#[test]
fn timing_excludes_idle_gaps_open_turns_and_overlapping_tools_from_total() {
    let s = codex(&[
        (0, start()),
        (5, call("a")),
        (10, call("b")),
        (35, result("a")),
        (40, result("b")),
        (60, end()),
        (3600, start()),
        (3620, end()),
        (7200, start()),
        (7201, call("pending")),
    ]);
    let t = summarize(&s, at(7240));
    assert_eq!(t.total_ms, Some(80_000));
    assert_eq!(t.turns.len(), 3);
    assert_eq!(t.turns[t.longest_turn.unwrap()].duration_ms, Some(60_000));
    assert_eq!(t.longest_tool.unwrap().duration_ms, 30_000);
    assert_eq!(t.open_ms, Some(40_000));
    assert!(!t.partial);
}

#[test]
fn claude_final_response_and_duration_record_are_one_turn() {
    let mut s = Session::new(Provider::Claude, PathBuf::from("claude.jsonl"));
    let records = [
        (0, json!({"type":"user","message":{"content":"Run tests"}})),
        (
            5,
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"test","name":"Bash","input":{"command":"cargo test"}}]}}),
        ),
        (
            15,
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"test","content":"Passed"}]}}),
        ),
        (
            20,
            json!({"type":"assistant","uuid":"final","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Done"}]}}),
        ),
        (
            25,
            json!({"type":"system","subtype":"turn_duration","durationMs":21000}),
        ),
        (
            100,
            json!({"type":"user","message":{"content":"Next task"}}),
        ),
    ];
    for (i, (time, value)) in records.into_iter().enumerate() {
        add(&mut s, value, time, i as u64, 100);
    }
    let t = summarize(&s, at(105));
    assert_eq!(t.turns.len(), 2);
    assert_eq!(t.total_ms, Some(20_000)); // Final response, not the delayed duration record.
    assert_eq!(
        t.turns[0].longest_tool.as_ref().unwrap().duration_ms,
        10_000
    );
    assert_eq!(t.open_ms, Some(5000));
    assert!(!t.partial);
}

#[test]
fn retention_marks_partial_and_never_uses_first_remaining_tool_as_turn_start() {
    let mut s = codex(&[
        (0, start()),
        (5, call("test")),
        (20, result("test")),
        (30, end()),
    ]);
    s.events.pop_front();
    s.dropped = 1;
    let t = summarize(&s, at(40));
    assert!(t.partial);
    assert_eq!(t.total_ms, None);
    assert_eq!(t.longest_turn, None);
    assert_eq!(t.longest_tool.unwrap().duration_ms, 15_000);
    assert!(t.turns[0].ended);
}

#[test]
fn reported_duration_can_recover_missing_start_but_keeps_provenance() {
    let mut s = Session::new(Provider::Claude, PathBuf::from("claude.jsonl"));
    add(
        &mut s,
        json!({"type":"system","subtype":"turn_duration","durationMs":12500}),
        30,
        0,
        1,
    );
    let t = summarize(&s, at(35));
    assert_eq!(t.total_ms, Some(12_500));
    assert!(t.turns[0].reported);
    assert!(t.partial);
    assert_eq!(t.turns[0].start, None);
    // An invalid or negative reported duration cannot become a duration.
    let mut invalid = Session::new(Provider::Claude, PathBuf::from("invalid.jsonl"));
    add(
        &mut invalid,
        json!({"type":"system","subtype":"turn_duration","durationMs":-1}),
        30,
        0,
        1,
    );
    assert_eq!(summarize(&invalid, at(35)).total_ms, None);
}

#[test]
fn claude_streamed_completion_uses_final_record_time_without_rewriting_event_time() {
    let mut s = Session::new(Provider::Claude, PathBuf::from("stream.jsonl"));
    add(
        &mut s,
        json!({"type":"user","message":{"content":"Run tests"}}),
        0,
        0,
        100,
    );
    add(
        &mut s,
        json!({"type":"assistant","uuid":"response","message":{"content":[{"type":"text","text":"Testing"}]}}),
        5,
        1,
        100,
    );
    add(
        &mut s,
        json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"test","name":"Bash","input":{}}]}}),
        10,
        2,
        100,
    );
    add(
        &mut s,
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"test","content":"done"}]}}),
        15,
        3,
        100,
    );
    add(
        &mut s,
        json!({"type":"assistant","uuid":"response","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Testing complete"}]}}),
        20,
        4,
        100,
    );
    assert_eq!(s.events[1].time, at(5));
    let timing = summarize(&s, at(30));
    assert_eq!(timing.total_ms, Some(20_000));
    assert_eq!(
        timing.turns[0].longest_tool.as_ref().unwrap().duration_ms,
        5000
    );
}

#[test]
fn unknown_open_start_is_not_an_elapsed_time_and_submillisecond_reversal_is_unknown() {
    let s = codex(&[(10, call("test"))]);
    let t = summarize(&s, at(100));
    assert_eq!(t.open_ms, None);
    assert!(t.partial);
    assert_eq!(
        agent_flow::timing::elapsed(at(1), at(1) - Duration::nanoseconds(1)),
        None
    );
}

#[test]
fn unknown_or_reversed_times_are_not_zero_and_interruptions_are_ended() {
    let s = codex(&[(10, start()), (5, end())]);
    assert_eq!(summarize(&s, at(20)).total_ms, None);
    let s = codex(&[
        (0, start()),
        (
            7,
            json!({"type":"event_msg","payload":{"type":"turn_aborted"}}),
        ),
    ]);
    let t = summarize(&s, at(100));
    assert_eq!(t.total_ms, Some(7000));
    assert_eq!(t.open_ms, None);
    assert!(t.turns[0].interrupted);
    let mut s = Session::new(Provider::Codex, PathBuf::from("invalid.jsonl"));
    parse_line(&mut s, &serde_json::to_vec(&start()).unwrap(), 0, 10);
    add(&mut s, end(), 10, 1, 10);
    assert_eq!(summarize(&s, at(20)).total_ms, None);
}

#[test]
fn new_start_does_not_turn_a_missing_completion_into_hours_of_runtime() {
    let s = codex(&[
        (0, start()),
        (5, call("test")),
        (3600, start()),
        (3610, end()),
    ]);
    let t = summarize(&s, at(3700));
    assert_eq!(t.total_ms, Some(10_000));
    assert_eq!(t.turns[0].duration_ms, None);
    assert!(!t.turns[0].open);
    assert!(t.partial);
}

#[test]
fn adjacent_prompt_and_start_are_one_turn_in_either_order_and_steering_is_not_a_new_turn() {
    let prompt = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"Run tests"}]}});
    for before in [true, false] {
        let (a, b) = if before {
            (prompt.clone(), start())
        } else {
            (start(), prompt.clone())
        };
        let s = codex(&[
            (0, a),
            (1, b),
            (2, call("test")),
            (10, prompt.clone()),
            (20, end()),
        ]);
        let t = summarize(&s, at(25));
        assert_eq!(t.turns.len(), 1);
        assert_eq!(t.total_ms, Some(if before { 19_000 } else { 20_000 }));
    }
}

#[test]
fn overlapping_turn_records_are_unioned_and_children_are_separate_rows() {
    let mut parent = codex(&[(0, start()), (60, end()), (30, start()), (90, end())]);
    parent.key = "Codex:parent".into();
    let mut child = codex(&[(0, start()), (120, end())]);
    child.key = "Codex:child".into();
    child.parent = Some(parent.key.clone());
    let app = App::new(
        Snapshot {
            sessions: vec![Arc::new(parent), Arc::new(child)],
            scanned_at: at(130),
            ..Snapshot::default()
        },
        false,
    );
    assert_eq!(app.dashboard.rows[0].key, "Codex:child");
    assert_eq!(app.dashboard.rows[1].timing.total_ms, Some(90_000));
}

#[test]
fn drill_down_clears_flow_filters_selects_exact_event_and_stays_there_on_refresh() {
    let mut app = App::new(demo::snapshot(), true);
    app.tools_only = true;
    app.errors_only = true;
    app.event_query = "no match".into();
    app.rebuild();
    let target = app.dashboard.target(true).unwrap();
    key(&mut app, KeyCode::Char('x'));
    assert!(!app.dashboard.visible);
    assert!(!app.follow);
    assert_eq!(app.pane, Pane::Flow);
    assert_eq!(app.selected_event().unwrap().1.id, target.1);
    let mut snapshot = app.snapshot.clone();
    snapshot.sessions.reverse();
    app.update(snapshot);
    assert_eq!(app.selected_event().unwrap().1.id, target.1);
    key(&mut app, KeyCode::Char('d'));
    assert!(app.dashboard.visible);
    assert_eq!(app.dashboard.selected().unwrap().key, target.0);
}

#[test]
fn dashboard_keeps_selected_session_and_turn_when_live_ranking_changes() {
    let mut app = App::new(demo::snapshot(), true);
    key(&mut app, KeyCode::Char('2'));
    key(&mut app, KeyCode::Char('j'));
    let selected_session = app.dashboard.selected().unwrap().key.clone();
    let selected_turn = app.dashboard.selected_turn().unwrap().event_id.clone();
    let mut snapshot = app.snapshot.clone();
    let mut fast = codex(&[(0, start()), (900, end())]);
    fast.key = "Codex:new-long-turn".into();
    snapshot.sessions.insert(0, Arc::new(fast));
    app.update(snapshot);
    assert_eq!(app.dashboard.selected().unwrap().key, selected_session);
    assert_eq!(
        app.dashboard.selected_turn().unwrap().event_id,
        selected_turn
    );
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.selected_event().unwrap().1.id, selected_turn);
}

#[test]
fn sort_modes_filtering_and_open_turn_navigation_use_their_own_measurements() {
    let mut app = App::new(demo::snapshot(), true);
    for sort in [Sort::Total, Sort::LongestTool, Sort::Open] {
        key(&mut app, KeyCode::Char('o'));
        assert_eq!(app.dashboard.sort, sort);
    }
    let target = app.dashboard.target(false).unwrap();
    let open = app
        .dashboard
        .selected()
        .unwrap()
        .timing
        .turns
        .iter()
        .find(|t| t.open)
        .unwrap();
    assert_eq!(target.1, open.event_id);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.selected_event().unwrap().1.id, target.1);
    key(&mut app, KeyCode::Char('d'));
    key(&mut app, KeyCode::Char('p'));
    assert!(
        app.dashboard
            .rows
            .iter()
            .all(|r| app.snapshot.sessions[r.session].provider == Provider::Codex)
    );
    app.agent_query = "no-such-session".into();
    app.rebuild();
    assert!(app.dashboard.rows.is_empty());
    for c in ['j', 'k', 'x', 'o'] {
        key(&mut app, KeyCode::Char(c));
    }
    assert!(
        ui::render_text(&mut app, 110, 35)
            .unwrap()
            .contains("No sessions match")
    );
}

#[test]
fn dashboard_renders_session_and_turn_views_at_supported_terminal_sizes() {
    let mut app = App::new(demo::snapshot(), true);
    for (w, h) in [(180, 44), (125, 30), (110, 35), (80, 24), (42, 12)] {
        for focus in [false, true] {
            app.dashboard.focus_turns = focus;
            let text = ui::render_text(&mut app, w, h).unwrap();
            assert!(text.contains("DASHBOARD"));
            assert!(text.contains(if focus { "2 TURNS" } else { "1 SESSIONS" }));
            assert!(!text.contains("NaN"));
        }
    }
    let wide = ui::render_text(&mut app, 160, 42).unwrap();
    assert!(wide.contains("Fix payment retry boundary"));
    assert!(wide.contains("3m 10s"));
    assert!(wide.contains("1m 45s"));
    assert!(wide.contains("timestamps"));
}
