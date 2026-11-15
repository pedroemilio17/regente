//! Claude Code adapter (`claude -p --output-format stream-json --verbose`).

use std::collections::HashMap;

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};

use super::{clip, first_line, CommandSpec, LineParser, Parsed, TurnCtx};
use crate::model::{full_text, truncate, EventKind, NewEvent, Usage};
use crate::security;

pub struct ClaudeParser {
    agent_id: String,
    /// Tool calls waiting for their result: id -> (tool name, input).
    pending: HashMap<String, (String, Value)>,
}

impl ClaudeParser {
    pub fn new(agent_id: &str) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            pending: HashMap::new(),
        }
    }

    fn event(&self, kind: EventKind, summary: impl Into<String>) -> NewEvent {
        NewEvent::new(&self.agent_id, kind, summary)
    }

    fn tool_use(&mut self, block: &Value) -> Option<Parsed> {
        let name = block.get("name").and_then(Value::as_str).unwrap_or("");
        let id = block
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let input = block.get("input").cloned().unwrap_or(Value::Null);
        let text = |key: &str| {
            input
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        match name {
            "Read" => {
                let file = text("file_path");
                Some(Parsed::Event(
                    self.event(EventKind::FileRead, format!("Leu {file}"))
                        .target(file),
                ))
            }
            "Glob" | "Grep" => {
                let pattern = text("pattern");
                Some(Parsed::Event(
                    self.event(EventKind::FileRead, format!("Buscou {pattern}"))
                        .target(pattern),
                ))
            }
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
                self.pending.insert(id, (name.to_string(), input));
                None
            }
            "Bash" => {
                let command = text("command");
                self.pending.insert(id, (name.to_string(), input));
                Some(Parsed::Event(
                    self.event(EventKind::Command, truncate(&command, 160))
                        .target(command),
                ))
            }
            "ToolSearch" | "TodoWrite" => None,
            _ if name.starts_with("mcp__regente__") => None,
            _ if name.starts_with("mcp__") => {
                let label = name.trim_start_matches("mcp__").replacen("__", "/", 1);
                Some(Parsed::Event(
                    self.event(EventKind::Command, format!("MCP {label}"))
                        .target(format!("mcp:{label}")),
                ))
            }
            _ => Some(Parsed::Event(
                self.event(EventKind::Command, format!("Ferramenta {name}"))
                    .target(name),
            )),
        }
    }

    fn tool_result(&mut self, block: &Value, result: &Value) -> Option<Parsed> {
        let id = block.get("tool_use_id").and_then(Value::as_str)?;
        let (name, input) = self.pending.remove(id)?;
        let content = match block.get("content") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        if block.get("is_error").and_then(Value::as_bool) == Some(true) {
            return Some(Parsed::Event(
                self.event(
                    EventKind::Error,
                    format!("{name}: {}", first_line(&content, "falhou")),
                )
                .target(name),
            ));
        }
        if name == "Bash" {
            let command = input
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let stdout = result
                .get("stdout")
                .and_then(Value::as_str)
                .unwrap_or(&content);
            let event = self
                .event(EventKind::CommandResult, first_line(stdout, "(sem saída)"))
                .target(command)
                .with("output", json!(clip(stdout, 4000)));
            return Some(Parsed::Event(event));
        }
        let file = input
            .get("file_path")
            .or_else(|| input.get("notebook_path"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let (mut added, mut removed, mut diff) = (0, 0, Vec::new());
        if let Some(hunks) = result.get("structuredPatch").and_then(Value::as_array) {
            for line in hunks
                .iter()
                .filter_map(|hunk| hunk.get("lines").and_then(Value::as_array))
                .flatten()
            {
                let Some(line) = line.as_str() else { continue };
                if line.starts_with('+') {
                    added += 1;
                } else if line.starts_with('-') {
                    removed += 1;
                }
                diff.push(line.to_string());
            }
        }
        if added == 0 && removed == 0 {
            if let Some(new_content) = input.get("content").and_then(Value::as_str) {
                added = new_content.lines().count() as i64;
                diff = new_content.lines().map(|line| format!("+{line}")).collect();
            }
        }
        let verb = if name == "Write" {
            "Escreveu"
        } else {
            "Editou"
        };
        let mut event = self
            .event(EventKind::FileEdit, format!("{verb} {file}"))
            .target(file)
            .lines(added, removed);
        if !diff.is_empty() {
            event = event.with("diff", json!(clip(&diff.join("\n"), 12000)));
        }
        Some(Parsed::Event(event))
    }
}

fn epoch_to_rfc3339(seconds: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Millis, true))
}

impl LineParser for ClaudeParser {
    fn parse_line(&mut self, line: &str) -> Vec<Parsed> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "system" => {
                if value.get("subtype").and_then(Value::as_str) == Some("init") {
                    if let Some(id) = value.get("session_id").and_then(Value::as_str) {
                        out.push(Parsed::Session(id.to_string()));
                    }
                }
            }
            "rate_limit_event" => {
                let windows = value.pointer("/rate_limit_info/unifiedWindows");
                for (key, window) in [("five_hour", "5h"), ("seven_day", "week")] {
                    let Some(info) = windows.and_then(|w| w.get(key)) else {
                        continue;
                    };
                    if let Some(pct) = info.get("utilization").and_then(Value::as_f64) {
                        let resets_at = info
                            .get("resetsAt")
                            .and_then(Value::as_i64)
                            .and_then(epoch_to_rfc3339);
                        out.push(Parsed::RateLimit {
                            window,
                            pct,
                            resets_at,
                        });
                    }
                }
            }
            "assistant" => {
                for block in value
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    match block.get("type").and_then(Value::as_str).unwrap_or("") {
                        "thinking" => {
                            let text = block.get("thinking").and_then(Value::as_str).unwrap_or("");
                            let summary = if text.trim().is_empty() {
                                "Pensando".to_string()
                            } else {
                                truncate(text, 160)
                            };
                            out.push(Parsed::Event(self.event(EventKind::Thinking, summary)));
                        }
                        "text" => {
                            let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                            if !text.trim().is_empty() {
                                out.push(Parsed::Event(
                                    self.event(EventKind::Thinking, truncate(text, 200))
                                        .with("text", json!(full_text(text))),
                                ));
                            }
                        }
                        "tool_use" => out.extend(self.tool_use(block)),
                        _ => {}
                    }
                }
            }
            "user" => {
                let result = value.get("tool_use_result").cloned().unwrap_or(Value::Null);
                for block in value
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                        out.extend(self.tool_result(block, &result));
                    }
                }
            }
            "result" => {
                let usage = value.get("usage");
                let tokens = |key: &str| {
                    usage
                        .and_then(|u| u.get(key))
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                };
                let cost = value.get("total_cost_usd").and_then(Value::as_f64);
                out.push(Parsed::Usage(Usage {
                    tokens_in: tokens("input_tokens") + tokens("cache_creation_input_tokens"),
                    tokens_out: tokens("output_tokens"),
                    tokens_cache: tokens("cache_read_input_tokens"),
                    cost_usd: cost.unwrap_or(0.0),
                    cost_source: if cost.is_some() {
                        "reported"
                    } else {
                        "estimated"
                    }
                    .to_string(),
                }));
                let text = value
                    .get("result")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if value.get("is_error").and_then(Value::as_bool) == Some(true) {
                    let reason = if text.is_empty() {
                        value
                            .get("subtype")
                            .and_then(Value::as_str)
                            .unwrap_or("erro")
                            .to_string()
                    } else {
                        text
                    };
                    out.push(Parsed::Failed(reason));
                } else {
                    out.push(Parsed::Done(text));
                }
            }
            _ => {}
        }
        out
    }
}

/// Command line of one Claude Code turn. User hooks and plugins are kept out
/// (`--setting-sources project`); the Regente rules go in through `--settings`.
pub fn command(ctx: &TurnCtx) -> CommandSpec {
    let mcp = json!({ "mcpServers": { "regente": { "type": "http", "url": ctx.mcp_url } } });
    let mut args: Vec<String> = vec![
        "-p".into(),
        ctx.prompt.clone(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--setting-sources".into(),
        "project".into(),
        "--strict-mcp-config".into(),
        "--mcp-config".into(),
        mcp.to_string(),
        "--settings".into(),
        security::claude_settings(&ctx.blocked()).to_string(),
        "--permission-mode".into(),
        "acceptEdits".into(),
        "--permission-prompts".into(),
        "none".into(),
    ];
    if let Some(session) = &ctx.resume {
        args.extend(["--resume".to_string(), session.clone()]);
    }
    let model = ctx.model();
    if !model.is_empty() {
        args.extend(["--model".to_string(), model]);
    }
    // `claude --help` (2.1.x) lists `--effort <level>`: low, medium, high, xhigh, max.
    if let Some(effort) = ctx.effort().filter(|level| *level != "ultra") {
        args.extend(["--effort".to_string(), effort.to_string()]);
    }
    if let Some(left) = ctx.budget_left.filter(|left| *left > 0.0) {
        args.extend(["--max-budget-usd".to_string(), format!("{left:.2}")]);
    }
    let login_key = ctx
        .account
        .as_ref()
        .is_some_and(|account| account.login_type == "api_key");
    CommandSpec {
        program: ctx.program(),
        args,
        env: Vec::new(),
        keep_env: if login_key {
            vec!["ANTHROPIC_API_KEY"]
        } else {
            Vec::new()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{events, kinds, parse_fixture};
    use super::*;

    #[test]
    fn fixture_produces_the_expected_sequence() {
        let parsed = parse_fixture("claude", "claude/basic.jsonl");
        assert!(matches!(&parsed[0], Parsed::Session(id) if id.len() == 36));
        assert_eq!(
            kinds(&parsed),
            vec![
                EventKind::Thinking,
                EventKind::FileRead,
                EventKind::Thinking,
                EventKind::FileEdit,
                EventKind::Command,
                // The test MCP tool, called in the same message as `ls`.
                EventKind::Command,
                EventKind::CommandResult,
                EventKind::Thinking,
            ]
        );
        let all = events(&parsed);
        assert!(all
            .iter()
            .any(|e| e.target.as_deref() == Some("mcp:regente_test/ping")));
        let read = all.iter().find(|e| e.kind == EventKind::FileRead).unwrap();
        assert_eq!(read.target.as_deref(), Some("/work/repo/notes.txt"));
        let edit = all.iter().find(|e| e.kind == EventKind::FileEdit).unwrap();
        assert_eq!((edit.lines_added, edit.lines_removed), (Some(1), Some(1)));
        assert!(edit.data.as_ref().unwrap()["diff"]
            .as_str()
            .unwrap()
            .contains("+version: 2"));
        let command = all.iter().find(|e| e.kind == EventKind::Command).unwrap();
        assert_eq!(command.target.as_deref(), Some("ls"));
        let result = all
            .iter()
            .find(|e| e.kind == EventKind::CommandResult)
            .unwrap();
        assert_eq!(result.summary, "notes.txt");
    }

    #[test]
    fn fixture_reports_cost_and_plan_usage() {
        let parsed = parse_fixture("claude", "claude/basic.jsonl");
        let usage = parsed
            .iter()
            .find_map(|p| {
                if let Parsed::Usage(u) = p {
                    Some(u)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(usage.cost_source, "reported");
        assert!(usage.cost_usd > 0.0);
        assert!(usage.tokens_cache > 0);
        let limits: Vec<_> = parsed
            .iter()
            .filter_map(|p| {
                if let Parsed::RateLimit { window, pct, .. } = p {
                    Some((*window, *pct))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(limits, vec![("5h", 0.43), ("week", 0.13)]);
        assert_eq!(parsed.last(), Some(&Parsed::Done("ok".to_string())));
    }

    #[test]
    fn errors_and_other_tools() {
        let mut parser = ClaudeParser::new("ag");
        let call = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git push"}},{"type":"tool_use","id":"t2","name":"mcp__github__create_issue","input":{}},{"type":"tool_use","id":"t3","name":"mcp__regente__send_message","input":{}}]}}"#;
        let parsed = parser.parse_line(call);
        assert_eq!(kinds(&parsed), vec![EventKind::Command, EventKind::Command]);
        assert_eq!(
            events(&parsed)[1].target.as_deref(),
            Some("mcp:github/create_issue")
        );
        let denied = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"Permission to use Bash with command git push has been denied."}]}}"#;
        let parsed = parser.parse_line(denied);
        assert_eq!(kinds(&parsed), vec![EventKind::Error]);
        let failed = r#"{"type":"result","subtype":"error_max_budget_usd","is_error":true,"usage":{"input_tokens":5,"output_tokens":1}}"#;
        let parsed = parser.parse_line(failed);
        assert!(matches!(&parsed[0], Parsed::Usage(u) if u.cost_source == "estimated"));
        assert_eq!(parsed[1], Parsed::Failed("error_max_budget_usd".into()));
        assert!(parser.parse_line("not json").is_empty());
    }

    #[test]
    fn write_counts_new_lines() {
        let mut parser = ClaudeParser::new("ag");
        parser.parse_line(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"w1","name":"Write","input":{"file_path":"/w/a.ts","content":"a\nb\nc\n"}}]}}"#);
        let parsed = parser.parse_line(r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"w1","content":"ok"}]},"tool_use_result":{"type":"create"}}"#);
        let event = events(&parsed)[0].clone();
        assert_eq!(event.summary, "Escreveu /w/a.ts");
        assert_eq!((event.lines_added, event.lines_removed), (Some(3), Some(0)));
    }
}
