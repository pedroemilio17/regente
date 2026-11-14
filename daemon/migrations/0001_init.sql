-- Regente schema v1. `events` is the append-only source of truth;
-- `usage` and `file_changes` are derived from it and can be rebuilt.

CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  root TEXT NOT NULL,
  base_branch TEXT NOT NULL DEFAULT 'main',
  context TEXT NOT NULL DEFAULT '',
  folders TEXT NOT NULL DEFAULT '[]',
  blocked TEXT NOT NULL DEFAULT '[]',
  day_budget_usd REAL,
  created_at TEXT NOT NULL
);

CREATE TABLE accounts (
  id TEXT PRIMARY KEY,
  provider TEXT NOT NULL,
  login_type TEXT NOT NULL DEFAULT 'subscription',
  cli_path TEXT NOT NULL DEFAULT '',
  default_model TEXT NOT NULL DEFAULT '',
  limit_rule TEXT NOT NULL DEFAULT '{}',
  max_concurrent INTEGER NOT NULL DEFAULT 2
);

CREATE TABLE agents (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id),
  name TEXT NOT NULL,
  cli TEXT NOT NULL,
  role TEXT NOT NULL DEFAULT '',
  is_orchestrator INTEGER NOT NULL DEFAULT 0,
  parent_id TEXT,
  account_id TEXT,
  model TEXT NOT NULL DEFAULT '',
  pet TEXT NOT NULL DEFAULT '{}',
  permissions TEXT NOT NULL DEFAULT '{}',
  settings TEXT NOT NULL DEFAULT '{}',
  budget_usd REAL,
  paused INTEGER NOT NULL DEFAULT 0,
  pause_reason TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE tasks (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id),
  parent_task_id TEXT,
  root_task_id TEXT NOT NULL,
  owner_agent_id TEXT NOT NULL,
  title TEXT NOT NULL,
  brief TEXT NOT NULL DEFAULT '',
  done_criteria TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'queued',
  branch TEXT,
  budget_usd REAL,
  depends_on TEXT NOT NULL DEFAULT '[]',
  summary TEXT,
  message_count INTEGER NOT NULL DEFAULT 0,
  stalled_rounds INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  finished_at TEXT
);

CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL,
  task_id TEXT NOT NULL,
  cli_session_id TEXT,
  worktree TEXT,
  started_at TEXT NOT NULL,
  ended_at TEXT
);

CREATE TABLE events (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  id TEXT NOT NULL UNIQUE,
  ts TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  task_id TEXT,
  kind TEXT NOT NULL,
  target TEXT,
  summary TEXT NOT NULL DEFAULT '',
  payload TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE messages (
  id TEXT PRIMARY KEY,
  task_id TEXT,
  from_agent TEXT NOT NULL,
  to_agent TEXT NOT NULL,
  text TEXT NOT NULL,
  created_at TEXT NOT NULL,
  delivered_at TEXT
);

CREATE TABLE usage (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  event_id TEXT NOT NULL,
  ts TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  account_id TEXT,
  task_id TEXT,
  tokens_in INTEGER NOT NULL DEFAULT 0,
  tokens_out INTEGER NOT NULL DEFAULT 0,
  tokens_cache INTEGER NOT NULL DEFAULT 0,
  cost_usd REAL NOT NULL DEFAULT 0,
  cost_source TEXT NOT NULL DEFAULT 'estimated'
);

CREATE TABLE file_changes (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  event_id TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  ts TEXT NOT NULL,
  path TEXT NOT NULL,
  lines_added INTEGER NOT NULL DEFAULT 0,
  lines_removed INTEGER NOT NULL DEFAULT 0,
  diff_ref TEXT
);

CREATE TABLE approvals (
  id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL,
  task_id TEXT,
  action TEXT NOT NULL,
  detail TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'pending',
  decided_by TEXT,
  created_at TEXT NOT NULL,
  decided_at TEXT
);

CREATE TABLE usage_windows (
  account_id TEXT NOT NULL,
  window TEXT NOT NULL,
  start TEXT NOT NULL,
  tokens INTEGER NOT NULL DEFAULT 0,
  cost REAL NOT NULL DEFAULT 0,
  pct REAL,
  reported_pct REAL,
  resets_at TEXT,
  PRIMARY KEY (account_id, window)
);

CREATE INDEX idx_events_agent ON events(agent_id, seq);
CREATE INDEX idx_events_task ON events(task_id, seq);
CREATE INDEX idx_usage_account ON usage(account_id, ts);
CREATE INDEX idx_usage_task ON usage(task_id);
CREATE INDEX idx_tasks_project ON tasks(project_id, status);
CREATE INDEX idx_messages_to ON messages(to_agent, delivered_at);
