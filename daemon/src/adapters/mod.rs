//! CLI adapters: each one turns the structured output of a CLI into
//! normalized events. One process per turn; sessions are resumed by id.

pub mod agy;
pub mod claude;
pub mod codex;
pub mod fake;
pub mod process;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::app::{App, AppResult};
use crate::model::{Account, Agent, EventKind, NewEvent, Project, Task, Usage};
use crate::worktree;

/// What a parser extracts from one line of CLI output.
#[derive(Clone, Debug, PartialEq)]
pub enum Parsed {
    Event(NewEvent),
    /// The CLI's own session id, used to resume the conversation next turn.
    Session(String),
    /// Token usage of the turn (cost filled in when the CLI reports it).
    Usage(Usage),
    /// Plan utilization reported by the CLI (0.0–1.0) for a window.
    RateLimit {
        window: &'static str,
        pct: f64,
        resets_at: Option<String>,
    },
    /// The turn finished; carries the final answer.
    Done(String),
    Failed(String),
}

pub trait LineParser: Send {
    fn parse_line(&mut self, line: &str) -> Vec<Parsed>;
}

/// Parser for the output format of a CLI.
pub fn parser_for(cli: &str, agent_id: &str) -> Option<Box<dyn LineParser>> {
    match cli {
        "claude" => Some(Box::new(claude::ClaudeParser::new(agent_id))),
        "codex" => Some(Box::new(codex::CodexParser::new(agent_id))),
        "agy" => Some(Box::new(agy::AgyParser::new(agent_id))),
        _ => None,
    }
}

/// Everything needed to run one turn of an agent.
#[derive(Clone, Debug)]
pub struct TurnCtx {
    pub agent: Agent,
    pub project: Project,
    pub task: Task,
    pub account: Option<Account>,
    pub session_id: String,
    pub prompt: String,
    pub cwd: PathBuf,
    /// True when `cwd` is a git worktree.
    pub git: bool,
    /// CLI session id of the previous turn on this task, if any.
    pub resume: Option<String>,
    pub mcp_url: String,
    /// How many turns this agent already had on this task.
    pub turn_index: usize,
    /// Money left in the task budget, when there is one.
    pub budget_left: Option<f64>,
}

impl TurnCtx {
    pub fn model(&self) -> String {
        if !self.agent.model.is_empty() {
            self.agent.model.clone()
        } else {
            self.account
                .as_ref()
                .map(|account| account.default_model.clone())
                .unwrap_or_default()
        }
    }

    pub fn program(&self) -> String {
        match self
            .account
            .as_ref()
            .map(|account| account.cli_path.as_str())
            .filter(|path| !path.is_empty())
        {
            Some(path)
                if self
                    .account
                    .as_ref()
                    .is_some_and(|account| account.provider == self.agent.cli) =>
            {
                path.to_string()
            }
            _ => self.agent.cli.clone(),
        }
    }

    pub fn blocked(&self) -> Vec<String> {
        let mut patterns = self.project.blocked_patterns();
        if let Some(extra) = self
            .agent
            .permissions
            .get("blocked")
            .and_then(Value::as_array)
        {
            patterns.extend(extra.iter().filter_map(|v| v.as_str().map(str::to_string)));
        }
        if patterns.is_empty() {
            patterns = crate::security::DEFAULT_BLOCKED
                .iter()
                .map(|p| p.to_string())
                .collect();
        }
        patterns
    }
}

/// Program, arguments and extra environment of a CLI turn.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Secret-looking variables the CLI needs for its own login.
    pub keep_env: Vec<&'static str>,
}

pub fn command_for(ctx: &TurnCtx) -> Option<CommandSpec> {
    match ctx.agent.cli.as_str() {
        "claude" => Some(claude::command(ctx)),
        "codex" => Some(codex::command(ctx)),
        "agy" => Some(agy::command(ctx)),
        _ => None,
    }
}

/// Result of a turn, whatever the adapter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TurnOutcome {
    pub done: Option<String>,
    pub failed: Option<String>,
    pub cancelled: bool,
}

const INLINE_DIFF_LIMIT: usize = 16 * 1024;

/// Keep small diffs inline; large ones go to disk and the event keeps the reference.
pub fn attach_diff(app: &App, event: NewEvent, diff: String) -> NewEvent {
    if diff.len() <= INLINE_DIFF_LIMIT {
        return event.with("diff", json!(diff));
    }
    let dir = app.cfg.diffs_dir();
    let path = dir.join(format!("{}.diff", crate::model::new_id("diff")));
    match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &diff)) {
        Ok(()) => event.with("diff_ref", json!(path.to_string_lossy())),
        Err(_) => event.with(
            "diff",
            json!(diff.chars().take(INLINE_DIFF_LIMIT).collect::<String>()),
        ),
    }
}

fn relative(target: &str, cwd: &Path) -> String {
    Path::new(target)
        .strip_prefix(cwd)
        .map(|rel| rel.to_string_lossy().into_owned())
        .unwrap_or_else(|_| target.to_string())
}

/// Apply one parsed item: fill in task, paths, line counts and cost, then emit.
pub async fn handle_parsed(
    app: &Arc<App>,
    ctx: &TurnCtx,
    parsed: Parsed,
    raw_ref: Option<&str>,
    outcome: &mut TurnOutcome,
) -> AppResult<()> {
    match parsed {
        Parsed::Event(mut event) => {
            event.task_id = Some(ctx.task.id.clone());
            event.raw_ref = raw_ref.map(str::to_string);
            let cwd_text = ctx.cwd.to_string_lossy().into_owned();
            if let Some(target) = event.target.clone() {
                let shown = relative(&target, &ctx.cwd);
                event.summary = event
                    .summary
                    .replace(&target, &shown)
                    .replace(&format!("{cwd_text}/"), "");
                event.target = Some(shown);
            }
            if event.kind == EventKind::FileEdit && ctx.git {
                if let Some(file) = event.target.clone() {
                    if event.lines_added.is_none() {
                        if let Some((added, removed)) = worktree::numstat(&ctx.cwd, &file).await {
                            event = event.lines(added, removed);
                        }
                    }
                    let has_diff = event.data.as_ref().is_some_and(|d| d.get("diff").is_some());
                    if !has_diff {
                        if let Some(diff) = worktree::diff(&ctx.cwd, &file).await {
                            event = attach_diff(app, event, diff);
                        }
                    }
                }
            }
            if let Some(usage) = event.usage.as_mut() {
                price(app, ctx, usage);
            }
            app.emit(event)?;
        }
        Parsed::Session(cli_session_id) => app
            .store
            .set_cli_session(&ctx.session_id, &cli_session_id)?,
        Parsed::Usage(mut usage) => {
            price(app, ctx, &mut usage);
            let summary = format!(
                "Uso do turno: {} tokens",
                usage.tokens_in + usage.tokens_out
            );
            app.emit(
                NewEvent::status(&ctx.agent.id, "working", summary)
                    .task(Some(&ctx.task.id))
                    .usage(usage),
            )?;
        }
        Parsed::RateLimit {
            window,
            pct,
            resets_at,
        } => {
            if let Some(account) = &ctx.account {
                app.store
                    .save_reported_pct(&account.id, window, pct, resets_at.as_deref())?;
            }
        }
        Parsed::Done(summary) => outcome.done = Some(summary),
        // The orchestrator records the error event when it closes the turn.
        Parsed::Failed(reason) => outcome.failed = Some(reason),
    }
    Ok(())
}

/// Estimate the cost when the CLI did not report one.
fn price(app: &App, ctx: &TurnCtx, usage: &mut Usage) {
    if usage.cost_source != "reported" {
        let prices = app.prices.read().unwrap_or_else(|p| p.into_inner());
        usage.cost_usd = prices.estimate(&ctx.agent.cli, &ctx.model(), usage);
        usage.cost_source = "estimated".to_string();
    }
}

/// First non-empty line of a text, shortened.
pub(crate) fn first_line(text: &str, fallback: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| crate::model::truncate(line, 160))
        .unwrap_or_else(|| fallback.to_string())
}

/// Keep at most `max` characters of a tool output for the timeline.
pub(crate) fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let head: String = text.chars().take(max).collect();
        format!("{head}\n… (saída cortada)")
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::app::testing::fixtures_dir;

    /// Parse a recorded fixture with the adapter of a CLI.
    pub fn parse_fixture(cli: &str, file: &str) -> Vec<Parsed> {
        let text = std::fs::read_to_string(fixtures_dir().join(file)).unwrap();
        let mut parser = parser_for(cli, "ag_test").unwrap();
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .flat_map(|line| parser.parse_line(line))
            .collect()
    }

    pub fn events(parsed: &[Parsed]) -> Vec<&NewEvent> {
        parsed
            .iter()
            .filter_map(|item| {
                if let Parsed::Event(event) = item {
                    Some(event)
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn kinds(parsed: &[Parsed]) -> Vec<EventKind> {
        events(parsed).iter().map(|event| event.kind).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_and_clipping() {
        assert_eq!(
            relative("/work/repo/src/a.ts", Path::new("/work/repo")),
            "src/a.ts"
        );
        assert_eq!(relative("src/a.ts", Path::new("/work/repo")), "src/a.ts");
        assert_eq!(first_line("\n  hello \nworld", "x"), "hello");
        assert_eq!(first_line("", "(sem saída)"), "(sem saída)");
        assert!(clip(&"a".repeat(50), 10).ends_with("(saída cortada)"));
        assert!(parser_for("fake", "ag").is_none());
    }
}
