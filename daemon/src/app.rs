//! Shared daemon state: event emission, live state, pause and approvals.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot, watch, Notify};

use crate::config::{Config, Prices};
use crate::model::{new_id, now, Account, Approval, Event, EventKind, NewEvent, API_VERSION};
use crate::security::SecretSet;
use crate::store::{Store, StoreError};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Store(StoreError),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("git: {0}")]
    Git(String),
    #[error("{0}")]
    Other(String),
}

impl From<StoreError> for AppError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NotFound(what) => AppError::NotFound(what),
            other => AppError::Store(other),
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;

/// One recent file change shown on the overview.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Change {
    pub event_id: String,
    pub ts: String,
    pub path: String,
    pub summary: String,
    pub lines_added: i64,
    pub lines_removed: i64,
}

/// Live state of one agent, derived only from events.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AgentLive {
    /// working, thinking, waiting, idle or paused
    pub status: String,
    pub doing_now: String,
    pub task_id: Option<String>,
    pub recent_changes: Vec<Change>,
    pub cost_usd: f64,
    pub tokens: i64,
    pub last_ts: Option<String>,
    pub pause_reason: Option<String>,
}

const RECENT_CHANGES: usize = 20;

/// In-memory cache of the live state; rebuilt from the event log on start.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Live {
    pub agents: HashMap<String, AgentLive>,
    pub last_seq: i64,
}

impl Live {
    pub fn from_events(events: &[Event]) -> Self {
        let mut live = Self::default();
        for event in events {
            live.apply(event);
        }
        live
    }

    pub fn apply(&mut self, event: &Event) {
        self.last_seq = self.last_seq.max(event.seq);
        let agent = self
            .agents
            .entry(event.agent_id.clone())
            .or_insert_with(|| AgentLive {
                status: "idle".into(),
                ..Default::default()
            });
        agent.last_ts = Some(event.ts.clone());
        if let Some(usage) = &event.usage {
            agent.cost_usd += usage.cost_usd;
            agent.tokens += usage.tokens_in + usage.tokens_out;
        }
        let paused = agent.status == "paused";
        match event.kind {
            EventKind::Status => {
                let status = event
                    .data
                    .as_ref()
                    .and_then(|d| d.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or("idle");
                agent.status = status.to_string();
                agent.doing_now = event.summary.clone();
                agent.pause_reason = if status == "paused" {
                    event
                        .data
                        .as_ref()
                        .and_then(|d| d.get("reason"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                } else {
                    None
                };
                if event.task_id.is_some() {
                    agent.task_id = event.task_id.clone();
                }
            }
            EventKind::Thinking => {
                if !paused {
                    agent.status = "thinking".into();
                    agent.doing_now = event.summary.clone();
                }
            }
            EventKind::FileRead
            | EventKind::FileEdit
            | EventKind::Command
            | EventKind::CommandResult => {
                if !paused {
                    agent.status = "working".into();
                    agent.doing_now = event.summary.clone();
                }
                if event.kind == EventKind::FileEdit {
                    if let Some(path) = &event.target {
                        agent.recent_changes.insert(
                            0,
                            Change {
                                event_id: event.id.clone(),
                                ts: event.ts.clone(),
                                path: path.clone(),
                                summary: event.summary.clone(),
                                lines_added: event.lines_added.unwrap_or(0),
                                lines_removed: event.lines_removed.unwrap_or(0),
                            },
                        );
                        agent.recent_changes.truncate(RECENT_CHANGES);
                    }
                }
            }
            EventKind::ApprovalRequested => {
                if !paused {
                    agent.status = "waiting".into();
                    agent.doing_now = event.summary.clone();
                }
            }
            EventKind::TaskAssigned => {
                agent.task_id = event.task_id.clone();
            }
            EventKind::TaskDone => {
                if !paused {
                    agent.status = "idle".into();
                    agent.doing_now = event.summary.clone();
                }
                if agent.task_id == event.task_id {
                    agent.task_id = None;
                }
            }
            EventKind::MessageSent
            | EventKind::MessageReceived
            | EventKind::ApprovalDecided
            | EventKind::SecurityAlert
            | EventKind::Error => {}
        }
    }
}

/// A turn currently running for an agent.
pub struct Running {
    pub task_id: String,
    pub account_id: Option<String>,
    pub cancel: watch::Sender<bool>,
}

pub struct App {
    pub cfg: Config,
    pub store: Store,
    pub prices: RwLock<Prices>,
    live: RwLock<Live>,
    events_tx: mpsc::UnboundedSender<Event>,
    /// Serialized WebSocket messages for every connected client.
    pub bus: broadcast::Sender<String>,
    /// Wakes the scheduler.
    pub wake: Notify,
    running: Mutex<HashMap<String, Running>>,
    approval_waiters: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    /// Hashes of secret values per project (never the values).
    pub secrets: RwLock<HashMap<String, Arc<SecretSet>>>,
    /// Budget warnings already sent, so each scope warns once.
    pub warned: Mutex<HashSet<String>>,
    /// Port the HTTP server is listening on (set after bind).
    pub bound_port: RwLock<u16>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl App {
    /// Build the app from an opened store, rebuilding live state from the events.
    pub fn new(
        cfg: Config,
        store: Store,
    ) -> AppResult<(Arc<Self>, mpsc::UnboundedReceiver<Event>)> {
        let live = Live::from_events(&store.all_events()?);
        let prices = Prices::load_or_create(&cfg.prices_path());
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (bus, _) = broadcast::channel(1024);
        let app = Arc::new(Self {
            cfg,
            store,
            prices: RwLock::new(prices),
            live: RwLock::new(live),
            events_tx,
            bus,
            wake: Notify::new(),
            running: Mutex::new(HashMap::new()),
            approval_waiters: Mutex::new(HashMap::new()),
            secrets: RwLock::new(HashMap::new()),
            warned: Mutex::new(HashSet::new()),
            bound_port: RwLock::new(0),
        });
        app.seed_accounts()?;
        Ok((app, events_rx))
    }

    /// Open the database in the config directory and build the app.
    pub fn open(cfg: Config) -> AppResult<(Arc<Self>, mpsc::UnboundedReceiver<Event>)> {
        let store = Store::open(&cfg.db_path())?;
        Self::new(cfg, store)
    }

    fn seed_accounts(&self) -> AppResult<()> {
        if !self.store.list_accounts()?.is_empty() {
            return Ok(());
        }
        for (provider, binary, max) in [
            ("claude", "claude", 3),
            ("codex", "codex", 2),
            ("agy", "agy", 2),
        ] {
            self.store.upsert_account(&Account {
                id: format!("acc_{provider}"),
                provider: provider.to_string(),
                login_type: "subscription".to_string(),
                cli_path: binary.to_string(),
                default_model: String::new(),
                limit_rule: json!({}),
                max_concurrent: max,
            })?;
        }
        Ok(())
    }

    /// Tasks left `running` by a previous process go back to the queue.
    pub fn recover(&self) -> AppResult<()> {
        for task in self.store.tasks_with_status("running")? {
            self.store.set_task_status(&task.id, "queued")?;
        }
        Ok(())
    }

    // ---- events ---------------------------------------------------------

    /// Append an event, update the live state and queue it for the next batch.
    pub fn emit(&self, new: NewEvent) -> AppResult<Event> {
        let agent = self.store.get_agent(&new.agent_id).ok();
        let account_id = agent.as_ref().and_then(|agent| agent.account_id.clone());
        let mut event = Event {
            seq: 0,
            id: new_id("evt"),
            ts: now(),
            agent_id: new.agent_id,
            task_id: new.task_id,
            kind: new.kind,
            target: new.target,
            summary: new.summary,
            lines_added: new.lines_added,
            lines_removed: new.lines_removed,
            usage: new.usage,
            raw_ref: new.raw_ref,
            data: new.data,
        };
        // Last line of defense: a secret value never reaches an event.
        let leaked = agent
            .as_ref()
            .is_some_and(|agent| self.redact(&agent.project_id, &mut event));
        event.seq = self.store.append_event(&event, account_id.as_deref())?;
        self.live
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .apply(&event);
        let _ = self.events_tx.send(event.clone());
        if event.usage.is_some() {
            crate::usage::after_usage(self, &event)?;
        }
        if leaked {
            let task_id = event.task_id.clone();
            self.emit(
                NewEvent::new(
                    &event.agent_id,
                    EventKind::SecurityAlert,
                    "Um evento continha um valor de segredo e foi removido",
                )
                .task(task_id.as_deref())
                .data(json!({ "severity": "critical", "reason": "valor de segredo do .env" })),
            )?;
            self.pause_agent(&event.agent_id, "Saída com valor de segredo")?;
        }
        Ok(event)
    }

    /// Replace the text of an event that carries a secret value of the project.
    fn redact(&self, project_id: &str, event: &mut Event) -> bool {
        let secrets = self
            .secrets
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(project_id)
            .cloned();
        let Some(secrets) = secrets.filter(|set| !set.is_empty()) else {
            return false;
        };
        let data = event
            .data
            .as_ref()
            .map(Value::to_string)
            .unwrap_or_default();
        let text = format!(
            "{}\n{}\n{data}",
            event.summary,
            event.target.as_deref().unwrap_or("")
        );
        if !secrets.contains_secret(&text) {
            return false;
        }
        event.summary = "[conteúdo removido: continha um valor de segredo]".to_string();
        event.target = None;
        event.data = Some(json!({ "redacted": true }));
        true
    }

    pub fn live_of(&self, agent_id: &str) -> AgentLive {
        self.live
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .agents
            .get(agent_id)
            .cloned()
            .unwrap_or_else(|| AgentLive {
                status: "idle".into(),
                ..Default::default()
            })
    }

    pub fn last_seq(&self) -> i64 {
        self.live.read().unwrap_or_else(|p| p.into_inner()).last_seq
    }

    /// Send a ready-made message to every WebSocket client.
    pub fn broadcast(&self, message: &Value) {
        let _ = self.bus.send(message.to_string());
    }

    /// Full state for a client that just connected.
    pub fn snapshot(&self) -> AppResult<Value> {
        let agents: Vec<Value> = self
            .store
            .list_agents(None)?
            .into_iter()
            .map(|agent| {
                let live = self.live_of(&agent.id);
                let mut value = serde_json::to_value(&agent).unwrap_or(Value::Null);
                if let Value::Object(map) = &mut value {
                    map.insert(
                        "live".into(),
                        serde_json::to_value(&live).unwrap_or(Value::Null),
                    );
                }
                value
            })
            .collect();
        Ok(json!({
            "type": "snapshot",
            "api_version": API_VERSION,
            "version": env!("CARGO_PKG_VERSION"),
            "last_seq": self.last_seq(),
            "projects": self.store.list_projects()?,
            "agents": agents,
            "tasks": self.store.list_tasks(None)?,
            "accounts": self.store.list_accounts()?,
            "usage": crate::usage::report(self)?,
            "approvals": self.store.list_approvals(Some("pending"))?,
            "messages": self.store.recent_messages(100)?,
        }))
    }

    // ---- running turns --------------------------------------------------

    /// Register a turn; returns the cancel receiver, or None if the agent is busy.
    pub fn start_running(
        &self,
        agent_id: &str,
        task_id: &str,
        account_id: Option<String>,
    ) -> Option<watch::Receiver<bool>> {
        let mut running = lock(&self.running);
        if running.contains_key(agent_id) {
            return None;
        }
        let (cancel, rx) = watch::channel(false);
        running.insert(
            agent_id.to_string(),
            Running {
                task_id: task_id.to_string(),
                account_id,
                cancel,
            },
        );
        Some(rx)
    }

    pub fn finish_running(&self, agent_id: &str) {
        lock(&self.running).remove(agent_id);
    }

    pub fn is_running(&self, agent_id: &str) -> bool {
        lock(&self.running).contains_key(agent_id)
    }

    pub fn running_task(&self, agent_id: &str) -> Option<String> {
        lock(&self.running).get(agent_id).map(|r| r.task_id.clone())
    }

    pub fn running_on_account(&self, account_id: &str) -> usize {
        lock(&self.running)
            .values()
            .filter(|r| r.account_id.as_deref() == Some(account_id))
            .count()
    }

    pub fn running_count(&self) -> usize {
        lock(&self.running).len()
    }

    /// Ask the running turn of an agent to stop.
    pub fn cancel_turn(&self, agent_id: &str) {
        if let Some(running) = lock(&self.running).get(agent_id) {
            let _ = running.cancel.send(true);
        }
    }

    /// The task an agent is working on: the running one, else its oldest open task.
    pub fn current_task(&self, agent_id: &str) -> AppResult<Option<crate::model::Task>> {
        if let Some(task_id) = self.running_task(agent_id) {
            return Ok(Some(self.store.get_task(&task_id)?));
        }
        Ok(self.store.open_tasks_of(agent_id)?.into_iter().next())
    }

    // ---- pause and resume -----------------------------------------------

    /// Pause an agent: stop its turn, block its open tasks and tell the UI why.
    pub fn pause_agent(&self, agent_id: &str, reason: &str) -> AppResult<()> {
        let agent = self.store.get_agent(agent_id)?;
        if agent.paused {
            return Ok(());
        }
        self.store.set_agent_paused(agent_id, true, Some(reason))?;
        let task_id = self.running_task(agent_id);
        for task in self.store.open_tasks_of(agent_id)? {
            if task.status == "running" || task.status == "queued" {
                self.store.set_task_status(&task.id, "blocked")?;
            }
        }
        self.cancel_turn(agent_id);
        self.emit(
            NewEvent::status(agent_id, "paused", format!("Pausado: {reason}"))
                .task(task_id.as_deref())
                .with("reason", json!(reason)),
        )?;
        Ok(())
    }

    /// Resume a paused agent and put its blocked tasks back in the queue.
    pub fn resume_agent(&self, agent_id: &str) -> AppResult<()> {
        self.store.get_agent(agent_id)?;
        self.store.set_agent_paused(agent_id, false, None)?;
        for task in self.store.open_tasks_of(agent_id)? {
            if task.status == "blocked" {
                self.store.set_task_stalled(&task.id, 0)?;
                self.store.set_task_status(&task.id, "queued")?;
            }
        }
        lock(&self.warned).retain(|key| !key.contains(agent_id));
        self.emit(NewEvent::status(agent_id, "idle", "Retomado"))?;
        self.wake.notify_one();
        Ok(())
    }

    // ---- approvals ------------------------------------------------------

    /// Record an approval request. The receiver resolves when it is decided.
    pub fn request_approval(
        &self,
        agent_id: &str,
        task_id: Option<&str>,
        action: &str,
        detail: &str,
    ) -> AppResult<(Approval, oneshot::Receiver<bool>)> {
        let approval = Approval {
            id: new_id("ap"),
            agent_id: agent_id.to_string(),
            task_id: task_id.map(str::to_string),
            action: action.to_string(),
            detail: detail.to_string(),
            status: "pending".to_string(),
            decided_by: None,
            created_at: now(),
            decided_at: None,
        };
        self.store.insert_approval(&approval)?;
        let (tx, rx) = oneshot::channel();
        lock(&self.approval_waiters).insert(approval.id.clone(), tx);
        self.emit(
            NewEvent::new(
                agent_id,
                EventKind::ApprovalRequested,
                format!("Aguardando aprovação: {detail}"),
            )
            .task(task_id)
            .target(action)
            .data(json!({ "approval_id": approval.id, "action": action })),
        )?;
        self.broadcast(&json!({ "type": "approval", "approval": approval }));
        Ok((approval, rx))
    }

    /// Decide a pending approval. Approvals never expire by themselves.
    pub fn decide_approval(&self, id: &str, approve: bool, by: &str) -> AppResult<Approval> {
        let approval = self.store.get_approval(id)?;
        if approval.status != "pending" {
            return Err(AppError::BadRequest(format!(
                "approval {id} already {}",
                approval.status
            )));
        }
        let status = if approve { "approved" } else { "denied" };
        self.store.decide_approval(id, status, by)?;
        let approval = self.store.get_approval(id)?;
        let label = if approve { "Aprovado" } else { "Negado" };
        self.emit(
            NewEvent::new(
                &approval.agent_id,
                EventKind::ApprovalDecided,
                format!("{label}: {}", approval.detail),
            )
            .task(approval.task_id.as_deref())
            .target(approval.action.clone())
            .data(json!({ "approval_id": approval.id, "status": status, "decided_by": by })),
        )?;
        if let Some(waiter) = lock(&self.approval_waiters).remove(id) {
            let _ = waiter.send(approve);
        }
        self.broadcast(&json!({ "type": "approval", "approval": approval }));
        Ok(approval)
    }
}

/// Group events and send them to the clients at most every `batch_ms`.
pub fn spawn_batcher(
    app: Arc<App>,
    mut rx: mpsc::UnboundedReceiver<Event>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Sleeps until the first event arrives, so an idle daemon does not wake up.
        while let Some(first) = rx.recv().await {
            tokio::time::sleep(std::time::Duration::from_millis(app.cfg.batch_ms)).await;
            let mut items = vec![first];
            while let Ok(event) = rx.try_recv() {
                items.push(event);
            }
            let usage_changed = items.iter().any(|event| event.usage.is_some());
            app.broadcast(&json!({ "type": "events", "items": items }));
            if usage_changed {
                if let Ok(usage) = crate::usage::report(&app) {
                    app.broadcast(&json!({ "type": "usage", "usage": usage }));
                }
            }
        }
    })
}

#[cfg(test)]
pub mod testing {
    //! Helpers shared by unit tests across modules.
    use super::*;
    use crate::model::{Agent, Project};
    use std::path::{Path, PathBuf};

    pub fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures")
    }

    /// App over an in-memory database with the batcher running.
    pub fn app(home: &Path) -> Arc<App> {
        let cfg = Config::for_tests(home, &fixtures_dir());
        let (app, rx) = App::new(cfg, Store::open_memory().unwrap()).unwrap();
        spawn_batcher(app.clone(), rx);
        app
    }

    pub fn project(app: &App, root: &Path) -> Project {
        let project = Project {
            id: new_id("pr"),
            name: "Demo".into(),
            root: root.to_string_lossy().into_owned(),
            base_branch: "main".into(),
            context: "Projeto de teste".into(),
            folders: json!([]),
            blocked: json!([".env*", "secrets/", "*.pem", "id_*"]),
            day_budget_usd: None,
            created_at: now(),
        };
        app.store.insert_project(&project).unwrap();
        project
    }

    pub fn agent(
        app: &App,
        project: &Project,
        name: &str,
        scenario: &str,
        orchestrator: bool,
    ) -> Agent {
        let agent = Agent {
            id: format!("ag_{}", name.to_lowercase()),
            project_id: project.id.clone(),
            name: name.into(),
            cli: "fake".into(),
            role: name.into(),
            is_orchestrator: orchestrator,
            parent_id: None,
            account_id: Some("acc_claude".into()),
            model: String::new(),
            pet: json!({}),
            permissions: json!({}),
            settings: json!({ "fake_scenario": scenario }),
            budget_usd: None,
            paused: false,
            pause_reason: None,
            created_at: now(),
        };
        app.store.insert_agent(&agent).unwrap();
        agent
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[tokio::test]
    async fn emit_updates_live_and_batches() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let project = project(&app, dir.path());
        let agent = agent(&app, &project, "Backend", "basic", false);
        let mut bus = app.bus.subscribe();
        app.emit(NewEvent::new(&agent.id, EventKind::Thinking, "Planejando"))
            .unwrap();
        app.emit(
            NewEvent::new(&agent.id, EventKind::FileEdit, "Editou a.rs")
                .target("a.rs")
                .lines(3, 1),
        )
        .unwrap();
        let live = app.live_of(&agent.id);
        assert_eq!(live.status, "working");
        assert_eq!(live.doing_now, "Editou a.rs");
        assert_eq!(live.recent_changes.len(), 1);
        let message: Value = serde_json::from_str(&bus.recv().await.unwrap()).unwrap();
        assert_eq!(message["type"], "events");
        let seqs: Vec<i64> = message["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["seq"].as_i64().unwrap())
            .collect();
        assert_eq!(seqs, vec![1, 2]);
    }

    #[tokio::test]
    async fn live_state_rebuilds_from_events() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let project = project(&app, dir.path());
        let agent = agent(&app, &project, "Backend", "basic", false);
        app.emit(
            NewEvent::new(&agent.id, EventKind::FileEdit, "x")
                .target("a.rs")
                .lines(1, 0),
        )
        .unwrap();
        app.emit(NewEvent::status(&agent.id, "waiting", "Aguardando"))
            .unwrap();
        app.pause_agent(&agent.id, "teste").unwrap();
        let rebuilt = Live::from_events(&app.store.all_events().unwrap());
        assert_eq!(rebuilt.agents[&agent.id], app.live_of(&agent.id));
        assert_eq!(rebuilt.agents[&agent.id].status, "paused");
        assert_eq!(
            rebuilt.agents[&agent.id].pause_reason.as_deref(),
            Some("teste")
        );
    }

    #[tokio::test]
    async fn pause_and_resume_change_state() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let project = project(&app, dir.path());
        let agent = agent(&app, &project, "Backend", "basic", false);
        app.pause_agent(&agent.id, "orçamento").unwrap();
        assert!(app.store.get_agent(&agent.id).unwrap().paused);
        // Activity while paused does not hide the pause.
        app.emit(NewEvent::new(&agent.id, EventKind::Thinking, "x"))
            .unwrap();
        assert_eq!(app.live_of(&agent.id).status, "paused");
        app.resume_agent(&agent.id).unwrap();
        assert!(!app.store.get_agent(&agent.id).unwrap().paused);
        assert_eq!(app.live_of(&agent.id).status, "idle");
    }

    #[tokio::test]
    async fn approvals_wait_for_a_decision() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let project = project(&app, dir.path());
        let agent = agent(&app, &project, "Backend", "basic", false);
        let (approval, rx) = app
            .request_approval(&agent.id, None, "push", "git push origin main")
            .unwrap();
        assert_eq!(app.live_of(&agent.id).status, "waiting");
        assert_eq!(app.store.list_approvals(Some("pending")).unwrap().len(), 1);
        app.decide_approval(&approval.id, true, "pedro").unwrap();
        assert!(rx.await.unwrap());
        assert!(app.decide_approval(&approval.id, false, "pedro").is_err());
    }

    #[tokio::test]
    async fn long_text_travels_whole_in_data_and_is_redacted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "API_TOKEN=tok_fake_0123456789\n").unwrap();
        let app = app(dir.path());
        let project = project(&app, dir.path());
        let agent = agent(&app, &project, "Backend", "basic", false);
        crate::security::watcher::refresh_secrets(&app, &project.id, dir.path());
        let long = "palavra ".repeat(100);
        let event = app
            .emit(
                NewEvent::new(
                    &agent.id,
                    EventKind::Thinking,
                    crate::model::truncate(&long, 200),
                )
                .with("text", json!(crate::model::full_text(&long))),
            )
            .unwrap();
        assert!(event.summary.chars().count() <= 200);
        assert_eq!(event.data.unwrap()["text"].as_str().unwrap(), long.trim());
        assert_eq!(
            crate::model::full_text(&"a".repeat(70_000)).chars().count(),
            64 * 1024 + 1
        );
        let leaked = app
            .emit(
                NewEvent::new(&agent.id, EventKind::Thinking, "curto")
                    .with("text", json!("o token é tok_fake_0123456789")),
            )
            .unwrap();
        assert_eq!(leaked.data.unwrap(), json!({ "redacted": true }));
    }

    #[tokio::test]
    async fn events_never_carry_secret_values() {
        let dir = tempfile::tempdir().unwrap();
        // Fake value only.
        std::fs::write(dir.path().join(".env"), "API_TOKEN=tok_fake_0123456789\n").unwrap();
        let app = app(dir.path());
        let project = project(&app, dir.path());
        let agent = agent(&app, &project, "Backend", "basic", false);
        crate::security::watcher::refresh_secrets(&app, &project.id, dir.path());
        let event = app
            .emit(
                NewEvent::new(
                    &agent.id,
                    EventKind::CommandResult,
                    "token=tok_fake_0123456789",
                )
                .target("printenv")
                .with("output", json!("API_TOKEN=tok_fake_0123456789")),
            )
            .unwrap();
        assert!(event.summary.contains("conteúdo removido"));
        assert_eq!(event.data.unwrap()["redacted"], true);
        let stored = serde_json::to_string(&app.store.all_events().unwrap()).unwrap();
        assert!(!stored.contains("tok_fake_0123456789"));
        assert!(app.store.get_agent(&agent.id).unwrap().paused);
        let clean = app
            .emit(NewEvent::new(&agent.id, EventKind::Thinking, "nada demais"))
            .unwrap();
        assert_eq!(clean.summary, "nada demais");
    }

    #[tokio::test]
    async fn one_turn_per_agent() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        assert!(app
            .start_running("ag_1", "tk_1", Some("acc_claude".into()))
            .is_some());
        assert!(app.start_running("ag_1", "tk_2", None).is_none());
        assert_eq!(app.running_on_account("acc_claude"), 1);
        app.finish_running("ag_1");
        assert!(!app.is_running("ag_1"));
    }
}
