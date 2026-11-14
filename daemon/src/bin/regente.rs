//! `regente`: helper CLI used by agents.
//!
//! `regente run -- <command>` runs a command with the project's secrets
//! injected only into that child process, and prints its output with every
//! secret value masked. Irreversible commands wait for the user's approval.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use regente_daemon::mcp::http_request;
use regente_daemon::security::{classify_command, mask, parse_dotenv};
use serde_json::{json, Value};

/// The project root: the folder that holds `.regente/` (worktrees live in
/// `.regente/wt/…`), or the nearest folder with a `.git`.
fn project_root(cwd: &Path) -> PathBuf {
    let mut inside_regente = None;
    for ancestor in cwd.ancestors() {
        if ancestor.file_name().and_then(|name| name.to_str()) == Some(".regente") {
            inside_regente = ancestor.parent();
        }
    }
    if let Some(root) = inside_regente {
        return root.to_path_buf();
    }
    cwd.ancestors()
        .find(|dir| dir.join(".regente").is_dir() || dir.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

fn load_secrets(root: &Path) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for name in [".env", ".env.local"] {
        if let Ok(text) = std::fs::read_to_string(root.join(name)) {
            pairs.extend(parse_dotenv(&text));
        }
    }
    pairs
}

/// Ask the daemon for approval and wait for the decision. Never times out
/// into an approval: only an explicit "approved" lets the command run.
fn wait_for_approval(action: &str, command: &str) -> Result<(), String> {
    let api = std::env::var("REGENTE_API")
        .map_err(|_| "ação irreversível fora de um agente do Regente: recusada".to_string())?;
    let agent = std::env::var("REGENTE_AGENT_ID")
        .map_err(|_| "REGENTE_AGENT_ID ausente: recusada".to_string())?;
    let body = json!({ "agent_id": agent, "action": action, "detail": command }).to_string();
    let (status, reply) = http_request("POST", &format!("{api}/api/v1/approvals"), Some(&body))
        .map_err(|error| format!("daemon inacessível: {error}"))?;
    if status != 201 {
        return Err(format!("o daemon recusou o pedido de aprovação ({status})"));
    }
    let id = serde_json::from_str::<Value>(&reply)
        .ok()
        .and_then(|value| value.get("id").and_then(Value::as_str).map(str::to_string))
        .ok_or("resposta inválida do daemon")?;
    eprintln!("regente: aguardando aprovação do usuário para: {command}");
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let Ok((_, reply)) = http_request("GET", &format!("{api}/api/v1/approvals/{id}"), None)
        else {
            continue;
        };
        match serde_json::from_str::<Value>(&reply)
            .ok()
            .and_then(|value| {
                value
                    .get("status")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .as_deref()
        {
            Some("approved") => return Ok(()),
            Some("denied") => return Err("ação negada pelo usuário".to_string()),
            _ => {}
        }
    }
}

fn pump<R: Read + Send + 'static, W: Write + Send + 'static>(
    reader: R,
    mut writer: W,
    values: Vec<String>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            let Ok(line) = line else { break };
            let _ = writeln!(writer, "{}", mask(&line, &values));
        }
    })
}

fn run(command: &[String]) -> Result<i32, String> {
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    let secrets = load_secrets(&project_root(&cwd));
    let joined = command.join(" ");
    if let Some(action) = classify_command(&joined) {
        wait_for_approval(action, &joined)?;
    }
    let mut child = Command::new(&command[0])
        .args(&command[1..])
        .envs(secrets.iter().cloned())
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("não foi possível rodar {}: {error}", command[0]))?;
    let values: Vec<String> = secrets.into_iter().map(|(_, value)| value).collect();
    let out = child
        .stdout
        .take()
        .map(|stdout| pump(stdout, std::io::stdout(), values.clone()));
    let err = child
        .stderr
        .take()
        .map(|stderr| pump(stderr, std::io::stderr(), values));
    let status = child.wait().map_err(|error| error.to_string())?;
    for handle in [out, err].into_iter().flatten() {
        let _ = handle.join();
    }
    Ok(status.code().unwrap_or(1))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command: Vec<String> = match args.split_first() {
        Some((first, rest)) if first == "run" => rest
            .iter()
            .skip_while(|arg| *arg == "--")
            .cloned()
            .collect(),
        _ => {
            eprintln!("uso: regente run -- <comando>");
            return ExitCode::from(2);
        }
    };
    if command.is_empty() {
        eprintln!("uso: regente run -- <comando>");
        return ExitCode::from(2);
    }
    match run(&command) {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(reason) => {
            eprintln!("regente: {reason}");
            ExitCode::from(3)
        }
    }
}
