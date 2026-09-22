use agent_flow::{app::App, demo, model::*, parser::parse_line, ui};
use serde_json::{Value, json};
use std::path::PathBuf;

fn add(s: &mut Session, v: Value, sequence: u64, limit: usize) {
    parse_line(s, &serde_json::to_vec(&v).unwrap(), sequence, limit);
}
fn claude_usage(id: &str, output: u64) -> Value {
    json!({"type":"assistant","sessionId":"session","message":{"id":id,"usage":{
        "input_tokens":10,"cache_read_input_tokens":2000,"cache_creation_input_tokens":500,
        "cache_creation":{"ephemeral_5m_input_tokens":500},"output_tokens":output
    },"content":[{"type":"text","text":"response"}]}})
}

#[test]
fn claude_deduplicates_repeated_blocks_and_adds_streamed_output_delta() {
    let mut s = Session::new(Provider::Claude, PathBuf::from("session.jsonl"));
    assert_eq!(s.token_usage, None);
    add(&mut s, claude_usage("one", 20), 0, 10);
    add(&mut s, claude_usage("one", 20), 1, 10);
    assert_eq!(s.token_usage.unwrap().total_tokens, 2530);
    add(&mut s, claude_usage("one", 45), 2, 10);
    add(&mut s, claude_usage("one", 5), 3, 10); // An older duplicate cannot lower the total.
    assert_eq!(s.token_usage.unwrap().total_tokens, 2555);
    add(&mut s, claude_usage("two", 10), 4, 10);
    let u = s.token_usage.unwrap();
    assert_eq!(u.input_tokens, 5020);
    assert_eq!(u.output_tokens, 55);
    assert_eq!(u.cache_read_tokens, 4000);
    assert_eq!(u.cache_write_tokens, 1000);
    assert_eq!(u.total_tokens, 5075);
}

#[test]
fn usage_survives_event_retention_and_parent_does_not_include_child() {
    let mut parent = Session::new(Provider::Claude, PathBuf::from("parent.jsonl"));
    let mut child = Session::new(
        Provider::Claude,
        PathBuf::from("parent/subagents/agent-worker.jsonl"),
    );
    for i in 0..4 {
        add(&mut parent, claude_usage(&format!("msg-{i}"), 20), i, 1);
    }
    add(&mut child, claude_usage("child-msg", 50), 0, 1);
    assert_eq!(parent.events.len(), 1);
    assert_eq!(parent.token_usage.unwrap().total_tokens, 4 * 2530);
    assert_eq!(child.token_usage.unwrap().total_tokens, 2560);
    add(&mut parent, claude_usage("msg-0", 20), 5, 1);
    assert_eq!(parent.token_usage.unwrap().total_tokens, 4 * 2530);
}

#[test]
fn codex_uses_cumulative_totals_without_double_counting_updates_or_cache() {
    let mut s = Session::new(Provider::Codex, PathBuf::from("rollout.jsonl"));
    add(
        &mut s,
        json!({"type":"session_meta","payload":{"id":"s"}}),
        0,
        10,
    );
    let total = json!({"input_tokens":1000,"cached_input_tokens":800,"cache_write_input_tokens":100,"output_tokens":200,"reasoning_output_tokens":50,"total_tokens":1200});
    add(
        &mut s,
        json!({"type":"token_usage_record","payload":{"thread_id":"s","usage":total,"thread_token_usage":total}}),
        1,
        10,
    );
    add(
        &mut s,
        json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":total,"last_token_usage":total}}}),
        2,
        10,
    );
    let u = s.token_usage.unwrap();
    assert_eq!(u.total_tokens, 1200);
    assert_eq!(u.input_tokens, 1000);
    assert_eq!(u.reasoning_tokens, 50);
    add(
        &mut s,
        json!({"type":"event_msg","payload":{"type":"token_count","info":null}}),
        3,
        10,
    );
    assert_eq!(s.token_usage, Some(u));
    add(
        &mut s,
        json!({"type":"token_usage_record","payload":{"thread_id":"someone-else","thread_token_usage":{"input_tokens":999999,"output_tokens":999999}}}),
        4,
        10,
    );
    assert_eq!(s.token_usage, Some(u));
    add(
        &mut s,
        json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2000,"output_tokens":300,"total_tokens":2300}}}}),
        5,
        10,
    );
    assert_eq!(s.token_usage.unwrap().total_tokens, 2300);
}

#[test]
fn codex_child_does_not_inherit_parent_usage() {
    let mut s = Session::new(Provider::Codex, PathBuf::from("child.jsonl"));
    add(
        &mut s,
        json!({"ordinal":0,"type":"session_meta","payload":{"id":"child","parent_thread_id":"parent","subagent_history_start_ordinal":5}}),
        0,
        10,
    );
    add(
        &mut s,
        json!({"ordinal":3,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":99999,"output_tokens":99999}}}}),
        3,
        10,
    );
    assert_eq!(s.token_usage, None);
    add(
        &mut s,
        json!({"ordinal":5,"type":"token_usage_record","payload":{"thread_id":"child","thread_token_usage":{"input_tokens":100,"output_tokens":20}}}),
        5,
        10,
    );
    assert_eq!(s.token_usage.unwrap().total_tokens, 120);
}

#[test]
fn renders_session_tokens_separately_from_events_in_wide_and_narrow_panels() {
    let mut app = App::new(demo::snapshot(), true);
    let wide = ui::render_text(&mut app, 160, 42).unwrap();
    assert!(wide.contains("Main ~20.0k/200.0k 90% free"));
    assert!(wide.contains("Sub ~12.9k / ? ctx"));
    assert!(wide.contains("CONTEXT AT EVENT · MAIN"));
    assert!(wide.contains("Input 19000 · Output 1000"));
    assert!(wide.contains("6 ev"));
    let narrow = ui::render_text(&mut app, 110, 35).unwrap();
    assert!(narrow.contains("Main ~20.0k/200.0k 90% free"));
    assert_eq!(compact_count(980), "980");
    assert_eq!(compact_count(1_500), "1.5k");
    assert_eq!(compact_count(2_350_000), "2.4M");
}

#[test]
fn codex_event_snapshot_stays_fixed_after_new_usage_and_tool_result() {
    let mut s = Session::new(Provider::Codex, PathBuf::from("rollout.jsonl"));
    add(
        &mut s,
        json!({"type":"event_msg","timestamp":"2026-09-22T01:00:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":900,"output_tokens":100,"total_tokens":1000}}}}),
        0,
        10,
    );
    add(
        &mut s,
        json!({"type":"response_item","timestamp":"2026-09-22T01:00:01Z","payload":{"type":"function_call","name":"exec_command","call_id":"call","arguments":"{}"}}),
        1,
        10,
    );
    let snapshot = s.events[0].token_usage_at_event;
    let recorded_at = s.events[0].usage_recorded_at;
    assert_eq!(snapshot.unwrap().total_tokens, 1000);
    add(
        &mut s,
        json!({"type":"event_msg","timestamp":"2026-09-22T01:00:02Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":4900,"output_tokens":100,"total_tokens":5000}}}}),
        2,
        10,
    );
    add(
        &mut s,
        json!({"type":"response_item","timestamp":"2026-09-22T01:00:03Z","payload":{"type":"function_call_output","call_id":"call","output":"Done"}}),
        3,
        10,
    );
    assert_eq!(s.token_usage.unwrap().total_tokens, 5000);
    assert_eq!(s.events[0].token_usage_at_event, snapshot);
    assert_eq!(s.events[0].usage_recorded_at, recorded_at);
    assert_eq!(s.events[0].output.as_deref(), Some("Done"));
}

#[test]
fn claude_event_snapshot_is_response_usage_at_that_time_not_latest_session_usage() {
    let mut s = Session::new(Provider::Claude, PathBuf::from("session.jsonl"));
    let mut first = claude_usage("first", 20);
    first["timestamp"] = json!("2026-09-22T01:00:01Z");
    add(&mut s, first, 0, 10);
    let first_usage = s.events[0].token_usage_at_event;
    let mut next = claude_usage("next", 100);
    next["timestamp"] = json!("2026-09-22T01:00:02Z");
    add(&mut s, next, 1, 10);
    assert_eq!(s.events[0].token_usage_at_event, first_usage);
    assert_eq!(first_usage.unwrap().total_tokens, 2530);
    assert_eq!(s.events[1].token_usage_at_event.unwrap().total_tokens, 5140);
}

#[test]
fn unknown_past_usage_is_not_backfilled_and_backdated_calls_never_get_future_usage() {
    let mut s = Session::new(Provider::Codex, PathBuf::from("session.jsonl"));
    add(
        &mut s,
        json!({"type":"response_item","timestamp":"2026-09-22T01:00:00Z","payload":{"type":"message","role":"user","content":[{"text":"first"}]}}),
        0,
        10,
    );
    add(
        &mut s,
        json!({"type":"event_msg","timestamp":"2026-09-22T01:00:01Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":90,"output_tokens":10}}}}),
        1,
        10,
    );
    add(
        &mut s,
        json!({"type":"event_msg","timestamp":"2026-09-22T01:00:04Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":990,"output_tokens":10}}}}),
        2,
        10,
    );
    add(
        &mut s,
        json!({"type":"event_msg","timestamp":"2026-09-22T01:00:05Z","payload":{"type":"item_completed","item":{"type":"CommandExecution","id":"backdated","command":"test","aggregated_output":"ok","exit_code":0,"duration":{"secs":3,"nanos":0}}}}),
        3,
        10,
    );
    assert_eq!(s.events[0].token_usage_at_event, None);
    assert_eq!(s.events[1].token_usage_at_event.unwrap().total_tokens, 100);
    assert_eq!(s.token_usage.unwrap().total_tokens, 1000);
}

#[test]
fn inspector_remains_unchanged_while_agents_current_context_changes() {
    use std::sync::Arc;
    let mut app = App::new(demo::snapshot(), true);
    app.follow = false;
    let before = ui::render_text(&mut app, 160, 42).unwrap();
    let mut snapshot = app.snapshot.clone();
    let s = Arc::make_mut(&mut snapshot.sessions[0]);
    add(
        s,
        json!({"type":"event_msg","timestamp":chrono::Utc::now().to_rfc3339(),"payload":{"type":"token_count","info":{"model_context_window":200000,"last_token_usage":{"input_tokens":90000,"output_tokens":10000,"total_tokens":100000},"total_token_usage":{"input_tokens":90000,"output_tokens":10000,"total_tokens":100000}}}}),
        999,
        1500,
    );
    app.update(snapshot);
    let after = ui::render_text(&mut app, 160, 42).unwrap();
    assert!(after.contains("Main ~100.0k/200.0k 50% free"));
    let inspect = |text: &str| {
        text.lines()
            .map(|line| line.chars().skip(102).collect::<String>())
            .collect::<Vec<_>>()
    };
    assert_eq!(inspect(&before), inspect(&after));
    assert!(after.contains("Used ~20000 tokens (last request)"));
}

#[test]
fn codex_context_uses_last_request_not_cumulative_and_can_shrink_after_compaction() {
    let mut s = Session::new(Provider::Codex, PathBuf::from("context.jsonl"));
    add(
        &mut s,
        json!({"type":"event_msg","timestamp":"2026-09-22T01:00:00Z","payload":{"type":"token_count","info":{"model_context_window":200000,"last_token_usage":{"input_tokens":90000,"output_tokens":10000},"total_token_usage":{"input_tokens":4500000,"output_tokens":100000}}}}),
        0,
        10,
    );
    assert_eq!(s.context.used(), Some(100000));
    assert_eq!(s.context.remaining(), Some(100000));
    assert_eq!(s.context.free_percent(), Some(50.0));
    add(
        &mut s,
        json!({"type":"compacted","timestamp":"2026-09-22T01:00:01Z","payload":{}}),
        1,
        10,
    );
    assert_eq!(s.context.used(), None);
    assert_eq!(s.context.remaining(), None);
    add(
        &mut s,
        json!({"type":"event_msg","timestamp":"2026-09-22T01:00:02Z","payload":{"type":"token_count","info":{"model_context_window":200000,"last_token_usage":{"input_tokens":15000,"output_tokens":1000},"total_token_usage":{"input_tokens":4515000,"output_tokens":101000}}}}),
        2,
        10,
    );
    assert_eq!(s.context.used(), Some(16000));
    assert_eq!(s.context.remaining(), Some(184000));
    assert_eq!(s.token_usage.unwrap().total_tokens, 4616000);
}

#[test]
fn claude_context_is_last_response_plus_cache_and_preserves_recorded_1m_tag() {
    let mut s = Session::new(Provider::Claude, PathBuf::from("context.jsonl"));
    add(
        &mut s,
        json!({"type":"attachment","timestamp":"2026-09-22T01:00:00Z","attachment":{"type":"model","identity":{"modelId":"claude-opus-5[1m]"}}}),
        0,
        10,
    );
    let mut first = claude_usage("first", 20);
    first["timestamp"] = json!("2026-09-22T01:00:01Z");
    first["message"]["model"] = json!("claude-opus-5");
    add(&mut s, first, 1, 10);
    assert_eq!(s.context.used(), Some(2530));
    assert_eq!(s.context.limit_tokens, Some(1000000));
    assert_eq!(s.context.limit_source, Some(ContextLimitSource::ModelTag));
    let snapshot = s.events[0].context_at_event;
    let mut second = claude_usage("second", 100);
    second["timestamp"] = json!("2026-09-22T01:00:02Z");
    second["message"]["model"] = json!("claude-opus-5");
    add(&mut s, second, 2, 10);
    assert_eq!(s.context.used(), Some(2610));
    assert_eq!(s.context.remaining(), Some(997390));
    assert_eq!(s.events[0].context_at_event, snapshot);
    assert_eq!(s.token_usage.unwrap().total_tokens, 5140);
    add(
        &mut s,
        json!({"type":"system","subtype":"compact_boundary","timestamp":"2026-09-22T01:00:03Z"}),
        3,
        10,
    );
    assert_eq!(s.context.used(), None);
}

#[test]
fn model_change_resets_context_and_unknown_or_contradicted_limits_do_not_invent_headroom() {
    let mut s = Session::new(Provider::Claude, PathBuf::from("context.jsonl"));
    let mut first = claude_usage("first", 20);
    first["message"]["model"] = json!("claude-fable-5-1");
    add(&mut s, first, 0, 10);
    assert_eq!(s.context.limit_tokens, Some(1000000));
    let mut second = claude_usage("second", 20);
    second["message"]["model"] = json!("unknown-custom-model");
    add(&mut s, second, 1, 10);
    assert_eq!(s.context.limit_tokens, None);
    assert_eq!(s.context.remaining(), None);
    let mut third = claude_usage("third", 20);
    third["message"]["model"] = json!("claude-sonnet-4-6");
    third["message"]["usage"]["cache_read_input_tokens"] = json!(300000);
    add(&mut s, third, 2, 10);
    assert_eq!(s.context.limit_tokens, None);
    assert_eq!(s.context.remaining(), None);
    let mut future = claude_usage("future", 20);
    future["message"]["model"] = json!("claude-opus-4-99");
    add(&mut s, future, 3, 10);
    assert_eq!(s.context.limit_tokens, None);
    assert_eq!(s.context.remaining(), None);
}
