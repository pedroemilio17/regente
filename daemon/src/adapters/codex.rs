//! Codex adapter (`codex exec --json`).

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};

use super::{clip, first_line, CommandSpec, LineParser, Parsed, TurnCtx};
use crate::model::{full_text, truncate, EventKind, NewEvent, Usage};
use crate::security;

pub struct CodexParser {
    agent_id: String,
    last_message: String,
}

/// Codex wraps every command in `/bin/bash -lc '…'`.
fn strip_shell(command: &str) -> String {
    for prefix in [
        "/bin/bash -lc ",
        "bash -lc ",
        "/bin/sh -c ",
        "sh -c ",
        "/bin/zsh -lc ",
    ] {
        if let Some(rest) = command.strip_prefix(prefix) {
            let rest = rest.trim();
            let unquoted = rest
                .strip_prefix('\'')
                .and_then(|r| r.strip_suffix('\''))
                .or_else(|| rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')))
                .unwrap_or(rest);
            return unquoted.to_string();
        }
    }
    command.to_string()
}

/// Codex has no file-read event: reads show up as `cat`, `sed -n`, `head`…
fn read_target(command: &str) -> Option<String> {
    if command.contains(['|', ';', '&', '>']) {
        return None;
    }
    let mut words = command.split_whitespace();
    let program = words.next()?;
    let is_read = match program {
        "cat" | "head" | "tail" | "nl" | "less" => true,
        "sed" => command.contains(" -n "),
        _ => false,
    };
    if !is_read {
        return None;
    }
    words
        .rfind(|word| {
            !word.starts_with('-') && !word.starts_with('\'') && word.parse::<i64>().is_err()
        })
        .map(str::to_string)
}

impl CodexParser {
    pub fn new(agent_id: &str) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            last_message: String::new(),
        }
    }

    fn event(&self, kind: EventKind, summary: impl Into<String>) -> NewEvent {
        NewEvent::new(&self.agent_id, kind, summary)
    }

    fn item(&mut self, phase: &str, item: &Value) -> Vec<Parsed> {
        let text = |key: &str| {
            item.get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let started = phase == "item.started";
        let mut out = Vec::new();
        match item.get("type").and_then(Value::as_str).unwrap_or("") {
            "agent_message" | "reasoning" if !started => {
                let message = text("text");
                if !message.trim().is_empty() {
                    out.push(Parsed::Event(
                        self.event(EventKind::Thinking, truncate(&message, 200))
                            .with("text", json!(full_text(&message))),
                    ));
                    if item.get("type").and_then(Value::as_str) == Some("agent_message") {
                        self.last_message = message;
                    }
                }
            }
            "command_execution" => {
                let command = strip_shell(&text("command"));
                let read = read_target(&command);
                if started {
                    out.push(Parsed::Event(match read {
                        Some(file) => self
                            .event(EventKind::FileRead, format!("Leu {file}"))
                            .target(file),
                        None => self
                            .event(EventKind::Command, truncate(&command, 160))
                            .target(command),
                    }));
                } else if read.is_none() {
                    let output = text("aggregated_output");
                    let exit = item.get("exit_code").and_then(Value::as_i64);
                    let fallback = exit
                        .map(|code| format!("exit {code}"))
                        .unwrap_or_else(|| "(sem saída)".to_string());
                    out.push(Parsed::Event(
                        self.event(EventKind::CommandResult, first_line(&output, &fallback))
                            .target(command)
                            .with("output", json!(clip(&output, 4000)))
                            .with("exit_code", json!(exit)),
                    ));
                }
            }
            "file_change" if !started => {
                for change in item
                    .get("changes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let path = change
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let verb = match change
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("update")
                    {
                        "add" => "Criou",
                        "delete" => "Apagou",
                        _ => "Editou",
                    };
                    // Codex gives no line counts; the runner fills them in from `git diff`.
                    out.push(Parsed::Event(
                        self.event(EventKind::FileEdit, format!("{verb} {path}"))
                            .target(path),
                    ));
                }
            }
            "mcp_tool_call" => {
                let label = format!("{}/{}", text("server"), text("tool"));
                let own = item.get("server").and_then(Value::as_str) == Some("regente");
                if started && !own {
                    out.push(Parsed::Event(
                        self.event(EventKind::Command, format!("MCP {label}"))
                            .target(format!("mcp:{label}")),
                    ));
                } else if !started && item.get("status").and_then(Value::as_str) == Some("failed") {
                    let reason = item
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("falhou");
                    out.push(Parsed::Event(
                        self.event(
                            EventKind::Error,
                            format!("MCP {label}: {}", truncate(reason, 200)),
                        )
                        .target(format!("mcp:{label}")),
                    ));
                }
            }
            "error" if !started => {
                out.push(Parsed::Event(
                    self.event(EventKind::Error, truncate(&text("message"), 200)),
                ));
            }
            _ => {}
        }
        out
    }
}

/// Plan utilization from a `token_count` event: `rate_limits.primary/secondary.
/// {used_percent, window_minutes, resets_at}` under `payload` (confirmed in the
/// session JSONL, 15/11/2026, codex-cli 0.166.0). Only numbers are read.
pub(crate) fn rate_limits(value: &Value) -> Vec<Parsed> {
    let Some(limits) = ["/rate_limits", "/msg/rate_limits", "/payload/rate_limits"]
        .iter()
        .find_map(|path| value.pointer(path))
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for key in ["primary", "secondary"] {
        let Some(info) = limits.get(key) else {
            continue;
        };
        let Some(used) = info.get("used_percent").and_then(Value::as_f64) else {
            continue;
        };
        let window = match info.get("window_minutes").and_then(Value::as_i64) {
            Some(300) => "5h",
            Some(10080) => "week",
            _ => continue,
        };
        let resets_at = info
            .get("resets_at")
            .and_then(Value::as_i64)
            .or_else(|| {
                info.get("resets_in_seconds")
                    .and_then(Value::as_i64)
                    .map(|secs| Utc::now().timestamp() + secs)
            })
            .and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0))
            .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Millis, true));
        out.push(Parsed::RateLimit {
            window,
            pct: (used / 100.0).clamp(0.0, 1.0),
            resets_at,
        });
    }
    out
}

/// Newest `.jsonl` under the sessions dir (`YYYY/MM/DD/…`), descending by name.
fn newest_session(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            let modified = entry.metadata().and_then(|m| m.modified()).ok()?;
            files.push((modified, path));
        }
    }
    dirs.sort();
    for sub in dirs.iter().rev() {
        if let Some(found) = newest_session(sub) {
            return Some(found);
        }
    }
    files.into_iter().max().map(|(_, path)| path)
}

/// Codex has no usage command: read the last `token_count` of the most recent
/// session file. Windows already reset are dropped. No conversation text is kept.
pub fn latest_rate_limits(codex_home: &std::path::Path) -> Vec<Parsed> {
    use std::io::{Read, Seek, SeekFrom};
    let Some(path) = newest_session(&codex_home.join("sessions")) else {
        return Vec::new();
    };
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let tail = len.min(2 * 1024 * 1024);
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(len - tail)).is_err() || file.read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let now = Utc::now();
    for line in text.lines().rev() {
        if !line.contains("rate_limits") {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.pointer("/payload/type").and_then(Value::as_str) != Some("token_count") {
            continue;
        }
        let limits = rate_limits(&value);
        if limits.is_empty() {
            continue;
        }
        return limits
            .into_iter()
            .filter(|item| match item {
                Parsed::RateLimit { resets_at, .. } => resets_at
                    .as_deref()
                    .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
                    .is_some_and(|at| at > now),
                _ => false,
            })
            .collect();
    }
    Vec::new()
}

impl LineParser for CodexParser {
    fn parse_line(&mut self, line: &str) -> Vec<Parsed> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "event_msg" => rate_limits(&value),
            "thread.started" => value
                .get("thread_id")
                .and_then(Value::as_str)
                .map(|id| vec![Parsed::Session(id.to_string())])
                .unwrap_or_default(),
            "item.started" | "item.completed" => value
                .get("item")
                .map(|item| self.item(kind, item))
                .unwrap_or_default(),
            "turn.completed" => {
                let usage = value.get("usage");
                let tokens = |key: &str| {
                    usage
                        .and_then(|u| u.get(key))
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                };
                let cached = tokens("cached_input_tokens");
                vec![
                    // Almost all input is cache: count it apart so it is priced as cache.
                    Parsed::Usage(Usage {
                        tokens_in: (tokens("input_tokens") - cached).max(0),
                        tokens_out: tokens("output_tokens"),
                        tokens_cache: cached,
                        cost_usd: 0.0,
                        cost_source: "estimated".to_string(),
                    }),
                    Parsed::Done(self.last_message.clone()),
                ]
            }
            "turn.failed" | "error" => {
                let reason = value
                    .pointer("/error/message")
                    .or_else(|| value.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("o turno falhou")
                    .to_string();
                vec![Parsed::Failed(reason)]
            }
            _ => Vec::new(),
        }
    }
}

/// Command line of one Codex turn: writes only inside the worktree, network off.
pub fn command(ctx: &TurnCtx) -> CommandSpec {
    let mut args: Vec<String> = vec!["exec".into()];
    if let Some(session) = &ctx.resume {
        args.extend(["resume".to_string(), session.clone()]);
    }
    args.extend(["--json".to_string(), "--skip-git-repo-check".to_string()]);
    for setting in security::codex_config(&ctx.mcp_url) {
        args.extend(["-c".to_string(), setting]);
    }
    let model = ctx.model();
    if !model.is_empty() {
        args.extend(["-m".to_string(), model]);
    }
    if let Some(effort) = ctx.effort() {
        args.extend(["-c".to_string(), format!("model_reasoning_effort={effort}")]);
    }
    args.push(ctx.prompt.clone());
    let login_key = ctx
        .account
        .as_ref()
        .is_some_and(|account| account.login_type == "api_key");
    CommandSpec {
        program: ctx.program(),
        args,
        env: Vec::new(),
        keep_env: if login_key {
            vec!["OPENAI_API_KEY"]
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
    fn session_file_feeds_the_windows() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/11/15");
        std::fs::create_dir_all(&day).unwrap();
        let fixture =
            crate::app::testing::fixtures_dir().join("codex/session-rate-limits.synthetic.jsonl");
        std::fs::copy(fixture, day.join("rollout.jsonl")).unwrap();
        let limits = latest_rate_limits(dir.path());
        // The week window of the last event already reset, so only 5h remains.
        assert_eq!(limits.len(), 1);
        assert!(matches!(
            &limits[0],
            Parsed::RateLimit { window: "5h", pct, resets_at: Some(_) } if (*pct - 0.06).abs() < 1e-9
        ));
        assert!(latest_rate_limits(&dir.path().join("none")).is_empty());
    }

    #[test]
    fn fixture_produces_the_expected_sequence() {
        let parsed = parse_fixture("codex", "codex/basic.jsonl");
        assert!(matches!(&parsed[0], Parsed::Session(_)));
        assert_eq!(
            kinds(&parsed),
            vec![
                EventKind::Thinking,
                EventKind::FileRead,
                EventKind::FileEdit,
                EventKind::Command,
                EventKind::CommandResult,
                EventKind::Command,
                EventKind::Thinking,
            ]
        );
        let all = events(&parsed);
        assert_eq!(all[1].target.as_deref(), Some("notes.txt"));
        assert_eq!(all[2].target.as_deref(), Some("/work/repo/notes.txt"));
        assert_eq!(all[2].lines_added, None, "line counts come from git diff");
        assert_eq!(all[3].target.as_deref(), Some("ls"));
        assert_eq!(all[4].summary, "notes.txt");
        assert_eq!(all[5].target.as_deref(), Some("mcp:regente_test/ping"));
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
        assert_eq!(usage.cost_source, "estimated");
        assert_eq!(usage.tokens_cache, 115328);
        assert_eq!(usage.tokens_in, 129686 - 115328);
        assert_eq!(parsed.last(), Some(&Parsed::Done("ok".to_string())));
    }

    #[test]
    fn denied_mcp_call_becomes_an_error() {
        let parsed = parse_fixture("codex", "codex/mcp-denied.jsonl");
        let all = events(&parsed);
        let error = all.iter().find(|e| e.kind == EventKind::Error).unwrap();
        assert!(error.summary.contains("requires approval"));
    }

    #[test]
    fn shell_wrapper_and_reads() {
        assert_eq!(
            strip_shell("/bin/bash -lc 'cat notes.txt'"),
            "cat notes.txt"
        );
        assert_eq!(strip_shell("ls -la"), "ls -la");
        assert_eq!(read_target("cat notes.txt"), Some("notes.txt".into()));
        assert_eq!(
            read_target("sed -n 1,20p src/a.ts"),
            Some("src/a.ts".into())
        );
        assert_eq!(read_target("cat a.txt | grep x"), None);
        assert_eq!(read_target("ls"), None);
    }

    #[test]
    fn failures_and_own_mcp_calls() {
        let mut parser = CodexParser::new("ag");
        let own = r#"{"type":"item.started","item":{"type":"mcp_tool_call","server":"regente","tool":"send_message"}}"#;
        assert!(parser.parse_line(own).is_empty());
        let failed =
            parser.parse_line(r#"{"type":"turn.failed","error":{"message":"rate limit"}}"#);
        assert_eq!(failed, vec![Parsed::Failed("rate limit".into())]);
        let exit = parser.parse_line(r#"{"type":"item.completed","item":{"type":"command_execution","command":"/bin/bash -lc 'false'","aggregated_output":"","exit_code":1,"status":"failed"}}"#);
        assert_eq!(events(&exit)[0].summary, "exit 1");
    }
}
