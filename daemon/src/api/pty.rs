//! On-demand terminal: a PTY that exists only while the terminal window is
//! open (`/api/v1/agents/{id}/pty`). The UI never renders terminals by default.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::app::{App, AppError, AppResult};
use crate::security;

pub async fn open(
    ws: WebSocketUpgrade,
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let agent = app.store.get_agent(&id)?;
    let project = app.store.get_project(&agent.project_id)?;
    // The agent's latest worktree, or the project root when it has none yet.
    let cwd = app
        .store
        .latest_session_of_agent(&id)?
        .and_then(|session| session.worktree)
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| PathBuf::from(&project.root));
    Ok(ws.on_upgrade(move |socket| async move {
        if let Err(error) = session(socket, cwd).await {
            tracing::warn!(%error, "terminal session ended with error");
        }
    }))
}

fn pty_error<E: std::fmt::Display>(error: E) -> AppError {
    AppError::Other(format!("terminal: {error}"))
}

async fn session(mut socket: WebSocket, cwd: PathBuf) -> AppResult<()> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(pty_error)?;
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut command = CommandBuilder::new(shell);
    command.cwd(cwd);
    command.env_clear();
    for (name, value) in security::clean_env(std::env::vars(), &[]) {
        command.env(name, value);
    }
    command.env("TERM", "xterm-256color");
    let mut child = pair.slave.spawn_command(command).map_err(pty_error)?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(pty_error)?;
    let mut writer = pair.master.take_writer().map_err(pty_error)?;

    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        while let Ok(read) = reader.read(&mut buffer) {
            if read == 0 || tx.blocking_send(buffer[..read].to_vec()).is_err() {
                break;
            }
        }
    });

    loop {
        tokio::select! {
            output = rx.recv() => match output {
                Some(bytes) => {
                    if socket.send(WsMessage::Binary(bytes.into())).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(WsMessage::Text(text))) => {
                    // `{"resize":[cols,rows]}` resizes; any other text is typed input.
                    let resize = serde_json::from_str::<Value>(text.as_str()).ok().and_then(|value| {
                        let size = value.get("resize")?.as_array()?;
                        Some((size.first()?.as_u64()? as u16, size.get(1)?.as_u64()? as u16))
                    });
                    match resize {
                        Some((cols, rows)) => {
                            let _ = pair.master.resize(PtySize { rows: rows.max(2), cols: cols.max(2), pixel_width: 0, pixel_height: 0 });
                        }
                        None => {
                            if writer.write_all(text.as_bytes()).is_err() {
                                break;
                            }
                        }
                    }
                }
                Some(Ok(WsMessage::Binary(bytes))) => {
                    if writer.write_all(&bytes).is_err() {
                        break;
                    }
                }
                Some(Ok(WsMessage::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
        }
    }
    // Closing the window destroys the terminal.
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}
