//! File watcher for agent worktrees: every write is scanned for secret
//! values and key patterns. A hit raises a critical alert and pauses the agent.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::json;

use crate::app::{App, AppError, AppResult};
use crate::model::{EventKind, NewEvent};
use crate::security::{scan, SecretSet};

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Keeps the OS watcher alive; dropping it stops the watch.
pub struct WorktreeWatch {
    _watcher: RecommendedWatcher,
}

fn ignored(path: &Path) -> bool {
    path.components().any(|part| {
        matches!(
            part.as_os_str().to_str(),
            Some(".git") | Some("node_modules") | Some("target")
        )
    }) || path.file_name().and_then(|name| name.to_str()) == Some(".env.regente")
}

/// Load (or refresh) the secret hashes of a project.
pub fn refresh_secrets(app: &App, project_id: &str, project_root: &Path) -> Arc<SecretSet> {
    let set = Arc::new(SecretSet::load(project_root));
    app.secrets
        .write()
        .unwrap_or_else(|p| p.into_inner())
        .insert(project_id.to_string(), set.clone());
    set
}

/// Scan one written file. Returns true when a leak was found; the agent is
/// then paused and a `security_alert` event is recorded.
pub fn check_written_file(
    app: &App,
    agent_id: &str,
    task_id: Option<&str>,
    project_id: &str,
    root: &Path,
    path: &Path,
) -> AppResult<bool> {
    if ignored(path) || !path.is_file() {
        return Ok(false);
    }
    let size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    if size == 0 || size > MAX_FILE_BYTES {
        return Ok(false);
    }
    let Ok(bytes) = std::fs::read(path) else {
        return Ok(false);
    };
    let text = String::from_utf8_lossy(&bytes);
    let secrets = app
        .secrets
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .get(project_id)
        .cloned();
    let Some(reason) = scan(&text, secrets.as_deref()) else {
        return Ok(false);
    };
    if app.store.get_agent(agent_id)?.paused {
        return Ok(true);
    }
    let shown = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned();
    app.emit(
        NewEvent::new(
            agent_id,
            EventKind::SecurityAlert,
            format!("Vazamento de segredo em {shown}: {reason}"),
        )
        .task(task_id)
        .target(shown.clone())
        .data(json!({ "severity": "critical", "reason": reason })),
    )?;
    app.pause_agent(agent_id, &format!("Vazamento de segredo em {shown}"))?;
    Ok(true)
}

/// Watch a worktree for the duration of a turn.
pub fn watch(
    app: Arc<App>,
    agent_id: String,
    task_id: Option<String>,
    project_id: String,
    root: PathBuf,
) -> AppResult<WorktreeWatch> {
    let watched_root = root.clone();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else { return };
        if !matches!(
            event.kind,
            notify::EventKind::Create(_) | notify::EventKind::Modify(_)
        ) {
            return;
        }
        for path in &event.paths {
            if let Err(error) = check_written_file(
                &app,
                &agent_id,
                task_id.as_deref(),
                &project_id,
                &watched_root,
                path,
            ) {
                tracing::warn!(%error, "secret scan failed");
            }
        }
    })
    .map_err(|error| AppError::Other(format!("file watcher: {error}")))?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|error| AppError::Other(format!("file watcher: {error}")))?;
    Ok(WorktreeWatch { _watcher: watcher })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::{agent, app, project};

    // Fake value only.
    const FAKE_ENV: &str = "API_TOKEN=tok_fake_0123456789\n";

    #[tokio::test]
    async fn writing_a_secret_value_alerts_and_pauses() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("wt");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(dir.path().join(".env"), FAKE_ENV).unwrap();
        let app = app(dir.path());
        let project = project(&app, dir.path());
        let agent = agent(&app, &project, "Backend", "basic", false);
        refresh_secrets(&app, &project.id, dir.path());
        let _watch = watch(
            app.clone(),
            agent.id.clone(),
            None,
            project.id.clone(),
            worktree.clone(),
        )
        .unwrap();

        std::fs::write(worktree.join("clean.ts"), "export const ok = 1;\n").unwrap();
        std::fs::write(
            worktree.join("config.ts"),
            "export const token = 'tok_fake_0123456789';\n",
        )
        .unwrap();

        let mut paused = false;
        for _ in 0..100 {
            if app.store.get_agent(&agent.id).unwrap().paused {
                paused = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(paused, "agent should be paused by the watcher");
        let events = app.store.all_events().unwrap();
        let alert = events
            .iter()
            .find(|e| e.kind == EventKind::SecurityAlert)
            .unwrap();
        assert_eq!(alert.target.as_deref(), Some("config.ts"));
        assert_eq!(alert.data.as_ref().unwrap()["severity"], "critical");
        // The value never reaches an event.
        assert!(!serde_json::to_string(&events)
            .unwrap()
            .contains("tok_fake_0123456789"));
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == EventKind::SecurityAlert)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn key_patterns_are_caught_without_env_file() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let project = project(&app, dir.path());
        let agent = agent(&app, &project, "Backend", "basic", false);
        let file = dir.path().join("aws.txt");
        std::fs::write(&file, "key AKIAABCDEFGHIJKLMNOP\n").unwrap();
        assert!(check_written_file(&app, &agent.id, None, &project.id, dir.path(), &file).unwrap());
        assert!(app.store.get_agent(&agent.id).unwrap().paused);
        let git_file = dir.path().join(".git").join("x");
        std::fs::create_dir_all(git_file.parent().unwrap()).unwrap();
        std::fs::write(&git_file, "AKIAABCDEFGHIJKLMNOP").unwrap();
        assert!(
            !check_written_file(&app, &agent.id, None, &project.id, dir.path(), &git_file).unwrap()
        );
    }
}
