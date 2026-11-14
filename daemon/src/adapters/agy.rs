//! Antigravity CLI adapter (`agy -p --output-format stream-json`).

use serde_json::{json, Value};

use super::{clip, first_line, CommandSpec, LineParser, Parsed, TurnCtx};
use crate::model::{truncate, EventKind, NewEvent, Usage};
use crate::security;

pub struct AgyParser {
    agent_id: String,
}

fn is_edit_tool(name: &str) -> bool {
    ["edit", "write", "create", "replace", "delete_file"]
        .iter()
        .any(|part| name.contains(part))
}

fn is_read_tool(name: &str) -> bool {
    ["read", "view", "list_dir", "search", "grep", "find"]
        .iter()
        .any(|part| name.contains(part))
}

impl AgyParser {
    pub fn new(agent_id: &str) -> Self {
        Self {
            agent_id: agent_id.to_string(),
        }
    }

    fn event(&self, kind: EventKind, summary: impl Into<String>) -> NewEvent {
        NewEvent::new(&self.agent_id, kind, summary)
    }

    fn step(&self, step: &Value) -> Vec<Parsed> {
        let state = step.get("state").and_then(Value::as_str).unwrap_or("");
        let mut out = Vec::new();
        if step.get("step_type").and_then(Value::as_str) == Some("agent_response") {
            if state == "DONE" {
                let usage = step.get("usage");
                let tokens = |key: &str| {
                    usage
                        .and_then(|u| u.get(key))
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                };
                let mut event = self.event(EventKind::Thinking, "Pensando");
                if usage.is_some() {
                    event = event.usage(Usage {
                        tokens_in: tokens("input_tokens"),
                        tokens_out: tokens("output_tokens") + tokens("thinking_tokens"),
                        tokens_cache: tokens("cache_read_tokens"),
                        cost_usd: 0.0,
                        cost_source: "estimated".to_string(),
                    });
                }
                out.push(Parsed::Event(event));
            }
            return out;
        }
        if step.get("step_type").and_then(Value::as_str) != Some("tool") {
            return out;
        }
        let name = step.get("tool_name").and_then(Value::as_str).unwrap_or("");
        let info = step.get("tool_info");
        let param = |key: &str| {
            info.and_then(|i| i.pointer(&format!("/parameters/{key}")))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let output = info
            .and_then(|i| i.get("output"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let own_mcp = name == "call_mcp_tool" && param("ServerName") == "regente";
        match state {
            "ACTIVE" => {
                if name == "run_command" {
                    let command = param("CommandLine");
                    out.push(Parsed::Event(
                        self.event(EventKind::Command, truncate(&command, 160))
                            .target(command),
                    ));
                } else if name == "call_mcp_tool" {
                    if !own_mcp {
                        let label = format!("{}/{}", param("ServerName"), param("ToolName"));
                        out.push(Parsed::Event(
                            self.event(EventKind::Command, format!("MCP {label}"))
                                .target(format!("mcp:{label}")),
                        ));
                    }
                } else if !is_edit_tool(name) && is_read_tool(name) {
                    let path = param("Path");
                    // agy reads the tool description file before each MCP call: not project work.
                    if !path.contains("/antigravity-cli/mcp/") {
                        out.push(Parsed::Event(
                            self.event(EventKind::FileRead, format!("Leu {path}"))
                                .target(path),
                        ));
                    }
                }
            }
            "DONE" => {
                if is_edit_tool(name) {
                    let path = param("Path");
                    // agy gives no diff; the runner fills in the line counts from `git diff`.
                    out.push(Parsed::Event(
                        self.event(EventKind::FileEdit, format!("Editou {path}"))
                            .target(path),
                    ));
                } else if name == "run_command" {
                    let body = output
                        .lines()
                        .filter(|line| {
                            !line.starts_with("Created At:") && !line.starts_with("Completed At:")
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    out.push(Parsed::Event(
                        self.event(EventKind::CommandResult, first_line(&body, "(sem saída)"))
                            .target(param("CommandLine"))
                            .with("output", json!(clip(&body, 4000))),
                    ));
                }
            }
            "ERROR" => {
                let reason = info
                    .and_then(|i| i.pointer("/error/message"))
                    .and_then(Value::as_str)
                    .unwrap_or("a ferramenta falhou");
                out.push(Parsed::Event(
                    self.event(
                        EventKind::Error,
                        format!("{name}: {}", truncate(reason, 200)),
                    )
                    .target(name),
                ));
            }
            _ => {}
        }
        out
    }
}

impl LineParser for AgyParser {
    fn parse_line(&mut self, line: &str) -> Vec<Parsed> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        match value.get("event").and_then(Value::as_str).unwrap_or("") {
            "init" => value
                .get("conversation_id")
                .and_then(Value::as_str)
                .map(|id| vec![Parsed::Session(id.to_string())])
                .unwrap_or_default(),
            "step_update" => value
                .get("step_update")
                .map(|step| self.step(step))
                .unwrap_or_default(),
            "result" => {
                let result = value.get("result").cloned().unwrap_or(Value::Null);
                let mut out: Vec<Parsed> = result
                    .get("denied_actions")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|denied| {
                        let action = denied
                            .get("display_name")
                            .or_else(|| denied.get("action"))
                            .and_then(Value::as_str)
                            .unwrap_or("ação");
                        Parsed::Event(
                            self.event(
                                EventKind::Error,
                                format!("Ação negada em modo headless: {action}"),
                            )
                            .target(action),
                        )
                    })
                    .collect();
                let response = result
                    .get("response")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if result.get("status").and_then(Value::as_str) == Some("SUCCESS") {
                    out.push(Parsed::Done(response));
                } else {
                    let status = result
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("erro");
                    out.push(Parsed::Failed(if response.is_empty() {
                        status.to_string()
                    } else {
                        response
                    }));
                }
                out
            }
            _ => Vec::new(),
        }
    }
}

/// Command line of one agy turn. The messenger reaches agy through the stdio
/// bridge registered once in its global MCP config; the agent's identity
/// travels in `REGENTE_MCP_URL`.
pub fn command(ctx: &TurnCtx) -> CommandSpec {
    let extra: Vec<String> = ctx
        .agent
        .permissions
        .get("agy_allow")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut args: Vec<String> = vec![
        "-p".into(),
        ctx.prompt.clone(),
        "--output-format".into(),
        "stream-json".into(),
        "--mode".into(),
        "accept-edits".into(),
        "--settings".into(),
        security::agy_settings(&extra).to_string(),
    ];
    if let Some(conversation) = &ctx.resume {
        args.extend(["--conversation".to_string(), conversation.clone()]);
    }
    let model = ctx.model();
    if !model.is_empty() {
        args.extend(["--model".to_string(), model]);
    }
    let login_key = ctx
        .account
        .as_ref()
        .is_some_and(|account| account.login_type == "api_key");
    CommandSpec {
        program: ctx.program(),
        args,
        env: vec![("REGENTE_MCP_URL".to_string(), ctx.mcp_url.clone())],
        keep_env: if login_key {
            vec!["GEMINI_API_KEY", "GOOGLE_API_KEY"]
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
        let parsed = parse_fixture("agy", "agy/basic.jsonl");
        assert!(matches!(&parsed[0], Parsed::Session(id) if id.len() == 36));
        let all = events(&parsed);
        let kinds = kinds(&parsed);
        for kind in [
            EventKind::Thinking,
            EventKind::FileRead,
            EventKind::FileEdit,
            EventKind::Command,
            EventKind::CommandResult,
        ] {
            assert!(kinds.contains(&kind), "missing {kind:?}");
        }
        assert!(!kinds.contains(&EventKind::Error));
        let read = all.iter().find(|e| e.kind == EventKind::FileRead).unwrap();
        assert_eq!(read.target.as_deref(), Some("/work/repo/notes.txt"));
        let edit = all.iter().find(|e| e.kind == EventKind::FileEdit).unwrap();
        assert_eq!(edit.target.as_deref(), Some("/work/repo/notes.txt"));
        let command = all
            .iter()
            .find(|e| e.kind == EventKind::Command && e.target.as_deref() == Some("ls"))
            .unwrap();
        assert_eq!(command.summary, "ls");
        assert!(all
            .iter()
            .any(|e| e.target.as_deref() == Some("mcp:regente_test/ping")));
        let tokens: i64 = all
            .iter()
            .filter_map(|e| e.usage.as_ref())
            .map(|u| u.tokens_in)
            .sum();
        assert_eq!(tokens, 15170, "per-step usage adds up to the result total");
        assert_eq!(parsed.last(), Some(&Parsed::Done("ok".to_string())));
    }

    #[test]
    fn headless_denials_become_errors() {
        let command_denied = parse_fixture("agy", "agy/command-denied.jsonl");
        assert!(events(&command_denied)
            .iter()
            .any(|e| e.kind == EventKind::Error && e.summary.contains("RunCommand")));
        let mcp_denied = parse_fixture("agy", "agy/mcp-denied.jsonl");
        let errors: Vec<_> = events(&mcp_denied)
            .into_iter()
            .filter(|e| e.kind == EventKind::Error)
            .collect();
        assert!(errors
            .iter()
            .any(|e| e.summary.contains("permission check failed")));
        assert!(errors.iter().any(|e| e.summary.contains("McpTool")));
    }

    #[test]
    fn own_messenger_calls_are_not_duplicated() {
        let parser = AgyParser::new("ag");
        let step: Value = serde_json::from_str(
            r#"{"state":"ACTIVE","step_type":"tool","tool_name":"call_mcp_tool","tool_info":{"parameters":{"ServerName":"regente","ToolName":"report_done"}}}"#,
        )
        .unwrap();
        assert!(parser.step(&step).is_empty());
        let mut parser = AgyParser::new("ag");
        let failed =
            parser.parse_line(r#"{"event":"result","result":{"status":"ERROR","response":""}}"#);
        assert_eq!(failed, vec![Parsed::Failed("ERROR".into())]);
    }
}
