//! Git worktree lifecycle: one worktree per agent and task, created and
//! removed by the daemon. Checkpoints are automatic commits per step.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::app::{AppError, AppResult};

/// Active worktrees allowed per project.
pub const MAX_WORKTREES: usize = 8;
const PORT_BASE: u16 = 4100;
const PORTS_PER_WORKTREE: u16 = 10;

#[derive(Clone, Debug, PartialEq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
    /// First port of the range reserved for this worktree.
    pub port_base: u16,
}

/// Lowercase slug safe for branch and directory names.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "agent".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Run git in a directory and return its trimmed stdout.
pub async fn git(dir: &Path, args: &[&str]) -> AppResult<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .map_err(|error| AppError::Git(format!("could not run git: {error}")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(AppError::Git(format!(
            "git {} failed: {stderr}",
            args.first().copied().unwrap_or("")
        )))
    }
}

pub async fn is_git_repo(dir: &Path) -> bool {
    dir.is_dir()
        && git(dir, &["rev-parse", "--is-inside-work-tree"])
            .await
            .is_ok_and(|out| out == "true")
}

pub fn worktrees_dir(root: &Path) -> PathBuf {
    root.join(".regente").join("wt")
}

/// Keep `.regente/` and `.env.regente` out of the repository without touching `.gitignore`.
async fn ensure_excluded(root: &Path) -> AppResult<()> {
    let git_dir = git(root, &["rev-parse", "--git-common-dir"]).await?;
    let exclude = root.join(git_dir).join("info").join("exclude");
    let current = tokio::fs::read_to_string(&exclude)
        .await
        .unwrap_or_default();
    let mut updated = current.clone();
    for entry in [".regente/", ".env.regente"] {
        if !current.lines().any(|line| line.trim() == entry) {
            if !updated.is_empty() && !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(entry);
            updated.push('\n');
        }
    }
    if updated != current {
        if let Some(dir) = exclude.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        tokio::fs::write(&exclude, updated).await?;
    }
    Ok(())
}

fn active(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(worktrees_dir(root))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}

/// Create (or reuse) the worktree of an agent for a task:
/// `.regente/wt/<agent>-<task>` on branch `regente/<task>/<agent>`.
pub async fn create(
    root: &Path,
    base_branch: &str,
    agent_name: &str,
    task_id: &str,
) -> AppResult<Worktree> {
    let agent = slug(agent_name);
    let path = worktrees_dir(root).join(format!("{agent}-{task_id}"));
    let branch = format!("regente/{task_id}/{agent}");
    let existing = active(root);
    if let Some(index) = existing.iter().position(|dir| dir == &path) {
        let port_base =
            read_port_base(&path).unwrap_or(PORT_BASE + PORTS_PER_WORKTREE * index as u16);
        return Ok(Worktree {
            path,
            branch,
            port_base,
        });
    }
    if existing.len() >= MAX_WORKTREES {
        return Err(AppError::Git(format!(
            "limite de {MAX_WORKTREES} worktrees ativos atingido"
        )));
    }
    ensure_excluded(root).await?;
    tokio::fs::create_dir_all(worktrees_dir(root)).await?;
    let path_text = path.to_string_lossy().into_owned();
    let base = if git(root, &["rev-parse", "--verify", base_branch])
        .await
        .is_ok()
    {
        base_branch
    } else {
        "HEAD"
    };
    if git(root, &["rev-parse", "--verify", &branch]).await.is_ok() {
        git(root, &["worktree", "add", &path_text, &branch]).await?;
    } else {
        git(root, &["worktree", "add", &path_text, "-b", &branch, base]).await?;
    }
    // Lowest free slot, so port ranges are reused after cleanup.
    let used: Vec<u16> = existing
        .iter()
        .filter_map(|dir| read_port_base(dir))
        .collect();
    let slot = (0..MAX_WORKTREES as u16)
        .find(|slot| !used.contains(&(PORT_BASE + PORTS_PER_WORKTREE * slot)))
        .unwrap_or(0);
    Ok(Worktree {
        path,
        branch,
        port_base: PORT_BASE + PORTS_PER_WORKTREE * slot,
    })
}

fn read_port_base(worktree: &Path) -> Option<u16> {
    let text = std::fs::read_to_string(worktree.join(".env.regente")).ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("REGENTE_PORT_BASE="))
        .and_then(|value| value.trim().parse().ok())
}

/// Prepare a fresh worktree: `.env.regente` with fake values for the project's
/// variable names and the reserved port range. Real secrets only ever reach a
/// child process through `regente run`.
pub async fn prepare(worktree: &Worktree, env_keys: &[String]) -> AppResult<()> {
    let mut text = String::from(
        "# Generated by Regente. Fake values: real secrets are injected only by `regente run`.\n",
    );
    text.push_str(&format!("REGENTE_PORT_BASE={}\n", worktree.port_base));
    text.push_str(&format!("PORT={}\n", worktree.port_base));
    for key in env_keys.iter().filter(|key| key.as_str() != "PORT") {
        text.push_str(&format!("{key}=regente-fake-value\n"));
    }
    tokio::fs::write(worktree.path.join(".env.regente"), text).await?;
    Ok(())
}

/// Merge the branches a task depends on into its worktree, so the agent
/// starts from the work it must build on.
pub async fn merge_dependencies(worktree: &Path, branches: &[String]) -> AppResult<()> {
    for branch in branches {
        if git(worktree, &["rev-parse", "--verify", branch])
            .await
            .is_err()
        {
            continue;
        }
        let merged = git(
            worktree,
            &[
                "-c",
                "user.name=Regente",
                "-c",
                "user.email=regente@localhost",
                "merge",
                "--no-edit",
                branch,
            ],
        )
        .await;
        if let Err(error) = merged {
            let _ = git(worktree, &["merge", "--abort"]).await;
            return Err(error);
        }
    }
    Ok(())
}

/// Files changed in the worktree since the last commit (tracked and new).
pub async fn changed_files(worktree: &Path) -> AppResult<Vec<String>> {
    let out = git(
        worktree,
        &["status", "--porcelain", "--untracked-files=all"],
    )
    .await?;
    Ok(out
        .lines()
        .filter_map(|line| line.get(3..))
        .map(|path| {
            path.rsplit(" -> ")
                .next()
                .unwrap_or(path)
                .trim_matches('"')
                .to_string()
        })
        .collect())
}

/// Lines added and removed in a file compared with the last commit.
pub async fn numstat(worktree: &Path, file: &str) -> Option<(i64, i64)> {
    let out = git(worktree, &["diff", "--numstat", "HEAD", "--", file])
        .await
        .ok()?;
    if let Some(line) = out.lines().next() {
        let mut parts = line.split_whitespace();
        let added = parts.next()?.parse().ok()?;
        let removed = parts.next()?.parse().ok()?;
        return Some((added, removed));
    }
    // Untracked file: every line is new.
    let text = tokio::fs::read_to_string(worktree.join(file)).await.ok()?;
    Some((text.lines().count() as i64, 0))
}

/// Unified diff of a file compared with the last commit.
pub async fn diff(worktree: &Path, file: &str) -> Option<String> {
    let out = git(worktree, &["diff", "HEAD", "--", file]).await.ok()?;
    if !out.is_empty() {
        return Some(out);
    }
    let text = tokio::fs::read_to_string(worktree.join(file)).await.ok()?;
    Some(
        text.lines()
            .map(|line| format!("+{line}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Automatic commit of everything the agent changed. Returns the commit hash,
/// or None when there was nothing to commit.
pub async fn checkpoint(
    worktree: &Path,
    agent_name: &str,
    task_id: &str,
    message: &str,
) -> AppResult<Option<String>> {
    git(worktree, &["add", "-A"]).await?;
    if git(worktree, &["diff", "--cached", "--quiet"])
        .await
        .is_ok()
    {
        return Ok(None);
    }
    let author = format!("user.name=Regente/{agent_name}");
    let subject = format!("regente({task_id}): {message}");
    git(
        worktree,
        &[
            "-c",
            &author,
            "-c",
            "user.email=regente@localhost",
            "commit",
            "-q",
            "-m",
            &subject,
        ],
    )
    .await?;
    Ok(Some(
        git(worktree, &["rev-parse", "--short", "HEAD"]).await?,
    ))
}

/// True when the branch has commits that the base branch does not.
pub async fn has_new_commits(root: &Path, base: &str, branch: &str) -> bool {
    git(root, &["rev-list", "--count", &format!("{base}..{branch}")])
        .await
        .is_ok_and(|count| count.trim() != "0")
}

/// Squash a branch into the checked-out base branch as one commit.
pub async fn merge_squash(root: &Path, branch: &str, message: &str) -> AppResult<()> {
    if !git(root, &["status", "--porcelain", "--untracked-files=no"])
        .await?
        .is_empty()
    {
        return Err(AppError::Git(
            "a branch base tem alterações não commitadas".to_string(),
        ));
    }
    if let Err(error) = git(root, &["merge", "--squash", branch]).await {
        let _ = git(root, &["reset", "--hard", "HEAD"]).await;
        return Err(error);
    }
    if git(root, &["diff", "--cached", "--quiet"]).await.is_ok() {
        return Ok(());
    }
    git(
        root,
        &[
            "-c",
            "user.name=Regente",
            "-c",
            "user.email=regente@localhost",
            "commit",
            "-q",
            "-m",
            message,
        ],
    )
    .await?;
    Ok(())
}

/// Remove a worktree and delete its branch.
pub async fn remove(root: &Path, worktree: &Path, branch: &str) -> AppResult<()> {
    if worktree.exists() {
        git(
            root,
            &["worktree", "remove", "--force", &worktree.to_string_lossy()],
        )
        .await?;
    }
    let _ = git(root, &["branch", "-D", branch]).await;
    Ok(())
}

/// Worktrees not touched for more than `days`, for the user to decide on.
pub fn stale(root: &Path, days: u64) -> Vec<PathBuf> {
    let limit = std::time::Duration::from_secs(days * 24 * 3600);
    active(root)
        .into_iter()
        .filter(|dir| {
            std::fs::metadata(dir)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > limit)
        })
        .collect()
}

#[cfg(test)]
pub mod testing {
    use super::*;

    /// Temporary git repository with one commit on `main`.
    pub async fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q", "-b", "main"]).await.unwrap();
        std::fs::write(root.join("README.md"), "# demo\n").unwrap();
        std::fs::write(root.join(".gitignore"), ".env\n").unwrap();
        git(root, &["add", "-A"]).await.unwrap();
        git(
            root,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "-m",
                "init",
            ],
        )
        .await
        .unwrap();
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::testing::repo;
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Backend API"), "backend-api");
        assert_eq!(slug("  QA!! "), "qa");
        assert_eq!(slug("çã"), "agent");
    }

    #[tokio::test]
    async fn lifecycle_create_checkpoint_merge_remove() {
        let dir = repo().await;
        let root = dir.path();
        assert!(is_git_repo(root).await);
        // A secret file ignored by git never reaches the worktree.
        std::fs::write(root.join(".env"), "API_TOKEN=tok_fake_0123456789\n").unwrap();

        let wt = create(root, "main", "Backend", "tk_1").await.unwrap();
        assert_eq!(wt.branch, "regente/tk_1/backend");
        assert!(wt.path.join("README.md").exists());
        assert!(!wt.path.join(".env").exists());
        assert_eq!(wt.port_base, 4100);
        let again = create(root, "main", "Backend", "tk_1").await.unwrap();
        assert_eq!(again.path, wt.path);

        prepare(&wt, &["API_TOKEN".to_string()]).await.unwrap();
        let env = std::fs::read_to_string(wt.path.join(".env.regente")).unwrap();
        assert!(env.contains("API_TOKEN=regente-fake-value"));
        assert!(env.contains("REGENTE_PORT_BASE=4100"));

        let second = create(root, "main", "Frontend", "tk_1").await.unwrap();
        prepare(&second, &[]).await.unwrap();
        assert_eq!(second.port_base, 4110);

        std::fs::write(
            wt.path.join("health.ts"),
            "export const v = 1;\nexport const w = 2;\n",
        )
        .unwrap();
        assert_eq!(
            changed_files(&wt.path).await.unwrap(),
            vec!["health.ts".to_string()]
        );
        assert_eq!(numstat(&wt.path, "health.ts").await, Some((2, 0)));
        assert!(diff(&wt.path, "health.ts")
            .await
            .unwrap()
            .contains("+export const v = 1;"));

        let commit = checkpoint(&wt.path, "Backend", "tk_1", "rota health")
            .await
            .unwrap();
        assert!(commit.is_some());
        assert_eq!(
            checkpoint(&wt.path, "Backend", "tk_1", "nada")
                .await
                .unwrap(),
            None
        );
        let author = git(&wt.path, &["log", "-1", "--format=%an|%s"])
            .await
            .unwrap();
        assert_eq!(author, "Regente/Backend|regente(tk_1): rota health");
        // The generated env file is excluded from commits.
        assert!(!git(&wt.path, &["ls-files"])
            .await
            .unwrap()
            .contains(".env.regente"));

        assert!(has_new_commits(root, "main", &wt.branch).await);
        assert!(!has_new_commits(root, "main", &second.branch).await);
        merge_squash(root, &wt.branch, "feat: health")
            .await
            .unwrap();
        assert!(root.join("health.ts").exists());
        assert_eq!(
            git(root, &["log", "-1", "--format=%s"]).await.unwrap(),
            "feat: health"
        );

        remove(root, &wt.path, &wt.branch).await.unwrap();
        remove(root, &second.path, &second.branch).await.unwrap();
        assert!(!wt.path.exists());
        assert!(git(root, &["rev-parse", "--verify", &wt.branch])
            .await
            .is_err());
        assert!(stale(root, 7).is_empty());
    }

    #[tokio::test]
    async fn dependencies_are_merged_into_the_new_worktree() {
        let dir = repo().await;
        let root = dir.path();
        let backend = create(root, "main", "Backend", "tk_a").await.unwrap();
        std::fs::write(backend.path.join("api.ts"), "export const api = 1;\n").unwrap();
        checkpoint(&backend.path, "Backend", "tk_a", "api")
            .await
            .unwrap();
        let qa = create(root, "main", "QA", "tk_b").await.unwrap();
        assert!(!qa.path.join("api.ts").exists());
        merge_dependencies(
            &qa.path,
            &[backend.branch.clone(), "regente/missing/x".to_string()],
        )
        .await
        .unwrap();
        assert!(qa.path.join("api.ts").exists());
    }

    #[tokio::test]
    async fn merge_refuses_dirty_base() {
        let dir = repo().await;
        let root = dir.path();
        let wt = create(root, "main", "Backend", "tk_1").await.unwrap();
        std::fs::write(wt.path.join("a.txt"), "a\n").unwrap();
        checkpoint(&wt.path, "Backend", "tk_1", "a").await.unwrap();
        std::fs::write(root.join("README.md"), "changed\n").unwrap();
        assert!(merge_squash(root, &wt.branch, "x").await.is_err());
    }
}
