//! Model lists per account: run the provider's own list command at use time,
//! with a short timeout and the clean environment of `account_status`.
//! The output formats of `codex debug models` and `agy models` are NOT
//! confirmed by a real run (D23), so the parsers are tolerant.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::model::Account;

/// Reasoning levels accepted as process arguments.
pub const EFFORTS: [&str; 6] = ["low", "medium", "high", "xhigh", "max", "ultra"];
const CACHE_TTL: Duration = Duration::from_secs(300);

/// `Some(level)` only for a value in the allowed list.
pub fn valid_effort(value: &str) -> Option<&'static str> {
    EFFORTS.iter().copied().find(|level| *level == value.trim())
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    pub label: Option<String>,
    pub efforts: Vec<String>,
    pub default_effort: Option<String>,
}

impl ModelInfo {
    fn to_json(&self) -> Value {
        let mut out = json!({ "id": self.id });
        if let Some(label) = &self.label {
            out["label"] = json!(label);
        }
        if !self.efforts.is_empty() {
            out["efforts"] = json!(self.efforts);
        }
        if let Some(level) = &self.default_effort {
            out["default_effort"] = json!(level);
        }
        out
    }
}

fn plain(id: &str) -> ModelInfo {
    ModelInfo {
        id: id.to_string(),
        label: None,
        efforts: Vec::new(),
        default_effort: None,
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/'))
}

fn efforts_of(object: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for key in [
        "supported_reasoning_levels",
        "supported_reasoning_efforts",
        "reasoning_levels",
        "efforts",
    ] {
        for item in object
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let name = item
                .as_str()
                .or_else(|| item.get("effort").and_then(Value::as_str))
                .or_else(|| item.get("id").and_then(Value::as_str));
            if let Some(level) = name.and_then(valid_effort) {
                if !out.iter().any(|known| known == level) {
                    out.push(level.to_string());
                }
            }
        }
    }
    out
}

fn collect(value: &Value, out: &mut Vec<ModelInfo>) {
    match value {
        Value::Array(items) => {
            for item in items {
                if let Some(id) = item.as_object().and_then(|_| {
                    ["slug", "id", "model"]
                        .iter()
                        .find_map(|key| item.get(*key).and_then(Value::as_str))
                }) {
                    // Confirmed 15/11/2026: hidden models carry `visibility: "hide"`.
                    let visible = item
                        .get("visibility")
                        .and_then(Value::as_str)
                        .is_none_or(|v| v == "list");
                    if visible && valid_id(id) && !out.iter().any(|known| known.id == id) {
                        out.push(ModelInfo {
                            id: id.to_string(),
                            label: item
                                .get("display_name")
                                .and_then(Value::as_str)
                                .filter(|l| !l.is_empty() && l.len() <= 80)
                                .map(str::to_string),
                            efforts: efforts_of(item),
                            default_effort: item
                                .get("default_reasoning_level")
                                .and_then(Value::as_str)
                                .and_then(valid_effort)
                                .map(str::to_string),
                        });
                    }
                } else if item.is_array() || item.is_object() {
                    collect(item, out);
                } else if let Some(id) = item.as_str().filter(|id| valid_id(id)) {
                    if !out.iter().any(|known| known.id == id) {
                        out.push(plain(id));
                    }
                }
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                if item.is_array() || item.is_object() {
                    collect(item, out);
                }
            }
        }
        _ => {}
    }
}

/// Codex: JSON; collect arrays of objects with `slug`/`id`/`model`.
pub fn parse_codex(text: &str) -> Vec<ModelInfo> {
    let mut out = Vec::new();
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        collect(&value, &mut out);
    }
    out
}

/// agy: `slug<TAB>Display name` per line (confirmed 15/11/2026, agy 1.5.9);
/// lines without a TAB are ignored. JSON is tolerated too. The reasoning level
/// is part of the slug, so there are no separate efforts.
pub fn parse_agy(text: &str) -> Vec<ModelInfo> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        return parse_codex(text);
    }
    let mut out: Vec<ModelInfo> = Vec::new();
    for line in text.lines() {
        let Some((slug, name)) = line.split_once('\t') else {
            continue;
        };
        let slug = slug.trim();
        let name = name.trim();
        if !valid_id(slug) || out.iter().any(|known| known.id == slug) {
            continue;
        }
        let mut info = plain(slug);
        if !name.is_empty() && name.len() <= 80 {
            info.label = Some(name.to_string());
        }
        out.push(info);
    }
    out
}

fn fixed(ids: &[&str]) -> Vec<ModelInfo> {
    ids.iter().map(|id| plain(id)).collect()
}

fn cache() -> &'static Mutex<HashMap<String, (Instant, Value)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, Value)>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

async fn run_list(account: &Account, args: &[&str]) -> Result<String, String> {
    let program = if account.cli_path.is_empty() {
        account.provider.as_str()
    } else {
        account.cli_path.as_str()
    };
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(crate::security::clean_env(std::env::vars(), &[]))
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = match tokio::time::timeout(Duration::from_secs(8), command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(format!("Não foi possível executar o CLI: {error}")),
        Err(_) => return Err("O CLI demorou demais para listar os modelos.".to_string()),
    };
    if !output.status.success() {
        return Err("O CLI não conseguiu listar os modelos.".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `{"models": [{"id", "efforts"?}], "detail"?}`; failure gives an empty list and a detail.
pub async fn models_for(account: &Account) -> Value {
    if let Some((at, value)) = cache()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&account.id)
    {
        if at.elapsed() < CACHE_TTL {
            return value.clone();
        }
    }
    let result: Result<Vec<ModelInfo>, String> = match account.provider.as_str() {
        // Aliases listed in `claude --help`; the CLI has no list command.
        "claude" => Ok(fixed(&["opus", "sonnet", "haiku", "fable"])),
        "fake" => Ok(fixed(&["fake-fast", "fake-deep"])),
        "codex" => run_list(account, &["debug", "models"])
            .await
            .map(|text| parse_codex(&text)),
        "agy" => run_list(account, &["models"])
            .await
            .map(|text| parse_agy(&text)),
        _ => Err("Provedor sem lista de modelos.".to_string()),
    };
    let (value, cacheable) = match result {
        Ok(models) if !models.is_empty() => (
            json!({ "models": models.iter().map(ModelInfo::to_json).collect::<Vec<_>>() }),
            true,
        ),
        Ok(_) => (
            json!({ "models": [], "detail": "O CLI não devolveu modelos reconhecíveis. Use \"Outro…\"." }),
            false,
        ),
        Err(detail) => (json!({ "models": [], "detail": detail }), false),
    };
    if cacheable {
        cache()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(account.id.clone(), (Instant::now(), value.clone()));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::fixtures_dir;

    #[test]
    fn codex_json_shows_only_listed_models() {
        let text = std::fs::read_to_string(
            fixtures_dir().join("models/codex-debug-models.synthetic.json"),
        )
        .unwrap();
        let models = parse_codex(&text);
        let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["gpt-synth-1", "gpt-synth-mini"]);
        assert_eq!(models[0].label.as_deref(), Some("Synth 1"));
        assert_eq!(models[0].efforts, ["low", "medium", "high", "ultra"]);
        assert_eq!(models[0].default_effort.as_deref(), Some("low"));
        assert!(parse_codex("not json").is_empty());
        assert_eq!(parse_codex(r#"["a-b","c d"]"#), vec![plain("a-b")]);
    }

    #[test]
    fn agy_lines_use_tab_and_skip_the_rest() {
        let text = std::fs::read_to_string(fixtures_dir().join("models/agy-models.synthetic.txt"))
            .unwrap();
        let models = parse_agy(&text);
        let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["gemini-synth-pro-high", "claude-synth-medium"]);
        assert_eq!(models[0].label.as_deref(), Some("Gemini Synth Pro (High)"));
        assert_eq!(parse_agy(r#"{"models":[{"id":"x1"}]}"#), vec![plain("x1")]);
    }

    #[test]
    fn effort_is_checked_against_the_allowed_list() {
        assert_eq!(valid_effort("xhigh"), Some("xhigh"));
        assert_eq!(valid_effort(" high "), Some("high"));
        assert_eq!(valid_effort("high; rm -rf"), None);
        assert_eq!(valid_effort("ultra"), Some("ultra"));
        assert_eq!(valid_effort(""), None);
    }
}
