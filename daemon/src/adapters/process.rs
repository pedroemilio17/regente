//! Runs one CLI turn as a child process: clean environment, line-by-line
//! parsing, timeout and cancellation.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::watch;

use super::{command_for, handle_parsed, parser_for, TurnCtx, TurnOutcome};
use crate::app::{App, AppError, AppResult};
use crate::model::truncate;
use crate::security;

pub async fn run(
    app: &Arc<App>,
    ctx: &TurnCtx,
    mut cancel: watch::Receiver<bool>,
) -> AppResult<TurnOutcome> {
    let spec = command_for(ctx)
        .ok_or_else(|| AppError::BadRequest(format!("CLI desconhecido: {}", ctx.agent.cli)))?;
    let mut parser = parser_for(&ctx.agent.cli, &ctx.agent.id)
        .ok_or_else(|| AppError::BadRequest(format!("CLI desconhecido: {}", ctx.agent.cli)))?;

    let raw_rel = format!("raw/{}/{}.jsonl", ctx.agent.id, ctx.session_id);
    let raw_path = app.cfg.home.join(&raw_rel);
    if let Some(dir) = raw_path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let mut raw_file = tokio::fs::File::create(&raw_path).await?;

    if ctx.agent.cli == "agy" {
        register_agy_messenger(app, &spec.program).await;
    }

    let mut child = Command::new(&spec.program)
        .args(&spec.args)
        .current_dir(&ctx.cwd)
        .env_clear()
        .envs(security::clean_env(std::env::vars(), &spec.keep_env))
        .envs(spec.env.iter().cloned())
        .env("REGENTE_AGENT_ID", &ctx.agent.id)
        .env(
            "REGENTE_API",
            format!(
                "http://127.0.0.1:{}",
                *app.bound_port.read().unwrap_or_else(|p| p.into_inner())
            ),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            AppError::Other(format!(
                "não foi possível iniciar {}: {error}",
                spec.program
            ))
        })?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Other("stdout indisponível".into()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::Other("stderr indisponível".into()))?;
    let stderr_task = tokio::spawn(async move {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text).await;
        text
    });

    let secrets = app
        .secrets
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .get(&ctx.project.id)
        .cloned();
    let mut lines = BufReader::new(stdout).lines();
    let mut outcome = TurnOutcome::default();
    let deadline = tokio::time::sleep(Duration::from_secs(app.cfg.turn_timeout_secs));
    tokio::pin!(deadline);
    let mut timed_out = false;

    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { break };
                // Raw output with a secret value is never written to disk.
                if !secrets.as_ref().is_some_and(|set| set.contains_secret(&line)) {
                    raw_file.write_all(line.as_bytes()).await?;
                    raw_file.write_all(b"\n").await?;
                }
                for parsed in parser.parse_line(&line) {
                    handle_parsed(app, ctx, parsed, Some(&raw_rel), &mut outcome).await?;
                }
            }
            _ = cancel.changed() => {
                outcome.cancelled = true;
                break;
            }
            _ = &mut deadline => {
                timed_out = true;
                break;
            }
        }
    }

    if outcome.cancelled || timed_out {
        let _ = child.kill().await;
    }
    let status = child.wait().await?;
    let stderr_text = stderr_task.await.unwrap_or_default();
    if timed_out {
        outcome.failed = Some(format!(
            "tempo limite de {}s excedido",
            app.cfg.turn_timeout_secs
        ));
    } else if !outcome.cancelled && outcome.done.is_none() && outcome.failed.is_none() {
        let detail = if security::scan(&stderr_text, secrets.as_deref()).is_some() {
            String::new()
        } else {
            truncate(stderr_text.lines().last().unwrap_or(""), 200)
        };
        outcome.failed = Some(
            format!(
                "{} terminou sem resultado ({status}) {detail}",
                spec.program
            )
            .trim()
            .to_string(),
        );
    }
    Ok(outcome)
}

/// agy only reads MCP servers from its global config, so the messenger is
/// registered there once, as a stdio bridge back to this daemon.
async fn register_agy_messenger(app: &App, program: &str) {
    let marker = app.cfg.home.join("agy-mcp-registered");
    if marker.exists() {
        return;
    }
    let Ok(daemon) = std::env::current_exe() else {
        return;
    };
    let added = Command::new(program)
        .args(["mcp", "add", "regente"])
        .arg(&daemon)
        .arg("mcp-stdio")
        .stdin(Stdio::null())
        .output()
        .await;
    match added {
        Ok(output) if output.status.success() => {
            let _ = tokio::fs::write(&marker, daemon.to_string_lossy().as_bytes()).await;
        }
        Ok(output) => {
            tracing::warn!(stderr = %String::from_utf8_lossy(&output.stderr), "agy mcp add failed")
        }
        Err(error) => tracing::warn!(%error, "could not run agy mcp add"),
    }
}
