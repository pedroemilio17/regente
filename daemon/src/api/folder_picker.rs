//! `POST /pick-folder`: the daemon opens the system folder picker for the UI,
//! since a browser page cannot learn absolute paths (D22).

use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

const UNAVAILABLE: &str = "Não foi possível abrir o seletor de pastas; digite o caminho.";
const BUSY: &str = "O seletor de pastas já está aberto.";

/// Only one dialog at a time, across all clients.
static OPEN: AtomicBool = AtomicBool::new(false);

struct Release;

impl Drop for Release {
    fn drop(&mut self) {
        OPEN.store(false, Ordering::SeqCst);
    }
}

pub(super) async fn pick_folder() -> Response {
    pick_with(native_pick).await
}

/// `Ok(None)` is a cancel; `Err` means no dialog could be shown.
async fn pick_with<F, Fut>(open: F) -> Response
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Option<PathBuf>, ()>>,
{
    if OPEN.swap(true, Ordering::SeqCst) {
        return error(StatusCode::CONFLICT, BUSY);
    }
    let _release = Release;
    match open().await {
        Ok(path) => {
            Json(json!({ "path": path.map(|p| p.to_string_lossy().into_owned()) })).into_response()
        }
        Err(()) => error(StatusCode::SERVICE_UNAVAILABLE, UNAVAILABLE),
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Linux: xdg-desktop-portal through rfd (zenity as rfd's own fallback).
/// rfd reports failures as a cancel, so check for a desktop session first.
#[cfg(target_os = "linux")]
async fn native_pick() -> Result<Option<PathBuf>, ()> {
    let env = |key: &str| std::env::var_os(key).is_some_and(|v| !v.is_empty());
    let display = env("WAYLAND_DISPLAY") || env("DISPLAY");
    let bus = env("DBUS_SESSION_BUS_ADDRESS")
        || std::env::var_os("XDG_RUNTIME_DIR")
            .is_some_and(|dir| PathBuf::from(dir).join("bus").exists());
    if !display || !bus {
        return Err(());
    }
    let mut dialog = rfd::AsyncFileDialog::new().set_title("Escolher pasta do projeto");
    if let Some(home) = home() {
        dialog = dialog.set_directory(home);
    }
    Ok(dialog
        .pick_folder()
        .await
        .map(|handle| handle.path().to_path_buf()))
}

/// macOS: rfd needs the AppKit main loop, which the daemon does not run, so
/// ask AppleScript (`choose folder`) instead. Cancel exits with error -128.
#[cfg(target_os = "macos")]
async fn native_pick() -> Result<Option<PathBuf>, ()> {
    let location = home()
        .map(|h| format!(" default location (POSIX file \"{}\")", h.display()))
        .unwrap_or_default();
    let script = format!(
        "POSIX path of (choose folder with prompt \"Escolher pasta do projeto\"{location})"
    );
    let output = tokio::process::Command::new("osascript")
        .args(["-e", &script])
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|_| ())?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let path = path
            .strip_suffix('/')
            .filter(|p| !p.is_empty())
            .unwrap_or(&path);
        return Ok(Some(PathBuf::from(path)));
    }
    if String::from_utf8_lossy(&output.stderr).contains("-128") {
        return Ok(None);
    }
    Err(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
async fn native_pick() -> Result<Option<PathBuf>, ()> {
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use serde_json::Value;

    async fn body(response: Response) -> (StatusCode, Value) {
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1 << 16).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    // One test so the shared "dialog open" flag is never raced.
    #[tokio::test]
    async fn picks_cancels_fails_and_refuses_a_second_dialog() {
        let (status, value) =
            body(pick_with(|| async { Ok(Some("/tmp/loja".into())) }).await).await;
        assert_eq!(
            (status, value),
            (StatusCode::OK, json!({ "path": "/tmp/loja" }))
        );

        let (status, value) = body(pick_with(|| async { Ok(None) }).await).await;
        assert_eq!((status, value), (StatusCode::OK, json!({ "path": null })));

        let (status, value) = body(pick_with(|| async { Err(()) }).await).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(value["error"], UNAVAILABLE);

        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        let first = tokio::spawn(pick_with(|| async move {
            wait.await.ok();
            Ok(None)
        }));
        while !OPEN.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        let (status, value) = body(pick_with(|| async { Ok(None) }).await).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(value["error"], BUSY);
        release.send(()).unwrap();
        assert_eq!(first.await.unwrap().status(), StatusCode::OK);
        assert!(!OPEN.load(Ordering::SeqCst));
    }
}
