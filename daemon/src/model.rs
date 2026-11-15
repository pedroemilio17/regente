//! Core data types shared by the store, the API and the adapters.

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Contract version of the REST/WebSocket API between daemon and UI.
pub const API_VERSION: &str = "1";

/// Current UTC time as RFC 3339 with milliseconds.
pub fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// New id with a readable prefix, e.g. `evt_3f2a…`.
pub fn new_id(prefix: &str) -> String {
    let raw = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}_{}", &raw[..16])
}

/// Truncate a string to at most `max` characters, adding an ellipsis.
/// Longest full text kept in `data.text` of an event.
pub const MAX_FULL_TEXT: usize = 64 * 1024;

/// Full text for `data.text`: whitespace kept, cut at a safe size.
pub fn full_text(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(MAX_FULL_TEXT) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

pub fn truncate(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        let cut: String = flat.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Status,
    Thinking,
    FileRead,
    FileEdit,
    Command,
    CommandResult,
    MessageSent,
    MessageReceived,
    TaskAssigned,
    TaskDone,
    ApprovalRequested,
    ApprovalDecided,
    SecurityAlert,
    Error,
}

impl EventKind {
    pub const ALL: [EventKind; 14] = [
        EventKind::Status,
        EventKind::Thinking,
        EventKind::FileRead,
        EventKind::FileEdit,
        EventKind::Command,
        EventKind::CommandResult,
        EventKind::MessageSent,
        EventKind::MessageReceived,
        EventKind::TaskAssigned,
        EventKind::TaskDone,
        EventKind::ApprovalRequested,
        EventKind::ApprovalDecided,
        EventKind::SecurityAlert,
        EventKind::Error,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Status => "status",
            EventKind::Thinking => "thinking",
            EventKind::FileRead => "file_read",
            EventKind::FileEdit => "file_edit",
            EventKind::Command => "command",
            EventKind::CommandResult => "command_result",
            EventKind::MessageSent => "message_sent",
            EventKind::MessageReceived => "message_received",
            EventKind::TaskAssigned => "task_assigned",
            EventKind::TaskDone => "task_done",
            EventKind::ApprovalRequested => "approval_requested",
            EventKind::ApprovalDecided => "approval_decided",
            EventKind::SecurityAlert => "security_alert",
            EventKind::Error => "error",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == text)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub tokens_in: i64,
    #[serde(default)]
    pub tokens_out: i64,
    #[serde(default)]
    pub tokens_cache: i64,
    #[serde(default)]
    pub cost_usd: f64,
    /// `reported` when the CLI gave the cost, `estimated` when computed from tokens.
    #[serde(default = "estimated")]
    pub cost_source: String,
}

fn estimated() -> String {
    "estimated".to_string()
}

/// Normalized event: everything the UI shows comes from one of these.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    #[serde(default)]
    pub seq: i64,
    pub id: String,
    pub ts: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub kind: EventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines_added: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines_removed: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_ref: Option<String>,
    /// Extra structured detail (status value, diff text, approval id…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Event before it gets an id, a timestamp and a sequence number.
#[derive(Clone, Debug, PartialEq)]
pub struct NewEvent {
    pub agent_id: String,
    pub task_id: Option<String>,
    pub kind: EventKind,
    pub target: Option<String>,
    pub summary: String,
    pub lines_added: Option<i64>,
    pub lines_removed: Option<i64>,
    pub usage: Option<Usage>,
    pub raw_ref: Option<String>,
    pub data: Option<Value>,
}

impl NewEvent {
    pub fn new(agent_id: &str, kind: EventKind, summary: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            task_id: None,
            kind,
            target: None,
            summary: summary.into(),
            lines_added: None,
            lines_removed: None,
            usage: None,
            raw_ref: None,
            data: None,
        }
    }

    /// Status event; `status` is one of working, thinking, waiting, idle, paused.
    pub fn status(agent_id: &str, status: &str, summary: impl Into<String>) -> Self {
        Self::new(agent_id, EventKind::Status, summary)
            .data(serde_json::json!({ "status": status }))
    }

    pub fn task(mut self, task_id: Option<&str>) -> Self {
        self.task_id = task_id.map(str::to_string);
        self
    }

    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn lines(mut self, added: i64, removed: i64) -> Self {
        self.lines_added = Some(added);
        self.lines_removed = Some(removed);
        self
    }

    pub fn usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    /// Merge extra keys into `data`, keeping what is already there.
    pub fn with(mut self, key: &str, value: Value) -> Self {
        let mut map = match self.data.take() {
            Some(Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        };
        map.insert(key.to_string(), value);
        self.data = Some(Value::Object(map));
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub root: String,
    pub base_branch: String,
    pub context: String,
    pub folders: Value,
    pub blocked: Value,
    pub day_budget_usd: Option<f64>,
    pub created_at: String,
}

impl Project {
    pub fn blocked_patterns(&self) -> Vec<String> {
        self.blocked
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub provider: String,
    pub login_type: String,
    pub cli_path: String,
    pub default_model: String,
    pub limit_rule: Value,
    pub max_concurrent: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub cli: String,
    pub role: String,
    pub is_orchestrator: bool,
    pub parent_id: Option<String>,
    pub account_id: Option<String>,
    pub model: String,
    pub pet: Value,
    pub permissions: Value,
    pub settings: Value,
    pub budget_usd: Option<f64>,
    pub paused: bool,
    pub pause_reason: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub parent_task_id: Option<String>,
    pub root_task_id: String,
    pub owner_agent_id: String,
    pub title: String,
    pub brief: String,
    pub done_criteria: String,
    /// queued, running, waiting, blocked, done, cancelled
    pub status: String,
    pub branch: Option<String>,
    pub budget_usd: Option<f64>,
    pub depends_on: Vec<String>,
    pub summary: Option<String>,
    pub message_count: i64,
    pub stalled_rounds: i64,
    pub created_at: String,
    pub finished_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub task_id: String,
    pub cli_session_id: Option<String>,
    pub worktree: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub task_id: Option<String>,
    pub from_agent: String,
    pub to_agent: String,
    pub text: String,
    pub created_at: String,
    pub delivered_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    pub id: String,
    pub agent_id: String,
    pub task_id: Option<String>,
    /// merge, command, push…
    pub action: String,
    pub detail: String,
    /// pending, approved, denied
    pub status: String,
    pub decided_by: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageRow {
    pub ts: String,
    pub agent_id: String,
    pub account_id: Option<String>,
    pub task_id: Option<String>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub tokens_cache: i64,
    pub cost_usd: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub event_id: String,
    pub agent_id: String,
    pub ts: String,
    pub path: String,
    pub lines_added: i64,
    pub lines_removed: i64,
    pub diff_ref: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip() {
        for kind in EventKind::ALL {
            assert_eq!(EventKind::parse(kind.as_str()), Some(kind));
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
        }
        assert_eq!(EventKind::parse("nope"), None);
    }

    #[test]
    fn truncate_flattens_and_cuts() {
        assert_eq!(truncate("a\n b", 10), "a b");
        assert_eq!(truncate("abcdefghij", 5), "abcd…");
    }

    #[test]
    fn event_serializes_like_the_contract() {
        let event = Event {
            seq: 1,
            id: "evt_1".into(),
            ts: "2026-11-14T18:13:02Z".into(),
            agent_id: "ag_backend".into(),
            task_id: Some("tk_sensores".into()),
            kind: EventKind::FileEdit,
            target: Some("src/api/sensores.ts".into()),
            summary: "Adicionou from/to na rota".into(),
            lines_added: Some(46),
            lines_removed: Some(12),
            usage: Some(Usage {
                tokens_in: 1200,
                tokens_out: 340,
                tokens_cache: 900,
                cost_usd: 0.012,
                cost_source: "estimated".into(),
            }),
            raw_ref: None,
            data: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["kind"], "file_edit");
        assert_eq!(json["usage"]["cost_source"], "estimated");
        assert!(json.get("raw_ref").is_none());
        let back: Event = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn new_event_with_merges_data() {
        let event = NewEvent::status("ag", "working", "x").with("reason", serde_json::json!("y"));
        let data = event.data.unwrap();
        assert_eq!(data["status"], "working");
        assert_eq!(data["reason"], "y");
    }
}
