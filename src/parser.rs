//! Provider-specific transcripts are normalized here. Unknown records are ignored;
//! only explicit lifecycle events establish a completed turn.
use crate::model::*;
use chrono::{DateTime, Utc};
use serde_json::Value;

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(t) => clean(t),
        Value::Array(items) => items
            .iter()
            .map(|v| {
                if v.get("text").is_some() {
                    text(&v["text"])
                } else if v
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|t| matches!(t, "image" | "input_image"))
                {
                    "[image attachment]".into()
                } else {
                    text(v)
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => clean(&serde_json::to_string_pretty(v).unwrap_or_default()),
    }
}
fn decode(v: &Value) -> Value {
    v.as_str()
        .and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or_else(|| v.clone())
}
fn time(v: &Value) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s(v, "timestamp"))
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or(DateTime::UNIX_EPOCH)
}
fn event(id: String, at: DateTime<Utc>, kind: Kind, name: &str, input: String) -> FlowEvent {
    FlowEvent {
        id,
        time: at,
        kind,
        name: clean(name),
        input: clean(&input),
        output: None,
        outcome: if matches!(kind, Kind::Tool | Kind::Spawn) {
            Outcome::Pending
        } else {
            Outcome::Info
        },
        target: None,
        completed_at: None,
        turn_boundary: None,
        token_usage_at_event: None,
        usage_recorded_at: None,
        context_at_event: None,
        context_recorded_at: None,
    }
}
fn working(session: &mut Session, at: DateTime<Utc>) {
    session.turn_open = true;
    session.turn_known = true;
    session.last_activity = session.last_activity.max(at);
}
fn failed(v: &Value) -> bool {
    v.get("is_error").and_then(Value::as_bool) == Some(true)
        || v.get("isError").and_then(Value::as_bool) == Some(true)
        || v.get("exit_code")
            .and_then(Value::as_i64)
            .is_some_and(|c| c != 0)
        || matches!(s(v, "status"), "failed" | "error")
        || v.get("error").is_some_and(|e| !e.is_null() && e != false)
}
fn result_failed(v: &Value, rendered: &str) -> bool {
    if failed(v) {
        return true;
    }
    // CLI tool responses commonly embed the exit code in an otherwise opaque string.
    for marker in ["Process exited with code ", "Exit code: "] {
        if let Some(rest) = rendered.split(marker).nth(1)
            && rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<i64>().ok())
                .is_some_and(|c| c != 0)
        {
            return true;
        }
    }
    false
}

fn attach(
    session: &mut Session,
    call_id: &str,
    value: &Value,
    at: DateTime<Utc>,
    is_error: bool,
    target: Option<String>,
    limit: usize,
) {
    let decoded = decode(value);
    let output = text(value);
    let outcome = if is_error || result_failed(&decoded, &output) {
        Outcome::Error
    } else {
        Outcome::Returned
    };
    if let Some(e) = session.events.iter_mut().rev().find(|e| e.id == call_id) {
        e.output = Some(match e.output.take() {
            Some(prior) => clean(&format!("{prior}\n{output}")),
            None => output,
        });
        if e.outcome != Outcome::Error {
            e.outcome = outcome;
        }
        e.completed_at = Some(at);
        if target.is_some() {
            e.target = target;
        } else if e.kind == Kind::Spawn {
            e.target = ["agent_id", "task_name", "thread_id"]
                .iter()
                .find_map(|key| decoded.get(key).and_then(Value::as_str).map(str::to_owned))
                .or_else(|| e.target.clone());
        }
    } else {
        let mut e = event(
            format!("result:{call_id}"),
            at,
            Kind::Notice,
            "Result (call outside retained history)",
            String::new(),
        );
        e.output = Some(output);
        e.outcome = outcome;
        e.target = target;
        session.push(e, limit);
    }
    session.last_activity = session.last_activity.max(at);
}

pub fn parse_line(session: &mut Session, line: &[u8], sequence: u64, limit: usize) {
    let Ok(v) = serde_json::from_slice::<Value>(line) else {
        session.malformed += 1;
        return;
    };
    match session.provider {
        Provider::Codex => codex(session, &v, sequence, limit),
        Provider::Claude => claude(session, &v, sequence, limit),
    }
}

fn usage(v: &Value, provider: Provider) -> Option<TokenUsage> {
    let keys = [
        "input_tokens",
        "output_tokens",
        "total_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
    ];
    if !keys
        .iter()
        .any(|key| v.get(key).and_then(Value::as_u64).is_some())
    {
        return None;
    }
    let count = |key: &str| v.get(key).and_then(Value::as_u64).unwrap_or(0);
    let (cache_read_tokens, cache_write_tokens) = match provider {
        Provider::Claude => (
            count("cache_read_input_tokens"),
            count("cache_creation_input_tokens"),
        ),
        Provider::Codex => (
            count("cached_input_tokens"),
            count("cache_write_input_tokens"),
        ),
    };
    // Claude's input_tokens excludes both cache categories; Codex's includes them.
    let input_tokens = if provider == Provider::Claude {
        count("input_tokens")
            .saturating_add(cache_read_tokens)
            .saturating_add(cache_write_tokens)
    } else {
        count("input_tokens")
    };
    let output_tokens = count("output_tokens");
    Some(TokenUsage {
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        reasoning_tokens: if provider == Provider::Claude {
            v["output_tokens_details"]["thinking_tokens"]
                .as_u64()
                .unwrap_or(0)
        } else {
            count("reasoning_output_tokens")
        },
        total_tokens: if provider == Provider::Codex {
            v.get("total_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| input_tokens.saturating_add(output_tokens))
        } else {
            input_tokens.saturating_add(output_tokens)
        },
    })
}

fn cumulative_usage(session: &mut Session, value: &Value, at: DateTime<Utc>) {
    if let Some(usage) = usage(value, Provider::Codex) {
        session.token_usage = Some(session.token_usage.unwrap_or_default().maximum(usage));
        session.record_usage(at);
    }
}

fn context_usage(session: &mut Session, usage: TokenUsage, at: DateTime<Utc>) {
    if session.context_updated_at.is_some_and(|known| at < known) {
        return;
    }
    // Replace per-request usage: compaction and model changes may reduce it.
    session.context.input_tokens = Some(usage.input_tokens);
    session.context.output_tokens = usage.output_tokens;
    if session.context.limit_source == Some(ContextLimitSource::ModelDefault)
        && session
            .context
            .limit_tokens
            .is_some_and(|limit| usage.input_tokens.saturating_add(usage.output_tokens) > limit)
    {
        session.context.limit_tokens = None;
        session.context.limit_source = None;
    }
    session.record_context(at);
}

fn context_limit(session: &mut Session, value: &Value, at: DateTime<Utc>) {
    if session.context_updated_at.is_some_and(|known| at < known) {
        return;
    }
    if let Some(limit) = value.as_u64().filter(|n| *n > 0) {
        session.context.limit_tokens = Some(limit);
        session.context.limit_source = Some(ContextLimitSource::Recorded);
        session.record_context(at);
    }
}

fn claude_model(session: &mut Session, model: &str, at: DateTime<Utc>) {
    if model.is_empty() || model == "<synthetic>" {
        return;
    }
    let base = |name: &str| name.split('[').next().unwrap_or("").to_ascii_lowercase();
    let changed = !session.model.is_empty() && base(&session.model) != base(model);
    if changed {
        session.context = ContextUsage::default();
    }
    if session.model.is_empty() || changed || model.contains('[') {
        session.model = clean(model);
    }
    if session.context.limit_source == Some(ContextLimitSource::Recorded) {
        return;
    }
    let name = session.model.to_ascii_lowercase();
    // Match known versions or their dated API IDs, never guess a future model's window.
    let matches_model = |model: &str| {
        name == model
            || name.strip_prefix(model).is_some_and(|suffix| {
                suffix
                    .strip_prefix('-')
                    .is_some_and(|date| date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()))
            })
    };
    let (limit, source) = if name.contains("[1m]") {
        (Some(1_000_000), Some(ContextLimitSource::ModelTag))
    } else if [
        "claude-fable-5",
        "claude-fable-5-1",
        "claude-sonnet-5",
        "claude-opus-5",
        "claude-opus-4-8",
        "claude-opus-4-7",
    ]
    .iter()
    .any(|m| matches_model(m))
    {
        (Some(1_000_000), Some(ContextLimitSource::ModelDefault))
    } else if [
        "claude-sonnet-4",
        "claude-sonnet-4-5",
        "claude-sonnet-4-6",
        "claude-opus-4",
        "claude-opus-4-1",
        "claude-opus-4-5",
        "claude-opus-4-6",
        "claude-haiku-4-5",
    ]
    .iter()
    .any(|m| matches_model(m))
    {
        (Some(200_000), Some(ContextLimitSource::ModelDefault))
    } else {
        (None, None)
    };
    if session.context.limit_tokens != limit || session.context.limit_source != source || changed {
        session.context.limit_tokens = limit;
        session.context.limit_source = source;
        session.record_context(at);
    }
}

fn codex(session: &mut Session, v: &Value, sequence: u64, limit: usize) {
    let p = &v["payload"];
    let at = time(v);
    let record_type = s(v, "type");
    if record_type == "session_meta" {
        // Forked transcripts can contain a second session_meta for their parent.
        if !session.metadata_seen {
            session.metadata_seen = true;
            let id = s(p, "id");
            if !id.is_empty() {
                session.id = id.into();
                session.key = session.provider.key(id);
            }
            session.cwd = clean(s(p, "cwd"));
            let spawn = &p["source"]["subagent"]["thread_spawn"];
            let parent = p
                .get("parent_thread_id")
                .and_then(Value::as_str)
                .or_else(|| spawn.get("parent_thread_id").and_then(Value::as_str));
            session.parent = parent.map(|p| session.provider.key(p));
            session.agent_path = clean(
                p.get("agent_path")
                    .and_then(Value::as_str)
                    .or_else(|| spawn.get("agent_path").and_then(Value::as_str))
                    .unwrap_or(""),
            );
            session.own_history_start = p
                .get("subagent_history_start_ordinal")
                .and_then(Value::as_u64);
        }
        return;
    }
    if session
        .own_history_start
        .is_some_and(|start| v.get("ordinal").and_then(Value::as_u64).unwrap_or(sequence) < start)
    {
        session.inherited_skipped += 1;
        return;
    }
    if record_type == "turn_context" {
        if !session.model.is_empty() && session.model != s(p, "model") {
            session.context = ContextUsage::default();
            session.record_context(at);
        }
        session.model = clean(s(p, "model"));
        if !s(p, "cwd").is_empty() {
            session.cwd = clean(s(p, "cwd"));
        }
        return;
    }
    if record_type == "compacted" {
        session.clear_context(at);
        session.push(
            event(
                format!("compact:{sequence}"),
                at,
                Kind::Notice,
                "Context compacted",
                String::new(),
            ),
            limit,
        );
        return;
    }
    if record_type == "token_usage_record" {
        if p.get("thread_id")
            .and_then(Value::as_str)
            .is_none_or(|id| id == session.id)
        {
            cumulative_usage(session, &p["thread_token_usage"], at);
            if let Some(usage) = usage(&p["usage"], Provider::Codex) {
                context_usage(session, usage, at);
            }
        }
        return;
    }
    if record_type == "event_msg" {
        match s(p, "type") {
            "token_count" => {
                cumulative_usage(session, &p["info"]["total_token_usage"], at);
                context_limit(session, &p["info"]["model_context_window"], at);
                if let Some(usage) = usage(&p["info"]["last_token_usage"], Provider::Codex) {
                    context_usage(session, usage, at);
                }
            }
            "task_started" | "turn_started" => {
                context_limit(session, &p["model_context_window"], at);
                working(session, at);
                let mut e = event(
                    format!("start:{sequence}"),
                    at,
                    Kind::Turn,
                    "Turn started",
                    String::new(),
                );
                e.turn_boundary = Some(TurnBoundary::Start);
                session.push(e, limit);
            }
            "task_complete" | "task_completed" | "turn_complete" | "turn_aborted" => {
                session.turn_open = false;
                session.turn_known = true;
                let mut e = event(
                    format!("end:{sequence}"),
                    at,
                    Kind::Turn,
                    if s(p, "type") == "turn_aborted" {
                        "Turn interrupted"
                    } else {
                        "Turn completed"
                    },
                    text(&p["last_agent_message"]),
                );
                if s(p, "type") == "turn_aborted" {
                    e.outcome = Outcome::Error;
                    e.turn_boundary = Some(TurnBoundary::Interrupted);
                } else {
                    e.turn_boundary = Some(TurnBoundary::Completed {
                        at,
                        reported_ms: None,
                    });
                }
                session.push(e, limit);
            }
            "item_completed" => completed_item(session, &p["item"], at, sequence, limit),
            _ => {}
        }
        return;
    }
    if record_type != "response_item" {
        return;
    }
    let id = p
        .get("call_id")
        .or_else(|| p.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("event:{sequence}"));
    match s(p, "type") {
        "function_call" | "custom_tool_call" => {
            let name = s(p, "name");
            let args = decode(
                p.get("arguments")
                    .or_else(|| p.get("input"))
                    .unwrap_or(&Value::Null),
            );
            let is_spawn = name.contains("spawn_agent");
            let is_message = name.contains("send_message") || name.contains("followup_task");
            let kind = if is_spawn {
                Kind::Spawn
            } else if is_message {
                Kind::Message
            } else {
                Kind::Tool
            };
            let mut e = event(id, at, kind, name, text(&args));
            e.outcome = Outcome::Pending;
            e.target = args
                .get("target")
                .or_else(|| args.get("agent_id"))
                .or_else(|| args.get("task_name"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            working(session, at);
            session.push(e, limit);
        }
        "function_call_output" | "custom_tool_call_output" => attach(
            session,
            s(p, "call_id"),
            &p["output"],
            at,
            false,
            None,
            limit,
        ),
        "message" => {
            let role = s(p, "role");
            if !matches!(role, "user" | "assistant") || s(p, "channel") == "analysis" {
                return;
            }
            let content = text(&p["content"]);
            if role == "user"
                && (content.starts_with("<environment_context>")
                    || content.starts_with("<permissions instructions>"))
            {
                return;
            }
            if role == "user" && session.title.is_empty() {
                session.title = one_line(&content, 140);
            }
            working(session, at);
            session.push(
                event(
                    id,
                    at,
                    if role == "user" {
                        Kind::User
                    } else {
                        Kind::Assistant
                    },
                    if role == "user" { "User" } else { "Assistant" },
                    content,
                ),
                limit,
            );
        }
        "agent_message" => {
            let name = format!("{} → {}", s(p, "author"), s(p, "recipient"));
            let mut e = event(id, at, Kind::Message, &name, text(&p["content"]));
            e.target = Some(s(p, "author").into());
            session.push(e, limit);
        }
        // Hidden reasoning and encrypted payloads are deliberately not displayed.
        _ => {}
    }
}

fn completed_item(
    session: &mut Session,
    item: &Value,
    at: DateTime<Utc>,
    sequence: u64,
    limit: usize,
) {
    let typ = s(item, "type");
    if !matches!(
        typ,
        "CommandExecution" | "McpToolCall" | "Extension" | "WebSearch" | "FileChange"
    ) {
        return;
    }
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("item:{sequence}"));
    if session.events.iter().any(|e| e.id == id) {
        return;
    }
    let (name, input, output) = match typ {
        "CommandExecution" => (
            "exec_command".to_owned(),
            text(&item["command"]),
            text(
                item.get("aggregated_output")
                    .filter(|v| !v.is_null())
                    .or_else(|| item.get("formatted_output"))
                    .unwrap_or(&Value::Null),
            ),
        ),
        "McpToolCall" => (
            format!("{}.{}", s(item, "server"), s(item, "tool")),
            text(&item["arguments"]),
            text(item.get("result").unwrap_or(&item["error"])),
        ),
        "Extension" => (
            s(item, "kind").to_owned(),
            text(item.get("query").unwrap_or(&item["action"])),
            text(&item["results"]),
        ),
        _ => (typ.to_owned(), text(item), String::new()),
    };
    let mut e = event(id, at, Kind::Tool, &name, input);
    e.output = Some(output);
    if let Some(secs) = item["duration"]["secs"].as_i64() {
        let nanos = item["duration"]["nanos"].as_i64().unwrap_or(0);
        let millis = secs.saturating_mul(1000).saturating_add(nanos / 1_000_000);
        if let Some(duration) = chrono::Duration::try_milliseconds(millis)
            && let Some(start) = at.checked_sub_signed(duration)
        {
            e.time = start;
            e.completed_at = Some(at);
        }
    }
    e.outcome = if failed(item) {
        Outcome::Error
    } else {
        Outcome::Returned
    };
    session.push(e, limit);
}

fn claude(session: &mut Session, v: &Value, sequence: u64, limit: usize) {
    let at = time(v);
    let sid = s(v, "sessionId");
    let aid = s(v, "agentId");
    if !sid.is_empty() && !session.metadata_seen {
        session.metadata_seen = true;
        let child = !aid.is_empty() || session.parent.is_some();
        if child {
            let agent_id = if aid.is_empty() {
                session.id.rsplit('/').next().unwrap_or("")
            } else {
                aid
            }
            .to_owned();
            session.id = format!("{sid}/{agent_id}");
            session.parent = Some(session.provider.key(sid));
            session.agent_path = agent_id;
        } else {
            session.id = sid.to_owned();
        }
        session.key = session.provider.key(&session.id);
    }
    if !s(v, "cwd").is_empty() {
        session.cwd = clean(s(v, "cwd"));
    }
    let message = &v["message"];
    claude_model(session, s(message, "model"), at);
    context_limit(session, &v["context_window"]["context_window_size"], at);
    match s(v, "type") {
        "attachment" if s(&v["attachment"], "type") == "model" => {
            claude_model(session, s(&v["attachment"]["identity"], "modelId"), at);
            return;
        }
        "system" if s(v, "subtype") == "compact_boundary" => {
            session.clear_context(at);
            session.push(
                event(
                    format!("compact:{sequence}"),
                    at,
                    Kind::Notice,
                    "Context compacted",
                    String::new(),
                ),
                limit,
            );
            return;
        }
        "ai-title" => {
            session.title = clean(s(v, "aiTitle"));
            return;
        }
        "system" if s(v, "subtype") == "turn_duration" => {
            session.turn_open = false;
            session.turn_known = true;
            let mut e = event(
                format!("end:{sequence}"),
                at,
                Kind::Turn,
                "Turn completed",
                text(v.get("durationMs").unwrap_or(&Value::Null)),
            );
            e.turn_boundary = Some(TurnBoundary::Completed {
                at,
                reported_ms: v.get("durationMs").and_then(Value::as_u64),
            });
            session.push(e, limit);
            return;
        }
        "progress" => {
            let data = &v["data"];
            if let Some(agent) = data.get("agentId").and_then(Value::as_str)
                && let Some(e) = session
                    .events
                    .iter_mut()
                    .rev()
                    .find(|e| e.id == s(v, "parentToolUseID"))
            {
                e.target = Some(format!("{sid}/{agent}"));
            }
            return;
        }
        "user" | "assistant" => {}
        _ => return,
    }
    if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return;
    }
    let role = s(v, "type");
    let base_id = v
        .get("uuid")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("record:{sequence}"));
    if role == "assistant"
        && let Some(mut current) = usage(&message["usage"], Provider::Claude)
    {
        let id = message
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .unwrap_or(&base_id)
            .to_owned();
        let previous = session.message_usage.get(&id).copied().unwrap_or_default();
        current = previous.maximum(current);
        current.total_tokens = current.input_tokens.saturating_add(current.output_tokens);
        session
            .token_usage
            .get_or_insert_default()
            .add_delta(previous, current);
        session.message_usage.insert(id, current);
        session.record_usage(at);
        context_usage(session, current, at);
    }
    let blocks = match &message["content"] {
        Value::Array(a) => a.clone(),
        Value::String(t) => vec![serde_json::json!({"type":"text","text":t})],
        _ => vec![],
    };
    for (idx, block) in blocks.iter().enumerate() {
        match s(block, "type") {
            "tool_use" => {
                let name = s(block, "name");
                let kind = if matches!(name, "Agent" | "Task") {
                    Kind::Spawn
                } else if name == "SendMessage" {
                    Kind::Message
                } else {
                    Kind::Tool
                };
                let mut e = event(s(block, "id").into(), at, kind, name, text(&block["input"]));
                e.outcome = Outcome::Pending;
                working(session, at);
                session.push(e, limit);
            }
            "tool_result" => {
                let r = &v["toolUseResult"];
                let target = r
                    .get("agentId")
                    .and_then(Value::as_str)
                    .map(|a| format!("{sid}/{a}"));
                attach(
                    session,
                    s(block, "tool_use_id"),
                    &block["content"],
                    at,
                    block.get("is_error").and_then(Value::as_bool) == Some(true),
                    target,
                    limit,
                );
            }
            "text" => {
                let body = text(&block["text"]);
                if session.title.is_empty() && role == "user" {
                    session.title = one_line(&body, 140);
                }
                working(session, at);
                session.push(
                    event(
                        format!("{base_id}:{idx}"),
                        at,
                        if role == "user" {
                            Kind::User
                        } else {
                            Kind::Assistant
                        },
                        if role == "user" { "User" } else { "Assistant" },
                        body,
                    ),
                    limit,
                );
            }
            _ => {}
        }
    }
    if role == "assistant" && matches!(s(message, "stop_reason"), "end_turn" | "stop_sequence") {
        session.turn_open = false;
        session.turn_known = true;
        // Keep completion on a newly recorded final response. A streamed update to an
        // older response needs a new boundary after any intervening tool events.
        if let Some(e) = session.events.iter_mut().rev().find(|e| {
            e.kind == Kind::Assistant && e.time == at && e.id.starts_with(&format!("{base_id}:"))
        }) {
            e.turn_boundary = Some(TurnBoundary::Completed {
                at,
                reported_ms: None,
            });
        } else {
            let mut e = event(
                format!("end:{base_id}"),
                at,
                Kind::Turn,
                "Turn completed",
                String::new(),
            );
            e.turn_boundary = Some(TurnBoundary::Completed {
                at,
                reported_ms: None,
            });
            session.push(e, limit);
        }
    }
}
