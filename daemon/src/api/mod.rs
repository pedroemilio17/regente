//! REST and WebSocket API (`/api/v1`), the messenger endpoint and the static UI.

mod folder_picker;
mod pty;

use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{header, Request, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;

use crate::app::{App, AppError, AppResult};
use crate::model::{new_id, now, Agent, Project, API_VERSION};
use crate::{mcp, orchestrator, security, team, usage};

type Shared = State<Arc<App>>;

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}

pub fn router(app: Arc<App>) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/snapshot", get(snapshot))
        .route("/stream", get(stream))
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/{id}", axum::routing::patch(patch_project))
        .route("/pick-folder", post(folder_picker::pick_folder))
        .route("/agents", get(list_agents).post(create_agent))
        .route("/agents/{id}", get(get_agent).patch(patch_agent))
        .route("/agents/{id}/pause", post(pause_agent))
        .route("/agents/{id}/resume", post(resume_agent))
        .route("/agents/{id}/message", post(message_agent))
        .route("/agents/{id}/timeline", get(timeline))
        .route("/agents/{id}/diff/latest", get(latest_diff))
        .route("/agents/{id}/pty", get(pty::open))
        .route("/tasks", get(list_tasks).post(create_task))
        .route("/messages", get(list_messages))
        .route("/usage", get(get_usage))
        .route("/accounts", get(list_accounts))
        .route("/accounts/{id}", axum::routing::patch(patch_account))
        .route("/accounts/{id}/status", get(account_status))
        .route("/accounts/{id}/models", get(account_models))
        .route("/accounts/{id}/login", get(pty::login))
        .route("/approvals", get(list_approvals).post(create_approval))
        .route("/approvals/{id}", get(get_approval).post(decide_approval));
    Router::new()
        .nest("/api/v1", api)
        .route("/mcp/{agent_id}", post(mcp_rpc))
        .fallback(static_ui)
        .layer(middleware::from_fn(loopback_guard))
        .with_state(app)
}

/// Browsers can send forms and WebSocket upgrades to localhost without CORS.
/// Reject untrusted origins and DNS-rebinding Host headers before any route.
async fn loopback_guard(request: Request<Body>, next: Next) -> Response {
    let headers = request.headers();
    let hosts: Vec<_> = headers.get_all(header::HOST).iter().collect();
    let origins: Vec<_> = headers.get_all(header::ORIGIN).iter().collect();
    let valid_host = hosts.len() <= 1
        && hosts
            .first()
            .is_none_or(|value| value.to_str().ok().is_some_and(loopback_authority));
    let valid_origin = origins.len() <= 1
        && origins
            .first()
            .is_none_or(|value| value.to_str().ok().is_some_and(loopback_origin));
    if !valid_host || !valid_origin {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}

fn loopback_authority(value: &str) -> bool {
    let host_end = if value.starts_with('[') {
        match value.find(']') {
            Some(end) => end + 1,
            None => return false,
        }
    } else {
        value.find(':').unwrap_or(value.len())
    };
    let (host, suffix) = value.split_at(host_end);
    let valid_host =
        host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "[::1]";
    let valid_port = suffix.is_empty()
        || suffix.strip_prefix(':').is_some_and(|port| {
            !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|p| p != 0)
        });
    valid_host && valid_port
}

fn loopback_origin(value: &str) -> bool {
    value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))
        .is_some_and(loopback_authority)
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION"), "api_version": API_VERSION }))
}

async fn snapshot(State(app): Shared) -> AppResult<Json<Value>> {
    Ok(Json(app.snapshot()?))
}

// ---- WebSocket stream -------------------------------------------------------

async fn stream(ws: WebSocketUpgrade, State(app): Shared) -> Response {
    ws.on_upgrade(move |socket| client(socket, app))
}

async fn send_snapshot(socket: &mut WebSocket, app: &App) -> bool {
    match app.snapshot() {
        Ok(snapshot) => socket
            .send(WsMessage::Text(snapshot.to_string().into()))
            .await
            .is_ok(),
        Err(_) => false,
    }
}

/// One client: a snapshot first, then only batches of new events.
async fn client(mut socket: WebSocket, app: Arc<App>) {
    let mut bus = app.bus.subscribe();
    if !send_snapshot(&mut socket, &app).await {
        return;
    }
    loop {
        tokio::select! {
            message = bus.recv() => match message {
                Ok(text) => {
                    if socket.send(WsMessage::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                // A slow client missed batches: give it a fresh snapshot.
                Err(RecvError::Lagged(_)) => {
                    if !send_snapshot(&mut socket, &app).await {
                        break;
                    }
                }
                Err(RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(WsMessage::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
        }
    }
}

// ---- projects ---------------------------------------------------------------

async fn list_projects(State(app): Shared) -> AppResult<Json<Value>> {
    Ok(Json(json!(app.store.list_projects()?)))
}

#[derive(Deserialize)]
struct NewProject {
    name: String,
    root: String,
    base_branch: Option<String>,
    context: Option<String>,
    folders: Option<Value>,
    blocked: Option<Vec<String>>,
    day_budget_usd: Option<f64>,
    #[serde(default)]
    agents: Vec<NewAgent>,
}

async fn create_project(
    State(app): Shared,
    Json(mut body): Json<NewProject>,
) -> AppResult<(StatusCode, Json<Value>)> {
    if body.name.trim().is_empty() || body.root.trim().is_empty() {
        return Err(AppError::BadRequest(
            "nome e pasta do projeto são obrigatórios".into(),
        ));
    }
    let root = check_root(&body.root).await?;
    // No base branch given: use the branch checked out in the repository.
    let base = match body.base_branch.as_deref().map(str::trim) {
        Some(base) if !base.is_empty() => base.to_string(),
        _ => crate::worktree::git(&root, &["rev-parse", "--abbrev-ref", "HEAD"])
            .await
            .map_err(|_| AppError::BadRequest("branch base não encontrada".into()))?,
    };
    check_base_branch(&root, &base).await?;
    let folders = body.folders.unwrap_or_else(|| json!([]));
    check_folders(&root, &folders)?;
    let mut names: Vec<String> = Vec::new();
    for agent in &mut body.agents {
        let name = agent_name(agent, &names);
        if !CLIS.contains(&agent.cli.as_str())
            || names.iter().any(|taken| taken.eq_ignore_ascii_case(&name))
        {
            return Err(AppError::BadRequest(
                "nome ou CLI inválido, ou agente duplicado".into(),
            ));
        }
        agent.name = Some(name.clone());
        names.push(name);
    }
    let blocked = body.blocked.unwrap_or_else(|| {
        security::DEFAULT_BLOCKED
            .iter()
            .map(|p| p.to_string())
            .collect()
    });
    let project = Project {
        id: new_id("pr"),
        name: body.name.trim().to_string(),
        root: root.to_string_lossy().into_owned(),
        base_branch: base,
        context: body.context.unwrap_or_default(),
        folders,
        blocked: json!(blocked),
        day_budget_usd: body.day_budget_usd,
        created_at: now(),
    };
    let mut agents = Vec::new();
    for new_agent in body.agents {
        agents.push(make_agent(&app, &project.id, new_agent));
    }
    let paths = [
        ".regente/team.yaml",
        ".regente/.gitignore",
        "CLAUDE.md",
        "AGENTS.md",
    ];
    let mut backups = Vec::new();
    for path in paths {
        let target = root.join(path);
        let content = match std::fs::read(&target) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        backups.push((target, content));
    }
    let result = async {
        let files = team::write_files(&project, &agents).await?;
        app.store.insert_project_team(&project, &agents)?;
        Ok::<_, AppError>(files)
    }
    .await;
    let files = match result {
        Ok(files) => files,
        Err(error) => {
            for (path, content) in backups {
                match content {
                    Some(bytes) => std::fs::write(&path, bytes)?,
                    None if path.exists() => std::fs::remove_file(&path)?,
                    None => {}
                }
            }
            return Err(error);
        }
    };
    Ok((
        StatusCode::CREATED,
        Json(json!({ "project": project, "agents": agents, "files": files })),
    ))
}

/// A project folder is the absolute root of a Git repository whose config
/// files are not symlinks.
async fn check_root(raw: &str) -> AppResult<PathBuf> {
    let root = PathBuf::from(raw.trim());
    if !root.is_absolute() {
        return Err(AppError::BadRequest(
            "a pasta do projeto precisa ser um caminho absoluto".into(),
        ));
    }
    if !crate::worktree::is_git_repo(&root).await {
        return Err(AppError::BadRequest(
            "a pasta precisa ser um repositório Git existente".into(),
        ));
    }
    let actual_root = crate::worktree::git(&root, &["rev-parse", "--show-toplevel"]).await?;
    if std::fs::canonicalize(&root)? != std::fs::canonicalize(actual_root)? {
        return Err(AppError::BadRequest(
            "informe a raiz do repositório Git".into(),
        ));
    }
    for path in [
        ".regente",
        ".regente/team.yaml",
        ".regente/.gitignore",
        "CLAUDE.md",
        "AGENTS.md",
    ] {
        if !security::safe_relative_path(&root, path) {
            return Err(AppError::BadRequest(
                "arquivos de configuração não podem usar links simbólicos".into(),
            ));
        }
    }
    Ok(root)
}

/// The base branch must exist in the repository (D15).
async fn check_base_branch(root: &FsPath, base: &str) -> AppResult<()> {
    let reference = format!("refs/heads/{base}^{{commit}}");
    if base.starts_with('-')
        || crate::worktree::git(root, &["rev-parse", "--verify", &reference])
            .await
            .is_err()
    {
        return Err(AppError::BadRequest("branch base não encontrada".into()));
    }
    Ok(())
}

fn check_folders(root: &FsPath, folders: &Value) -> AppResult<()> {
    let Some(folder_list) = folders.as_array() else {
        return Err(AppError::BadRequest("pastas inválidas".into()));
    };
    for folder in folder_list {
        let path = folder.get("path").and_then(Value::as_str).unwrap_or("");
        let access = folder.get("access").and_then(Value::as_str).unwrap_or("");
        if !security::safe_relative_path(root, path) || !["read", "write", "none"].contains(&access)
        {
            return Err(AppError::BadRequest(
                "pasta ou nível de acesso inválido".into(),
            ));
        }
    }
    Ok(())
}

async fn patch_project(
    State(app): Shared,
    Path(id): Path<String>,
    Json(patch): Json<Value>,
) -> AppResult<Json<Project>> {
    Ok(Json(update_project(&app, &id, &patch, true).await?))
}

/// Change the editable settings of a project with the same checks as its
/// creation, then regenerate the team files. Name and folder are the user's
/// call only (`by_user`), never the orchestrator's.
pub(crate) async fn update_project(
    app: &App,
    id: &str,
    patch: &Value,
    by_user: bool,
) -> AppResult<Project> {
    let mut project = app.store.get_project(id)?;
    let mut root = PathBuf::from(&project.root);
    if !by_user && (patch.get("name").is_some() || patch.get("root").is_some()) {
        return Err(AppError::BadRequest(
            "nome e pasta do projeto só podem ser alterados pelo usuário".into(),
        ));
    }
    if let Some(name) = patch.get("name") {
        let name = name.as_str().map(str::trim).unwrap_or("");
        if name.is_empty() {
            return Err(AppError::BadRequest("nome do projeto é obrigatório".into()));
        }
        project.name = name.to_string();
    }
    if let Some(new_root) = patch.get("root") {
        let new_root = check_root(new_root.as_str().unwrap_or("")).await?;
        if new_root != root {
            // Worktrees live inside the project folder: never move it under running work.
            let active = app
                .store
                .list_tasks(Some(id))?
                .into_iter()
                .any(|task| !matches!(task.status.as_str(), "done" | "cancelled"));
            if active {
                return Err(AppError::BadRequest(
                    "conclua ou cancele as tarefas ativas antes de trocar a pasta".into(),
                ));
            }
            // What the patch does not change must still hold in the new repository.
            if patch.get("base_branch").is_none() {
                check_base_branch(&new_root, &project.base_branch).await?;
            }
            if patch.get("folders").is_none() {
                check_folders(&new_root, &project.folders)?;
            }
            root = new_root;
            project.root = root.to_string_lossy().into_owned();
        }
    }
    if let Some(base) = patch.get("base_branch") {
        let base = base
            .as_str()
            .map(str::trim)
            .ok_or_else(|| AppError::BadRequest("branch base inválida".into()))?;
        check_base_branch(&root, base).await?;
        project.base_branch = base.to_string();
    }
    if let Some(context) = patch.get("context") {
        project.context = context
            .as_str()
            .ok_or_else(|| AppError::BadRequest("contexto inválido".into()))?
            .to_string();
    }
    if let Some(folders) = patch.get("folders") {
        check_folders(&root, folders)?;
        project.folders = folders.clone();
    }
    if let Some(blocked) = patch.get("blocked") {
        if !blocked
            .as_array()
            .is_some_and(|list| list.iter().all(Value::is_string))
        {
            return Err(AppError::BadRequest("bloqueios inválidos".into()));
        }
        project.blocked = blocked.clone();
    }
    if let Some(budget) = patch.get("day_budget_usd") {
        if !budget.is_null() && !budget.as_f64().is_some_and(|value| value >= 0.0) {
            return Err(AppError::BadRequest("orçamento inválido".into()));
        }
        project.day_budget_usd = budget.as_f64();
    }
    app.store.update_project(&project)?;
    team::write_team_files(app, &project).await?;
    app.broadcast(&app.snapshot()?);
    Ok(project)
}

// ---- agents -----------------------------------------------------------------

#[derive(Deserialize)]
struct ProjectFilter {
    project: Option<String>,
}

fn agent_view(app: &App, agent: &Agent) -> Value {
    let mut value = serde_json::to_value(agent).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut value {
        map.insert(
            "live".into(),
            serde_json::to_value(app.live_of(&agent.id)).unwrap_or(Value::Null),
        );
    }
    value
}

async fn list_agents(
    State(app): Shared,
    Query(filter): Query<ProjectFilter>,
) -> AppResult<Json<Value>> {
    let agents = app.store.list_agents(filter.project.as_deref())?;
    Ok(Json(json!(agents
        .iter()
        .map(|agent| agent_view(&app, agent))
        .collect::<Vec<_>>())))
}

async fn get_agent(State(app): Shared, Path(id): Path<String>) -> AppResult<Json<Value>> {
    Ok(Json(agent_view(&app, &app.store.get_agent(&id)?)))
}

#[derive(Deserialize, Default)]
pub(crate) struct NewAgent {
    pub project_id: Option<String>,
    pub name: Option<String>,
    pub cli: String,
    pub role: Option<String>,
    #[serde(default)]
    pub is_orchestrator: bool,
    pub parent_id: Option<String>,
    pub account_id: Option<String>,
    pub model: Option<String>,
    pub pet: Option<Value>,
    pub permissions: Option<Value>,
    pub settings: Option<Value>,
    pub budget_usd: Option<f64>,
}

const CLIS: [&str; 4] = ["claude", "codex", "agy", "fake"];

/// The given name, or the engine label with a suffix free in the project
/// ("Claude", "Claude 2", ...).
fn agent_name(body: &NewAgent, taken: &[String]) -> String {
    if let Some(name) = body
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        return name.to_string();
    }
    let label = match body.cli.as_str() {
        "claude" => "Claude",
        "codex" => "Codex",
        "agy" => "Antigravity",
        _ => "Teste",
    };
    (1..)
        .map(|n| match n {
            1 => label.to_string(),
            n => format!("{label} {n}"),
        })
        .find(|name| !taken.iter().any(|t| t.eq_ignore_ascii_case(name)))
        .unwrap_or_default()
}

fn insert_agent(app: &App, project_id: &str, mut body: NewAgent) -> AppResult<Agent> {
    if !CLIS.contains(&body.cli.as_str()) {
        return Err(AppError::BadRequest(format!(
            "CLI desconhecido: {}",
            body.cli
        )));
    }
    app.store.get_project(project_id)?;
    let existing = app.store.list_agents(Some(project_id))?;
    if body.parent_id.is_none() && !body.is_orchestrator {
        body.parent_id = existing
            .iter()
            .find(|agent| agent.is_orchestrator && agent.parent_id.is_none())
            .map(|agent| agent.id.clone());
    }
    let taken: Vec<String> = existing.into_iter().map(|agent| agent.name).collect();
    let name = agent_name(&body, &taken);
    if taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
        return Err(AppError::BadRequest(format!(
            "já existe um agente chamado {name}"
        )));
    }
    body.name = Some(name);
    let agent = make_agent(app, project_id, body);
    app.store.insert_agent(&agent)?;
    Ok(agent)
}

fn make_agent(app: &App, project_id: &str, body: NewAgent) -> Agent {
    let default_account = format!("acc_{}", body.cli);
    let account_id = body.account_id.or_else(|| {
        app.store
            .get_account(&default_account)
            .ok()
            .map(|account| account.id)
    });
    let role = body
        .role
        .filter(|role| !role.trim().is_empty())
        .unwrap_or_else(|| {
            if body.is_orchestrator {
                "Orquestrador"
            } else {
                "Agente"
            }
            .into()
        });
    Agent {
        id: new_id("ag"),
        project_id: project_id.to_string(),
        name: body.name.unwrap_or_default().trim().to_string(),
        cli: body.cli,
        role,
        is_orchestrator: body.is_orchestrator,
        parent_id: body.parent_id,
        account_id,
        model: body.model.unwrap_or_default(),
        pet: body.pet.unwrap_or_else(|| json!({})),
        permissions: body.permissions.unwrap_or_else(|| json!({})),
        settings: body.settings.unwrap_or_else(|| json!({})),
        budget_usd: body.budget_usd,
        paused: false,
        pause_reason: None,
        created_at: now(),
    }
}

/// Create an agent and regenerate the project's team files.
pub(crate) async fn add_agent(app: &App, project_id: &str, body: NewAgent) -> AppResult<Agent> {
    let agent = insert_agent(app, project_id, body)?;
    let project = app.store.get_project(project_id)?;
    team::write_team_files(app, &project).await?;
    app.broadcast(&app.snapshot()?);
    Ok(agent)
}

async fn create_agent(
    State(app): Shared,
    Json(body): Json<NewAgent>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let project_id = body
        .project_id
        .clone()
        .ok_or_else(|| AppError::BadRequest("project_id é obrigatório".into()))?;
    let agent = add_agent(&app, &project_id, body).await?;
    Ok((StatusCode::CREATED, Json(agent_view(&app, &agent))))
}

/// Apply a partial change to an agent and regenerate the team files.
pub(crate) async fn update_agent(app: &App, id: &str, patch: Value) -> AppResult<Agent> {
    let current = app.store.get_agent(id)?;
    let mut value = serde_json::to_value(&current).unwrap_or(Value::Null);
    let (Value::Object(target), Value::Object(changes)) = (&mut value, patch) else {
        return Err(AppError::BadRequest(
            "o corpo precisa ser um objeto JSON".into(),
        ));
    };
    const LOCKED: [&str; 6] = [
        "id",
        "project_id",
        "created_at",
        "paused",
        "pause_reason",
        "live",
    ];
    for (key, change) in changes {
        if !LOCKED.contains(&key.as_str()) {
            target.insert(key, change);
        }
    }
    let updated: Agent = serde_json::from_value(value)
        .map_err(|error| AppError::BadRequest(format!("campo inválido: {error}")))?;
    if !CLIS.contains(&updated.cli.as_str()) {
        return Err(AppError::BadRequest(format!(
            "CLI desconhecido: {}",
            updated.cli
        )));
    }
    if updated.name.trim().is_empty()
        || app
            .store
            .list_agents(Some(&updated.project_id))?
            .iter()
            .any(|other| other.id != updated.id && other.name.eq_ignore_ascii_case(&updated.name))
    {
        return Err(AppError::BadRequest(format!(
            "nome inválido ou já usado: {}",
            updated.name
        )));
    }
    app.store.update_agent(&updated)?;
    let project = app.store.get_project(&updated.project_id)?;
    team::write_team_files(app, &project).await?;
    app.broadcast(&app.snapshot()?);
    app.store.get_agent(id).map_err(Into::into)
}

async fn patch_agent(
    State(app): Shared,
    Path(id): Path<String>,
    Json(patch): Json<Value>,
) -> AppResult<Json<Value>> {
    let agent = update_agent(&app, &id, patch).await?;
    Ok(Json(agent_view(&app, &agent)))
}

#[derive(Deserialize, Default)]
struct PauseBody {
    reason: Option<String>,
}

async fn pause_agent(
    State(app): Shared,
    Path(id): Path<String>,
    body: Option<Json<PauseBody>>,
) -> AppResult<Json<Value>> {
    let reason = body
        .and_then(|Json(body)| body.reason)
        .unwrap_or_else(|| "Pausado por você".into());
    app.pause_agent(&id, &reason)?;
    Ok(Json(agent_view(&app, &app.store.get_agent(&id)?)))
}

async fn resume_agent(State(app): Shared, Path(id): Path<String>) -> AppResult<Json<Value>> {
    app.resume_agent(&id)?;
    Ok(Json(agent_view(&app, &app.store.get_agent(&id)?)))
}

#[derive(Deserialize)]
struct MessageBody {
    text: String,
}

async fn message_agent(
    State(app): Shared,
    Path(id): Path<String>,
    Json(body): Json<MessageBody>,
) -> AppResult<Json<Value>> {
    let agent = app.store.get_agent(&id)?;
    orchestrator::user_message(&app, &agent, &body.text)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct TimelineQuery {
    before: Option<i64>,
    limit: Option<i64>,
}

async fn timeline(
    State(app): Shared,
    Path(id): Path<String>,
    Query(query): Query<TimelineQuery>,
) -> AppResult<Json<Value>> {
    app.store.get_agent(&id)?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let items = app.store.timeline(&id, query.before, limit)?;
    let next_before = if items.len() as i64 == limit {
        items.last().map(|event| event.seq)
    } else {
        None
    };
    Ok(Json(json!({ "items": items, "next_before": next_before })))
}

async fn latest_diff(State(app): Shared, Path(id): Path<String>) -> AppResult<Json<Value>> {
    app.store.get_agent(&id)?;
    let Some(event) = app.store.latest_file_edit(&id)? else {
        return Ok(Json(json!({ "event": null, "diff": null })));
    };
    let inline = event
        .data
        .as_ref()
        .and_then(|data| data.get("diff"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let diff = match inline {
        Some(diff) => Some(diff),
        None => match event
            .data
            .as_ref()
            .and_then(|data| data.get("diff_ref"))
            .and_then(Value::as_str)
        {
            // Only files inside the daemon's own diff folder are served.
            Some(reference) if FsPath::new(reference).starts_with(app.cfg.diffs_dir()) => {
                tokio::fs::read_to_string(reference).await.ok()
            }
            _ => None,
        },
    };
    Ok(Json(json!({ "event": event, "diff": diff })))
}

// ---- tasks and messages -----------------------------------------------------

#[derive(Deserialize)]
struct NewTask {
    project_id: String,
    title: String,
    brief: Option<String>,
    budget_usd: Option<f64>,
    agent_id: Option<String>,
}

async fn create_task(
    State(app): Shared,
    Json(body): Json<NewTask>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let task = orchestrator::create_task(
        &app,
        &body.project_id,
        &body.title,
        body.brief.as_deref().unwrap_or(""),
        body.budget_usd,
        body.agent_id.as_deref(),
    )?;
    Ok((StatusCode::CREATED, Json(json!(task))))
}

async fn list_tasks(
    State(app): Shared,
    Query(filter): Query<ProjectFilter>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!(app
        .store
        .list_tasks(filter.project.as_deref())?)))
}

async fn list_messages(State(app): Shared) -> AppResult<Json<Value>> {
    Ok(Json(json!(app.store.recent_messages(200)?)))
}

// ---- usage and accounts -----------------------------------------------------

#[derive(Deserialize)]
struct UsageQuery {
    window: Option<String>,
}

async fn get_usage(State(app): Shared, Query(query): Query<UsageQuery>) -> AppResult<Json<Value>> {
    let mut report = usage::report(&app)?;
    if let Some(window) = query.window {
        if !["5h", "day", "week"].contains(&window.as_str()) {
            return Err(AppError::BadRequest(
                "window precisa ser 5h, day ou week".into(),
            ));
        }
        for group in ["accounts", "agents"] {
            for entry in report
                .get_mut(group)
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                if let Some(Value::Object(windows)) = entry.get_mut("windows") {
                    windows.retain(|key, _| key == &window);
                }
            }
        }
    }
    Ok(Json(report))
}

async fn list_accounts(State(app): Shared) -> AppResult<Json<Value>> {
    Ok(Json(json!(app.store.list_accounts()?)))
}

async fn account_status(State(app): Shared, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let account = app.store.get_account(&id)?;
    Ok(Json(crate::adapters::account_status(&account).await))
}

async fn account_models(State(app): Shared, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let account = app.store.get_account(&id)?;
    Ok(Json(crate::adapters::models::models_for(&account).await))
}

async fn patch_account(
    State(app): Shared,
    Path(id): Path<String>,
    Json(patch): Json<Value>,
) -> AppResult<Json<Value>> {
    let mut account = app.store.get_account(&id)?;
    let text = |key: &str| patch.get(key).and_then(Value::as_str).map(str::to_string);
    if let Some(value) = text("login_type") {
        account.login_type = value;
    }
    if let Some(value) = text("cli_path") {
        account.cli_path = value;
    }
    if let Some(value) = text("default_model") {
        account.default_model = value;
    }
    if let Some(rule) = patch.get("limit_rule").filter(|rule| rule.is_object()) {
        account.limit_rule = rule.clone();
    }
    if let Some(max) = patch.get("max_concurrent").and_then(Value::as_i64) {
        if max < 1 {
            return Err(AppError::BadRequest(
                "max_concurrent precisa ser pelo menos 1".into(),
            ));
        }
        account.max_concurrent = max;
    }
    app.store.upsert_account(&account)?;
    app.broadcast(&json!({ "type": "usage", "usage": usage::report(&app)? }));
    app.wake.notify_one();
    Ok(Json(json!(account)))
}

// ---- approvals --------------------------------------------------------------

#[derive(Deserialize)]
struct ApprovalFilter {
    status: Option<String>,
}

async fn list_approvals(
    State(app): Shared,
    Query(filter): Query<ApprovalFilter>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!(app
        .store
        .list_approvals(filter.status.as_deref())?)))
}

async fn get_approval(State(app): Shared, Path(id): Path<String>) -> AppResult<Json<Value>> {
    Ok(Json(json!(app.store.get_approval(&id)?)))
}

#[derive(Deserialize)]
struct NewApproval {
    agent_id: String,
    action: String,
    detail: String,
}

/// Used by `regente run` when an agent tries an irreversible command.
async fn create_approval(
    State(app): Shared,
    Json(body): Json<NewApproval>,
) -> AppResult<(StatusCode, Json<Value>)> {
    app.store.get_agent(&body.agent_id)?;
    let task_id = app.running_task(&body.agent_id);
    let (approval, _) = app.request_approval(
        &body.agent_id,
        task_id.as_deref(),
        &body.action,
        &body.detail,
    )?;
    Ok((StatusCode::CREATED, Json(json!(approval))))
}

#[derive(Deserialize)]
struct Decision {
    decision: String,
    by: Option<String>,
}

async fn decide_approval(
    State(app): Shared,
    Path(id): Path<String>,
    Json(body): Json<Decision>,
) -> AppResult<Json<Value>> {
    let approve = match body.decision.as_str() {
        "approve" | "approved" => true,
        "deny" | "denied" => false,
        other => return Err(AppError::BadRequest(format!("decisão inválida: {other}"))),
    };
    let approval = app.decide_approval(&id, approve, body.by.as_deref().unwrap_or("user"))?;
    let background = app.clone();
    let decided = approval.clone();
    tokio::spawn(async move {
        if let Err(error) = orchestrator::after_approval(&background, &decided).await {
            tracing::error!(%error, "post-approval action failed");
        }
    });
    Ok(Json(json!(approval)))
}

// ---- messenger --------------------------------------------------------------

async fn mcp_rpc(
    State(app): Shared,
    Path(agent_id): Path<String>,
    Json(request): Json<Value>,
) -> Response {
    match mcp::handle_rpc(&app, &agent_id, &request).await {
        Some(reply) => Json(reply).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

// ---- static UI --------------------------------------------------------------

fn content_type(path: &FsPath) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        _ => "application/octet-stream",
    }
}

/// Serve the built UI; unknown paths fall back to `index.html` (single-page app).
async fn static_ui(State(app): Shared, uri: Uri) -> Response {
    let Some(dir) = &app.cfg.ui_dir else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response();
    };
    if uri.path().starts_with("/api/") || uri.path().starts_with("/mcp/") {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response();
    }
    let relative = FsPath::new(uri.path().trim_start_matches('/'));
    let safe = relative
        .components()
        .all(|part| matches!(part, Component::Normal(_)));
    let mut file = if safe && !relative.as_os_str().is_empty() {
        dir.join(relative)
    } else {
        dir.join("index.html")
    };
    if !file.is_file() {
        file = dir.join("index.html");
    }
    match tokio::fs::read(&file).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, content_type(&file))], bytes).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "UI not built").into_response(),
    }
}
