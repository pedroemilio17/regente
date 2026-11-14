//! REST and WebSocket API (`/api/v1`), the messenger endpoint and the static UI.

mod pty;

use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode, Uri};
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
        .route("/approvals", get(list_approvals).post(create_approval))
        .route("/approvals/{id}", get(get_approval).post(decide_approval));
    Router::new()
        .nest("/api/v1", api)
        .route("/mcp/{agent_id}", post(mcp_rpc))
        .fallback(static_ui)
        .with_state(app)
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
    Json(body): Json<NewProject>,
) -> AppResult<(StatusCode, Json<Value>)> {
    if body.name.trim().is_empty() || body.root.trim().is_empty() {
        return Err(AppError::BadRequest(
            "nome e pasta do projeto são obrigatórios".into(),
        ));
    }
    let root = PathBuf::from(body.root.trim());
    if !root.is_absolute() {
        return Err(AppError::BadRequest(
            "a pasta do projeto precisa ser um caminho absoluto".into(),
        ));
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
        base_branch: body
            .base_branch
            .filter(|b| !b.trim().is_empty())
            .unwrap_or_else(|| "main".into()),
        context: body.context.unwrap_or_default(),
        folders: body.folders.unwrap_or_else(|| json!([])),
        blocked: json!(blocked),
        day_budget_usd: body.day_budget_usd,
        created_at: now(),
    };
    app.store.insert_project(&project)?;
    let mut agents = Vec::new();
    for new_agent in body.agents {
        agents.push(insert_agent(&app, &project.id, new_agent)?);
    }
    let files = team::write_team_files(&app, &project).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "project": project, "agents": agents, "files": files })),
    ))
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

#[derive(Deserialize)]
struct NewAgent {
    project_id: Option<String>,
    name: String,
    cli: String,
    role: Option<String>,
    #[serde(default)]
    is_orchestrator: bool,
    parent_id: Option<String>,
    account_id: Option<String>,
    model: Option<String>,
    pet: Option<Value>,
    permissions: Option<Value>,
    settings: Option<Value>,
    budget_usd: Option<f64>,
}

const CLIS: [&str; 4] = ["claude", "codex", "agy", "fake"];

fn insert_agent(app: &App, project_id: &str, body: NewAgent) -> AppResult<Agent> {
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("o agente precisa de um nome".into()));
    }
    if !CLIS.contains(&body.cli.as_str()) {
        return Err(AppError::BadRequest(format!(
            "CLI desconhecido: {}",
            body.cli
        )));
    }
    app.store.get_project(project_id)?;
    let existing = app.store.list_agents(Some(project_id))?;
    if existing
        .iter()
        .any(|agent| agent.name.eq_ignore_ascii_case(body.name.trim()))
    {
        return Err(AppError::BadRequest(format!(
            "já existe um agente chamado {}",
            body.name.trim()
        )));
    }
    let default_account = format!("acc_{}", body.cli);
    let account_id = body.account_id.or_else(|| {
        app.store
            .get_account(&default_account)
            .ok()
            .map(|account| account.id)
    });
    let agent = Agent {
        id: new_id("ag"),
        project_id: project_id.to_string(),
        name: body.name.trim().to_string(),
        cli: body.cli,
        role: body.role.unwrap_or_default(),
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
    };
    app.store.insert_agent(&agent)?;
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
    let agent = insert_agent(&app, &project_id, body)?;
    let project = app.store.get_project(&project_id)?;
    team::write_team_files(&app, &project).await?;
    app.broadcast(&app.snapshot()?);
    Ok((StatusCode::CREATED, Json(agent_view(&app, &agent))))
}

async fn patch_agent(
    State(app): Shared,
    Path(id): Path<String>,
    Json(patch): Json<Value>,
) -> AppResult<Json<Value>> {
    let current = app.store.get_agent(&id)?;
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
    app.store.update_agent(&updated)?;
    let project = app.store.get_project(&updated.project_id)?;
    team::write_team_files(&app, &project).await?;
    app.broadcast(&app.snapshot()?);
    Ok(Json(agent_view(&app, &app.store.get_agent(&id)?)))
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
    match mcp::handle_rpc(&app, &agent_id, &request) {
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
