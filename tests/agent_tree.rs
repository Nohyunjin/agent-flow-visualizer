use agent_flow::{
    app::{App, Pane},
    demo,
    model::Snapshot,
    ui,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::Arc;

const ROOT: &str = "Codex:codex-demo";
const REVIEWER: &str = "Codex:review-demo";
const NESTED: &str = "Codex:nested-probe";

fn key(app: &mut App, code: KeyCode) {
    assert!(!app.key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn char_key(app: &mut App, c: char) {
    key(app, KeyCode::Char(c));
}
fn explorer(snapshot: Snapshot) -> App {
    let mut app = App::new(snapshot, true);
    app.dashboard.visible = false;
    app
}
fn add_child(snapshot: &mut Snapshot, name: &str, parent: &str) {
    let mut child = snapshot.sessions[1].as_ref().clone();
    child.key = format!("Codex:{name}");
    child.id = name.into();
    child.title = name.into();
    child.agent_path = format!("/root/{name}");
    child.parent = Some(parent.into());
    snapshot.sessions.push(Arc::new(child));
    snapshot.discovered = snapshot.sessions.len();
}
fn visible_keys(app: &App) -> Vec<&str> {
    app.agents
        .iter()
        .map(|row| app.snapshot.sessions[row.index].key.as_str())
        .collect()
}

#[test]
fn large_families_start_folded_and_show_counts_before_long_titles() {
    let mut snapshot = demo::snapshot();
    for i in 0..120 {
        add_child(&mut snapshot, &format!("worker-{i}"), ROOT);
    }
    Arc::make_mut(&mut snapshot.sessions[0]).title = "A very long main task ".repeat(20);
    let mut app = explorer(snapshot);
    assert_eq!(visible_keys(&app), vec![ROOT, "Claude:claude-demo"]);
    assert_eq!(app.agents[0].descendants, 122);
    assert!(!app.agents[0].expanded);
    for (width, height) in [(160, 42), (80, 24), (42, 12)] {
        let text = ui::render_text(&mut app, width, height).unwrap();
        assert!(text.contains("▸ 122 sub"));
        assert!(text.contains("z fold"));
    }
    char_key(&mut app, 'j');
    assert_eq!(app.selected_key.as_deref(), Some("Claude:claude-demo"));
}

#[test]
fn nested_branches_expand_independently_and_arrows_navigate_the_tree() {
    let mut snapshot = demo::snapshot();
    add_child(&mut snapshot, "nested-probe", REVIEWER);
    let mut app = explorer(snapshot);
    key(&mut app, KeyCode::Right);
    assert_eq!(app.selected_key.as_deref(), Some(ROOT));
    assert!(visible_keys(&app).contains(&REVIEWER));
    assert!(!visible_keys(&app).contains(&NESTED));
    key(&mut app, KeyCode::Right);
    assert_eq!(app.selected_key.as_deref(), Some(REVIEWER));
    char_key(&mut app, 'z');
    assert!(visible_keys(&app).contains(&NESTED));
    char_key(&mut app, 'l');
    assert_eq!(app.selected_key.as_deref(), Some(NESTED));
    char_key(&mut app, 'z'); // Leaf toggle is harmless.
    char_key(&mut app, 'h');
    assert_eq!(app.selected_key.as_deref(), Some(REVIEWER));
    key(&mut app, KeyCode::Left);
    assert!(!visible_keys(&app).contains(&NESTED));
    key(&mut app, KeyCode::Left);
    assert_eq!(app.selected_key.as_deref(), Some(ROOT));
    key(&mut app, KeyCode::Left);
    assert_eq!(app.agents.len(), 2);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.pane, Pane::Flow);
    key(&mut app, KeyCode::Right);
    assert_eq!(app.pane, Pane::Detail);
}

#[test]
fn live_updates_keep_expansion_and_manual_event_selection_by_identity() {
    let mut app = explorer(demo::snapshot());
    char_key(&mut app, '2');
    char_key(&mut app, 'g');
    let event = app.selected_event().unwrap().1.id.clone();
    char_key(&mut app, '1');
    char_key(&mut app, 'z');
    let mut snapshot = app.snapshot.clone();
    add_child(&mut snapshot, "new-worker", ROOT);
    snapshot.sessions.reverse();
    app.update(snapshot);
    assert!(visible_keys(&app).contains(&"Codex:new-worker"));
    assert_eq!(app.selected_key.as_deref(), Some(ROOT));
    assert_eq!(app.selected_event().unwrap().1.id, event);
    assert!(!app.follow);

    char_key(&mut app, 'z');
    let mut snapshot = demo::snapshot();
    add_child(&mut snapshot, "another-worker", ROOT);
    app.update(snapshot);
    assert_eq!(visible_keys(&app), vec![ROOT, "Claude:claude-demo"]);
    assert_eq!(app.agents[0].descendants, 3);
    assert_eq!(app.selected_event().unwrap().1.id, event);
    assert!(!app.follow);
}

#[test]
fn fold_all_preserves_same_agent_event_or_switches_to_latest_root_event() {
    let mut snapshot = demo::snapshot();
    add_child(&mut snapshot, "nested-probe", REVIEWER);
    let mut app = explorer(snapshot);
    char_key(&mut app, '2');
    char_key(&mut app, 'g');
    let first = app.selected_event().unwrap().1.id.clone();
    char_key(&mut app, '1');
    char_key(&mut app, 'z');
    char_key(&mut app, 'Z');
    assert_eq!(app.selected_event().unwrap().1.id, first);
    app.jump_to(NESTED);
    assert!(visible_keys(&app).contains(&NESTED));
    char_key(&mut app, 'g');
    char_key(&mut app, '1');
    app.detail_scroll = 10;
    char_key(&mut app, 'Z');
    assert_eq!(app.selected_key.as_deref(), Some(ROOT));
    assert_eq!(app.flow_state.selected(), Some(app.flow.len() - 1));
    assert_eq!(app.agents.len(), 2);
    assert_eq!(app.detail_scroll, 0);
    assert!(!app.follow);
    app.update(app.snapshot.clone());
    assert_eq!(app.agents.len(), 2);
}

#[test]
fn search_and_linked_navigation_reveal_paths_but_refresh_respects_manual_folding() {
    let mut snapshot = demo::snapshot();
    add_child(&mut snapshot, "nested-probe", REVIEWER);
    let mut app = explorer(snapshot);
    app.agent_query = "nested-probe".into();
    app.rebuild();
    assert_eq!(visible_keys(&app), vec![ROOT, REVIEWER, NESTED]);
    char_key(&mut app, 'z');
    app.update(app.snapshot.clone());
    assert_eq!(visible_keys(&app), vec![ROOT]);
    app.jump_to(NESTED);
    assert!(app.agent_query.is_empty());
    assert!(visible_keys(&app).contains(&NESTED));
    assert_eq!(app.selected_key.as_deref(), Some(NESTED));
    assert_eq!(app.flow_state.selected(), Some(app.flow.len() - 1));
}

#[test]
fn folding_changes_only_tree_visibility_not_combined_flow_or_parallel_lanes() {
    let mut app = explorer(demo::snapshot());
    char_key(&mut app, 's');
    let combined = app.flow.clone();
    assert!(
        combined
            .iter()
            .any(|(si, _)| app.snapshot.sessions[*si].key == REVIEWER)
    );
    char_key(&mut app, 'z');
    char_key(&mut app, 'z');
    assert_eq!(app.flow, combined);
    assert!(app.follow);
    char_key(&mut app, 'v');
    assert_eq!(app.parallel.lanes.len(), 3);
    char_key(&mut app, 'l');
    char_key(&mut app, 'g');
    char_key(&mut app, 'j');
    let target = app.parallel.target().unwrap();
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.selected_key.as_deref(), Some(REVIEWER));
    assert!(visible_keys(&app).contains(&REVIEWER));
    assert_eq!(
        app.selected_event().map(|(_, event)| event.id.clone()),
        target.1
    );
}
