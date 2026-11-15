//! Integration tests: the real HTTP server and scheduler, with fake agents.
//! Nothing here runs a real CLI or spends tokens.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use regente_daemon::app::App;
use regente_daemon::config::Config;
use regente_daemon::mcp::http_request;
use regente_daemon::model::{Event, EventKind};
use serde_json::{json, Value};

// Fake value only: used to prove that secrets never leak.
const FAKE_SECRET: &str = "tok_fake_0123456789";

struct Daemon {
    app: Arc<App>,
    addr: std::net::SocketAddr,
    home: tempfile::TempDir,
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
}

async fn daemon_with(tune: impl FnOnce(&mut Config)) -> Daemon {
    let home = tempfile::tempdir().unwrap();
    let mut cfg = Config::for_tests(home.path(), &fixtures());
    tune(&mut cfg);
    start_at(cfg, home).await
}

async fn start_at(cfg: Config, home: tempfile::TempDir) -> Daemon {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (app, addr) = regente_daemon::start(cfg, listener).await.unwrap();
    Daemon { app, addr, home }
}

async fn daemon() -> Daemon {
    daemon_with(|_| {}).await
}

impl Daemon {
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let url = format!("http://{}{path}", self.addr);
        let method = method.to_string();
        tokio::task::spawn_blocking(move || {
            let body = body.map(|value| value.to_string());
            let (status, text) = http_request(&method, &url, body.as_deref()).unwrap();
            (status, serde_json::from_str(&text).unwrap_or(Value::Null))
        })
        .await
        .unwrap()
    }

    async fn get(&self, path: &str) -> Value {
        let (status, value) = self.call("GET", path, None).await;
        assert_eq!(status, 200, "GET {path}: {value}");
        value
    }

    async fn post(&self, path: &str, body: Value) -> Value {
        let (status, value) = self.call("POST", path, Some(body)).await;
        assert!(
            status == 200 || status == 201,
            "POST {path} -> {status}: {value}"
        );
        value
    }

    async fn project(&self, root: &Path) -> String {
        if !root.join(".git").exists() {
            init_repo(root);
        }
        let created = self
            .post(
                "/api/v1/projects",
                json!({ "name": "Demo", "root": root, "context": "Projeto de teste" }),
            )
            .await;
        created["project"]["id"].as_str().unwrap().to_string()
    }

    async fn agent(&self, project: &str, name: &str, scenario: &str, extra: Value) -> String {
        let mut body = json!({
            "project_id": project,
            "name": name,
            "cli": "fake",
            "role": name,
            "account_id": "acc_claude",
            "settings": { "fake_scenario": scenario },
        });
        if let (Value::Object(target), Value::Object(more)) = (&mut body, extra) {
            target.extend(more);
        }
        self.post("/api/v1/agents", body).await["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn task(&self, project: &str, title: &str, extra: Value) -> String {
        let mut body = json!({ "project_id": project, "title": title });
        if let (Value::Object(target), Value::Object(more)) = (&mut body, extra) {
            target.extend(more);
        }
        self.post("/api/v1/tasks", body).await["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn events(&self) -> Vec<Event> {
        self.app.store.all_events().unwrap()
    }

    fn kinds_of(&self, agent: &str) -> Vec<EventKind> {
        self.events()
            .into_iter()
            .filter(|event| event.agent_id == agent)
            .map(|event| event.kind)
            .collect()
    }
}

/// Poll until the condition holds; fail the test after ~20 s.
async fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..800 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for: {what}");
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn init_repo(root: &Path) {
    git(root, &["init", "-q", "-b", "main"]);
    git(
        root,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "init",
        ],
    );
}

/// Git repository with one commit on `main` and an ignored `.env` holding a fake secret.
fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join("README.md"), "# demo\n").unwrap();
    std::fs::write(root.join(".gitignore"), ".env\n").unwrap();
    git(root, &["add", "-A"]);
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
    );
    std::fs::write(root.join(".env"), format!("API_TOKEN={FAKE_SECRET}\n")).unwrap();
    dir
}

// ---- M1: foundation ---------------------------------------------------------

#[tokio::test]
async fn rest_routes_work() {
    let d = daemon().await;
    let root = tempfile::tempdir().unwrap();
    assert_eq!(d.get("/api/v1/health").await["ok"], true);
    assert_eq!(d.get("/api/v1/projects").await, json!([]));

    let (status, error) = d
        .call(
            "POST",
            "/api/v1/projects",
            Some(json!({ "name": "x", "root": "relative/path" })),
        )
        .await;
    assert_eq!(status, 400);
    assert!(error["error"].as_str().unwrap().contains("absoluto"));

    let project = d.project(root.path()).await;
    assert_eq!(d.get("/api/v1/projects").await.as_array().unwrap().len(), 1);

    let (status, _) = d
        .call(
            "POST",
            "/api/v1/agents",
            Some(json!({ "project_id": project, "name": "X", "cli": "emacs" })),
        )
        .await;
    assert_eq!(status, 400);
    let agent = d.agent(&project, "Backend", "basic", json!({})).await;
    let (status, _) = d
        .call(
            "POST",
            "/api/v1/agents",
            Some(json!({ "project_id": project, "name": "backend", "cli": "fake" })),
        )
        .await;
    assert_eq!(status, 400, "duplicate names are refused");
    let listed = d.get(&format!("/api/v1/agents?project={project}")).await;
    assert_eq!(listed[0]["live"]["status"], "idle");
    assert_eq!(listed[0]["account_id"], "acc_claude");

    let (status, patched) = d
        .call(
            "PATCH",
            &format!("/api/v1/agents/{agent}"),
            Some(json!({ "role": "API", "budget_usd": 3.5, "id": "hack" })),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(patched["role"], "API");
    assert_eq!(patched["budget_usd"], 3.5);
    assert_eq!(patched["id"], agent.as_str(), "id cannot be changed");
    assert_eq!(
        d.call("GET", "/api/v1/agents/ag_missing", None).await.0,
        404
    );

    let paused = d
        .post(
            &format!("/api/v1/agents/{agent}/pause"),
            json!({ "reason": "teste" }),
        )
        .await;
    assert_eq!(paused["live"]["status"], "paused");
    assert_eq!(paused["live"]["pause_reason"], "teste");
    let resumed = d
        .post(&format!("/api/v1/agents/{agent}/resume"), json!({}))
        .await;
    assert_eq!(resumed["paused"], false);

    let accounts = d.get("/api/v1/accounts").await;
    assert_eq!(accounts.as_array().unwrap().len(), 3);
    let (status, account) = d
        .call(
            "PATCH",
            "/api/v1/accounts/acc_codex",
            Some(json!({ "max_concurrent": 1, "limit_rule": { "limit_5h_usd": 10 } })),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(account["max_concurrent"], 1);
    assert_eq!(
        d.call(
            "PATCH",
            "/api/v1/accounts/acc_codex",
            Some(json!({ "max_concurrent": 0 }))
        )
        .await
        .0,
        400
    );

    let usage = d.get("/api/v1/usage?window=5h").await;
    let windows = usage["accounts"][0]["windows"].as_object().unwrap();
    assert_eq!(windows.keys().collect::<Vec<_>>(), vec!["5h"]);
    assert_eq!(
        d.call("GET", "/api/v1/usage?window=month", None).await.0,
        400
    );

    assert_eq!(d.get("/api/v1/approvals?status=pending").await, json!([]));
    assert_eq!(
        d.call(
            "POST",
            "/api/v1/approvals/ap_missing",
            Some(json!({ "decision": "approve" }))
        )
        .await
        .0,
        404
    );
    assert_eq!(
        d.call(
            "POST",
            "/api/v1/tasks",
            Some(json!({ "project_id": project, "title": " " }))
        )
        .await
        .0,
        400
    );
    assert_eq!(
        d.get(&format!("/api/v1/agents/{agent}/diff/latest")).await["event"],
        Value::Null
    );
    assert_eq!(d.call("GET", "/api/v1/nothing", None).await.0, 404);

    let snapshot = d.get("/api/v1/snapshot").await;
    for key in [
        "projects",
        "agents",
        "tasks",
        "accounts",
        "usage",
        "approvals",
        "messages",
        "last_seq",
        "api_version",
    ] {
        assert!(snapshot.get(key).is_some(), "snapshot lacks {key}");
    }
}

#[tokio::test]
async fn account_models_route_lists_models() {
    let d = daemon().await;
    let claude = d.get("/api/v1/accounts/acc_claude/models").await;
    assert_eq!(claude["models"][0]["id"], "opus");
    assert_eq!(
        d.call("GET", "/api/v1/accounts/acc_nope/models", None)
            .await
            .0,
        404
    );
}

#[tokio::test]
async fn account_status_and_login_use_the_provider_table() {
    let d = daemon().await;
    // The `fake` provider runs `cli_path` with no arguments; `uname` prints a known line.
    let fake = |id: &str, cli_path: &str| regente_daemon::model::Account {
        id: id.into(),
        provider: "fake".into(),
        login_type: "subscription".into(),
        cli_path: cli_path.into(),
        default_model: String::new(),
        limit_rule: json!({}),
        max_concurrent: 1,
    };
    d.app
        .store
        .upsert_account(&fake("acc_ok", "uname"))
        .unwrap();
    d.app
        .store
        .upsert_account(&fake("acc_off", "false"))
        .unwrap();
    d.app
        .store
        .upsert_account(&fake("acc_missing", "/nonexistent/cli"))
        .unwrap();

    let ok = d.get("/api/v1/accounts/acc_ok/status").await;
    assert_eq!(ok, json!({ "state": "connected", "detail": "Linux" }));
    let off = d.get("/api/v1/accounts/acc_off/status").await;
    assert_eq!(off["state"], "disconnected");
    let missing = d.get("/api/v1/accounts/acc_missing/status").await;
    assert_eq!(missing["state"], "unknown");
    let agy = d.get("/api/v1/accounts/acc_agy/status").await;
    assert_eq!(agy["state"], "unknown");
    assert_eq!(
        d.call("GET", "/api/v1/accounts/acc_nope/status", None)
            .await
            .0,
        404
    );

    // The login terminal runs the login command in a PTY and closes when it exits.
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{}/api/v1/accounts/acc_ok/login", d.addr))
            .await
            .unwrap();
    let mut output = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(Ok(message)) = socket.next().await {
            output.extend_from_slice(&message.into_data());
        }
    })
    .await
    .expect("login terminal did not close");
    assert!(String::from_utf8_lossy(&output).contains("Linux"));
}

#[tokio::test]
async fn task_events_reach_the_websocket_in_order() {
    let d = daemon().await;
    let root = tempfile::tempdir().unwrap();
    let project = d.project(root.path()).await;
    let agent = d.agent(&project, "Backend", "basic", json!({})).await;

    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{}/api/v1/stream", d.addr))
            .await
            .unwrap();
    let first: Value =
        serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(first["type"], "snapshot");
    assert_eq!(first["agents"][0]["id"], agent.as_str());

    let task = d.task(&project, "Rota health", json!({})).await;
    let mut received: Vec<Value> = Vec::new();
    let mut saw_usage = false;
    let collect = async {
        while let Some(Ok(message)) = socket.next().await {
            let value: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            match value["type"].as_str().unwrap() {
                "events" => received.extend(value["items"].as_array().unwrap().iter().cloned()),
                "usage" => saw_usage = true,
                _ => {}
            }
            if received.iter().any(|event| event["kind"] == "task_done") && saw_usage {
                break;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(20), collect)
        .await
        .expect("events did not arrive");

    let seqs: Vec<i64> = received
        .iter()
        .map(|event| event["seq"].as_i64().unwrap())
        .collect();
    assert!(
        seqs.windows(2).all(|pair| pair[0] < pair[1]),
        "events out of order: {seqs:?}"
    );
    let kinds: Vec<&str> = received
        .iter()
        .map(|event| event["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.starts_with(&[
            "task_assigned",
            "status",
            "thinking",
            "file_read",
            "file_edit",
            "command",
            "command_result",
            "thinking",
            "task_done"
        ]),
        "unexpected event order: {kinds:?}"
    );
    assert!(received
        .iter()
        .all(|event| event["task_id"] == task.as_str()));
    let edit = received
        .iter()
        .find(|event| event["kind"] == "file_edit")
        .unwrap();
    assert_eq!(edit["target"], "src/health.ts");
    assert_eq!(edit["lines_added"], 3);
    let session = d.app.store.sessions_of(&agent, &task).unwrap().remove(0);
    let worktree = PathBuf::from(session.worktree.unwrap());
    assert!(worktree.join("src/health.ts").exists());
    assert!(!root.path().join("src/health.ts").exists());

    let timeline = d
        .get(&format!("/api/v1/agents/{agent}/timeline?limit=4"))
        .await;
    assert_eq!(timeline["items"].as_array().unwrap().len(), 4);
    let before = timeline["next_before"].as_i64().unwrap();
    let older = d
        .get(&format!("/api/v1/agents/{agent}/timeline?before={before}"))
        .await;
    let expected: Vec<_> = d
        .app
        .store
        .timeline(&agent, Some(before), 50)
        .unwrap()
        .iter()
        .map(|event| event.seq)
        .collect();
    let actual: Vec<_> = older["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(actual, expected);
    assert!(actual.iter().all(|seq| *seq < before));
    assert_eq!(older["next_before"], Value::Null);
    let diff = d.get(&format!("/api/v1/agents/{agent}/diff/latest")).await;
    assert!(diff["diff"]
        .as_str()
        .unwrap()
        .contains("+export function health()"));
}

#[tokio::test]
async fn restart_rebuilds_state_from_events() {
    let d = daemon().await;
    let root = tempfile::tempdir().unwrap();
    let project = d.project(root.path()).await;
    let agent = d.agent(&project, "Backend", "basic", json!({})).await;
    let task = d.task(&project, "Rota health", json!({})).await;
    wait_for("task done", || {
        d.app.store.get_task(&task).unwrap().status == "done"
    })
    .await;
    wait_for("turn finished with merge approval", || {
        !d.app.is_running(&agent)
            && d.app
                .store
                .list_approvals(Some("pending"))
                .unwrap()
                .iter()
                .any(|approval| {
                    approval.task_id.as_deref() == Some(&task) && approval.action == "merge"
                })
    })
    .await;
    let before = d.get("/api/v1/snapshot").await;
    let live_before = d.app.live_of(&agent);
    assert_eq!(live_before.recent_changes.len(), 1);
    assert!(live_before.cost_usd > 0.0);
    let events_before = d.events().len();

    // A second daemon over the same home is what a restart looks like.
    let Daemon { home, .. } = d;
    let cfg = Config::for_tests(home.path(), &fixtures());
    let restarted = start_at(cfg, home).await;
    assert_eq!(restarted.app.live_of(&agent), live_before);
    assert_eq!(
        restarted.events().len(),
        events_before,
        "no event lost or duplicated"
    );
    let after = restarted.get("/api/v1/snapshot").await;
    for key in [
        "projects", "agents", "tasks", "accounts", "messages", "last_seq",
    ] {
        assert_eq!(before[key], after[key], "{key} changed across restart");
    }
    restarted.app.store.rebuild_derived().unwrap();
    assert_eq!(
        restarted.get("/api/v1/snapshot").await["usage"]["agents"],
        after["usage"]["agents"]
    );
}

#[tokio::test]
async fn running_tasks_are_requeued_on_restart() {
    let home = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let cfg = Config::for_tests(home.path(), &fixtures());
    {
        let (app, _rx) = App::open(cfg.clone()).unwrap();
        let d_project = regente_daemon::model::Project {
            id: "pr_1".into(),
            name: "Demo".into(),
            root: root.path().to_string_lossy().into_owned(),
            base_branch: "main".into(),
            context: String::new(),
            folders: json!([]),
            blocked: json!([]),
            day_budget_usd: None,
            created_at: regente_daemon::model::now(),
        };
        app.store.insert_project(&d_project).unwrap();
        let agent: regente_daemon::model::Agent = serde_json::from_value(json!({
            "id": "ag_1", "project_id": "pr_1", "name": "Backend", "cli": "fake", "role": "", "is_orchestrator": false,
            "parent_id": null, "account_id": null, "model": "", "pet": {}, "permissions": {}, "settings": { "fake_scenario": "basic" },
            "budget_usd": null, "paused": false, "pause_reason": null, "created_at": regente_daemon::model::now()
        }))
        .unwrap();
        app.store.insert_agent(&agent).unwrap();
        let task =
            regente_daemon::orchestrator::create_task(&app, "pr_1", "Interrompida", "", None, None)
                .unwrap();
        // The process died in the middle of the turn.
        app.store.set_task_status(&task.id, "running").unwrap();
    }
    let d = start_at(cfg, home).await;
    wait_for("requeued task finishes", || {
        d.app
            .store
            .list_tasks(None)
            .unwrap()
            .iter()
            .all(|task| task.status == "done")
    })
    .await;
}

// ---- M2: adapters over recorded output --------------------------------------

#[tokio::test]
async fn recorded_cli_output_becomes_contract_events() {
    for cli in ["claude", "codex", "agy"] {
        let d = daemon().await;
        let root = tempfile::tempdir().unwrap();
        let project = d.project(root.path()).await;
        let agent = d
            .agent(&project, "Agente", &format!("replay-{cli}"), json!({}))
            .await;
        let task = d.task(&project, "Tarefa gravada", json!({})).await;
        wait_for("replay done", || {
            d.app.store.get_task(&task).unwrap().status == "done"
        })
        .await;
        let kinds = d.kinds_of(&agent);
        for kind in [
            EventKind::TaskAssigned,
            EventKind::Status,
            EventKind::Thinking,
            EventKind::FileRead,
            EventKind::FileEdit,
            EventKind::Command,
            EventKind::CommandResult,
            EventKind::TaskDone,
        ] {
            assert!(
                kinds.contains(&kind),
                "{cli}: missing {kind:?} in {kinds:?}"
            );
        }
        assert!(
            !kinds.contains(&EventKind::Error),
            "{cli}: unexpected error"
        );
        let usage = d.app.store.usage_since("").unwrap();
        assert!(!usage.is_empty(), "{cli}: usage recorded");
        assert!(
            usage.iter().map(|row| row.cost_usd).sum::<f64>() > 0.0,
            "{cli}: cost reported or estimated"
        );
        let sources: Vec<String> = d
            .events()
            .iter()
            .filter_map(|event| event.usage.as_ref().map(|usage| usage.cost_source.clone()))
            .collect();
        let expected = if cli == "claude" {
            "reported"
        } else {
            "estimated"
        };
        assert!(
            sources.iter().all(|source| source == expected),
            "{cli}: {sources:?}"
        );
        // Paths are shown relative to the working folder.
        let edit = d
            .events()
            .into_iter()
            .find(|event| event.kind == EventKind::FileEdit)
            .unwrap();
        assert!(edit.target.unwrap().ends_with("notes.txt"));
        assert_eq!(
            d.app.store.get_task(&task).unwrap().summary.as_deref(),
            Some("ok")
        );
        let session = d.app.store.sessions_of(&agent, &task).unwrap();
        assert!(
            session[0].cli_session_id.is_some(),
            "{cli}: session id stored for resume"
        );
    }
}

// ---- M3: worktrees and base security ----------------------------------------

#[tokio::test]
async fn reading_a_secret_file_is_blocked_and_writing_a_secret_pauses() {
    let d = daemon().await;
    let root = repo();
    let project = d.project(root.path()).await;
    let agent = d.agent(&project, "Curioso", "leaker", json!({})).await;
    let task = d.task(&project, "Configurar", json!({})).await;
    wait_for("agent paused", || {
        d.app.store.get_agent(&agent).unwrap().paused
    })
    .await;
    wait_for("turn over", || !d.app.is_running(&agent)).await;

    let events = d.events();
    // (1) Reading `.env` is blocked: an alert, never a file_read.
    let blocked = events
        .iter()
        .find(|event| {
            event.kind == EventKind::SecurityAlert && event.summary.contains("Leitura bloqueada")
        })
        .unwrap();
    assert_eq!(blocked.target.as_deref(), Some(".env"));
    assert!(!events.iter().any(|event| event.kind == EventKind::FileRead));
    // (2) Writing a secret value raises a critical alert and pauses the agent.
    let leak = events
        .iter()
        .find(|event| event.kind == EventKind::SecurityAlert && event.summary.contains("Vazamento"))
        .unwrap();
    assert_eq!(leak.target.as_deref(), Some("src/config.ts"));
    assert_eq!(leak.data.as_ref().unwrap()["severity"], "critical");
    let paused = d.app.store.get_agent(&agent).unwrap();
    assert!(paused
        .pause_reason
        .unwrap()
        .contains("Vazamento de segredo"));
    assert_eq!(d.app.store.get_task(&task).unwrap().status, "blocked");
    assert!(
        !events
            .iter()
            .any(|event| event.summary.contains("não deve rodar")),
        "the turn stopped at the leak"
    );
    // The value never reaches an event, and the commit was blocked.
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains(FAKE_SECRET));
    let branch = d.app.store.get_task(&task).unwrap().branch.unwrap();
    assert_eq!(
        git(
            root.path(),
            &["rev-list", "--count", &format!("main..{branch}")]
        ),
        "0"
    );
    // The worktree never had the real `.env`; it got fake values instead.
    let worktree = PathBuf::from(
        d.app.store.sessions_of(&agent, &task).unwrap()[0]
            .worktree
            .clone()
            .unwrap(),
    );
    assert!(!worktree.join(".env").exists());
    assert!(std::fs::read_to_string(worktree.join(".env.regente"))
        .unwrap()
        .contains("API_TOKEN=regente-fake-value"));
}

#[tokio::test]
async fn git_push_waits_for_approval() {
    let d = daemon().await;
    let root = repo();
    let project = d.project(root.path()).await;
    let agent = d.agent(&project, "Entregador", "pusher", json!({})).await;
    let task = d.task(&project, "Publicar", json!({})).await;
    wait_for("approval requested", || {
        !d.app
            .store
            .list_approvals(Some("pending"))
            .unwrap()
            .is_empty()
    })
    .await;

    // (3) The push is parked: nothing ran, and it stays parked.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let pending = d.get("/api/v1/approvals?status=pending").await;
    assert_eq!(pending[0]["action"], "push");
    assert_eq!(pending[0]["detail"], "git push origin main");
    assert_eq!(d.app.live_of(&agent).status, "waiting");
    assert!(!d.kinds_of(&agent).contains(&EventKind::Command));
    assert_eq!(d.app.store.get_task(&task).unwrap().status, "running");

    let id = pending[0]["id"].as_str().unwrap();
    let decided = d
        .post(
            &format!("/api/v1/approvals/{id}"),
            json!({ "decision": "approve", "by": "pedro" }),
        )
        .await;
    assert_eq!(decided["status"], "approved");
    assert_eq!(decided["decided_by"], "pedro");
    wait_for("task done after approval", || {
        d.app.store.get_task(&task).unwrap().status == "done"
    })
    .await;
    let kinds = d.kinds_of(&agent);
    assert!(
        kinds.contains(&EventKind::ApprovalRequested)
            && kinds.contains(&EventKind::ApprovalDecided)
    );
    assert!(kinds.contains(&EventKind::Command));
    assert_eq!(
        d.call(
            "POST",
            &format!("/api/v1/approvals/{id}"),
            Some(json!({ "decision": "deny" }))
        )
        .await
        .0,
        400
    );
}

#[tokio::test]
async fn denied_approval_does_not_run_the_command() {
    let d = daemon().await;
    let root = tempfile::tempdir().unwrap();
    let project = d.project(root.path()).await;
    let agent = d.agent(&project, "Entregador", "pusher", json!({})).await;
    let task = d.task(&project, "Publicar", json!({})).await;
    wait_for("approval requested", || {
        !d.app
            .store
            .list_approvals(Some("pending"))
            .unwrap()
            .is_empty()
    })
    .await;
    let id = d.app.store.list_approvals(Some("pending")).unwrap()[0]
        .id
        .clone();
    d.post(
        &format!("/api/v1/approvals/{id}"),
        json!({ "decision": "deny" }),
    )
    .await;
    wait_for("task done", || {
        d.app.store.get_task(&task).unwrap().status == "done"
    })
    .await;
    let events = d.events();
    assert!(!events
        .iter()
        .any(|event| event.agent_id == agent && event.kind == EventKind::Command));
    assert!(events
        .iter()
        .any(|event| event.kind == EventKind::Error && event.summary.contains("Ação negada")));
}

#[test]
fn regente_run_injects_and_masks_secrets() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".regente/wt/backend-tk_1")).unwrap();
    std::fs::write(
        dir.path().join(".env"),
        format!("API_TOKEN={FAKE_SECRET}\n"),
    )
    .unwrap();
    let cwd = dir.path().join(".regente/wt/backend-tk_1");
    let regente = env!("CARGO_BIN_EXE_regente");

    let output = Command::new(regente)
        .args([
            "run",
            "--",
            "sh",
            "-c",
            "echo token=$API_TOKEN; echo err=$API_TOKEN >&2; exit 7",
        ])
        .current_dir(&cwd)
        .env_remove("API_TOKEN")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7), "exit code is passed through");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stdout.trim(),
        "token=••••",
        "the child saw the secret and the output is masked"
    );
    assert_eq!(stderr.trim(), "err=••••");

    // An irreversible command outside a Regente agent is refused, not run.
    let refused = Command::new(regente)
        .args(["run", "--", "git", "push", "origin", "main"])
        .current_dir(&cwd)
        .env_remove("REGENTE_API")
        .env_remove("REGENTE_AGENT_ID")
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("recusada"));
    assert_eq!(
        Command::new(regente).output().unwrap().status.code(),
        Some(2)
    );
}

// ---- M4: messenger and orchestration ----------------------------------------

async fn team(d: &Daemon, project: &str) -> [String; 4] {
    [
        d.agent(
            project,
            "Maestro",
            "orchestrator",
            json!({ "is_orchestrator": true }),
        )
        .await,
        d.agent(project, "Backend", "backend", json!({})).await,
        d.agent(project, "Frontend", "frontend", json!({})).await,
        d.agent(project, "QA", "qa", json!({})).await,
    ]
}

#[tokio::test]
async fn orchestrator_delegates_agents_talk_and_merge_waits_for_approval() {
    let d = daemon().await;
    let root = repo();
    let project = d.project(root.path()).await;
    let [maestro, backend, frontend, qa] = team(&d, &project).await;
    let task = d
        .task(
            &project,
            "Adicionar GET /health, tela de status e testes",
            json!({ "budget_usd": 2.0 }),
        )
        .await;

    wait_for("root task done", || {
        d.app.store.get_task(&task).unwrap().status == "done"
    })
    .await;
    wait_for("merge approval", || {
        d.app
            .store
            .list_approvals(Some("pending"))
            .unwrap()
            .iter()
            .any(|a| a.action == "merge")
    })
    .await;

    let tasks = d.app.store.list_tasks(Some(&project)).unwrap();
    let step = |title: &str| tasks.iter().find(|t| t.title == title).unwrap().clone();
    let (api, screen, tests) = (step("API health"), step("Tela de status"), step("Testes"));
    assert!(
        tasks.iter().all(|t| t.status == "done"),
        "every step reported"
    );
    assert_eq!(tests.depends_on, vec![api.id.clone(), screen.id.clone()]);
    assert_eq!(
        (
            api.owner_agent_id.as_str(),
            screen.owner_agent_id.as_str(),
            tests.owner_agent_id.as_str()
        ),
        (backend.as_str(), frontend.as_str(), qa.as_str())
    );

    let events = d.events();
    let seq_of = |agent: &str, kind: EventKind, task_id: &str| {
        events
            .iter()
            .find(|e| {
                e.agent_id == agent && e.kind == kind && e.task_id.as_deref() == Some(task_id)
            })
            .map(|e| e.seq)
            .unwrap()
    };
    // QA only starts after both steps reported done.
    let qa_start = seq_of(&qa, EventKind::Status, &tests.id);
    assert!(qa_start > seq_of(&backend, EventKind::TaskDone, &api.id));
    assert!(qa_start > seq_of(&frontend, EventKind::TaskDone, &screen.id));
    // The orchestrator consolidated only after QA.
    assert!(
        seq_of(&maestro, EventKind::TaskDone, &task) > seq_of(&qa, EventKind::TaskDone, &tests.id)
    );

    // Frontend asked, Backend answered, and the conversation is in the events.
    let messages = d.app.store.recent_messages(50).unwrap();
    assert!(messages
        .iter()
        .any(|m| m.from_agent == frontend && m.to_agent == backend && m.text.contains("formato")));
    assert!(messages
        .iter()
        .any(|m| m.from_agent == backend && m.to_agent == frontend && m.text.contains("version")));
    assert!(
        messages.iter().all(|m| m.delivered_at.is_some()),
        "every message was read"
    );
    assert!(events
        .iter()
        .any(|e| e.kind == EventKind::MessageSent && e.agent_id == frontend));
    assert!(events
        .iter()
        .any(|e| e.kind == EventKind::MessageReceived && e.agent_id == frontend));

    // Separate worktrees and branches per agent.
    let worktree_of = |agent: &str, task_id: &str| {
        d.app.store.sessions_of(agent, task_id).unwrap()[0]
            .worktree
            .clone()
            .unwrap()
    };
    assert_ne!(
        worktree_of(&backend, &api.id),
        worktree_of(&frontend, &screen.id)
    );
    assert_ne!(api.branch, screen.branch);
    // QA's worktree started from the work it depends on.
    let qa_worktree = PathBuf::from(worktree_of(&qa, &tests.id));
    assert!(
        qa_worktree.join("src/health.ts").exists() && qa_worktree.join("src/Status.tsx").exists()
    );

    // A file changed through a shell command still shows up as a change.
    let by_shell = events
        .iter()
        .find(|e| e.kind == EventKind::FileEdit && e.target.as_deref() == Some("CHANGELOG.md"))
        .unwrap();
    assert_eq!(
        (by_shell.agent_id.as_str(), by_shell.lines_added),
        (backend.as_str(), Some(1))
    );
    // Agents that are done are shown as idle, not as working.
    for agent in [&backend, &frontend, &qa] {
        assert_eq!(d.app.live_of(agent).status, "idle");
    }

    // Budget respected, per-account usage visible, no secret anywhere.
    let spent = d.app.store.cost_of_task_tree(&task).unwrap();
    assert!(spent > 0.0 && spent < 2.0, "spent {spent}");
    let usage = d.get("/api/v1/usage").await;
    let claude = usage["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["account_id"] == "acc_claude")
        .unwrap();
    for window in ["5h", "day", "week"] {
        assert!(
            claude["windows"][window]["cost_usd"].as_f64().unwrap() > 0.0,
            "{window}"
        );
    }
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains(FAKE_SECRET));
    assert!(!serde_json::to_string(&messages)
        .unwrap()
        .contains(FAKE_SECRET));

    // The final merge is parked until the user approves it.
    assert!(!root.path().join("src/health.ts").exists());
    assert_eq!(d.app.live_of(&maestro).status, "waiting");
    let approval = d
        .app
        .store
        .list_approvals(Some("pending"))
        .unwrap()
        .remove(0);
    d.post(
        &format!("/api/v1/approvals/{}", approval.id),
        json!({ "decision": "approve" }),
    )
    .await;
    wait_for("merge applied", || {
        root.path().join("tests/e2e.test.ts").exists()
    })
    .await;
    wait_for("worktrees cleaned", || {
        !PathBuf::from(worktree_of(&qa, &tests.id)).exists()
    })
    .await;
    for file in [
        "src/health.ts",
        "src/health.test.ts",
        "src/Status.tsx",
        "tests/e2e.test.ts",
    ] {
        assert!(root.path().join(file).exists(), "{file} merged");
    }
    assert!(git(root.path(), &["log", "--format=%s"]).contains("regente: API health (Backend)"));
    assert_eq!(
        git(
            root.path(),
            &["status", "--porcelain", "--untracked-files=no"]
        ),
        ""
    );
}

#[tokio::test]
async fn a_conversation_loop_is_cut_by_the_message_limit() {
    let d = daemon_with(|cfg| cfg.stall_limit = 1000).await;
    let root = tempfile::tempdir().unwrap();
    let project = d.project(root.path()).await;
    let alfa = d
        .agent(
            &project,
            "Alfa",
            "chatter",
            json!({ "settings": { "fake_scenario": "chatter", "peer": "Beta" } }),
        )
        .await;
    let beta = d
        .agent(
            &project,
            "Beta",
            "chatter",
            json!({ "settings": { "fake_scenario": "chatter", "peer": "Alfa" } }),
        )
        .await;
    let task = d
        .task(&project, "Discutir sem fim", json!({ "agent_id": alfa }))
        .await;
    wait_for("conversation cut", || {
        let (a, b) = (
            d.app.store.get_agent(&alfa).unwrap(),
            d.app.store.get_agent(&beta).unwrap(),
        );
        a.paused || b.paused
    })
    .await;
    wait_for("turns over", || d.app.running_count() == 0).await;
    let root_task = d.app.store.get_task(&task).unwrap();
    assert_eq!(root_task.message_count, 21, "the 21st message was refused");
    assert_eq!(d.app.store.recent_messages(100).unwrap().len(), 20);
    let reasons: Vec<String> = [&alfa, &beta]
        .iter()
        .filter_map(|id| d.app.store.get_agent(id).unwrap().pause_reason)
        .collect();
    assert!(reasons
        .iter()
        .any(|reason| reason.contains("Limite de 20 mensagens")));
    assert!(d
        .events()
        .iter()
        .any(|e| e.kind == EventKind::Error && e.summary.contains("conversa cortada")));
    let open: Vec<_> = d
        .app
        .store
        .list_tasks(Some(&project))
        .unwrap()
        .into_iter()
        .filter(|t| t.status != "done")
        .collect();
    assert!(
        open.iter().all(|t| t.status == "blocked"),
        "nothing keeps running: {open:?}"
    );
}

#[tokio::test]
async fn three_rounds_without_progress_pause_the_agent() {
    let d = daemon().await;
    let root = tempfile::tempdir().unwrap();
    let project = d.project(root.path()).await;
    let alfa = d
        .agent(
            &project,
            "Alfa",
            "chatter",
            json!({ "settings": { "fake_scenario": "chatter", "peer": "Beta" } }),
        )
        .await;
    d.agent(
        &project,
        "Beta",
        "chatter",
        json!({ "settings": { "fake_scenario": "chatter", "peer": "Alfa" } }),
    )
    .await;
    let task = d
        .task(&project, "Discutir", json!({ "agent_id": alfa }))
        .await;
    wait_for("stalled agent paused", || {
        d.app.store.get_agent(&alfa).unwrap().paused
    })
    .await;
    assert_eq!(
        d.app
            .store
            .get_agent(&alfa)
            .unwrap()
            .pause_reason
            .as_deref(),
        Some("3 rodadas sem progresso")
    );
    assert_eq!(d.app.store.get_task(&task).unwrap().status, "blocked");
    assert!(d.app.store.get_task(&task).unwrap().message_count < 20);
}

#[tokio::test]
async fn account_concurrency_limit_is_respected() {
    let d = daemon_with(|cfg| cfg.fake_step_ms = 15).await;
    let root = tempfile::tempdir().unwrap();
    let project = d.project(root.path()).await;
    d.call(
        "PATCH",
        "/api/v1/accounts/acc_codex",
        Some(json!({ "max_concurrent": 1 })),
    )
    .await;
    let mut pairs = Vec::new();
    for name in ["Um", "Dois", "Tres"] {
        let agent = d
            .agent(
                &project,
                name,
                "basic",
                json!({ "account_id": "acc_codex" }),
            )
            .await;
        pairs.push((
            agent.clone(),
            d.task(
                &project,
                &format!("Tarefa {name}"),
                json!({ "agent_id": agent }),
            )
            .await,
        ));
    }
    let mut peak = 0;
    wait_for("all tasks done", || {
        peak = peak.max(d.app.running_on_account("acc_codex"));
        pairs
            .iter()
            .all(|(_, task)| d.app.store.get_task(task).unwrap().status == "done")
    })
    .await;
    assert_eq!(peak, 1, "never more than one agent of the account at once");
    let mut sessions: Vec<_> = pairs
        .iter()
        .map(|(agent, task)| d.app.store.sessions_of(agent, task).unwrap().remove(0))
        .collect();
    sessions.sort_by(|a, b| a.started_at.cmp(&b.started_at));
    for pair in sessions.windows(2) {
        assert!(
            pair[0].ended_at.as_ref().unwrap() <= &pair[1].started_at,
            "turns overlapped"
        );
    }
}

#[tokio::test]
async fn messenger_speaks_mcp_over_http() {
    let d = daemon().await;
    let root = tempfile::tempdir().unwrap();
    let project = d.project(root.path()).await;
    let backend = d.agent(&project, "Backend", "basic", json!({})).await;
    d.agent(&project, "Frontend", "basic", json!({})).await;
    let path = format!("/mcp/{backend}");
    let init = d.post(&path, json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18" } })).await;
    assert_eq!(init["result"]["serverInfo"]["name"], "regente");
    assert_eq!(
        d.call(
            "POST",
            &path,
            Some(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        )
        .await
        .0,
        202
    );
    let tools = d
        .post(
            &path,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        )
        .await;
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 9);
    let sent = d
        .post(&path, json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "send_message", "arguments": { "to": "Frontend", "text": "oi" } } }))
        .await;
    assert_eq!(sent["result"]["isError"], false);
    assert_eq!(d.get("/api/v1/messages").await.as_array().unwrap().len(), 1);
}

// ---- M6: budgets ------------------------------------------------------------

#[tokio::test]
async fn an_agent_over_budget_shows_up_paused_with_the_reason() {
    let d = daemon().await;
    let root = tempfile::tempdir().unwrap();
    let project = d.project(root.path()).await;
    let agent = d.agent(&project, "Gastador", "spender", json!({})).await;
    let task = d
        .task(&project, "Refatorar tudo", json!({ "budget_usd": 1.0 }))
        .await;
    wait_for("paused by budget", || {
        d.app.store.get_agent(&agent).unwrap().paused
    })
    .await;
    wait_for("turn over", || !d.app.is_running(&agent)).await;
    let view = d.get(&format!("/api/v1/agents/{agent}")).await;
    assert_eq!(view["live"]["status"], "paused");
    assert!(view["live"]["pause_reason"]
        .as_str()
        .unwrap()
        .contains("Orçamento da tarefa esgotado"));
    assert_eq!(d.app.store.get_task(&task).unwrap().status, "blocked");
    let spent = d.app.store.cost_of_task_tree(&task).unwrap();
    assert!(
        (1.0..1.5).contains(&spent),
        "stopped right after the limit: {spent}"
    );
    assert!(
        d.events().iter().any(|e| e
            .data
            .as_ref()
            .is_some_and(|data| data.get("budget_warning").is_some())),
        "warned at 80%"
    );
    // Resuming puts the task back in the queue (and it pauses again: still over budget).
    d.post(&format!("/api/v1/agents/{agent}/resume"), json!({}))
        .await;
    wait_for("paused again", || {
        d.app.store.get_agent(&agent).unwrap().paused
    })
    .await;
}

// ---- M7: create project and agents ------------------------------------------

#[tokio::test]
async fn creating_a_project_writes_team_files_and_runs_with_its_permissions() {
    let d = daemon().await;
    let root = tempfile::tempdir().unwrap();
    init_repo(root.path());
    std::fs::write(root.path().join("CLAUDE.md"), "# Notas do Pedro\n").unwrap();
    let created = d
        .post(
            "/api/v1/projects",
            json!({
                "name": "Loja",
                "root": root.path(),
                "context": "API de pedidos em TypeScript",
                "day_budget_usd": 5,
                "blocked": ["*.secret", "README.md"],
                "folders": [{ "path": "src", "access": "write" }, { "path": "docs", "access": "read" }],
                "agents": [
                    { "name": "Maestro", "cli": "fake", "role": "Coordena", "is_orchestrator": true, "settings": { "fake_scenario": "basic" } },
                    { "name": "Backend", "cli": "codex", "role": "API", "pet": { "species": "gato" } }
                ]
            }),
        )
        .await;
    assert_eq!(
        created["files"],
        json!([".regente/team.yaml", "CLAUDE.md", "AGENTS.md"])
    );
    let yaml = std::fs::read_to_string(root.path().join(".regente/team.yaml")).unwrap();
    assert!(
        yaml.contains("name: \"Loja\"")
            && yaml.contains("cli: \"codex\"")
            && yaml.contains("\"species\":\"gato\"")
    );
    assert!(!yaml.contains(&*root.path().to_string_lossy()));
    let claude_md = std::fs::read_to_string(root.path().join("CLAUDE.md")).unwrap();
    assert!(
        claude_md.starts_with("# Notas do Pedro"),
        "the user's text is kept"
    );
    assert!(claude_md.contains("API de pedidos em TypeScript") && claude_md.contains("*.secret"));

    // The team shows up for the sidebar.
    let project = created["project"]["id"].as_str().unwrap();
    let snapshot = d.get("/api/v1/snapshot").await;
    let names: Vec<&str> = snapshot["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Maestro", "Backend"]);
    assert_eq!(snapshot["agents"][1]["account_id"], "acc_codex");

    // A fake task runs with the chosen blocked patterns: README.md is now off limits.
    let task = d.task(project, "Primeira tarefa", json!({})).await;
    wait_for("task done", || {
        d.app.store.get_task(&task).unwrap().status == "done"
    })
    .await;
    let events = d.events();
    assert!(
        events
            .iter()
            .any(|e| e.kind == EventKind::SecurityAlert
                && e.summary == "Leitura bloqueada: README.md")
    );
    assert!(!events.iter().any(|e| e.kind == EventKind::FileRead));

    // Adding an agent later regenerates the files.
    d.agent(project, "QA", "basic", json!({})).await;
    assert!(
        std::fs::read_to_string(root.path().join(".regente/team.yaml"))
            .unwrap()
            .contains("name: \"QA\"")
    );
}

#[tokio::test]
async fn browser_origins_and_hosts_are_checked_before_mutations_and_websockets() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let daemon = daemon().await;
    let app = regente_daemon::api::router(daemon.app.clone());
    for (method, path, host, origin, blocked) in [
        (
            "POST",
            "/api/v1/tasks",
            "127.0.0.1:7717",
            Some("https://evil.example"),
            true,
        ),
        (
            "GET",
            "/api/v1/agents/a/pty",
            "127.0.0.1:7717",
            Some("https://evil.example"),
            true,
        ),
        (
            "GET",
            "/api/v1/stream",
            "evil.example:7717",
            Some("http://localhost:5173"),
            true,
        ),
        (
            "POST",
            "/api/v1/tasks",
            "localhost.evil.example",
            None,
            true,
        ),
        ("POST", "/api/v1/tasks", "127.0.0.1:bad", None, true),
        (
            "POST",
            "/api/v1/pick-folder",
            "127.0.0.1:7717",
            Some("https://evil.example"),
            true,
        ),
        // The route exists; GET never opens a dialog.
        ("GET", "/api/v1/pick-folder", "127.0.0.1:7717", None, false),
        (
            "POST",
            "/api/v1/tasks",
            "localhost:7717",
            Some("http://localhost:5173"),
            false,
        ),
        (
            "GET",
            "/api/v1/agents/a/pty",
            "[::1]:7717",
            Some("http://127.0.0.1:5173"),
            false,
        ),
        ("GET", "/api/v1/health", "127.0.0.1:7717", None, false),
    ] {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("host", host);
        if let Some(origin) = origin {
            builder = builder.header("origin", origin);
        }
        let response = app
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status() == StatusCode::FORBIDDEN,
            blocked,
            "{method} {path}, host {host}, origin {origin:?}"
        );
    }
}

#[tokio::test]
async fn project_and_agent_defaults_follow_the_repository_and_engine() {
    let d = daemon().await;
    let root = tempfile::tempdir().unwrap();
    init_repo(root.path());
    git(root.path(), &["checkout", "-q", "-b", "dev"]);
    let created = d
        .post(
            "/api/v1/projects",
            json!({
                "name": "Demo",
                "root": root.path(),
                "agents": [{ "cli": "fake", "is_orchestrator": true }, { "cli": "fake" }]
            }),
        )
        .await;
    assert_eq!(created["project"]["base_branch"], "dev");
    assert_eq!(created["agents"][0]["name"], "Teste");
    assert_eq!(created["agents"][0]["role"], "Orquestrador");
    assert_eq!(created["agents"][1]["name"], "Teste 2");
    assert_eq!(created["agents"][1]["role"], "Agente");
    let project = created["project"]["id"].as_str().unwrap();
    for (cli, expected) in [
        ("claude", "Claude"),
        ("claude", "Claude 2"),
        ("agy", "Antigravity"),
    ] {
        let agent = d
            .post(
                "/api/v1/agents",
                json!({ "project_id": project, "cli": cli }),
            )
            .await;
        assert_eq!(agent["name"], expected);
        assert_eq!(agent["parent_id"], created["agents"][0]["id"]);
    }
    let (status, _) = d
        .call(
            "POST",
            "/api/v1/agents",
            Some(json!({ "project_id": project, "cli": "codex", "name": "claude" })),
        )
        .await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn invalid_projects_leave_no_database_rows_or_generated_files() {
    let d = daemon().await;
    let root = repo();
    let cases = vec![
        json!({"agents":[{"name":"A","cli":"fake"},{"name":"a","cli":"fake"}]}),
        json!({"agents":[{"name":"A","cli":"invalid"}]}),
        json!({"base_branch":"missing"}),
        json!({"folders":[{"path":"../outside","access":"write"}]}),
        json!({"folders":[{"path":"/tmp/outside","access":"write"}]}),
    ];
    #[cfg(unix)]
    std::os::unix::fs::symlink("/tmp", root.path().join("link")).unwrap();
    let mut cases = cases;
    #[cfg(unix)]
    cases.push(json!({"folders":[{"path":"link/outside","access":"write"}]}));
    for extra in cases {
        let mut body = json!({"name":"Demo", "root":root.path()});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let (status, _) = d.call("POST", "/api/v1/projects", Some(body)).await;
        assert_eq!(status, 400);
        assert_eq!(d.get("/api/v1/projects").await, json!([]));
        assert!(!root.path().join(".regente").exists());
        assert!(!root.path().join("CLAUDE.md").exists());
    }
}

#[tokio::test]
async fn project_file_error_does_not_persist_project() {
    let d = daemon().await;
    let root = repo();
    std::fs::create_dir_all(root.path().join(".regente/team.yaml")).unwrap();
    std::fs::write(root.path().join("CLAUDE.md"), "existing").unwrap();
    let (status, _) = d
        .call(
            "POST",
            "/api/v1/projects",
            Some(json!({"name":"Demo", "root":root.path(), "agents":[{"name":"A","cli":"fake"}]})),
        )
        .await;
    assert!(status >= 400);
    assert_eq!(d.get("/api/v1/projects").await, json!([]));
    assert_eq!(
        std::fs::read_to_string(root.path().join("CLAUDE.md")).unwrap(),
        "existing"
    );
}

#[tokio::test]
async fn restricted_folders_block_real_cli_before_worktree_and_guard_fake_steps() {
    let d = daemon().await;
    let root = repo();
    let created = d.post("/api/v1/projects", json!({"name":"Demo", "root":root.path(), "folders":[{"path":"README.md","access":"none"},{"path":"src","access":"read"}], "agents":[{"name":"Fake","cli":"fake","is_orchestrator":true}]})).await;
    let project = created["project"]["id"].as_str().unwrap();
    let fake = created["agents"][0]["id"].as_str().unwrap();
    let task = d.task(project, "Fake", json!({})).await;
    wait_for("fake task done", || {
        d.app.store.get_task(&task).unwrap().status == "done"
    })
    .await;
    let events = d.events();
    assert!(events.iter().any(|e| e.agent_id == fake
        && e.kind == EventKind::SecurityAlert
        && e.summary.contains("Leitura bloqueada")));
    assert!(events.iter().any(|e| e.agent_id == fake
        && e.kind == EventKind::SecurityAlert
        && e.summary.contains("Escrita bloqueada")));
    assert!(!root.path().join("src/health.ts").exists());
    let real = d
        .post(
            "/api/v1/agents",
            json!({"project_id":project,"name":"Real","cli":"codex"}),
        )
        .await;
    let real_id = real["id"].as_str().unwrap();
    let real_task = d.task(project, "Real", json!({"agent_id":real_id})).await;
    wait_for("real task blocked", || {
        d.app.store.get_task(&real_task).unwrap().status == "blocked"
    })
    .await;
    assert!(d
        .app
        .store
        .sessions_of(real_id, &real_task)
        .unwrap()
        .is_empty());
    assert!(d.events().iter().any(|e| e.agent_id == real_id
        && e.kind == EventKind::Error
        && e.summary.contains("acesso restrito")));
}

#[tokio::test]
async fn checkpoint_does_not_commit_a_restricted_change_made_during_turn() {
    let fixture_dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(fixture_dir.path().join("fake")).unwrap();
    std::fs::write(
        fixture_dir.path().join("fake/hold.json"),
        r#"{"turns":[[{"do":"sleep","ms":1500}]]}"#,
    )
    .unwrap();
    let d = daemon_with(|cfg| cfg.fixtures_dir = fixture_dir.path().to_path_buf()).await;
    let root = repo();
    let created = d.post("/api/v1/projects", json!({"name":"Demo", "root":root.path(), "folders":[{"path":"protected","access":"read"}], "agents":[{"name":"Fake","cli":"fake","is_orchestrator":true,"settings":{"fake_scenario":"hold"}}]})).await;
    let project = created["project"]["id"].as_str().unwrap();
    let agent = created["agents"][0]["id"].as_str().unwrap();
    let task = d.task(project, "Alterar", json!({})).await;
    wait_for("session", || {
        !d.app.store.sessions_of(agent, &task).unwrap().is_empty()
    })
    .await;
    let session = d.app.store.sessions_of(agent, &task).unwrap().remove(0);
    let cwd = PathBuf::from(session.worktree.unwrap());
    std::fs::create_dir_all(cwd.join("protected")).unwrap();
    std::fs::write(cwd.join("protected/blocked.txt"), "restricted change").unwrap();
    wait_for("agent paused", || {
        d.app.store.get_agent(agent).unwrap().paused
    })
    .await;
    assert!(d.events().iter().any(|event| event.agent_id == agent
        && event.kind == EventKind::SecurityAlert
        && event.summary.contains("pasta restrita")));
    assert!(git(&cwd, &["status", "--porcelain"]).contains("protected/"));
    assert_eq!(
        git(&cwd, &["rev-parse", "HEAD"]),
        git(root.path(), &["rev-parse", "main"])
    );
}

#[tokio::test]
async fn report_done_waits_for_checkpoint_before_starting_dependent_qa() {
    let d = daemon().await;
    let root = repo();
    let project = d.project(root.path()).await;
    let maestro = d
        .agent(
            &project,
            "Maestro",
            "orchestrator",
            json!({"is_orchestrator":true}),
        )
        .await;
    let _backend = d.agent(&project, "Backend", "backend", json!({})).await;
    let frontend = d
        .agent(&project, "Frontend", "frontend-late", json!({}))
        .await;
    let qa = d.agent(&project, "QA", "qa", json!({})).await;
    let task = d
        .task(&project, "Teste de barreira", json!({"agent_id":maestro}))
        .await;
    wait_for("report intent", || {
        d.app
            .store
            .list_tasks(Some(&project))
            .unwrap()
            .iter()
            .any(|item| {
                item.owner_agent_id == frontend && item.summary.as_deref() == Some("Tela pronta.")
            })
    })
    .await;
    let tasks = d.app.store.list_tasks(Some(&project)).unwrap();
    let screen = tasks
        .iter()
        .find(|item| item.owner_agent_id == frontend)
        .unwrap();
    let tests = tasks.iter().find(|item| item.owner_agent_id == qa).unwrap();
    assert_eq!(screen.status, "running");
    assert_eq!(tests.status, "queued");
    assert!(d.app.store.sessions_of(&qa, &tests.id).unwrap().is_empty());
    assert!(!d
        .events()
        .iter()
        .any(|event| event.task_id.as_deref() == Some(&screen.id)
            && event.kind == EventKind::TaskDone));
    let screen_id = screen.id.clone();
    let qa_id = tests.id.clone();
    wait_for("qa done", || {
        d.app.store.get_task(&qa_id).unwrap().status == "done"
    })
    .await;
    let qa_dir = PathBuf::from(
        d.app.store.sessions_of(&qa, &qa_id).unwrap()[0]
            .worktree
            .clone()
            .unwrap(),
    );
    assert_eq!(
        std::fs::read_to_string(qa_dir.join("src/Status.tsx")).unwrap(),
        "export const status = 'ok';\n"
    );
    let events = d.events();
    let edit = events
        .iter()
        .find(|e| {
            e.agent_id == frontend
                && e.kind == EventKind::FileEdit
                && e.target.as_deref() == Some("src/Status.tsx")
        })
        .unwrap()
        .seq;
    let done = events
        .iter()
        .find(|e| e.task_id.as_deref() == Some(&screen_id) && e.kind == EventKind::TaskDone)
        .unwrap()
        .seq;
    let qa_start = events
        .iter()
        .find(|e| e.task_id.as_deref() == Some(&qa_id) && e.kind == EventKind::Status)
        .unwrap()
        .seq;
    assert!(edit < done && done < qa_start);
    wait_for("root completed", || {
        d.app.store.get_task(&task).unwrap().status == "done"
    })
    .await;
}

#[tokio::test]
async fn user_edits_project_settings_and_folder_after_creation() {
    let d = daemon().await;
    let root = tempfile::tempdir().unwrap();
    let project = d.project(root.path()).await;
    let path = format!("/api/v1/projects/{project}");

    let (status, edited) = d
        .call(
            "PATCH",
            &path,
            Some(json!({ "name": "Loja", "context": "Loja online" })),
        )
        .await;
    assert_eq!(status, 200, "{edited}");
    assert_eq!(edited["name"], "Loja");
    assert_eq!(edited["context"], "Loja online");

    let other = tempfile::tempdir().unwrap();
    init_repo(other.path());
    let (status, moved) = d
        .call("PATCH", &path, Some(json!({ "root": other.path() })))
        .await;
    assert_eq!(status, 200, "{moved}");
    assert_eq!(moved["root"], json!(other.path()));
    assert!(other.path().join(".regente/team.yaml").exists());
    let listed = d.get("/api/v1/projects").await;
    assert_eq!(listed[0]["root"], json!(other.path()));

    let plain = tempfile::tempdir().unwrap();
    let (status, _) = d
        .call("PATCH", &path, Some(json!({ "root": plain.path() })))
        .await;
    assert_eq!(status, 400, "a pasta sem Git é recusada");

    let agent = d.agent(&project, "Dev", "basic", json!({})).await;
    d.post(&format!("/api/v1/agents/{agent}/pause"), json!({}))
        .await;
    d.task(&project, "Em andamento", json!({})).await;
    let (status, refused) = d
        .call("PATCH", &path, Some(json!({ "root": root.path() })))
        .await;
    assert_eq!(status, 400, "tarefa ativa bloqueia a troca: {refused}");
    assert_eq!(
        d.get("/api/v1/projects").await[0]["root"],
        json!(other.path())
    );
}
