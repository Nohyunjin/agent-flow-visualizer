use agent_flow::{
    app::{App, Pane},
    demo,
    model::*,
    parser::parse_line,
    ui,
};
use chrono::{Duration, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use std::sync::Arc;

fn key(app: &mut App, code: KeyCode) {
    assert!(!app.key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn explorer() -> App {
    let mut app = App::new(demo::snapshot(), true);
    app.dashboard.visible = false;
    app
}
fn current(app: &App) -> (String, Option<String>) {
    app.parallel.target().unwrap()
}
fn append(snapshot: &mut Snapshot, key: &str, id: &str) {
    let session = snapshot.sessions.iter_mut().find(|s| s.key == key).unwrap();
    parse_line(
        Arc::make_mut(session),
        &serde_json::to_vec(&json!({
            "type":"response_item", "timestamp":(Utc::now() + Duration::seconds(1)).to_rfc3339(),
            "payload":{"id":id,"type":"message","role":"assistant","content":[{"text":"New event"}]}
        }))
        .unwrap(),
        900,
        2000,
    );
}

#[test]
fn agent_detail_is_own_events_and_combined_mode_remains_explicit() {
    let mut app = explorer();
    assert!(!app.subtree);
    assert!(
        app.flow
            .iter()
            .all(|(si, _)| app.snapshot.sessions[*si].key == "Codex:codex-demo")
    );
    let own = app.flow.len();
    assert!(
        ui::render_text(&mut app, 160, 40)
            .unwrap()
            .contains("AGENT DETAIL")
    );
    key(&mut app, KeyCode::Char('s'));
    assert!(app.subtree);
    assert!(app.flow.len() > own);
    assert!(
        ui::render_text(&mut app, 160, 40)
            .unwrap()
            .contains("COMBINED events")
    );
    key(&mut app, KeyCode::Char('s'));
    assert_eq!(app.flow.len(), own);
}

#[test]
fn opening_a_child_shows_only_its_family_even_when_the_tree_is_filtered() {
    let mut app = explorer();
    app.agent_query = "reviewer".into();
    app.rebuild();
    app.jump_to("Codex:review-demo");
    app.agent_query = "reviewer".into();
    app.rebuild();
    key(&mut app, KeyCode::Char('v'));
    assert_eq!(app.parallel.root.as_deref(), Some("Codex:codex-demo"));
    assert_eq!(current(&app).0, "Codex:review-demo");
    assert_eq!(
        app.parallel
            .lanes
            .iter()
            .map(|l| l.key.as_str())
            .collect::<Vec<_>>(),
        vec!["Codex:codex-demo", "Codex:review-demo", "Codex:tests-demo"]
    );
    for lane in &app.parallel.lanes {
        assert!(lane.events.windows(2).all(|pair| {
            app.snapshot.sessions[lane.session].events[pair[0]].time
                <= app.snapshot.sessions[lane.session].events[pair[1]].time
        }));
    }
    key(&mut app, KeyCode::Char('l'));
    let selected = current(&app);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.selected_key.as_deref(), Some("Codex:tests-demo"));
    assert_eq!(app.selected_event().map(|(_, e)| e.id.clone()), selected.1);
    assert!(!app.subtree);
    assert!(!app.follow);
    assert!(app.agent_query.is_empty());
}

#[test]
fn lanes_keep_independent_selection_and_follow_state_across_live_reordering() {
    let mut app = explorer();
    app.toggle_parallel();
    key(&mut app, KeyCode::Char('g'));
    let root = current(&app);
    key(&mut app, KeyCode::Char('l'));
    key(&mut app, KeyCode::Char('g'));
    key(&mut app, KeyCode::Char('j'));
    let reviewer = current(&app);
    let mut snapshot = app.snapshot.clone();
    append(&mut snapshot, "Codex:codex-demo", "root-new");
    append(&mut snapshot, "Codex:review-demo", "review-new");
    append(&mut snapshot, "Codex:tests-demo", "tests-new");
    snapshot.sessions.reverse();
    app.update(snapshot);
    assert_eq!(current(&app), reviewer);
    key(&mut app, KeyCode::Char('h'));
    assert_eq!(current(&app), root);
    key(&mut app, KeyCode::Char('l'));
    key(&mut app, KeyCode::Char('l'));
    assert_eq!(current(&app).1.as_deref(), Some("tests-new"));
    key(&mut app, KeyCode::Char('h'));
    key(&mut app, KeyCode::Char('f'));
    assert_eq!(current(&app).1.as_deref(), Some("review-new"));
    key(&mut app, KeyCode::Char('h'));
    assert_eq!(current(&app), root);
}

#[test]
fn drill_down_keeps_exact_tool_and_its_context_while_siblings_have_errors() {
    let mut app = explorer();
    app.toggle_parallel();
    key(&mut app, KeyCode::Char('l'));
    key(&mut app, KeyCode::Char('g'));
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(current(&app).1.as_deref(), Some("check"));
    key(&mut app, KeyCode::Enter);
    assert!(!app.parallel.visible);
    assert_eq!(app.pane, Pane::Flow);
    let (session, e) = app.selected_event().unwrap();
    assert_eq!(session.key, "Codex:review-demo");
    assert_eq!(e.id, "check");
    assert_eq!(e.outcome, Outcome::Error);
    assert!(
        app.flow
            .iter()
            .all(|(si, _)| app.snapshot.sessions[*si].key == "Codex:review-demo")
    );
    app.update(app.snapshot.clone());
    assert_eq!(app.selected_event().unwrap().1.id, "check");
    app.toggle_parallel();
    assert_eq!(current(&app).1.as_deref(), Some("check"));
    key(&mut app, KeyCode::Char('3'));
    assert_eq!(app.pane, Pane::Detail);
    assert!(
        ui::render_text(&mut app, 80, 24)
            .unwrap()
            .contains("FAILED retry_limit")
    );
}

#[test]
fn empty_filters_keep_family_lanes_and_never_open_an_unrelated_event() {
    let mut app = explorer();
    app.toggle_parallel();
    key(&mut app, KeyCode::Char('/'));
    for c in "no-such-event".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    assert!(app.parallel.lanes.iter().all(|l| l.events.is_empty()));
    assert_eq!(app.parallel.lanes.len(), 3);
    key(&mut app, KeyCode::Char('l'));
    for c in ['j', 'k', 'g', 'G', 'f'] {
        key(&mut app, KeyCode::Char(c));
    }
    assert!(
        ui::render_text(&mut app, 160, 40)
            .unwrap()
            .contains("No matching events")
    );
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.selected_key.as_deref(), Some("Codex:review-demo"));
    assert!(app.selected_event().is_none());
    key(&mut app, KeyCode::Esc);
    assert!(app.selected_event().is_some());
}

#[test]
fn filters_apply_to_each_lane_and_do_not_mix_events() {
    let mut app = explorer();
    app.toggle_parallel();
    key(&mut app, KeyCode::Char('e'));
    assert!(app.parallel.lanes[0].events.is_empty());
    assert_eq!(app.parallel.lanes[1].events.len(), 1);
    assert!(app.parallel.lanes[2].events.is_empty());
    key(&mut app, KeyCode::Char('l'));
    assert_eq!(current(&app).1.as_deref(), Some("check"));
    key(&mut app, KeyCode::Char('j')); // Manual inspection holds this event while filters change.
    key(&mut app, KeyCode::Char('e'));
    assert_eq!(current(&app).1.as_deref(), Some("check"));
}

#[test]
fn missing_parents_and_cyclic_metadata_cannot_break_family_navigation() {
    let mut snapshot = demo::snapshot();
    snapshot.sessions.retain(|s| s.key != "Codex:codex-demo");
    let mut app = App::new(snapshot, true);
    app.dashboard.visible = false;
    app.jump_to("Codex:review-demo");
    app.toggle_parallel();
    assert_eq!(app.parallel.lanes.len(), 2);
    assert!(
        ui::render_text(&mut app, 160, 30)
            .unwrap()
            .contains("Parent transcript not loaded")
    );
    let mut snapshot = app.snapshot.clone();
    let first = snapshot
        .sessions
        .iter_mut()
        .find(|s| s.key == "Codex:review-demo")
        .unwrap();
    Arc::make_mut(first).parent = Some("Codex:tests-demo".into());
    let second = snapshot
        .sessions
        .iter_mut()
        .find(|s| s.key == "Codex:tests-demo")
        .unwrap();
    Arc::make_mut(second).parent = Some("Codex:review-demo".into());
    let mut app = App::new(snapshot, true);
    app.dashboard.visible = false;
    app.jump_to("Codex:review-demo");
    app.toggle_parallel();
    assert_eq!(app.parallel.lanes.len(), 2);
}

#[test]
fn disappearing_events_and_agents_leave_valid_selections() {
    let mut app = explorer();
    app.toggle_parallel();
    key(&mut app, KeyCode::Char('l'));
    key(&mut app, KeyCode::Char('g'));
    let mut snapshot = app.snapshot.clone();
    let session = snapshot
        .sessions
        .iter_mut()
        .find(|s| s.key == "Codex:review-demo")
        .unwrap();
    Arc::make_mut(session).events.pop_front();
    app.update(snapshot);
    assert_eq!(current(&app).1.as_deref(), Some("check"));
    let mut snapshot = app.snapshot.clone();
    snapshot.sessions.retain(|s| s.key != "Codex:review-demo");
    app.update(snapshot);
    assert_eq!(current(&app).0, "Codex:tests-demo");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.selected_key.as_deref(), Some("Codex:tests-demo"));
}

#[test]
fn family_view_can_open_from_dashboard_and_renders_handoffs_and_narrow_lanes() {
    let mut app = App::new(demo::snapshot(), true);
    key(&mut app, KeyCode::Char('v'));
    assert!(!app.dashboard.visible);
    assert!(app.parallel.visible);
    let wide = ui::render_text(&mut app, 160, 42).unwrap();
    assert!(wide.contains("1 root · Main"));
    assert!(wide.contains("2 reviewer · Sub"));
    assert!(wide.contains("3 tests · Sub"));
    assert!(wide.contains("Found an off-by-one"));
    assert!(wide.contains("send_message → root"));
    assert!(wide.contains("spawn_agent → reviewer"));
    assert!(wide.contains("rows are not time-aligned"));
    for (width, height) in [(110, 35), (88, 24), (80, 24), (42, 12)] {
        for _ in 0..3 {
            let text = ui::render_text(&mut app, width, height).unwrap();
            assert!(text.contains("PARALLEL"));
            assert!(text.contains("Enter detail") || text.contains("Enter agent detail"));
            key(&mut app, KeyCode::Char('l'));
        }
        app.parallel.focus = 0;
    }
    key(&mut app, KeyCode::Char('d'));
    assert!(app.dashboard.visible);
    key(&mut app, KeyCode::Char('d'));
    assert!(app.parallel.visible);
}
