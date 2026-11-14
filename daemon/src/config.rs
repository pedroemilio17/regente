//! Daemon configuration (`~/.config/regente/`) and the editable price table.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::Usage;

#[derive(Clone, Debug)]
pub struct Config {
    /// Data and config directory (default `~/.config/regente`).
    pub home: PathBuf,
    pub port: u16,
    /// Where the fake adapter finds its scenarios and recorded CLI output.
    pub fixtures_dir: PathBuf,
    /// Built UI (`ui/dist`) served at `/`, when present.
    pub ui_dir: Option<PathBuf>,
    /// UI updates are grouped in batches of this many milliseconds.
    pub batch_ms: u64,
    pub turn_timeout_secs: u64,
    /// Messages allowed per task tree before the daemon cuts the conversation.
    pub message_limit: i64,
    /// Turns without progress before an agent is paused.
    pub stall_limit: i64,
    /// Delay between steps of the fake adapter.
    pub fake_step_ms: u64,
}

#[derive(Default, Deserialize)]
struct ConfigFile {
    port: Option<u16>,
    batch_ms: Option<u64>,
    turn_timeout_secs: Option<u64>,
    message_limit: Option<i64>,
    stall_limit: Option<i64>,
    fake_step_ms: Option<u64>,
    fixtures_dir: Option<PathBuf>,
    ui_dir: Option<PathBuf>,
}

fn env_parse<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
}

impl Config {
    /// Load `config.json` from the home directory, then apply `REGENTE_*` overrides.
    pub fn from_env() -> Self {
        let home = std::env::var_os("REGENTE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let base = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."));
                base.join(".config").join("regente")
            });
        let file: ConfigFile = std::fs::read_to_string(home.join("config.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let ui_dir = std::env::var_os("REGENTE_UI_DIR")
            .map(PathBuf::from)
            .or(file.ui_dir)
            .or_else(|| {
                let candidate = PathBuf::from("ui/dist");
                candidate.is_dir().then_some(candidate)
            });
        Self {
            port: env_parse("REGENTE_PORT").or(file.port).unwrap_or(7717),
            fixtures_dir: std::env::var_os("REGENTE_FIXTURES")
                .map(PathBuf::from)
                .or(file.fixtures_dir)
                .unwrap_or_else(|| PathBuf::from("fixtures")),
            ui_dir,
            batch_ms: env_parse("REGENTE_BATCH_MS")
                .or(file.batch_ms)
                .unwrap_or(250),
            turn_timeout_secs: env_parse("REGENTE_TURN_TIMEOUT")
                .or(file.turn_timeout_secs)
                .unwrap_or(900),
            message_limit: env_parse("REGENTE_MESSAGE_LIMIT")
                .or(file.message_limit)
                .unwrap_or(20),
            stall_limit: env_parse("REGENTE_STALL_LIMIT")
                .or(file.stall_limit)
                .unwrap_or(3),
            fake_step_ms: env_parse("REGENTE_FAKE_STEP_MS")
                .or(file.fake_step_ms)
                .unwrap_or(400),
            home,
        }
    }

    /// Fast configuration for tests: small delays, isolated home.
    pub fn for_tests(home: &Path, fixtures_dir: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
            port: 0,
            fixtures_dir: fixtures_dir.to_path_buf(),
            ui_dir: None,
            batch_ms: 20,
            turn_timeout_secs: 30,
            message_limit: 20,
            stall_limit: 3,
            fake_step_ms: 1,
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.home.join("regente.db")
    }

    pub fn prices_path(&self) -> PathBuf {
        self.home.join("prices.json")
    }

    /// Raw CLI output, one JSONL file per session.
    pub fn raw_dir(&self) -> PathBuf {
        self.home.join("raw")
    }

    /// Large diffs live on disk; the database keeps only the reference.
    pub fn diffs_dir(&self) -> PathBuf {
        self.home.join("diffs")
    }
}

/// Price per million tokens, in US dollars.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache: f64,
}

/// Editable price table, keyed by `cli` or `cli:model`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prices(pub HashMap<String, Price>);

impl Default for Prices {
    fn default() -> Self {
        let mut map = HashMap::new();
        map.insert(
            "claude".to_string(),
            Price {
                input: 3.0,
                output: 15.0,
                cache: 0.3,
            },
        );
        map.insert(
            "codex".to_string(),
            Price {
                input: 1.25,
                output: 10.0,
                cache: 0.125,
            },
        );
        map.insert(
            "agy".to_string(),
            Price {
                input: 1.25,
                output: 10.0,
                cache: 0.125,
            },
        );
        map.insert(
            "fake".to_string(),
            Price {
                input: 1.0,
                output: 5.0,
                cache: 0.1,
            },
        );
        Self(map)
    }
}

impl Prices {
    /// Read the price file; write the defaults when it does not exist yet.
    pub fn load_or_create(path: &Path) -> Self {
        match std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
        {
            Some(prices) => prices,
            None => {
                let prices = Self::default();
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Ok(text) = serde_json::to_string_pretty(&prices) {
                    let _ = std::fs::write(path, text);
                }
                prices
            }
        }
    }

    /// Estimated cost of a usage record for a CLI and model.
    pub fn estimate(&self, cli: &str, model: &str, usage: &Usage) -> f64 {
        let price = self
            .0
            .get(&format!("{cli}:{model}"))
            .or_else(|| self.0.get(cli));
        match price {
            Some(price) => {
                (usage.tokens_in as f64 * price.input
                    + usage.tokens_out as f64 * price.output
                    + usage.tokens_cache as f64 * price.cache)
                    / 1_000_000.0
            }
            None => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_uses_model_price_first() {
        let mut prices = Prices::default();
        prices.0.insert(
            "claude:haiku".into(),
            Price {
                input: 1.0,
                output: 5.0,
                cache: 0.1,
            },
        );
        let usage = Usage {
            tokens_in: 1_000_000,
            tokens_out: 1_000_000,
            tokens_cache: 1_000_000,
            cost_usd: 0.0,
            cost_source: "estimated".into(),
        };
        assert!((prices.estimate("claude", "haiku", &usage) - 6.1).abs() < 1e-9);
        assert!((prices.estimate("claude", "other", &usage) - 18.3).abs() < 1e-9);
        assert_eq!(prices.estimate("unknown", "", &usage), 0.0);
    }

    #[test]
    fn prices_file_is_created_and_editable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prices.json");
        let created = Prices::load_or_create(&path);
        assert_eq!(created, Prices::default());
        std::fs::write(&path, r#"{"claude":{"input":9,"output":9}}"#).unwrap();
        let edited = Prices::load_or_create(&path);
        assert_eq!(edited.0["claude"].input, 9.0);
        assert_eq!(edited.0["claude"].cache, 0.0);
    }

    #[test]
    fn test_config_is_fast() {
        let config = Config::for_tests(Path::new("/tmp/x"), Path::new("fixtures"));
        assert_eq!(config.db_path(), PathBuf::from("/tmp/x/regente.db"));
        assert!(config.batch_ms < 250);
    }
}
