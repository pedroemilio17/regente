//! Usage windows (5h, day, week) per account and budget enforcement.
//!
//! The daemon cuts spending, not the prompt: at 80% of a budget it warns,
//! at 100% it pauses the agent.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{json, Value};

use crate::app::{App, AppResult};
use crate::model::{Event, NewEvent, UsageRow};

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct WindowTotal {
    pub start: Option<String>,
    pub tokens: i64,
    pub cost_usd: f64,
}

fn parse_ts(ts: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn fmt_ts(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Start of the UTC calendar day of `now`, as stored in the database.
pub fn day_start(now: DateTime<Utc>) -> String {
    let midnight = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|naive| naive.and_utc())
        .unwrap_or(now);
    fmt_ts(midnight)
}

/// Provider-style window: it opens at the first usage after the previous
/// window ended and lasts `length`. An expired window counts as empty.
pub fn anchored(rows: &[UsageRow], length: Duration, now: DateTime<Utc>) -> WindowTotal {
    let mut start: Option<DateTime<Utc>> = None;
    let mut total = WindowTotal::default();
    for row in rows {
        let Some(ts) = parse_ts(&row.ts) else {
            continue;
        };
        if ts > now {
            break;
        }
        let rolled = match start {
            Some(open) => ts >= open + length,
            None => true,
        };
        if rolled {
            start = Some(ts);
            total = WindowTotal::default();
        }
        total.tokens += row.tokens_in + row.tokens_out;
        total.cost_usd += row.cost_usd;
    }
    match start {
        Some(open) if now < open + length => WindowTotal {
            start: Some(fmt_ts(open)),
            ..total
        },
        _ => WindowTotal::default(),
    }
}

/// Usage since the start of the UTC calendar day.
pub fn calendar_day(rows: &[UsageRow], now: DateTime<Utc>) -> WindowTotal {
    let start = day_start(now);
    let mut total = WindowTotal {
        start: Some(start.clone()),
        ..Default::default()
    };
    for row in rows
        .iter()
        .filter(|row| row.ts >= start && parse_ts(&row.ts).is_some_and(|ts| ts <= now))
    {
        total.tokens += row.tokens_in + row.tokens_out;
        total.cost_usd += row.cost_usd;
    }
    total
}

/// The three windows of one account (or agent), keyed `5h`, `day`, `week`.
pub fn windows(rows: &[UsageRow], now: DateTime<Utc>) -> BTreeMap<&'static str, WindowTotal> {
    BTreeMap::from([
        ("5h", anchored(rows, Duration::hours(5), now)),
        ("day", calendar_day(rows, now)),
        ("week", anchored(rows, Duration::days(7), now)),
    ])
}

/// Codex reports its plan usage only in its session files (D23).
fn refresh_codex(app: &App, account_id: &str) -> AppResult<()> {
    let Some(home) = std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".codex")))
    else {
        return Ok(());
    };
    for item in crate::adapters::codex::latest_rate_limits(&home) {
        if let crate::adapters::Parsed::RateLimit {
            window,
            pct,
            resets_at,
        } = item
        {
            app.store
                .save_reported_pct(account_id, window, pct, resets_at.as_deref())?;
        }
    }
    Ok(())
}

/// Usage per account and per agent, for the API and the sidebar card.
pub fn report(app: &App) -> AppResult<Value> {
    report_at(app, Utc::now())
}

pub fn report_at(app: &App, now: DateTime<Utc>) -> AppResult<Value> {
    let since = fmt_ts(now - Duration::days(15));
    let rows = app.store.usage_since(&since)?;
    let mut accounts = Vec::new();
    for account in app.store.list_accounts()? {
        if account.provider == "codex" {
            refresh_codex(app, &account.id)?;
        }
        let own: Vec<UsageRow> = rows
            .iter()
            .filter(|row| row.account_id.as_deref() == Some(&account.id))
            .cloned()
            .collect();
        let mut out = serde_json::Map::new();
        for (name, total) in windows(&own, now) {
            let reported = app
                .store
                .reported_pct(&account.id, name)?
                .filter(|(_, resets)| {
                    resets
                        .as_deref()
                        .and_then(parse_ts)
                        .is_none_or(|at| at > now)
                });
            // Only what the CLI reported (D23): no spend/limit estimate in dollars.
            let (pct, resets_at) = match reported {
                Some((pct, resets)) => (Some(pct), resets),
                None => (None, None),
            };
            let source = if pct.is_some() { "reported" } else { "none" };
            app.store.save_usage_window(
                &account.id,
                name,
                total.start.as_deref().unwrap_or(""),
                total.tokens,
                total.cost_usd,
                pct,
            )?;
            out.insert(
                name.to_string(),
                json!({
                    "start": total.start,
                    "tokens": total.tokens,
                    "cost_usd": total.cost_usd,
                    "pct": pct,
                    "pct_source": source,
                    "resets_at": resets_at,
                }),
            );
        }
        accounts.push(
            json!({ "account_id": account.id, "provider": account.provider, "windows": out }),
        );
    }
    let mut agents = Vec::new();
    for agent in app.store.list_agents(None)? {
        let own: Vec<UsageRow> = rows
            .iter()
            .filter(|row| row.agent_id == agent.id)
            .cloned()
            .collect();
        agents.push(json!({
            "agent_id": agent.id,
            "windows": windows(&own, now),
            "budget_usd": agent.budget_usd,
        }));
    }
    Ok(json!({ "generated_at": fmt_ts(now), "accounts": accounts, "agents": agents }))
}

/// Share of the project's daily budget already spent, when one is set.
pub fn project_day_ratio(app: &App, project_id: &str) -> AppResult<Option<f64>> {
    let project = app.store.get_project(project_id)?;
    let Some(budget) = project.day_budget_usd.filter(|b| *b > 0.0) else {
        return Ok(None);
    };
    let spent = app
        .store
        .cost_of_project_since(project_id, &day_start(Utc::now()))?;
    Ok(Some(spent / budget))
}

fn money(value: f64) -> String {
    format!("US$ {value:.2}")
}

/// Check every budget that applies to the agent after a usage event.
pub fn after_usage(app: &App, event: &Event) -> AppResult<()> {
    let agent = app.store.get_agent(&event.agent_id)?;
    let today = day_start(Utc::now());
    let mut scopes: Vec<(String, &str, f64, f64)> = Vec::new();
    if let Some(task_id) = &event.task_id {
        if let Ok(task) = app.store.get_task(task_id) {
            if let Ok(root) = app.store.get_task(&task.root_task_id) {
                if let Some(budget) = root.budget_usd.filter(|b| *b > 0.0) {
                    scopes.push((
                        format!("task:{}", root.id),
                        "da tarefa",
                        app.store.cost_of_task_tree(&root.id)?,
                        budget,
                    ));
                }
            }
        }
    }
    if let Some(budget) = agent.budget_usd.filter(|b| *b > 0.0) {
        scopes.push((
            format!("agent:{}", agent.id),
            "do agente",
            app.store.cost_of_agent_since(&agent.id, &today)?,
            budget,
        ));
    }
    if let Ok(project) = app.store.get_project(&agent.project_id) {
        if let Some(budget) = project.day_budget_usd.filter(|b| *b > 0.0) {
            scopes.push((
                format!("day:{}:{today}", project.id),
                "do dia",
                app.store.cost_of_project_since(&project.id, &today)?,
                budget,
            ));
        }
    }
    for (key, label, spent, budget) in scopes {
        let ratio = spent / budget;
        if ratio >= 1.0 {
            app.pause_agent(
                &agent.id,
                &format!(
                    "Orçamento {label} esgotado ({} de {})",
                    money(spent),
                    money(budget)
                ),
            )?;
            return Ok(());
        }
        if ratio >= 0.8 {
            let first = app
                .warned
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(format!("{key}:{}", agent.id));
            if first {
                let status = app.live_of(&agent.id).status;
                app.emit(
                    NewEvent::status(
                        &agent.id,
                        &status,
                        format!("Orçamento {label} em {:.0}%", ratio * 100.0),
                    )
                    .task(event.task_id.as_deref())
                    .with(
                        "budget_warning",
                        json!({ "scope": key, "spent_usd": spent, "budget_usd": budget }),
                    ),
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::{agent, app, project};
    use crate::model::{now, EventKind, Task, Usage};

    fn row(ts: &str, cost: f64) -> UsageRow {
        UsageRow {
            ts: ts.into(),
            agent_id: "ag".into(),
            account_id: Some("acc_claude".into()),
            task_id: None,
            tokens_in: 100,
            tokens_out: 50,
            tokens_cache: 0,
            cost_usd: cost,
        }
    }

    fn at(ts: &str) -> DateTime<Utc> {
        parse_ts(ts).unwrap()
    }

    #[test]
    fn five_hour_window_opens_at_first_usage() {
        let rows = vec![
            row("2026-11-14T10:00:00.000Z", 1.0),
            row("2026-11-14T12:30:00.000Z", 2.0),
        ];
        let total = anchored(&rows, Duration::hours(5), at("2026-11-14T14:00:00Z"));
        assert_eq!(total.start.as_deref(), Some("2026-11-14T10:00:00.000Z"));
        assert_eq!(total.cost_usd, 3.0);
        assert_eq!(total.tokens, 300);
    }

    #[test]
    fn five_hour_window_rolls_over() {
        let rows = vec![
            row("2026-11-14T10:00:00.000Z", 1.0),
            row("2026-11-14T14:59:00.000Z", 1.0),
            // 15:00 is exactly start + 5h: a new window opens here.
            row("2026-11-14T15:00:00.000Z", 4.0),
            row("2026-11-14T16:00:00.000Z", 0.5),
        ];
        let total = anchored(&rows, Duration::hours(5), at("2026-11-14T16:30:00Z"));
        assert_eq!(total.start.as_deref(), Some("2026-11-14T15:00:00.000Z"));
        assert_eq!(total.cost_usd, 4.5);
    }

    #[test]
    fn expired_window_is_empty() {
        let rows = vec![row("2026-11-14T10:00:00.000Z", 1.0)];
        let total = anchored(&rows, Duration::hours(5), at("2026-11-14T15:00:00Z"));
        assert_eq!(total, WindowTotal::default());
        let still_open = anchored(&rows, Duration::hours(5), at("2026-11-14T14:59:59Z"));
        assert_eq!(still_open.cost_usd, 1.0);
    }

    #[test]
    fn day_window_resets_at_midnight_utc() {
        let rows = vec![
            row("2026-11-13T23:59:59.000Z", 1.0),
            row("2026-11-14T00:00:00.000Z", 2.0),
            row("2026-11-14T09:00:00.000Z", 3.0),
        ];
        let total = calendar_day(&rows, at("2026-11-14T10:00:00Z"));
        assert_eq!(total.cost_usd, 5.0);
        assert_eq!(total.start.as_deref(), Some("2026-11-14T00:00:00.000Z"));
        let next_day = calendar_day(&rows, at("2026-11-15T00:00:01Z"));
        assert_eq!(next_day.cost_usd, 0.0);
    }

    #[test]
    fn week_window_spans_seven_days() {
        let rows = vec![
            row("2026-11-07T10:00:00.000Z", 1.0),
            row("2026-11-13T10:00:00.000Z", 2.0),
            row("2026-11-14T10:00:00.000Z", 4.0),
        ];
        let all = windows(&rows, at("2026-11-14T09:00:00Z"));
        assert_eq!(all["week"].cost_usd, 3.0);
        let rolled = windows(&rows, at("2026-11-14T11:00:00Z"));
        assert_eq!(rolled["week"].cost_usd, 4.0);
        assert_eq!(
            rolled["week"].start.as_deref(),
            Some("2026-11-14T10:00:00.000Z")
        );
    }

    fn usage(cost: f64) -> Usage {
        Usage {
            tokens_in: 1000,
            tokens_out: 100,
            tokens_cache: 0,
            cost_usd: cost,
            cost_source: "reported".into(),
        }
    }

    fn task(app: &App, project_id: &str, agent_id: &str, budget: Option<f64>) -> Task {
        let task = Task {
            id: "tk_1".into(),
            project_id: project_id.into(),
            parent_task_id: None,
            root_task_id: "tk_1".into(),
            owner_agent_id: agent_id.into(),
            title: "t".into(),
            brief: String::new(),
            done_criteria: String::new(),
            status: "running".into(),
            branch: None,
            budget_usd: budget,
            depends_on: vec![],
            summary: None,
            message_count: 0,
            stalled_rounds: 0,
            created_at: now(),
            finished_at: None,
        };
        app.store.insert_task(&task).unwrap();
        task
    }

    #[tokio::test]
    async fn task_budget_warns_at_80_and_pauses_at_100() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let project = project(&app, dir.path());
        let agent = agent(&app, &project, "Backend", "basic", false);
        let task = task(&app, &project.id, &agent.id, Some(1.0));
        let spend = |cost: f64| {
            app.emit(
                NewEvent::new(&agent.id, EventKind::Thinking, "x")
                    .task(Some(&task.id))
                    .usage(usage(cost)),
            )
            .unwrap();
        };
        spend(0.5);
        assert!(!app.store.get_agent(&agent.id).unwrap().paused);
        spend(0.35);
        let events = app.store.all_events().unwrap();
        let warnings = events
            .iter()
            .filter(|e| {
                e.data
                    .as_ref()
                    .is_some_and(|d| d.get("budget_warning").is_some())
            })
            .count();
        assert_eq!(warnings, 1);
        spend(0.01);
        let events = app.store.all_events().unwrap();
        let warnings = events
            .iter()
            .filter(|e| {
                e.data
                    .as_ref()
                    .is_some_and(|d| d.get("budget_warning").is_some())
            })
            .count();
        assert_eq!(warnings, 1, "warns only once");
        spend(0.2);
        let paused = app.store.get_agent(&agent.id).unwrap();
        assert!(paused.paused);
        assert!(paused
            .pause_reason
            .unwrap()
            .contains("Orçamento da tarefa esgotado"));
        assert_eq!(app.store.get_task(&task.id).unwrap().status, "blocked");
        assert_eq!(app.live_of(&agent.id).status, "paused");
    }

    #[tokio::test]
    async fn agent_and_day_budgets_pause_too() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let project = project(&app, dir.path());
        let mut agent = agent(&app, &project, "Backend", "basic", false);
        agent.budget_usd = Some(0.5);
        app.store.update_agent(&agent).unwrap();
        app.emit(NewEvent::new(&agent.id, EventKind::Thinking, "x").usage(usage(0.6)))
            .unwrap();
        let paused = app.store.get_agent(&agent.id).unwrap();
        assert!(paused.pause_reason.unwrap().contains("do agente"));
        assert_eq!(project_day_ratio(&app, &project.id).unwrap(), None);
    }

    #[tokio::test]
    async fn report_prefers_reported_utilization() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let project = project(&app, dir.path());
        let agent = agent(&app, &project, "Backend", "basic", false);
        app.emit(NewEvent::new(&agent.id, EventKind::Thinking, "x").usage(usage(0.25)))
            .unwrap();
        let report = super::report(&app).unwrap();
        let claude = report["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["account_id"] == "acc_claude")
            .unwrap();
        assert_eq!(claude["windows"]["5h"]["pct"], Value::Null);
        assert_eq!(claude["windows"]["5h"]["pct_source"], "none");
        assert_eq!(claude["windows"]["day"]["cost_usd"], 0.25);
        app.store
            .save_reported_pct("acc_claude", "5h", 0.43, None)
            .unwrap();
        let report = super::report(&app).unwrap();
        let claude = report["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["account_id"] == "acc_claude")
            .unwrap();
        assert_eq!(claude["windows"]["5h"]["pct"], 0.43);
        assert_eq!(claude["windows"]["5h"]["pct_source"], "reported");
        assert_eq!(report["agents"][0]["windows"]["5h"]["cost_usd"], 0.25);
    }
}
