//! SQLite store: numbered migrations, the append-only event log and the
//! tables derived from it.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;

use crate::model::{
    now, Account, Agent, Approval, Event, EventKind, FileChange, Message, Project, Session, Task,
    UsageRow,
};

/// Migrations in order; index + 1 is the schema version.
const MIGRATIONS: &[&str] = &[include_str!("../../migrations/0001_init.sql")];

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not found: {0}")]
    NotFound(String),
}

pub type StoreResult<T> = Result<T, StoreError>;

pub struct Store {
    conn: Mutex<Connection>,
}

fn json(text: String) -> Value {
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn project_row(row: &Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        root: row.get(2)?,
        base_branch: row.get(3)?,
        context: row.get(4)?,
        folders: json(row.get(5)?),
        blocked: json(row.get(6)?),
        day_budget_usd: row.get(7)?,
        created_at: row.get(8)?,
    })
}

const PROJECT_COLS: &str =
    "id, name, root, base_branch, context, folders, blocked, day_budget_usd, created_at";

fn account_row(row: &Row) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        provider: row.get(1)?,
        login_type: row.get(2)?,
        cli_path: row.get(3)?,
        default_model: row.get(4)?,
        limit_rule: json(row.get(5)?),
        max_concurrent: row.get(6)?,
    })
}

const ACCOUNT_COLS: &str =
    "id, provider, login_type, cli_path, default_model, limit_rule, max_concurrent";

fn agent_row(row: &Row) -> rusqlite::Result<Agent> {
    Ok(Agent {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        cli: row.get(3)?,
        role: row.get(4)?,
        is_orchestrator: row.get::<_, i64>(5)? != 0,
        parent_id: row.get(6)?,
        account_id: row.get(7)?,
        model: row.get(8)?,
        pet: json(row.get(9)?),
        permissions: json(row.get(10)?),
        settings: json(row.get(11)?),
        budget_usd: row.get(12)?,
        paused: row.get::<_, i64>(13)? != 0,
        pause_reason: row.get(14)?,
        created_at: row.get(15)?,
    })
}

const AGENT_COLS: &str =
    "id, project_id, name, cli, role, is_orchestrator, parent_id, account_id, model, pet, \
     permissions, settings, budget_usd, paused, pause_reason, created_at";

fn task_row(row: &Row) -> rusqlite::Result<Task> {
    let depends: String = row.get(11)?;
    Ok(Task {
        id: row.get(0)?,
        project_id: row.get(1)?,
        parent_task_id: row.get(2)?,
        root_task_id: row.get(3)?,
        owner_agent_id: row.get(4)?,
        title: row.get(5)?,
        brief: row.get(6)?,
        done_criteria: row.get(7)?,
        status: row.get(8)?,
        branch: row.get(9)?,
        budget_usd: row.get(10)?,
        depends_on: serde_json::from_str(&depends).unwrap_or_default(),
        summary: row.get(12)?,
        message_count: row.get(13)?,
        stalled_rounds: row.get(14)?,
        created_at: row.get(15)?,
        finished_at: row.get(16)?,
    })
}

const TASK_COLS: &str = "id, project_id, parent_task_id, root_task_id, owner_agent_id, title, brief, done_criteria, \
     status, branch, budget_usd, depends_on, summary, message_count, stalled_rounds, created_at, finished_at";

fn session_row(row: &Row) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        task_id: row.get(2)?,
        cli_session_id: row.get(3)?,
        worktree: row.get(4)?,
        started_at: row.get(5)?,
        ended_at: row.get(6)?,
    })
}

const SESSION_COLS: &str = "id, agent_id, task_id, cli_session_id, worktree, started_at, ended_at";

fn message_row(row: &Row) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get(0)?,
        task_id: row.get(1)?,
        from_agent: row.get(2)?,
        to_agent: row.get(3)?,
        text: row.get(4)?,
        created_at: row.get(5)?,
        delivered_at: row.get(6)?,
    })
}

const MESSAGE_COLS: &str = "id, task_id, from_agent, to_agent, text, created_at, delivered_at";

fn approval_row(row: &Row) -> rusqlite::Result<Approval> {
    Ok(Approval {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        task_id: row.get(2)?,
        action: row.get(3)?,
        detail: row.get(4)?,
        status: row.get(5)?,
        decided_by: row.get(6)?,
        created_at: row.get(7)?,
        decided_at: row.get(8)?,
    })
}

const APPROVAL_COLS: &str =
    "id, agent_id, task_id, action, detail, status, decided_by, created_at, decided_at";

fn event_row(row: &Row) -> rusqlite::Result<Event> {
    let kind: String = row.get(5)?;
    let payload = json(row.get(8)?);
    Ok(Event {
        seq: row.get(0)?,
        id: row.get(1)?,
        ts: row.get(2)?,
        agent_id: row.get(3)?,
        task_id: row.get(4)?,
        kind: EventKind::parse(&kind).unwrap_or(EventKind::Error),
        target: row.get(6)?,
        summary: row.get(7)?,
        lines_added: payload.get("lines_added").and_then(Value::as_i64),
        lines_removed: payload.get("lines_removed").and_then(Value::as_i64),
        usage: payload
            .get("usage")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok()),
        raw_ref: payload
            .get("raw_ref")
            .and_then(Value::as_str)
            .map(str::to_string),
        data: payload.get("data").filter(|v| !v.is_null()).cloned(),
    })
}

const EVENT_COLS: &str = "seq, id, ts, agent_id, task_id, kind, target, summary, payload";

impl Store {
    /// Open (or create) the database file and apply pending migrations.
    pub fn open(path: &Path) -> StoreResult<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let existed = path.exists();
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if existed && version > 0 && (version as usize) < MIGRATIONS.len() {
            // Automatic backup before touching an existing database.
            let backup = path.with_extension(format!("db.bak-v{version}"));
            std::fs::copy(path, backup)?;
        }
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    /// In-memory database, used by tests.
    pub fn open_memory() -> StoreResult<Self> {
        let store = Self {
            conn: Mutex::new(Connection::open_in_memory()?),
        };
        store.migrate()?;
        Ok(store)
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn migrate(&self) -> StoreResult<()> {
        let conn = self.conn();
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            conn.execute_batch(&format!("BEGIN;\n{sql}\nCOMMIT;"))?;
            conn.pragma_update(None, "user_version", (index + 1) as i64)?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> StoreResult<i64> {
        Ok(self
            .conn()
            .pragma_query_value(None, "user_version", |row| row.get(0))?)
    }

    // ---- events (append-only) -------------------------------------------

    /// Append an event and its derived rows. Returns the sequence number.
    pub fn append_event(&self, event: &Event, account_id: Option<&str>) -> StoreResult<i64> {
        let payload = serde_json::json!({
            "lines_added": event.lines_added,
            "lines_removed": event.lines_removed,
            "usage": event.usage,
            "raw_ref": event.raw_ref,
            "data": event.data,
        });
        let conn = self.conn();
        conn.execute(
            "INSERT INTO events (id, ts, agent_id, task_id, kind, target, summary, payload) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                event.id,
                event.ts,
                event.agent_id,
                event.task_id,
                event.kind.as_str(),
                event.target,
                event.summary,
                payload.to_string()
            ],
        )?;
        let seq = conn.last_insert_rowid();
        Self::derive(&conn, event, account_id)?;
        Ok(seq)
    }

    fn derive(conn: &Connection, event: &Event, account_id: Option<&str>) -> StoreResult<()> {
        if let Some(usage) = &event.usage {
            conn.execute(
                "INSERT INTO usage (event_id, ts, agent_id, account_id, task_id, tokens_in, tokens_out, tokens_cache, cost_usd, cost_source) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    event.id,
                    event.ts,
                    event.agent_id,
                    account_id,
                    event.task_id,
                    usage.tokens_in,
                    usage.tokens_out,
                    usage.tokens_cache,
                    usage.cost_usd,
                    usage.cost_source
                ],
            )?;
        }
        if event.kind == EventKind::FileEdit {
            if let Some(path) = &event.target {
                let diff_ref = event
                    .data
                    .as_ref()
                    .and_then(|d| d.get("diff_ref"))
                    .and_then(Value::as_str);
                conn.execute(
                    "INSERT INTO file_changes (event_id, agent_id, ts, path, lines_added, lines_removed, diff_ref) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    params![
                        event.id,
                        event.agent_id,
                        event.ts,
                        path,
                        event.lines_added.unwrap_or(0),
                        event.lines_removed.unwrap_or(0),
                        diff_ref
                    ],
                )?;
            }
        }
        Ok(())
    }

    /// Drop and rebuild `usage` and `file_changes` from the event log.
    pub fn rebuild_derived(&self) -> StoreResult<()> {
        let events = self.all_events()?;
        let conn = self.conn();
        let accounts: std::collections::HashMap<String, Option<String>> = {
            let mut stmt = conn.prepare("SELECT id, account_id FROM agents")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        conn.execute_batch("DELETE FROM usage; DELETE FROM file_changes;")?;
        for event in &events {
            let account = accounts.get(&event.agent_id).cloned().flatten();
            Self::derive(&conn, event, account.as_deref())?;
        }
        Ok(())
    }

    pub fn all_events(&self) -> StoreResult<Vec<Event>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {EVENT_COLS} FROM events ORDER BY seq"))?;
        let rows = stmt.query_map([], event_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Newest-first page of an agent's events, older than `before` when given.
    pub fn timeline(
        &self,
        agent_id: &str,
        before: Option<i64>,
        limit: i64,
    ) -> StoreResult<Vec<Event>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {EVENT_COLS} FROM events WHERE agent_id = ?1 AND seq < ?2 ORDER BY seq DESC LIMIT ?3"
        ))?;
        let rows = stmt.query_map(
            params![agent_id, before.unwrap_or(i64::MAX), limit],
            event_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Events of a task tree, oldest first.
    pub fn task_events(&self, task_id: &str) -> StoreResult<Vec<Event>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {EVENT_COLS} FROM events WHERE task_id = ?1 ORDER BY seq"
        ))?;
        let rows = stmt.query_map(params![task_id], event_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn latest_file_edit(&self, agent_id: &str) -> StoreResult<Option<Event>> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                &format!("SELECT {EVENT_COLS} FROM events WHERE agent_id = ?1 AND kind = 'file_edit' ORDER BY seq DESC LIMIT 1"),
                params![agent_id],
                event_row,
            )
            .optional()?)
    }

    pub fn recent_file_changes(&self, agent_id: &str, limit: i64) -> StoreResult<Vec<FileChange>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT event_id, agent_id, ts, path, lines_added, lines_removed, diff_ref FROM file_changes \
             WHERE agent_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![agent_id, limit], |row| {
            Ok(FileChange {
                event_id: row.get(0)?,
                agent_id: row.get(1)?,
                ts: row.get(2)?,
                path: row.get(3)?,
                lines_added: row.get(4)?,
                lines_removed: row.get(5)?,
                diff_ref: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---- projects -------------------------------------------------------

    pub fn insert_project(&self, project: &Project) -> StoreResult<()> {
        self.conn().execute(
            &format!("INSERT INTO projects ({PROJECT_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)"),
            params![
                project.id,
                project.name,
                project.root,
                project.base_branch,
                project.context,
                project.folders.to_string(),
                project.blocked.to_string(),
                project.day_budget_usd,
                project.created_at
            ],
        )?;
        Ok(())
    }

    pub fn list_projects(&self) -> StoreResult<Vec<Project>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {PROJECT_COLS} FROM projects ORDER BY created_at"
        ))?;
        let rows = stmt.query_map([], project_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_project(&self, id: &str) -> StoreResult<Project> {
        self.conn()
            .query_row(
                &format!("SELECT {PROJECT_COLS} FROM projects WHERE id = ?1"),
                params![id],
                project_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("project {id}")))
    }

    // ---- accounts -------------------------------------------------------

    pub fn upsert_account(&self, account: &Account) -> StoreResult<()> {
        self.conn().execute(
            &format!(
                "INSERT INTO accounts ({ACCOUNT_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7) \
                 ON CONFLICT(id) DO UPDATE SET provider=?2, login_type=?3, cli_path=?4, default_model=?5, limit_rule=?6, max_concurrent=?7"
            ),
            params![
                account.id,
                account.provider,
                account.login_type,
                account.cli_path,
                account.default_model,
                account.limit_rule.to_string(),
                account.max_concurrent
            ],
        )?;
        Ok(())
    }

    pub fn list_accounts(&self) -> StoreResult<Vec<Account>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {ACCOUNT_COLS} FROM accounts ORDER BY id"))?;
        let rows = stmt.query_map([], account_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_account(&self, id: &str) -> StoreResult<Account> {
        self.conn()
            .query_row(
                &format!("SELECT {ACCOUNT_COLS} FROM accounts WHERE id = ?1"),
                params![id],
                account_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("account {id}")))
    }

    // ---- agents ---------------------------------------------------------

    pub fn insert_agent(&self, agent: &Agent) -> StoreResult<()> {
        self.conn().execute(
            &format!("INSERT INTO agents ({AGENT_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)"),
            params![
                agent.id,
                agent.project_id,
                agent.name,
                agent.cli,
                agent.role,
                agent.is_orchestrator as i64,
                agent.parent_id,
                agent.account_id,
                agent.model,
                agent.pet.to_string(),
                agent.permissions.to_string(),
                agent.settings.to_string(),
                agent.budget_usd,
                agent.paused as i64,
                agent.pause_reason,
                agent.created_at
            ],
        )?;
        Ok(())
    }

    /// Replace the editable fields of an agent.
    pub fn update_agent(&self, agent: &Agent) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE agents SET name=?2, cli=?3, role=?4, is_orchestrator=?5, parent_id=?6, account_id=?7, model=?8, pet=?9, \
             permissions=?10, settings=?11, budget_usd=?12 WHERE id=?1",
            params![
                agent.id,
                agent.name,
                agent.cli,
                agent.role,
                agent.is_orchestrator as i64,
                agent.parent_id,
                agent.account_id,
                agent.model,
                agent.pet.to_string(),
                agent.permissions.to_string(),
                agent.settings.to_string(),
                agent.budget_usd
            ],
        )?;
        Ok(())
    }

    pub fn set_agent_paused(
        &self,
        id: &str,
        paused: bool,
        reason: Option<&str>,
    ) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE agents SET paused=?2, pause_reason=?3 WHERE id=?1",
            params![id, paused as i64, if paused { reason } else { None }],
        )?;
        Ok(())
    }

    pub fn list_agents(&self, project_id: Option<&str>) -> StoreResult<Vec<Agent>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {AGENT_COLS} FROM agents WHERE (?1 IS NULL OR project_id = ?1) ORDER BY is_orchestrator DESC, created_at"
        ))?;
        let rows = stmt.query_map(params![project_id], agent_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_agent(&self, id: &str) -> StoreResult<Agent> {
        self.conn()
            .query_row(
                &format!("SELECT {AGENT_COLS} FROM agents WHERE id = ?1"),
                params![id],
                agent_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("agent {id}")))
    }

    // ---- tasks ----------------------------------------------------------

    pub fn insert_task(&self, task: &Task) -> StoreResult<()> {
        self.conn().execute(
            &format!("INSERT INTO tasks ({TASK_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)"),
            params![
                task.id,
                task.project_id,
                task.parent_task_id,
                task.root_task_id,
                task.owner_agent_id,
                task.title,
                task.brief,
                task.done_criteria,
                task.status,
                task.branch,
                task.budget_usd,
                serde_json::json!(task.depends_on).to_string(),
                task.summary,
                task.message_count,
                task.stalled_rounds,
                task.created_at,
                task.finished_at
            ],
        )?;
        Ok(())
    }

    pub fn get_task(&self, id: &str) -> StoreResult<Task> {
        self.conn()
            .query_row(
                &format!("SELECT {TASK_COLS} FROM tasks WHERE id = ?1"),
                params![id],
                task_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("task {id}")))
    }

    pub fn list_tasks(&self, project_id: Option<&str>) -> StoreResult<Vec<Task>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TASK_COLS} FROM tasks WHERE (?1 IS NULL OR project_id = ?1) ORDER BY created_at, rowid"
        ))?;
        let rows = stmt.query_map(params![project_id], task_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn tasks_with_status(&self, status: &str) -> StoreResult<Vec<Task>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TASK_COLS} FROM tasks WHERE status = ?1 ORDER BY created_at, rowid"
        ))?;
        let rows = stmt.query_map(params![status], task_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn child_tasks(&self, parent_id: &str) -> StoreResult<Vec<Task>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TASK_COLS} FROM tasks WHERE parent_task_id = ?1 ORDER BY created_at, rowid"
        ))?;
        let rows = stmt.query_map(params![parent_id], task_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Open (not done/cancelled) tasks owned by an agent, oldest first.
    pub fn open_tasks_of(&self, agent_id: &str) -> StoreResult<Vec<Task>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TASK_COLS} FROM tasks WHERE owner_agent_id = ?1 AND status NOT IN ('done','cancelled') ORDER BY created_at, rowid"
        ))?;
        let rows = stmt.query_map(params![agent_id], task_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_task_status(&self, id: &str, status: &str) -> StoreResult<()> {
        let finished = if status == "done" || status == "cancelled" {
            Some(now())
        } else {
            None
        };
        self.conn().execute(
            "UPDATE tasks SET status=?2, finished_at=COALESCE(?3, finished_at) WHERE id=?1",
            params![id, status, finished],
        )?;
        Ok(())
    }

    pub fn set_task_summary(&self, id: &str, summary: &str) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE tasks SET summary=?2 WHERE id=?1",
            params![id, summary],
        )?;
        Ok(())
    }

    pub fn set_task_branch(&self, id: &str, branch: &str) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE tasks SET branch=?2 WHERE id=?1",
            params![id, branch],
        )?;
        Ok(())
    }

    pub fn set_task_stalled(&self, id: &str, rounds: i64) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE tasks SET stalled_rounds=?2 WHERE id=?1",
            params![id, rounds],
        )?;
        Ok(())
    }

    /// Increment the message counter of a task and return the new value.
    pub fn bump_task_messages(&self, id: &str) -> StoreResult<i64> {
        let conn = self.conn();
        conn.execute(
            "UPDATE tasks SET message_count = message_count + 1 WHERE id=?1",
            params![id],
        )?;
        Ok(conn.query_row(
            "SELECT message_count FROM tasks WHERE id=?1",
            params![id],
            |row| row.get(0),
        )?)
    }

    // ---- sessions -------------------------------------------------------

    pub fn insert_session(&self, session: &Session) -> StoreResult<()> {
        self.conn().execute(
            &format!("INSERT INTO sessions ({SESSION_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7)"),
            params![
                session.id,
                session.agent_id,
                session.task_id,
                session.cli_session_id,
                session.worktree,
                session.started_at,
                session.ended_at
            ],
        )?;
        Ok(())
    }

    pub fn set_cli_session(&self, id: &str, cli_session_id: &str) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE sessions SET cli_session_id=?2 WHERE id=?1",
            params![id, cli_session_id],
        )?;
        Ok(())
    }

    pub fn end_session(&self, id: &str) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE sessions SET ended_at=?2 WHERE id=?1",
            params![id, now()],
        )?;
        Ok(())
    }

    /// Sessions (turns) an agent already had on a task, oldest first.
    pub fn sessions_of(&self, agent_id: &str, task_id: &str) -> StoreResult<Vec<Session>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SESSION_COLS} FROM sessions WHERE agent_id = ?1 AND task_id = ?2 ORDER BY started_at, rowid"
        ))?;
        let rows = stmt.query_map(params![agent_id, task_id], session_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn latest_session_of_agent(&self, agent_id: &str) -> StoreResult<Option<Session>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {SESSION_COLS} FROM sessions WHERE agent_id = ?1 ORDER BY started_at DESC, rowid DESC LIMIT 1"),
                params![agent_id],
                session_row,
            )
            .optional()?)
    }

    // ---- messages -------------------------------------------------------

    pub fn insert_message(&self, message: &Message) -> StoreResult<()> {
        self.conn().execute(
            &format!("INSERT INTO messages ({MESSAGE_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7)"),
            params![
                message.id,
                message.task_id,
                message.from_agent,
                message.to_agent,
                message.text,
                message.created_at,
                message.delivered_at
            ],
        )?;
        Ok(())
    }

    pub fn pending_messages(&self, to_agent: &str) -> StoreResult<Vec<Message>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {MESSAGE_COLS} FROM messages WHERE to_agent = ?1 AND delivered_at IS NULL ORDER BY created_at, rowid"
        ))?;
        let rows = stmt.query_map(params![to_agent], message_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn mark_delivered(&self, id: &str) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE messages SET delivered_at=?2 WHERE id=?1",
            params![id, now()],
        )?;
        Ok(())
    }

    pub fn recent_messages(&self, limit: i64) -> StoreResult<Vec<Message>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {MESSAGE_COLS} FROM (SELECT rowid AS r, * FROM messages ORDER BY r DESC LIMIT ?1) ORDER BY r"
        ))?;
        let rows = stmt.query_map(params![limit], message_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---- usage ----------------------------------------------------------

    /// Usage rows at or after `since` (RFC 3339), oldest first.
    pub fn usage_since(&self, since: &str) -> StoreResult<Vec<UsageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT ts, agent_id, account_id, task_id, tokens_in, tokens_out, tokens_cache, cost_usd FROM usage WHERE ts >= ?1 ORDER BY ts, id",
        )?;
        let rows = stmt.query_map(params![since], |row| {
            Ok(UsageRow {
                ts: row.get(0)?,
                agent_id: row.get(1)?,
                account_id: row.get(2)?,
                task_id: row.get(3)?,
                tokens_in: row.get(4)?,
                tokens_out: row.get(5)?,
                tokens_cache: row.get(6)?,
                cost_usd: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Total cost of every task that shares the given root task.
    pub fn cost_of_task_tree(&self, root_task_id: &str) -> StoreResult<f64> {
        Ok(self.conn().query_row(
            "SELECT COALESCE(SUM(u.cost_usd), 0) FROM usage u JOIN tasks t ON t.id = u.task_id WHERE t.root_task_id = ?1",
            params![root_task_id],
            |row| row.get(0),
        )?)
    }

    pub fn cost_of_agent_since(&self, agent_id: &str, since: &str) -> StoreResult<f64> {
        Ok(self.conn().query_row(
            "SELECT COALESCE(SUM(cost_usd), 0) FROM usage WHERE agent_id = ?1 AND ts >= ?2",
            params![agent_id, since],
            |row| row.get(0),
        )?)
    }

    pub fn cost_of_project_since(&self, project_id: &str, since: &str) -> StoreResult<f64> {
        Ok(self.conn().query_row(
            "SELECT COALESCE(SUM(u.cost_usd), 0) FROM usage u JOIN agents a ON a.id = u.agent_id WHERE a.project_id = ?1 AND u.ts >= ?2",
            params![project_id, since],
            |row| row.get(0),
        )?)
    }

    pub fn save_usage_window(
        &self,
        account_id: &str,
        window: &str,
        start: &str,
        tokens: i64,
        cost: f64,
        pct: Option<f64>,
    ) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO usage_windows (account_id, window, start, tokens, cost, pct) VALUES (?1,?2,?3,?4,?5,?6) \
             ON CONFLICT(account_id, window) DO UPDATE SET start=?3, tokens=?4, cost=?5, pct=?6",
            params![account_id, window, start, tokens, cost, pct],
        )?;
        Ok(())
    }

    /// Store the plan utilization a CLI reported for a window (0.0–1.0).
    pub fn save_reported_pct(
        &self,
        account_id: &str,
        window: &str,
        pct: f64,
        resets_at: Option<&str>,
    ) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO usage_windows (account_id, window, start, reported_pct, resets_at) VALUES (?1,?2,?3,?4,?5) \
             ON CONFLICT(account_id, window) DO UPDATE SET reported_pct=?4, resets_at=?5",
            params![account_id, window, now(), pct, resets_at],
        )?;
        Ok(())
    }

    /// Reported utilization for an account window, if the CLI gave one.
    pub fn reported_pct(
        &self,
        account_id: &str,
        window: &str,
    ) -> StoreResult<Option<(f64, Option<String>)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT reported_pct, resets_at FROM usage_windows WHERE account_id=?1 AND window=?2 AND reported_pct IS NOT NULL",
                params![account_id, window],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    // ---- approvals ------------------------------------------------------

    pub fn insert_approval(&self, approval: &Approval) -> StoreResult<()> {
        self.conn().execute(
            &format!("INSERT INTO approvals ({APPROVAL_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)"),
            params![
                approval.id,
                approval.agent_id,
                approval.task_id,
                approval.action,
                approval.detail,
                approval.status,
                approval.decided_by,
                approval.created_at,
                approval.decided_at
            ],
        )?;
        Ok(())
    }

    pub fn get_approval(&self, id: &str) -> StoreResult<Approval> {
        self.conn()
            .query_row(
                &format!("SELECT {APPROVAL_COLS} FROM approvals WHERE id = ?1"),
                params![id],
                approval_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("approval {id}")))
    }

    pub fn list_approvals(&self, status: Option<&str>) -> StoreResult<Vec<Approval>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {APPROVAL_COLS} FROM approvals WHERE (?1 IS NULL OR status = ?1) ORDER BY created_at, rowid"
        ))?;
        let rows = stmt.query_map(params![status], approval_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn decide_approval(&self, id: &str, status: &str, decided_by: &str) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE approvals SET status=?2, decided_by=?3, decided_at=?4 WHERE id=?1",
            params![id, status, decided_by, now()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{new_id, Usage};

    fn project(store: &Store) -> Project {
        let project = Project {
            id: "pr_1".into(),
            name: "Demo".into(),
            root: "/tmp/demo".into(),
            base_branch: "main".into(),
            context: String::new(),
            folders: serde_json::json!([]),
            blocked: serde_json::json!([".env*"]),
            day_budget_usd: Some(5.0),
            created_at: now(),
        };
        store.insert_project(&project).unwrap();
        project
    }

    fn agent(store: &Store, id: &str) -> Agent {
        let agent = Agent {
            id: id.into(),
            project_id: "pr_1".into(),
            name: id.into(),
            cli: "fake".into(),
            role: "dev".into(),
            is_orchestrator: false,
            parent_id: None,
            account_id: Some("acc_claude".into()),
            model: String::new(),
            pet: serde_json::json!({}),
            permissions: serde_json::json!({}),
            settings: serde_json::json!({}),
            budget_usd: None,
            paused: false,
            pause_reason: None,
            created_at: now(),
        };
        store.insert_agent(&agent).unwrap();
        agent
    }

    fn event(agent: &str, kind: EventKind) -> Event {
        Event {
            seq: 0,
            id: new_id("evt"),
            ts: now(),
            agent_id: agent.into(),
            task_id: None,
            kind,
            target: Some("src/a.rs".into()),
            summary: "s".into(),
            lines_added: Some(3),
            lines_removed: Some(1),
            usage: Some(Usage {
                tokens_in: 10,
                tokens_out: 5,
                tokens_cache: 0,
                cost_usd: 0.5,
                cost_source: "estimated".into(),
            }),
            raw_ref: None,
            data: None,
        }
    }

    #[test]
    fn migrations_set_schema_version() {
        let store = Store::open_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len() as i64);
    }

    #[test]
    fn projects_agents_round_trip() {
        let store = Store::open_memory().unwrap();
        let project = project(&store);
        let agent = agent(&store, "ag_1");
        assert_eq!(store.get_project("pr_1").unwrap(), project);
        assert_eq!(store.list_agents(Some("pr_1")).unwrap(), vec![agent]);
        assert_eq!(project.blocked_patterns(), vec![".env*".to_string()]);
        assert!(matches!(
            store.get_agent("nope"),
            Err(StoreError::NotFound(_))
        ));
        store
            .set_agent_paused("ag_1", true, Some("budget"))
            .unwrap();
        let paused = store.get_agent("ag_1").unwrap();
        assert!(paused.paused);
        assert_eq!(paused.pause_reason.as_deref(), Some("budget"));
    }

    #[test]
    fn events_are_ordered_and_paginated() {
        let store = Store::open_memory().unwrap();
        for _ in 0..5 {
            store
                .append_event(&event("ag_1", EventKind::FileEdit), None)
                .unwrap();
        }
        let all = store.all_events().unwrap();
        assert_eq!(
            all.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );
        let page = store.timeline("ag_1", None, 2).unwrap();
        assert_eq!(page.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![5, 4]);
        let older = store.timeline("ag_1", Some(4), 10).unwrap();
        assert_eq!(
            older.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
        assert_eq!(all[0].lines_added, Some(3));
        assert_eq!(all[0].usage.as_ref().unwrap().cost_usd, 0.5);
    }

    #[test]
    fn derived_tables_rebuild_from_events() {
        let store = Store::open_memory().unwrap();
        project(&store);
        agent(&store, "ag_1");
        store
            .append_event(&event("ag_1", EventKind::FileEdit), Some("acc_claude"))
            .unwrap();
        store
            .append_event(&event("ag_1", EventKind::Thinking), Some("acc_claude"))
            .unwrap();
        let before = (
            store.usage_since("").unwrap(),
            store.recent_file_changes("ag_1", 20).unwrap(),
        );
        assert_eq!(before.0.len(), 2);
        assert_eq!(before.1.len(), 1);
        store.rebuild_derived().unwrap();
        let after = (
            store.usage_since("").unwrap(),
            store.recent_file_changes("ag_1", 20).unwrap(),
        );
        assert_eq!(before, after);
        assert_eq!(after.0[0].account_id.as_deref(), Some("acc_claude"));
    }

    #[test]
    fn tasks_messages_and_approvals() {
        let store = Store::open_memory().unwrap();
        project(&store);
        let task = Task {
            id: "tk_1".into(),
            project_id: "pr_1".into(),
            parent_task_id: None,
            root_task_id: "tk_1".into(),
            owner_agent_id: "ag_1".into(),
            title: "t".into(),
            brief: String::new(),
            done_criteria: String::new(),
            status: "queued".into(),
            branch: None,
            budget_usd: Some(2.0),
            depends_on: vec!["tk_0".into()],
            summary: None,
            message_count: 0,
            stalled_rounds: 0,
            created_at: now(),
            finished_at: None,
        };
        store.insert_task(&task).unwrap();
        assert_eq!(store.get_task("tk_1").unwrap(), task);
        assert_eq!(store.bump_task_messages("tk_1").unwrap(), 1);
        store.set_task_status("tk_1", "done").unwrap();
        assert!(store.get_task("tk_1").unwrap().finished_at.is_some());
        assert!(store.open_tasks_of("ag_1").unwrap().is_empty());

        let message = Message {
            id: "msg_1".into(),
            task_id: Some("tk_1".into()),
            from_agent: "ag_1".into(),
            to_agent: "ag_2".into(),
            text: "oi".into(),
            created_at: now(),
            delivered_at: None,
        };
        store.insert_message(&message).unwrap();
        assert_eq!(store.pending_messages("ag_2").unwrap().len(), 1);
        store.mark_delivered("msg_1").unwrap();
        assert!(store.pending_messages("ag_2").unwrap().is_empty());
        assert_eq!(store.recent_messages(10).unwrap().len(), 1);

        let approval = Approval {
            id: "ap_1".into(),
            agent_id: "ag_1".into(),
            task_id: Some("tk_1".into()),
            action: "merge".into(),
            detail: "d".into(),
            status: "pending".into(),
            decided_by: None,
            created_at: now(),
            decided_at: None,
        };
        store.insert_approval(&approval).unwrap();
        assert_eq!(store.list_approvals(Some("pending")).unwrap().len(), 1);
        store.decide_approval("ap_1", "approved", "pedro").unwrap();
        assert!(store.list_approvals(Some("pending")).unwrap().is_empty());
        assert_eq!(store.get_approval("ap_1").unwrap().status, "approved");
    }

    #[test]
    fn file_database_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("regente.db");
        {
            let store = Store::open(&path).unwrap();
            store
                .append_event(&event("ag_1", EventKind::Command), None)
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.all_events().unwrap().len(), 1);
    }
}
