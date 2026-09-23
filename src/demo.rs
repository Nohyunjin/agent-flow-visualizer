use crate::{model::*, parser::parse_line};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

pub fn snapshot() -> Snapshot {
    let now = Utc::now();
    let mut root = Session::new(Provider::Codex, PathBuf::from("demo/codex.jsonl"));
    let mut child = Session::new(Provider::Codex, PathBuf::from("demo/reviewer.jsonl"));
    let mut tester = Session::new(Provider::Codex, PathBuf::from("demo/tests.jsonl"));
    let mut claude = Session::new(Provider::Claude, PathBuf::from("demo/claude.jsonl"));
    let mut worker = Session::new(
        Provider::Claude,
        PathBuf::from("demo/claude-demo/subagents/agent-tests.jsonl"),
    );
    let mut seq = 0u64;
    let mut add = |s: &mut Session, mut v: Value, seconds: i64| {
        v["timestamp"] = json!((now - Duration::seconds(seconds)).to_rfc3339());
        parse_line(s, &serde_json::to_vec(&v).unwrap(), seq, 2000);
        seq += 1;
    };
    add(
        &mut root,
        json!({"type":"session_meta","payload":{"id":"codex-demo","cwd":"/workspace/payments","agent_path":"/root"}}),
        1000,
    );
    for (n, start, end) in [(1, 600, 480), (2, 420, 350)] {
        add(
            &mut root,
            json!({"type":"event_msg","payload":{"type":"task_started"}}),
            start,
        );
        add(
            &mut root,
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":format!("결제 재시도 조사 {n}: 테스트로 원인 확인")}]}}),
            start - 1,
        );
        add(
            &mut root,
            json!({"type":"response_item","payload":{"type":"function_call","call_id":format!("baseline-{n}"),"name":"exec_command","arguments":"{\"cmd\":\"cargo test payments -- --nocapture\"}"}}),
            start - 10,
        );
        add(
            &mut root,
            json!({"type":"response_item","payload":{"type":"function_call_output","call_id":format!("baseline-{n}"),"output":"Process exited with code 0\nBaseline tests passed"}}),
            end + 5,
        );
        add(
            &mut root,
            json!({"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"Baseline investigated."}}),
            end,
        );
    }
    add(
        &mut root,
        json!({"type":"event_msg","payload":{"type":"task_started"}}),
        95,
    );
    add(
        &mut root,
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"결제 재시도 오류를 수정하고 테스트해줘"}]}}),
        94,
    );
    add(
        &mut root,
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"read","name":"exec_command","arguments":"{\"cmd\":\"rg -n retry src/payments\"}"}}),
        90,
    );
    add(
        &mut root,
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"read","output":"src/payments/retry.rs:42: retry(attempt + 1)\nsrc/payments/retry.rs:58: MAX_RETRIES = 3"}}),
        89,
    );
    add(
        &mut root,
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"spawn","name":"spawn_agent","arguments":"{\"task_name\":\"reviewer\",\"message\":\"Review retry limits and check for duplicate charges\"}"}}),
        84,
    );
    add(
        &mut root,
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"spawn","output":"{\"agent_id\":\"review-demo\",\"task_name\":\"/root/reviewer\"}"}}),
        83,
    );
    add(
        &mut child,
        json!({"type":"session_meta","payload":{"id":"review-demo","parent_thread_id":"codex-demo","agent_path":"/root/reviewer","cwd":"/workspace/payments"}}),
        82,
    );
    add(
        &mut child,
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"Review retry limits and check for duplicate charges"}]}}),
        81,
    );
    add(
        &mut child,
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"check","name":"exec_command","arguments":"{\"cmd\":\"cargo test retry_limit\"}"}}),
        75,
    );
    add(
        &mut child,
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"check","output":"Process exited with code 1\nFAILED retry_limit: expected 3 attempts, received 4"}}),
        74,
    );
    add(
        &mut child,
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"report","name":"send_message","arguments":"{\"target\":\"/root\",\"message\":\"Found an off-by-one in the retry limit. Use attempt >= MAX_RETRIES.\"}"}}),
        65,
    );
    add(
        &mut child,
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"report","output":"Message delivered"}}),
        64,
    );
    add(
        &mut child,
        json!({"type":"token_usage_record","payload":{"thread_id":"review-demo","usage":{"input_tokens":11900,"output_tokens":1040},"thread_token_usage":{"input_tokens":11900,"output_tokens":1040,"cached_input_tokens":9200,"total_tokens":12940}}}),
        61,
    );
    add(
        &mut child,
        json!({"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"Review complete: one retry boundary defect found."}}),
        60,
    );
    add(
        &mut root,
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"spawn-tests","name":"spawn_agent","arguments":"{\"task_name\":\"tests\",\"message\":\"Run payment integration tests while reviewer checks retry limits\"}"}}),
        82,
    );
    add(
        &mut root,
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"spawn-tests","output":"{\"agent_id\":\"tests-demo\"}"}}),
        81,
    );
    add(
        &mut tester,
        json!({"type":"session_meta","payload":{"id":"tests-demo","parent_thread_id":"codex-demo","agent_path":"/root/tests","cwd":"/workspace/payments"}}),
        80,
    );
    add(
        &mut tester,
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"Run payment integration tests while reviewer checks retry limits"}]}}),
        79,
    );
    add(
        &mut tester,
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"integration","name":"exec_command","arguments":"{\"cmd\":\"cargo test --test payments_integration\"}"}}),
        73,
    );
    add(
        &mut tester,
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"integration","output":"Process exited with code 0\n18 integration tests passed"}}),
        35,
    );
    add(
        &mut tester,
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"test-report","name":"send_message","arguments":"{\"target\":\"/root\",\"message\":\"18 integration tests passed. Retry boundary needs reviewer follow-up.\"}"}}),
        34,
    );
    add(
        &mut tester,
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"test-report","output":"Message delivered"}}),
        33,
    );
    add(
        &mut tester,
        json!({"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"Payment integration tests complete."}}),
        32,
    );
    add(
        &mut root,
        json!({"type":"response_item","payload":{"type":"custom_tool_call","call_id":"patch","name":"apply_patch","input":"*** Update File: src/payments/retry.rs\n- if attempt > MAX_RETRIES {\n+ if attempt >= MAX_RETRIES {"}}),
        55,
    );
    add(
        &mut root,
        json!({"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"patch","output":"Patch applied to src/payments/retry.rs"}}),
        54,
    );
    add(
        &mut root,
        json!({"type":"event_msg","payload":{"type":"token_count","info":{"model_context_window":200000,"last_token_usage":{"input_tokens":19000,"output_tokens":1000},"total_token_usage":{"input_tokens":30200,"output_tokens":8000,"cached_input_tokens":20000,"reasoning_output_tokens":1600,"total_tokens":38200}}}}),
        5,
    );
    add(
        &mut root,
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"test","name":"exec_command","arguments":"{\"cmd\":\"cargo test payments\"}"}}),
        4,
    );
    add(
        &mut claude,
        json!({"type":"user","sessionId":"claude-demo","message":{"content":"접근성 검사 준비와 기존 테스트 확인"}}),
        500,
    );
    add(
        &mut claude,
        json!({"type":"assistant","sessionId":"claude-demo","message":{"content":[{"type":"tool_use","id":"baseline-claude","name":"Bash","input":{"command":"pnpm test accessibility"}}]}}),
        450,
    );
    add(
        &mut claude,
        json!({"type":"user","sessionId":"claude-demo","message":{"content":[{"type":"tool_result","tool_use_id":"baseline-claude","content":"Baseline accessibility tests passed"}]}}),
        420,
    );
    add(
        &mut claude,
        json!({"type":"assistant","sessionId":"claude-demo","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"기존 접근성 검사를 확인했습니다."}]}}),
        400,
    );
    add(
        &mut claude,
        json!({"type":"system","subtype":"turn_duration","durationMs":100000}),
        399,
    );
    add(
        &mut claude,
        json!({"type":"user","sessionId":"claude-demo","cwd":"/workspace/checkout","message":{"content":"체크아웃 접근성과 회귀 테스트를 확인해줘"}}),
        80,
    );
    add(
        &mut claude,
        json!({"type":"assistant","sessionId":"claude-demo","message":{"model":"claude-sonnet-4-6","id":"demo-usage","usage":{"input_tokens":400,"cache_read_input_tokens":60000,"cache_creation_input_tokens":2200,"output_tokens":1800},"content":[{"type":"tool_use","id":"agent","name":"Agent","input":{"description":"Run checkout regression tests","prompt":"Check keyboard navigation and payment recovery"}}]}}),
        70,
    );
    add(
        &mut claude,
        json!({"type":"user","sessionId":"claude-demo","toolUseResult":{"agentId":"tests"},"message":{"content":[{"type":"tool_result","tool_use_id":"agent","content":"Regression agent launched in background."}]}}),
        69,
    );
    add(
        &mut worker,
        json!({"type":"user","sessionId":"claude-demo","agentId":"tests","cwd":"/workspace/checkout","message":{"content":"Check keyboard navigation and payment recovery"}}),
        68,
    );
    add(
        &mut worker,
        json!({"type":"assistant","sessionId":"claude-demo","agentId":"tests","message":{"content":[{"type":"tool_use","id":"bash","name":"Bash","input":{"command":"pnpm test checkout --run"}}]}}),
        10,
    );
    add(
        &mut worker,
        json!({"type":"user","sessionId":"claude-demo","agentId":"tests","message":{"content":[{"type":"tool_result","tool_use_id":"bash","content":"✓ keyboard navigation\n✓ payment recovery\n12 tests passed in 1.42s"}]}}),
        8,
    );
    add(
        &mut worker,
        json!({"type":"assistant","sessionId":"claude-demo","agentId":"tests","message":{"model":"claude-sonnet-4-6","id":"demo-worker-usage","usage":{"input_tokens":500,"cache_read_input_tokens":9000,"cache_creation_input_tokens":1500,"output_tokens":300},"stop_reason":"end_turn","content":[{"type":"text","text":"키보드 탐색과 결제 복구 테스트 12개가 모두 통과했습니다."}]}}),
        7,
    );
    root.title = "Fix payment retry boundary".into();
    root.model = "gpt-5.6".into();
    claude.title = "Checkout accessibility audit".into();
    claude.model = "claude-sonnet-4-6".into();
    Snapshot {
        sessions: vec![
            Arc::new(root),
            Arc::new(child),
            Arc::new(claude),
            Arc::new(worker),
            Arc::new(tester),
        ],
        warnings: vec![],
        discovered: 5,
        scanned_at: now,
    }
}
