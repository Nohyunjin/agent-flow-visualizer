use agent_flow::{
    app::{App, Pane},
    demo,
    model::*,
    parser::parse_line,
    source::resolve_target,
    ui,
};
use chrono::{Duration, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

fn add(s: &mut Session, v: Value, n: u64, limit: usize) {
    parse_line(s, &serde_json::to_vec(&v).unwrap(), n, limit);
}
fn key(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
}

#[test]
fn codex_correlates_interleaved_calls_and_preserves_failures() {
    let mut s = Session::new(Provider::Codex, PathBuf::from("test.jsonl"));
    for (n,v) in [
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"a","name":"exec_command","arguments":"{\"cmd\":\"cargo test\"}"}}),
        json!({"type":"response_item","payload":{"type":"custom_tool_call","call_id":"b","name":"apply_patch","input":"patch body"}}),
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"a","output":"Process exited with code 7\nfailed"}}),
        json!({"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"b","output":"Done"}}),
    ].into_iter().enumerate(){add(&mut s,v,n as u64,10);}
    assert_eq!(s.events.len(), 2);
    assert_eq!(s.events[0].outcome, Outcome::Error);
    assert!(s.events[0].input.contains("cargo test"));
    assert_eq!(s.events[1].output.as_deref(), Some("Done"));
    assert_eq!(s.events[1].outcome, Outcome::Returned);
}

#[test]
fn codex_child_ignores_replayed_parent_metadata_and_history() {
    let mut s = Session::new(Provider::Codex, PathBuf::from("child.jsonl"));
    add(
        &mut s,
        json!({"ordinal":0,"type":"session_meta","payload":{"id":"child","parent_thread_id":"parent","agent_path":"/root/worker","subagent_history_start_ordinal":5}}),
        0,
        10,
    );
    add(
        &mut s,
        json!({"ordinal":1,"type":"session_meta","payload":{"id":"parent","cwd":"/parent"}}),
        1,
        10,
    );
    add(
        &mut s,
        json!({"ordinal":2,"type":"response_item","payload":{"type":"function_call","call_id":"parent-tool","name":"exec_command","arguments":"{}"}}),
        2,
        10,
    );
    add(
        &mut s,
        json!({"ordinal":5,"type":"response_item","payload":{"type":"function_call","call_id":"child-tool","name":"exec_command","arguments":"{}"}}),
        5,
        10,
    );
    assert_eq!(s.id, "child");
    assert_eq!(s.parent.as_deref(), Some("Codex:parent"));
    assert_eq!(s.events.len(), 1);
    assert_eq!(s.events[0].id, "child-tool");
    assert_eq!(s.inherited_skipped, 1);
}

#[test]
fn claude_spawn_result_links_child_and_error_result_is_not_a_user_prompt() {
    let mut s = Session::new(Provider::Claude, PathBuf::from("parent.jsonl"));
    add(
        &mut s,
        json!({"type":"assistant","sessionId":"parent","message":{"content":[{"type":"tool_use","id":"spawn","name":"Agent","input":{"prompt":"Review"}},{"type":"tool_use","id":"read","name":"Read","input":{"file_path":"missing"}}]}}),
        0,
        10,
    );
    add(
        &mut s,
        json!({"type":"user","sessionId":"parent","toolUseResult":{"agentId":"worker"},"message":{"content":[{"type":"tool_result","tool_use_id":"spawn","content":"Started"}]}}),
        1,
        10,
    );
    add(
        &mut s,
        json!({"type":"user","sessionId":"parent","message":{"content":[{"type":"tool_result","tool_use_id":"read","content":"File missing","is_error":true}]}}),
        2,
        10,
    );
    assert_eq!(s.events.len(), 2);
    assert_eq!(s.events[0].kind, Kind::Spawn);
    assert_eq!(s.events[0].target.as_deref(), Some("parent/worker"));
    assert_eq!(s.events[1].outcome, Outcome::Error);
    assert!(s.title.is_empty());
}

#[test]
fn retention_reports_unmatched_results_without_inventing_calls() {
    let mut s = Session::new(Provider::Codex, PathBuf::from("test.jsonl"));
    for n in 0..4 {
        add(
            &mut s,
            json!({"type":"response_item","payload":{"type":"function_call","call_id":format!("call-{n}"),"name":"exec","arguments":"{}"}}),
            n,
            2,
        );
    }
    assert_eq!(s.events.len(), 2);
    assert_eq!(s.dropped, 2);
    add(
        &mut s,
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"call-0","output":"returned"}}),
        5,
        2,
    );
    assert_eq!(s.events[1].kind, Kind::Notice);
    assert_eq!(s.events[1].output.as_deref(), Some("returned"));
}

#[test]
fn states_use_turn_evidence_and_age_without_claiming_process_liveness() {
    let mut s = Session::new(Provider::Claude, PathBuf::from("test.jsonl"));
    let now = Utc::now();
    assert_eq!(s.status(now), "UNKNOWN");
    add(
        &mut s,
        json!({"type":"user","timestamp":now.to_rfc3339(),"message":{"content":"hello"}}),
        0,
        10,
    );
    assert_eq!(s.status(now), "WORKING");
    assert_eq!(s.status(now + Duration::seconds(121)), "QUIET");
    add(
        &mut s,
        json!({"type":"system","subtype":"turn_duration","timestamp":now.to_rfc3339()}),
        1,
        10,
    );
    assert_eq!(s.status(now), "READY");
}

#[test]
fn terminal_control_sequences_are_removed_and_unicode_truncation_is_safe() {
    let input = format!(
        "\u{1b}]52;c;secret\u{7}\u{1b}[31mhello\u{1b}[0m{}",
        "한".repeat(30000)
    );
    let cleaned = clean(&input);
    assert!(!cleaned.contains('\u{1b}'));
    assert!(!cleaned.contains("secret"));
    assert!(cleaned.contains("display truncated"));
    assert!(cleaned.starts_with("hello한"));
}

#[test]
fn child_target_resolution_is_scoped_to_its_root() {
    let mut snapshot = demo::snapshot();
    let mut other = Session::new(Provider::Codex, PathBuf::from("other.jsonl"));
    other.key = "Codex:other".into();
    other.id = "other".into();
    other.agent_path = "/root/reviewer".into();
    snapshot.sessions.insert(0, Arc::new(other));
    let root = &snapshot.sessions[1];
    assert_eq!(
        resolve_target(&snapshot, root, "/root/reviewer"),
        Some("Codex:review-demo".into())
    );
    Arc::make_mut(&mut snapshot.sessions[1]).agent_path.clear();
    assert_eq!(
        resolve_target(&snapshot, &snapshot.sessions[2], "/root"),
        Some("Codex:codex-demo".into())
    );
}

#[test]
fn keyboard_drills_into_spawn_and_preserves_selection_during_updates() {
    let mut app = App::new(demo::snapshot(), true);
    app.pane = Pane::Flow;
    key(&mut app, 'g');
    let spawn = app
        .flow
        .iter()
        .position(|(s, e)| app.snapshot.sessions[*s].events[*e].kind == Kind::Spawn)
        .unwrap();
    app.flow_state.select(Some(spawn));
    assert!(!app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
    assert_eq!(app.selected_key.as_deref(), Some("Codex:review-demo"));
    assert_eq!(app.flow_state.selected(), Some(app.flow.len() - 1));
    key(&mut app, 'b');
    assert_eq!(app.selected_key.as_deref(), Some("Codex:codex-demo"));
    assert_eq!(app.selected_event().unwrap().1.id, "test");
    key(&mut app, 'g');
    let id = app.selected_event().unwrap().1.id.clone();
    let mut snapshot = app.snapshot.clone();
    let root = Arc::make_mut(&mut snapshot.sessions[0]);
    add(
        root,
        json!({"type":"response_item","timestamp":Utc::now().to_rfc3339(),"payload":{"type":"message","role":"assistant","content":[{"text":"new event"}]}}),
        300,
        2000,
    );
    app.update(snapshot);
    assert_eq!(app.selected_event().unwrap().1.id, id);
    key(&mut app, 'f');
    assert_eq!(app.selected_event().unwrap().1.input, "new event");
}

#[test]
fn switching_agents_returns_to_latest_flow_without_enabling_follow() {
    for subtree in [true, false] {
        let mut app = App::new(demo::snapshot(), true);
        app.subtree = subtree;
        app.rebuild();
        key(&mut app, '2');
        key(&mut app, 'g');
        let first = app.selected_event().unwrap().1.id.clone();
        key(&mut app, '1');
        key(&mut app, 'g'); // Still on A: preserve the event being read.
        assert_eq!(app.selected_event().unwrap().1.id, first);

        key(&mut app, 'j');
        key(&mut app, 'j');
        assert_eq!(app.selected_key.as_deref(), Some("Claude:claude-demo"));
        assert_eq!(app.flow_state.selected(), Some(app.flow.len() - 1));
        key(&mut app, '2');
        key(&mut app, 'g');
        let b_event = app.selected_event().unwrap().1.id.clone();

        let mut snapshot = app.snapshot.clone();
        add(
            Arc::make_mut(&mut snapshot.sessions[0]),
            json!({"type":"response_item","timestamp":Utc::now().to_rfc3339(),"payload":{"type":"message","role":"assistant","content":[{"text":"Latest message while away"}]}}),
            301,
            2000,
        );
        app.update(snapshot);
        assert_eq!(app.selected_event().unwrap().1.id, b_event);
        app.detail_scroll = 10;
        key(&mut app, '1');
        key(&mut app, 'g');
        assert_eq!(app.selected_key.as_deref(), Some("Codex:codex-demo"));
        assert_eq!(
            app.selected_event().unwrap().1.input,
            "Latest message while away"
        );
        assert_eq!(app.detail_scroll, 0);
        assert_eq!(app.pane, Pane::Agents);
        assert!(!app.follow);
        assert!(
            ui::render_text(&mut app, 110, 35)
                .unwrap()
                .contains("Latest message while away")
        );
    }
}

#[test]
fn switching_agents_respects_flow_filters_and_empty_results() {
    let mut app = App::new(demo::snapshot(), true);
    key(&mut app, '2');
    key(&mut app, 'g');
    key(&mut app, 'e');
    key(&mut app, '1');
    key(&mut app, 'j');
    key(&mut app, 'j');
    assert_eq!(app.selected_key.as_deref(), Some("Claude:claude-demo"));
    assert!(app.selected_event().is_none());
    key(&mut app, 'g');
    assert_eq!(app.selected_event().unwrap().1.id, "check");
    assert!(app.errors_only);
    assert!(!app.follow);
}

#[test]
fn renders_all_panels_at_multiple_sizes_and_handles_empty_filters() {
    let mut app = App::new(demo::snapshot(), true);
    for (w, h) in [(180, 44), (110, 35), (80, 24), (42, 12), (20, 8)] {
        for pane in [Pane::Agents, Pane::Flow, Pane::Detail] {
            app.pane = pane;
            let text = ui::render_text(&mut app, w, h).unwrap();
            assert!(text.contains("AGENT FLOW") || text.contains("Agent Flow"));
        }
    }
    app.pane = Pane::Agents;
    app.agent_query = "no-such-agent".into();
    app.rebuild();
    assert!(
        ui::render_text(&mut app, 160, 40)
            .unwrap()
            .contains("No agents match")
    );
    for c in ['j', 'k', 'g', 'G', 't', 'e', 's', 'f', 'b'] {
        key(&mut app, c);
    }
    assert!(app.selected_event().is_none());
}
