//! Experimental mount-plan fixture. This is not a production security boundary.
//! See docs/adr/isolamento-pastas.md for the missing race and Git protections.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub enum Mount {
    ReadOnly(PathBuf),
    Hidden(PathBuf),
}

/// Build a deliberately minimal bubblewrap command for an already prepared fixture.
/// The caller must ensure all paths are trusted and the Git directory is filtered.
pub fn fixture_args(worktree: &Path, git: &Path, mounts: &[Mount]) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["--unshare-user", "--unshare-pid", "--die-with-parent"]
        .into_iter()
        .map(Into::into)
        .collect();
    for path in ["/usr", "/bin", "/lib", "/lib64", "/etc"] {
        if Path::new(path).exists() {
            args.extend([OsString::from("--ro-bind"), path.into(), path.into()]);
        }
    }
    args.extend([
        "--proc".into(),
        "/proc".into(),
        "--dev".into(),
        "/dev".into(),
        "--tmpfs".into(),
        "/tmp".into(),
        "--dir".into(),
        "/home".into(),
        "--setenv".into(),
        "HOME".into(),
        "/tmp".into(),
        "--bind".into(),
        worktree.as_os_str().into(),
        worktree.as_os_str().into(),
        "--bind".into(),
        git.as_os_str().into(),
        worktree.join(".git").into_os_string(),
    ]);
    for mount in mounts {
        match mount {
            Mount::ReadOnly(path) => args.extend([
                "--ro-bind".into(),
                path.as_os_str().into(),
                path.as_os_str().into(),
            ]),
            Mount::Hidden(path) => args.extend(["--tmpfs".into(), path.as_os_str().into()]),
        }
    }
    args.extend(["--chdir".into(), worktree.as_os_str().into(), "--".into()]);
    args
}

/// Conservative fixture preflight: links cannot be trusted as an access boundary.
/// This check is inherently racy and is not sufficient for production use.
#[cfg(unix)]
pub fn fixture_reject_links(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || (metadata.is_file() && metadata.nlink() > 1) {
        return Err(std::io::Error::other("symlink or hard link in fixture"));
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            fixture_reject_links(&entry?.path())?;
        }
    } else if !metadata.is_file() {
        return Err(std::io::Error::other("special file in fixture"));
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    #[ignore = "requires Linux bubblewrap and unprivileged namespaces"]
    fn mount_fixture_restricts_paths_and_hides_original_git() {
        // Keep mount sources outside /tmp, which becomes private in the sandbox.
        let temp = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let root = temp.path().join("original");
        let work = temp.path().join("work");
        let git = temp.path().join("filtered-git");
        for path in [&root, &work, &git] {
            std::fs::create_dir_all(path).unwrap();
        }
        for path in ["read", "none", "write"] {
            std::fs::create_dir(work.join(path)).unwrap();
        }
        std::fs::write(root.join("secret"), "original-secret").unwrap();
        std::fs::write(work.join("none/secret"), "secret").unwrap();
        std::fs::write(work.join("read/file"), "readable").unwrap();
        std::fs::create_dir(work.join(".git")).unwrap();
        assert!(Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&git)
            .status()
            .unwrap()
            .success());
        // A filtered bare store is sufficient to exercise git show without exposing host history.
        std::fs::remove_dir_all(&git).unwrap();
        assert!(Command::new("git")
            .args(["init", "--bare", "--quiet"])
            .arg(&git)
            .status()
            .unwrap()
            .success());
        let mounts = [
            Mount::ReadOnly(work.join("read")),
            Mount::Hidden(work.join("none")),
        ];
        let mut args = fixture_args(&work, &git, &mounts);
        let script = format!("test ! -e '{}' && test ! -e none/secret && test \"$(cat read/file)\" = readable && ! touch read/new && touch write/new && ! git show HEAD:none/secret", root.display());
        args.extend(["/bin/sh".into(), "-c".into(), script.into()]);
        let output = Command::new("bwrap").args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(work.join("write/new").exists());
    }

    #[test]
    fn preflight_rejects_hard_links_and_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("file");
        std::fs::write(&file, "private").unwrap();
        std::fs::hard_link(&file, temp.path().join("alias")).unwrap();
        assert!(fixture_reject_links(temp.path()).is_err());
        std::fs::remove_file(temp.path().join("alias")).unwrap();
        std::os::unix::fs::symlink(&file, temp.path().join("link")).unwrap();
        assert!(fixture_reject_links(temp.path()).is_err());
    }
}
