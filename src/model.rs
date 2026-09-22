use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::Arc,
};

pub const TEXT_LIMIT: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ContextLimitSource {
    Recorded,
    ModelTag,
    ModelDefault,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ContextUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: u64,
    pub limit_tokens: Option<u64>,
    pub limit_source: Option<ContextLimitSource>,
}

impl ContextUsage {
    pub fn used(self) -> Option<u64> {
        self.input_tokens
            .map(|n| n.saturating_add(self.output_tokens))
    }
    pub fn remaining(self) -> Option<u64> {
        Some(self.limit_tokens?.saturating_sub(self.used()?))
    }
    pub fn free_percent(self) -> Option<f64> {
        Some(100.0 * self.remaining()? as f64 / self.limit_tokens.filter(|n| *n > 0)? as f64)
    }
}

/// Cache tokens are subsets of input; reasoning tokens are a subset of output.
/// The total is processed tokens, not context occupancy or a billing amount.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
}

impl TokenUsage {
    pub fn maximum(self, other: Self) -> Self {
        Self {
            input_tokens: self.input_tokens.max(other.input_tokens),
            output_tokens: self.output_tokens.max(other.output_tokens),
            cache_read_tokens: self.cache_read_tokens.max(other.cache_read_tokens),
            cache_write_tokens: self.cache_write_tokens.max(other.cache_write_tokens),
            reasoning_tokens: self.reasoning_tokens.max(other.reasoning_tokens),
            total_tokens: self.total_tokens.max(other.total_tokens),
        }
    }
    pub fn add_delta(&mut self, previous: Self, current: Self) {
        self.input_tokens = self
            .input_tokens
            .saturating_add(current.input_tokens.saturating_sub(previous.input_tokens));
        self.output_tokens = self
            .output_tokens
            .saturating_add(current.output_tokens.saturating_sub(previous.output_tokens));
        self.cache_read_tokens = self.cache_read_tokens.saturating_add(
            current
                .cache_read_tokens
                .saturating_sub(previous.cache_read_tokens),
        );
        self.cache_write_tokens = self.cache_write_tokens.saturating_add(
            current
                .cache_write_tokens
                .saturating_sub(previous.cache_write_tokens),
        );
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(
            current
                .reasoning_tokens
                .saturating_sub(previous.reasoning_tokens),
        );
        self.total_tokens = self
            .total_tokens
            .saturating_add(current.total_tokens.saturating_sub(previous.total_tokens));
    }
}

pub fn compact_count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Provider {
    Codex,
    Claude,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
        }
    }
    pub fn key(self, id: &str) -> String {
        format!("{}:{id}", self.label())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Kind {
    User,
    Assistant,
    Tool,
    Spawn,
    Message,
    Turn,
    Notice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Outcome {
    Pending,
    Returned,
    Error,
    Info,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "WAIT",
            Self::Returned => "RETURN",
            Self::Error => "ERROR",
            Self::Info => "",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FlowEvent {
    pub id: String,
    pub time: DateTime<Utc>,
    pub kind: Kind,
    pub name: String,
    pub input: String,
    pub output: Option<String>,
    pub outcome: Outcome,
    pub target: Option<String>,
    pub completed_at: Option<DateTime<Utc>>,
    /// Immutable usage known when this event was first recorded. Tool rows are
    /// anchored to the call start, even after their result arrives.
    pub token_usage_at_event: Option<TokenUsage>,
    pub usage_recorded_at: Option<DateTime<Utc>>,
    pub context_at_event: Option<ContextUsage>,
    pub context_recorded_at: Option<DateTime<Utc>>,
}

impl FlowEvent {
    pub fn summary(&self) -> String {
        one_line(self.output.as_deref().unwrap_or(&self.input), 180)
    }
    pub fn is_tool(&self) -> bool {
        matches!(self.kind, Kind::Tool | Kind::Spawn)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Session {
    pub key: String,
    pub id: String,
    pub provider: Provider,
    pub parent: Option<String>,
    pub agent_path: String,
    pub title: String,
    pub cwd: String,
    pub model: String,
    pub path: PathBuf,
    pub last_activity: DateTime<Utc>,
    pub turn_open: bool,
    pub turn_known: bool,
    pub events: VecDeque<FlowEvent>,
    pub dropped: usize,
    pub malformed: usize,
    pub inherited_skipped: usize,
    pub bytes_read: u64,
    pub file_size: u64,
    pub token_usage: Option<TokenUsage>,
    pub usage_updated_at: Option<DateTime<Utc>>,
    pub context: ContextUsage,
    pub context_updated_at: Option<DateTime<Utc>>,
    #[serde(skip)]
    pub message_usage: HashMap<String, TokenUsage>,
    #[serde(skip)]
    pub usage_history: VecDeque<(DateTime<Utc>, TokenUsage)>,
    #[serde(skip)]
    pub context_history: VecDeque<(DateTime<Utc>, ContextUsage)>,
    #[serde(skip)]
    pub own_history_start: Option<u64>,
    #[serde(skip)]
    pub metadata_seen: bool,
}

impl Session {
    pub fn new(provider: Provider, path: PathBuf) -> Self {
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut id = stem.clone();
        let mut parent = None;
        if provider == Provider::Claude
            && path
                .parent()
                .and_then(|p| p.file_name())
                .is_some_and(|n| n == "subagents")
        {
            let parent_id = path
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .unwrap_or_default()
                .to_string_lossy();
            id = format!("{parent_id}/{}", stem.trim_start_matches("agent-"));
            parent = Some(provider.key(&parent_id));
        }
        Self {
            key: provider.key(&id),
            id,
            provider,
            parent,
            agent_path: String::new(),
            title: String::new(),
            cwd: String::new(),
            model: String::new(),
            path,
            last_activity: DateTime::UNIX_EPOCH,
            turn_open: false,
            turn_known: false,
            events: VecDeque::new(),
            dropped: 0,
            malformed: 0,
            inherited_skipped: 0,
            bytes_read: 0,
            file_size: 0,
            token_usage: None,
            usage_updated_at: None,
            context: ContextUsage::default(),
            context_updated_at: None,
            message_usage: HashMap::new(),
            usage_history: VecDeque::new(),
            context_history: VecDeque::new(),
            own_history_start: None,
            metadata_seen: false,
        }
    }
    pub fn label(&self) -> String {
        if !self.agent_path.is_empty() {
            self.agent_path
                .rsplit('/')
                .next()
                .unwrap_or(&self.agent_path)
                .to_owned()
        } else if !self.title.is_empty() {
            one_line(&self.title, 70)
        } else {
            one_line(&self.id, 36)
        }
    }
    pub fn status(&self, now: DateTime<Utc>) -> &'static str {
        if self.turn_open {
            if now.signed_duration_since(self.last_activity).num_seconds() <= 120 {
                "WORKING"
            } else {
                "QUIET"
            }
        } else if self.turn_known {
            "READY"
        } else {
            "UNKNOWN"
        }
    }
    pub fn record_usage(&mut self, at: DateTime<Utc>) {
        self.usage_updated_at = Some(at);
        if let Some(usage) = self.token_usage {
            self.usage_history.push_back((at, usage));
            // Existing event snapshots are independent of this lookup history.
            // A very old backdated event gets an unknown value, never future usage.
            while self.usage_history.len() > 4096 {
                self.usage_history.pop_front();
            }
        }
    }
    pub fn record_context(&mut self, at: DateTime<Utc>) {
        self.context_updated_at = Some(at);
        self.context_history.push_back((at, self.context));
        while self.context_history.len() > 4096 {
            self.context_history.pop_front();
        }
    }
    pub fn clear_context(&mut self, at: DateTime<Utc>) {
        self.context.input_tokens = None;
        self.context.output_tokens = 0;
        self.record_context(at);
    }
    pub fn push(&mut self, mut event: FlowEvent, limit: usize) {
        if self.events.iter().any(|e| e.id == event.id) {
            return;
        }
        if let Some((at, usage)) = self
            .usage_history
            .iter()
            .rev()
            .find(|(at, _)| *at <= event.time)
        {
            event.token_usage_at_event = Some(*usage);
            event.usage_recorded_at = Some(*at);
        }
        if let Some((at, context)) = self
            .context_history
            .iter()
            .rev()
            .find(|(at, _)| *at <= event.time)
        {
            event.context_at_event = Some(*context);
            event.context_recorded_at = Some(*at);
        }
        self.last_activity = self.last_activity.max(event.time);
        self.events.push_back(event);
        while self.events.len() > limit.max(1) {
            self.events.pop_front();
            self.dropped += 1;
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Snapshot {
    pub sessions: Vec<Arc<Session>>,
    pub warnings: Vec<String>,
    pub discovered: usize,
    pub scanned_at: DateTime<Utc>,
}

pub fn clean(value: &str) -> String {
    let stripped = strip_ansi_escapes::strip_str(value);
    let safe: String = stripped
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    if safe.len() <= TEXT_LIMIT {
        safe
    } else {
        let mut end = TEXT_LIMIT;
        while !safe.is_char_boundary(end) {
            end -= 1;
        }
        format!(
            "{}\n[display truncated at 64 KiB; full content in source transcript]",
            &safe[..end]
        )
    }
}

pub fn one_line(value: &str, max: usize) -> String {
    let s = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = s.chars();
    let mut result: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}
