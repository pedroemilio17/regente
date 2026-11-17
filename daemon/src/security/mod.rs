//! Security layers: clean environment, blocked paths, secret hashes, leak
//! scanning, per-CLI deny rules and the classification of irreversible actions.
//!
//! Central rule: no agent sees a secret, and no text that comes from outside
//! becomes an instruction.

#[cfg(test)]
mod folder_sandbox;
pub mod watcher;

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Default patterns denied to every agent of a project.
pub const DEFAULT_BLOCKED: [&str; 4] = [".env*", "secrets/", "*.pem", "id_*"];

/// Reject absolute paths, traversal and symlink components, including dangling links.
pub fn safe_relative_path(root: &Path, path: &str) -> bool {
    use std::path::Component;
    let relative = Path::new(path);
    if path.is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return false;
    }
    let mut current = root.to_path_buf();
    for part in relative.components() {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => return false,
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return false,
            _ => {}
        }
    }
    true
}

/// Restricted directories apply to all descendants, regardless of overlapping rules.
pub fn folder_denied(folders: &Value, path: &str, write: bool) -> bool {
    let path = Path::new(path);
    folders.as_array().into_iter().flatten().any(|folder| {
        let Some(base) = folder.get("path").and_then(Value::as_str) else {
            return true;
        };
        let Some(access) = folder.get("access").and_then(Value::as_str) else {
            return true;
        };
        let matches = base == "." || path.starts_with(Path::new(base));
        matches
            && (access == "none"
                || (write && access == "read")
                || !["none", "read", "write"].contains(&access))
    })
}

pub fn has_restricted_folders(folders: &Value) -> bool {
    folders
        .as_array()
        .into_iter()
        .flatten()
        .any(|folder| !matches!(folder.get("access").and_then(Value::as_str), Some("write")))
}

/// Values shorter than this are not treated as secrets (`true`, `3000`…).
const MIN_SECRET_LEN: usize = 8;
/// Only the beginning of large files is scanned.
const MAX_SCAN_BYTES: usize = 256 * 1024;

// ---- clean environment ------------------------------------------------------

fn looks_secret(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    const SUFFIXES: [&str; 7] = [
        "_KEY",
        "_TOKEN",
        "_SECRET",
        "_PASSWORD",
        "_PASS",
        "_CREDENTIALS",
        "_DSN",
    ];
    const EXACT: [&str; 5] = [
        "DATABASE_URL",
        "REDIS_URL",
        "MONGODB_URI",
        "PGPASSWORD",
        "NPM_TOKEN",
    ];
    const CONTAINS: [&str; 4] = ["SECRET", "PASSWORD", "PRIVATE_KEY", "API_KEY"];
    SUFFIXES.iter().any(|suffix| upper.ends_with(suffix))
        || EXACT.contains(&upper.as_str())
        || CONTAINS.iter().any(|part| upper.contains(part))
        || upper.starts_with("AWS_")
}

/// Environment for an agent process: everything that looks like a secret is
/// removed, except the names in `keep` (the CLI's own login variable).
pub fn clean_env<I>(vars: I, keep: &[&str]) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (String, String)>,
{
    vars.into_iter()
        .filter(|(name, _)| keep.contains(&name.as_str()) || !looks_secret(name))
        .collect()
}

// ---- blocked paths ----------------------------------------------------------

/// Glob match with `*` (any run of characters except `/`), `**` (any run) and `?`.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    fn inner(p: &[u8], t: &[u8]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some(b'*') => {
                if p.get(1) == Some(&b'*') {
                    let rest = &p[2..];
                    let rest = rest.strip_prefix(b"/").unwrap_or(rest);
                    (0..=t.len()).any(|i| inner(rest, &t[i..]))
                } else {
                    let rest = &p[1..];
                    (0..=t.len())
                        .take_while(|i| *i == 0 || t[*i - 1] != b'/')
                        .any(|i| inner(rest, &t[i..]))
                }
            }
            Some(b'?') => !t.is_empty() && t[0] != b'/' && inner(&p[1..], &t[1..]),
            Some(c) => t.first() == Some(c) && inner(&p[1..], &t[1..]),
        }
    }
    inner(pattern.as_bytes(), text.as_bytes())
}

/// True when a path (relative or absolute) hits one of the blocked patterns.
/// `dir/` patterns match the directory anywhere in the path; other patterns
/// match the file name or the whole path.
pub fn is_blocked(path: &str, patterns: &[String]) -> bool {
    let normalized = path.replace('\\', "/");
    let trimmed = normalized.trim_start_matches("./");
    let components: Vec<&str> = trimmed.split('/').filter(|part| !part.is_empty()).collect();
    let name = components.last().copied().unwrap_or("");
    patterns.iter().any(|pattern| {
        if let Some(dir) = pattern.strip_suffix('/') {
            components.iter().any(|part| glob_match(dir, part))
        } else {
            glob_match(pattern, name)
                || glob_match(pattern, trimmed)
                || components.iter().any(|part| glob_match(pattern, part))
        }
    })
}

// ---- secret hashes ----------------------------------------------------------

/// Hashes of the values of a project's `.env` files. The values themselves
/// are never stored: only SHA-256 digests, the value lengths and the key names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SecretSet {
    hashes: HashSet<[u8; 32]>,
    lengths: BTreeSet<usize>,
    pub keys: Vec<String>,
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Parse `KEY=value` lines of a dotenv file.
pub fn parse_dotenv(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, value) = line.split_once('=')?;
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(value);
            Some((key.trim().to_string(), value.to_string()))
        })
        .collect()
}

impl SecretSet {
    pub fn from_dotenv(text: &str) -> Self {
        let mut set = Self::default();
        set.add_dotenv(text);
        set
    }

    pub fn add_dotenv(&mut self, text: &str) {
        for (key, value) in parse_dotenv(text) {
            if !self.keys.contains(&key) {
                self.keys.push(key);
            }
            if value.len() >= MIN_SECRET_LEN {
                self.hashes.insert(digest(value.as_bytes()));
                self.lengths.insert(value.len());
            }
        }
    }

    /// Hash every `.env*` file at the project root.
    pub fn load(project_root: &Path) -> Self {
        let mut set = Self::default();
        let Ok(entries) = std::fs::read_dir(project_root) else {
            return set;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".env")
                && name != ".env.regente"
                && name != ".env.example"
                && entry.path().is_file()
            {
                if let Ok(text) = std::fs::read_to_string(entry.path()) {
                    set.add_dotenv(&text);
                }
            }
        }
        set
    }

    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    /// True when the text contains one of the hashed values.
    pub fn contains_secret(&self, text: &str) -> bool {
        let bytes = text.as_bytes();
        let bytes = &bytes[..bytes.len().min(MAX_SCAN_BYTES)];
        self.lengths.iter().any(|&len| {
            bytes.len() >= len
                && bytes
                    .windows(len)
                    .any(|window| self.hashes.contains(&digest(window)))
        })
    }
}

// ---- key patterns -----------------------------------------------------------

fn run_len(bytes: &[u8], ok: impl Fn(u8) -> bool) -> usize {
    bytes.iter().take_while(|b| ok(**b)).count()
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// Name of a well-known key format found in the text, if any.
pub fn find_key_pattern(text: &str) -> Option<&'static str> {
    let bytes = text.as_bytes();
    let bytes = &bytes[..bytes.len().min(MAX_SCAN_BYTES)];
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY-----") {
        return Some("chave privada");
    }
    for start in 0..bytes.len() {
        let rest = &bytes[start..];
        let boundary = start == 0 || !is_token_char(bytes[start - 1]);
        if !boundary {
            continue;
        }
        if rest.starts_with(b"AKIA")
            && run_len(&rest[4..], |b| b.is_ascii_uppercase() || b.is_ascii_digit()) >= 16
        {
            return Some("chave AWS");
        }
        if rest.starts_with(b"sk_live_") && run_len(&rest[8..], is_token_char) >= 16 {
            return Some("chave Stripe");
        }
        if rest.starts_with(b"sk-") && run_len(&rest[3..], is_token_char) >= 20 {
            return Some("chave de API (sk-)");
        }
        if rest.starts_with(b"ghp_") && run_len(&rest[4..], |b| b.is_ascii_alphanumeric()) >= 30 {
            return Some("token do GitHub");
        }
        if rest.starts_with(b"eyJ") {
            let first = run_len(rest, is_token_char);
            if first >= 10 && rest.get(first) == Some(&b'.') {
                let second_part = &rest[first + 1..];
                let second = run_len(second_part, is_token_char);
                if second >= 10
                    && second_part.starts_with(b"eyJ")
                    && second_part.get(second) == Some(&b'.')
                {
                    return Some("token JWT");
                }
            }
        }
    }
    None
}

/// Reason a text must not leave the agent, or None when it is clean.
/// The reason never includes the secret itself.
pub fn scan(text: &str, secrets: Option<&SecretSet>) -> Option<String> {
    if secrets.is_some_and(|set| set.contains_secret(text)) {
        return Some("valor de segredo do .env".to_string());
    }
    find_key_pattern(text).map(str::to_string)
}

/// Replace every known secret value in the text with `••••`.
pub fn mask(text: &str, values: &[String]) -> String {
    let mut out = text.to_string();
    let mut sorted: Vec<&String> = values
        .iter()
        .filter(|value| value.len() >= MIN_SECRET_LEN)
        .collect();
    sorted.sort_by_key(|value| std::cmp::Reverse(value.len()));
    for value in sorted {
        out = out.replace(value.as_str(), "••••");
    }
    out
}

// ---- irreversible actions ---------------------------------------------------

/// Kind of irreversible action a shell command performs, if any. These wait
/// for the user's approval and are never approved by timeout.
pub fn classify_command(command: &str) -> Option<&'static str> {
    let flat = command.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = flat.to_ascii_lowercase();
    if lower.contains("git push") {
        return Some("push");
    }
    const MIGRATIONS: [&str; 6] = [
        "migrate deploy",
        "db push",
        "migrate:latest",
        "db:migrate",
        "alembic upgrade",
        "migrate up",
    ];
    if MIGRATIONS.iter().any(|needle| lower.contains(needle)) {
        return Some("db_migration");
    }
    const DEPLOYS: [&str; 7] = [
        "npm publish",
        "pnpm publish",
        "cargo publish",
        "terraform apply",
        "kubectl apply",
        "vercel --prod",
        " deploy",
    ];
    if DEPLOYS.iter().any(|needle| lower.contains(needle)) || lower.starts_with("deploy") {
        return Some("deploy");
    }
    let mut words = flat.split(' ');
    while let Some(word) = words.next() {
        if word == "rm" || word.ends_with("/rm") {
            let outside = words
                .clone()
                .take_while(|arg| !matches!(*arg, "&&" | "||" | ";" | "|"))
                .filter(|arg| !arg.starts_with('-'))
                .any(|arg| {
                    arg.starts_with('/')
                        || arg.starts_with('~')
                        || arg.starts_with("..")
                        || arg.contains("/../")
                });
            if outside {
                return Some("delete_outside_worktree");
            }
        }
    }
    None
}

/// Human label (pt-BR) for an irreversible action.
pub fn action_label(action: &str) -> &'static str {
    match action {
        "push" => "Enviar para o repositório remoto (git push)",
        "db_migration" => "Rodar migração em banco real",
        "deploy" => "Publicar ou fazer deploy",
        "delete_outside_worktree" => "Apagar arquivos fora do worktree",
        "merge" => "Integrar o trabalho na branch base",
        _ => "Ação irreversível",
    }
}

// ---- per-CLI deny and sandbox configuration ---------------------------------

fn claude_read_rules(pattern: &str) -> Vec<String> {
    match pattern.strip_suffix('/') {
        Some(dir) => vec![format!("Read(./{dir}/**)"), format!("Read(**/{dir}/**)")],
        None => vec![format!("Read(./{pattern})"), format!("Read(**/{pattern})")],
    }
}

/// `settings.json` for Claude Code: deny reads of the blocked patterns and
/// irreversible commands; the worktree is the only writable place.
pub fn claude_settings(blocked: &[String]) -> Value {
    let mut deny: Vec<String> = blocked
        .iter()
        .flat_map(|pattern| claude_read_rules(pattern))
        .collect();
    for pattern in blocked {
        let target = pattern
            .strip_suffix('/')
            .map(|dir| format!("{dir}/**"))
            .unwrap_or_else(|| pattern.clone());
        deny.push(format!("Edit(**/{target})"));
    }
    deny.extend(
        [
            "Bash(git push:*)",
            "Bash(npm publish:*)",
            "Bash(pnpm publish:*)",
            "Bash(cargo publish:*)",
            "Bash(sudo:*)",
            "WebFetch",
        ]
        .map(str::to_string),
    );
    json!({
        "permissions": {
            "deny": deny,
            "allow": ["Read", "Edit", "Write", "Glob", "Grep", "Bash", "mcp__regente__*"],
            "defaultMode": "acceptEdits",
        }
    })
}

/// `-c` overrides for Codex: write only inside the worktree, network off.
pub fn codex_config(mcp_url: &str) -> Vec<String> {
    vec![
        "sandbox_mode=\"workspace-write\"".to_string(),
        "sandbox_workspace_write.network_access=false".to_string(),
        "approval_policy=\"never\"".to_string(),
        "shell_environment_policy.inherit=\"core\"".to_string(),
        format!("mcp_servers.regente.url=\"{mcp_url}\""),
        "mcp_servers.regente.default_tools_approval_mode=\"approve\"".to_string(),
    ]
}

/// `--settings` for agy: headless mode denies anything not listed here.
pub fn agy_settings(extra_allow: &[String]) -> Value {
    let mut allow: Vec<String> = [
        "send_message",
        "delegate_task",
        "read_inbox",
        "list_agents",
        "report_done",
        "get_project",
        "update_project",
        "create_agent",
        "update_agent",
    ]
    .iter()
    .map(|tool| format!("mcp(regente/{tool})"))
    .collect();
    for command in [
        "ls",
        "cat",
        "git status",
        "git diff",
        "git log",
        "node",
        "pnpm test",
        "npm test",
        "cargo test",
        "regente run",
    ] {
        allow.push(format!("command({command})"));
    }
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        let mcp_dir = home.join(".gemini/antigravity-cli/mcp/regente");
        allow.push(format!("read_file({}/**)", mcp_dir.display()));
        if let Ok(canon) = std::fs::canonicalize(&mcp_dir) {
            if canon != mcp_dir {
                allow.push(format!("read_file({}/**)", canon.display()));
            }
        }
    }
    allow.extend(extra_allow.iter().cloned());
    json!({ "permissions": { "allow": allow } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_rules_restrict_descendants_and_reject_traversal() {
        let root = tempfile::tempdir().unwrap();
        let folders =
            json!([{ "path": "docs", "access": "read" }, { "path": "private", "access": "none" }]);
        assert!(!folder_denied(&folders, "docs/a.md", false));
        assert!(folder_denied(&folders, "docs/a.md", true));
        assert!(folder_denied(&folders, "private/a", false));
        assert!(!folder_denied(&folders, "docs-other/a", true));
        assert!(!safe_relative_path(root.path(), "../outside"));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/tmp", root.path().join("link")).unwrap();
            assert!(!safe_relative_path(root.path(), "link/file"));
        }
    }

    // Fake values only: nothing here is a real credential.
    const FAKE_ENV: &str = "# comment\nDATABASE_URL=\"postgres://fake:fake@localhost/db\"\nexport API_TOKEN='tok_fake_0123456789'\nPORT=3000\nDEBUG=true\n";

    fn patterns() -> Vec<String> {
        DEFAULT_BLOCKED.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn clean_env_removes_secret_looking_names() {
        let vars = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("HOME".to_string(), "/home/u".to_string()),
            ("OPENAI_API_KEY".to_string(), "x".to_string()),
            ("GITHUB_TOKEN".to_string(), "x".to_string()),
            ("DATABASE_URL".to_string(), "x".to_string()),
            ("AWS_PROFILE".to_string(), "x".to_string()),
            ("MY_SECRET_THING".to_string(), "x".to_string()),
            ("ANTHROPIC_API_KEY".to_string(), "x".to_string()),
        ];
        let cleaned: Vec<String> = clean_env(vars.clone(), &[])
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(cleaned, vec!["PATH", "HOME"]);
        let kept: Vec<String> = clean_env(vars, &["ANTHROPIC_API_KEY"])
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(kept, vec!["PATH", "HOME", "ANTHROPIC_API_KEY"]);
    }

    #[test]
    fn glob_matching() {
        assert!(glob_match(".env*", ".env"));
        assert!(glob_match(".env*", ".env.local"));
        assert!(!glob_match(".env*", "src/.env"));
        assert!(glob_match("**/.env*", "src/.env"));
        assert!(glob_match("*.pem", "key.pem"));
        assert!(!glob_match("*.pem", "key.pem.txt"));
        assert!(glob_match("id_?sa", "id_rsa"));
        assert!(glob_match("src/**/*.ts", "src/a/b/c.ts"));
    }

    #[test]
    fn blocked_paths() {
        let patterns = patterns();
        for path in [
            ".env",
            "./.env.local",
            "app/.env.production",
            "secrets/db.txt",
            "a/secrets/b/c",
            "certs/server.pem",
            "/home/u/.ssh/id_rsa",
        ] {
            assert!(is_blocked(path, &patterns), "{path} should be blocked");
        }
        for path in [
            "src/env.ts",
            "README.md",
            "src/secrets.ts",
            "docs/pem.md",
            "identity.rs",
        ] {
            assert!(!is_blocked(path, &patterns), "{path} should be allowed");
        }
    }

    #[test]
    fn dotenv_values_are_hashed_not_stored() {
        let set = SecretSet::from_dotenv(FAKE_ENV);
        assert_eq!(set.keys, vec!["DATABASE_URL", "API_TOKEN", "PORT", "DEBUG"]);
        assert!(set.contains_secret("const url = 'postgres://fake:fake@localhost/db';"));
        assert!(set.contains_secret("header: tok_fake_0123456789\n"));
        assert!(!set.contains_secret("port 3000 and true are too short to be secrets"));
        assert!(!set.contains_secret("nothing to see"));
        assert!(!format!("{set:?}").contains("tok_fake"));
    }

    #[test]
    fn secret_set_loads_env_files_from_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), FAKE_ENV).unwrap();
        std::fs::write(
            dir.path().join(".env.example"),
            "API_TOKEN=example-value-not-secret\n",
        )
        .unwrap();
        let set = SecretSet::load(dir.path());
        assert!(set.contains_secret("tok_fake_0123456789"));
        assert!(!set.contains_secret("example-value-not-secret"));
        assert!(SecretSet::load(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn key_patterns() {
        assert_eq!(
            find_key_pattern("aws AKIAABCDEFGHIJKLMNOP end"),
            Some("chave AWS")
        );
        assert_eq!(
            find_key_pattern("k=sk-abcdefghijklmnopqrstuvwx"),
            Some("chave de API (sk-)")
        );
        assert_eq!(
            find_key_pattern("sk_live_abcdefghijklmnop1234"),
            Some("chave Stripe")
        );
        assert_eq!(
            find_key_pattern("-----BEGIN RSA PRIVATE KEY-----\nabc"),
            Some("chave privada")
        );
        assert_eq!(
            find_key_pattern("t=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NSJ9.sig"),
            Some("token JWT")
        );
        assert_eq!(find_key_pattern("task-sk-short and risk-free text"), None);
        assert_eq!(find_key_pattern("plain text"), None);
    }

    #[test]
    fn scan_reports_reason_without_value() {
        let set = SecretSet::from_dotenv(FAKE_ENV);
        let reason = scan("x tok_fake_0123456789 y", Some(&set)).unwrap();
        assert!(!reason.contains("tok_fake"));
        assert_eq!(scan("hello", Some(&set)), None);
        assert!(scan("AKIAABCDEFGHIJKLMNOP", None).is_some());
    }

    #[test]
    fn mask_hides_values() {
        let values = vec!["tok_fake_0123456789".to_string(), "short".to_string()];
        assert_eq!(
            mask("a tok_fake_0123456789 b short", &values),
            "a •••• b short"
        );
    }

    #[test]
    fn irreversible_commands() {
        assert_eq!(classify_command("git push origin main"), Some("push"));
        assert_eq!(classify_command("cd x && git   push"), Some("push"));
        assert_eq!(
            classify_command("npx prisma migrate deploy"),
            Some("db_migration")
        );
        assert_eq!(classify_command("npm publish"), Some("deploy"));
        assert_eq!(
            classify_command("rm -rf /var/data"),
            Some("delete_outside_worktree")
        );
        assert_eq!(
            classify_command("rm -rf ../other"),
            Some("delete_outside_worktree")
        );
        assert_eq!(classify_command("rm -rf dist"), None);
        assert_eq!(classify_command("git status"), None);
        assert_eq!(classify_command("pnpm test"), None);
    }

    #[test]
    fn cli_configs_follow_blocked_patterns() {
        let settings = claude_settings(&patterns());
        let deny: Vec<&str> = settings["permissions"]["deny"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(deny.contains(&"Read(./.env*)"));
        assert!(deny.contains(&"Read(**/secrets/**)"));
        assert!(deny.contains(&"Bash(git push:*)"));
        let codex = codex_config("http://127.0.0.1:7717/mcp/ag_1");
        assert!(codex
            .iter()
            .any(|c| c == "sandbox_mode=\"workspace-write\""));
        assert!(codex.iter().any(|c| c.contains("network_access=false")));
        let agy = agy_settings(&["command(make)".to_string()]);
        let allow: Vec<&str> = agy["permissions"]["allow"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(allow.contains(&"mcp(regente/report_done)"));
        assert!(allow.contains(&"command(make)"));
    }
}
