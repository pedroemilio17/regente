//! Fake adapter: plays a scripted scenario from `fixtures/fake/` instead of
//! running a CLI. Every automatic test uses it, so no test spends tokens.
//!
//! Its actions go through the same guards as a real agent: blocked paths,
//! leak scanning, approvals and the messenger.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::watch;

use super::{attach_diff, handle_parsed, parser_for, TurnCtx, TurnOutcome};
use crate::app::{App, AppError, AppResult};
use crate::model::{truncate, EventKind, NewEvent, Usage};
use crate::security::{self, watcher};

/// Steps of the turn `turn_index` of a scenario file.
pub fn load_turn(
    fixtures_dir: &Path,
    scenario: &str,
    turn_index: usize,
    reply: bool,
) -> AppResult<Vec<Value>> {
    let path = fixtures_dir.join("fake").join(format!("{scenario}.json"));
    let text = std::fs::read_to_string(&path).map_err(|error| {
        AppError::BadRequest(format!(
            "cenário falso '{scenario}' não encontrado ({error})"
        ))
    })?;
    let value: Value = serde_json::from_str(&text).map_err(|error| {
        AppError::BadRequest(format!("cenário falso '{scenario}' inválido: {error}"))
    })?;
    if reply {
        // Steps used when the task exists only to answer a message.
        let steps = value.get("reply").and_then(Value::as_array).cloned();
        return Ok(
            steps.unwrap_or_else(|| vec![json!({ "do": "mcp", "tool": "read_inbox", "args": {} })])
        );
    }
    let turns = value
        .get("turns")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let repeat_last = value
        .get("repeat_last")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let turn = match turns.get(turn_index) {
        Some(turn) => Some(turn),
        None if repeat_last => turns.last(),
        None => None,
    };
    Ok(turn.and_then(Value::as_array).cloned().unwrap_or_default())
}

fn fill(value: &Value, ctx: &TurnCtx) -> Value {
    match value {
        Value::String(text) => Value::String(
            text.replace("{{task_id}}", &ctx.task.id)
                .replace("{{agent}}", &ctx.agent.name)
                .replace("{{task_title}}", &ctx.task.title)
                .replace(
                    "{{peer}}",
                    ctx.agent
                        .settings
                        .get("peer")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                ),
        ),
        Value::Array(items) => Value::Array(items.iter().map(|item| fill(item, ctx)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), fill(item, ctx)))
                .collect(),
        ),
        other => other.clone(),
    }
}

async fn pause_or_cancel(cancel: &mut watch::Receiver<bool>, millis: u64) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(millis)) => *cancel.borrow(),
        _ = cancel.changed() => true,
    }
}

pub async fn run(
    app: &Arc<App>,
    ctx: &TurnCtx,
    mut cancel: watch::Receiver<bool>,
) -> AppResult<TurnOutcome> {
    let scenario = ctx
        .agent
        .settings
        .get("fake_scenario")
        .and_then(Value::as_str)
        .unwrap_or("basic")
        .to_string();
    let reply = ctx.task.done_criteria == crate::orchestrator::REPLY_CRITERIA;
    let steps = load_turn(&app.cfg.fixtures_dir, &scenario, ctx.turn_index, reply)?;
    let mut outcome = TurnOutcome::default();
    for step in steps {
        if pause_or_cancel(&mut cancel, app.cfg.fake_step_ms).await {
            outcome.cancelled = true;
            break;
        }
        let step = fill(&step, ctx);
        run_step(app, ctx, &step, &mut cancel, &mut outcome).await?;
        if outcome.cancelled || outcome.failed.is_some() {
            break;
        }
        if app.store.get_agent(&ctx.agent.id)?.paused {
            outcome.cancelled = true;
            break;
        }
    }
    Ok(outcome)
}

async fn run_step(
    app: &Arc<App>,
    ctx: &TurnCtx,
    step: &Value,
    cancel: &mut watch::Receiver<bool>,
    outcome: &mut TurnOutcome,
) -> AppResult<()> {
    let agent = ctx.agent.id.as_str();
    let task = Some(ctx.task.id.as_str());
    let text = |key: &str| {
        step.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let number = |key: &str| step.get(key).and_then(Value::as_i64);
    match step.get("do").and_then(Value::as_str).unwrap_or("") {
        "think" => {
            app.emit(NewEvent::new(agent, EventKind::Thinking, text("text")).task(task))?;
        }
        "sleep" => {
            if pause_or_cancel(cancel, number("ms").unwrap_or(10).max(0) as u64).await {
                outcome.cancelled = true;
            }
        }
        "read" => {
            let path = text("path");
            if security::is_blocked(&path, &ctx.blocked()) {
                app.emit(
                    NewEvent::new(
                        agent,
                        EventKind::SecurityAlert,
                        format!("Leitura bloqueada: {path}"),
                    )
                    .task(task)
                    .target(path)
                    .data(json!({ "severity": "warning", "blocked": true })),
                )?;
            } else {
                app.emit(
                    NewEvent::new(agent, EventKind::FileRead, format!("Leu {path}"))
                        .task(task)
                        .target(path),
                )?;
            }
        }
        "edit" => edit(app, ctx, step, outcome).await?,
        "shell_write" => {
            // A file changed by a shell command: no edit event, like `printf > file` in a real CLI.
            let full = ctx.cwd.join(text("path"));
            if let Some(dir) = full.parent() {
                tokio::fs::create_dir_all(dir).await?;
            }
            tokio::fs::write(&full, text("content")).await?;
        }
        "command" => {
            let command = text("cmd");
            if let Some(action) = security::classify_command(&command) {
                let (_, decision) = app.request_approval(agent, task, action, &command)?;
                let approved = tokio::select! {
                    decided = decision => decided.unwrap_or(false),
                    _ = cancel.changed() => {
                        outcome.cancelled = true;
                        return Ok(());
                    }
                };
                if !approved {
                    app.emit(
                        NewEvent::new(agent, EventKind::Error, format!("Ação negada: {command}"))
                            .task(task)
                            .target(command),
                    )?;
                    return Ok(());
                }
            }
            app.emit(
                NewEvent::new(agent, EventKind::Command, truncate(&command, 160))
                    .task(task)
                    .target(command.clone()),
            )?;
            let output = text("output");
            app.emit(
                NewEvent::new(
                    agent,
                    EventKind::CommandResult,
                    super::first_line(&output, "(sem saída)"),
                )
                .task(task)
                .target(command)
                .with("output", json!(output)),
            )?;
        }
        "mcp" => {
            let tool = text("tool");
            let args = step.get("args").cloned().unwrap_or_else(|| json!({}));
            if let Err(reason) = crate::mcp::call_tool(app, agent, &tool, &args) {
                app.emit(
                    NewEvent::new(agent, EventKind::Error, format!("{tool}: {reason}"))
                        .task(task)
                        .target(tool),
                )?;
            }
        }
        "usage" => {
            let cost = step.get("cost_usd").and_then(Value::as_f64);
            let mut usage = Usage {
                tokens_in: number("tokens_in").unwrap_or(0),
                tokens_out: number("tokens_out").unwrap_or(0),
                tokens_cache: number("tokens_cache").unwrap_or(0),
                cost_usd: cost.unwrap_or(0.0),
                cost_source: if cost.is_some() {
                    "reported"
                } else {
                    "estimated"
                }
                .to_string(),
            };
            if cost.is_none() {
                usage.cost_usd = app
                    .prices
                    .read()
                    .unwrap_or_else(|p| p.into_inner())
                    .estimate("fake", "", &usage);
            }
            let summary = step
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("Pensando")
                .to_string();
            app.emit(
                NewEvent::new(agent, EventKind::Thinking, summary)
                    .task(task)
                    .usage(usage),
            )?;
        }
        "replay" => {
            let cli = text("cli");
            let file = app.cfg.fixtures_dir.join(text("file"));
            let recorded = tokio::fs::read_to_string(&file).await?;
            let mut parser = parser_for(&cli, agent)
                .ok_or_else(|| AppError::BadRequest(format!("CLI desconhecido: {cli}")))?;
            for line in recorded.lines() {
                if pause_or_cancel(cancel, app.cfg.fake_step_ms / 4).await {
                    outcome.cancelled = true;
                    return Ok(());
                }
                for parsed in parser.parse_line(line) {
                    handle_parsed(app, ctx, parsed, None, outcome).await?;
                }
            }
        }
        "fail" => outcome.failed = Some(text("text")),
        other => {
            return Err(AppError::BadRequest(format!(
                "passo desconhecido no cenário falso: {other}"
            )))
        }
    }
    Ok(())
}

async fn edit(
    app: &Arc<App>,
    ctx: &TurnCtx,
    step: &Value,
    outcome: &mut TurnOutcome,
) -> AppResult<()> {
    let agent = ctx.agent.id.as_str();
    let task = Some(ctx.task.id.as_str());
    let path = step
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if security::is_blocked(&path, &ctx.blocked())
        || path.starts_with('/')
        || path.split('/').any(|part| part == "..")
    {
        app.emit(
            NewEvent::new(
                agent,
                EventKind::SecurityAlert,
                format!("Escrita bloqueada: {path}"),
            )
            .task(task)
            .target(path)
            .data(json!({ "severity": "warning", "blocked": true })),
        )?;
        return Ok(());
    }
    let summary = step
        .get("summary")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("Editou {path}"));
    let mut event = NewEvent::new(agent, EventKind::FileEdit, summary)
        .task(task)
        .target(path.clone());
    let full = ctx.cwd.join(&path);
    match step.get("content").and_then(Value::as_str) {
        Some(content) => {
            let previous = tokio::fs::read_to_string(&full).await.unwrap_or_default();
            if let Some(dir) = full.parent() {
                tokio::fs::create_dir_all(dir).await?;
            }
            tokio::fs::write(&full, content).await?;
            let leaked =
                watcher::check_written_file(app, agent, task, &ctx.project.id, &ctx.cwd, &full)?;
            if leaked {
                // The alert carries the path; the content never reaches an event.
                outcome.cancelled = true;
                return Ok(());
            }
            let added = step
                .get("lines_added")
                .and_then(Value::as_i64)
                .unwrap_or(content.lines().count() as i64);
            let removed = step
                .get("lines_removed")
                .and_then(Value::as_i64)
                .unwrap_or(previous.lines().count() as i64);
            let mut diff: Vec<String> = previous
                .lines()
                .take(40)
                .map(|line| format!("-{line}"))
                .collect();
            diff.extend(content.lines().take(80).map(|line| format!("+{line}")));
            event = attach_diff(app, event.lines(added, removed), diff.join("\n"));
        }
        None => {
            event = event.lines(
                step.get("lines_added").and_then(Value::as_i64).unwrap_or(1),
                step.get("lines_removed")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
            );
            if let Some(diff) = step.get("diff").and_then(Value::as_str) {
                event = attach_diff(app, event, diff.to_string());
            }
        }
    }
    app.emit(event)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::fixtures_dir;

    #[test]
    fn scenarios_load_by_turn() {
        let first = load_turn(&fixtures_dir(), "basic", 0, false).unwrap();
        assert!(!first.is_empty());
        assert!(load_turn(&fixtures_dir(), "basic", 9, false)
            .unwrap()
            .is_empty());
        let looping = load_turn(&fixtures_dir(), "chatter", 9, false).unwrap();
        assert!(!looping.is_empty(), "repeat_last keeps the last turn");
        assert_eq!(
            load_turn(&fixtures_dir(), "backend", 0, true)
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            load_turn(&fixtures_dir(), "basic", 0, true).unwrap().len(),
            1,
            "default reply reads the inbox"
        );
        assert!(matches!(
            load_turn(&fixtures_dir(), "missing", 0, false),
            Err(AppError::BadRequest(_))
        ));
    }
}
