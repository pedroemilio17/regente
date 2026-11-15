//! Contract test: what the daemon really sends must match `shared/schema/`.
//! Uses a small validator for the subset of JSON Schema those files use.

use std::path::Path;
use std::time::Duration;

use regente_daemon::config::Config;
use regente_daemon::mcp::http_request;
use regente_daemon::model::EventKind;
use serde_json::{json, Value};

fn load(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../shared/schema")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

struct Schemas {
    api: Value,
    event: Value,
}

fn type_matches(expected: &str, value: &Value) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

impl Schemas {
    fn check(
        &self,
        root: &Value,
        schema: &Value,
        value: &Value,
        path: &str,
        errors: &mut Vec<String>,
    ) {
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            let (file, fragment) = reference.split_once('#').unwrap_or((reference, ""));
            let target_root = match file {
                "" => root,
                "event.schema.json" => &self.event,
                "api.schema.json" => &self.api,
                _ => {
                    errors.push(format!("{path}: unknown schema reference {reference}"));
                    return;
                }
            };
            let target = if fragment.is_empty() {
                Some(target_root)
            } else {
                target_root.pointer(fragment)
            };
            match target {
                Some(target) => self.check(target_root, target, value, path, errors),
                None => errors.push(format!("{path}: unknown schema reference {reference}")),
            }
            return;
        }
        for keyword in ["oneOf", "anyOf", "allOf"] {
            if let Some(options) = schema.get(keyword).and_then(Value::as_array) {
                let matches = options
                    .iter()
                    .filter(|option| {
                        let mut local = Vec::new();
                        self.check(root, option, value, path, &mut local);
                        local.is_empty()
                    })
                    .count();
                let valid = match keyword {
                    "oneOf" => matches == 1,
                    "anyOf" => matches > 0,
                    _ => matches == options.len(),
                };
                if !valid {
                    errors.push(format!(
                        "{path}: matched {matches} of the {keyword} options"
                    ));
                }
            }
        }
        if let Some(constant) = schema.get("const") {
            if constant != value {
                errors.push(format!("{path}: expected {constant}, got {value}"));
            }
        }
        match schema.get("type") {
            Some(Value::String(expected)) if !type_matches(expected, value) => {
                errors.push(format!("{path}: expected {expected}, got {value}"));
                return;
            }
            Some(Value::Array(options))
                if !options
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|option| type_matches(option, value)) =>
            {
                errors.push(format!("{path}: expected one of {options:?}, got {value}"));
                return;
            }
            _ => {}
        }
        if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
            if !allowed.contains(value) {
                errors.push(format!("{path}: {value} is not in {allowed:?}"));
            }
        }
        if let Value::Object(map) = value {
            for required in schema
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if !map.contains_key(required) {
                    errors.push(format!("{path}: missing {required}"));
                }
            }
            let properties = schema.get("properties").and_then(Value::as_object);
            for (key, item) in map {
                match properties.and_then(|properties| properties.get(key)) {
                    Some(property) => {
                        self.check(root, property, item, &format!("{path}.{key}"), errors)
                    }
                    None if schema.get("additionalProperties") == Some(&Value::Bool(false)) => {
                        errors.push(format!("{path}: unexpected key {key}"));
                    }
                    None => {}
                }
            }
        }
        if let (Value::Array(items), Some(item_schema)) = (value, schema.get("items")) {
            for (index, item) in items.iter().enumerate() {
                self.check(root, item_schema, item, &format!("{path}[{index}]"), errors);
            }
        }
    }

    fn assert_api(&self, definition: &str, value: &Value) {
        let mut errors = Vec::new();
        self.check(
            &self.api,
            &self.api["$defs"][definition],
            value,
            definition,
            &mut errors,
        );
        assert!(
            errors.is_empty(),
            "{definition} does not match the schema:\n{}",
            errors.join("\n")
        );
    }
}

fn call(addr: std::net::SocketAddr, method: &str, path: &str, body: Option<Value>) -> Value {
    let body = body.map(|value| value.to_string());
    let (status, text) =
        http_request(method, &format!("http://{addr}{path}"), body.as_deref()).unwrap();
    assert!(status < 300, "{method} {path} -> {status}: {text}");
    serde_json::from_str(&text).unwrap()
}

#[test]
fn event_kinds_match_the_schema() {
    let event = load("event.schema.json");
    let listed: Vec<&str> = event["$defs"]["EventKind"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let real: Vec<&str> = EventKind::ALL.iter().map(|kind| kind.as_str()).collect();
    assert_eq!(listed, real);
}

#[tokio::test(flavor = "multi_thread")]
async fn daemon_output_matches_the_schemas() {
    let schemas = Schemas {
        api: load("api.schema.json"),
        event: load("event.schema.json"),
    };
    let home = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "init",
        ],
    ] {
        assert!(std::process::Command::new("git")
            .current_dir(root.path())
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (app, addr) = regente_daemon::start(Config::for_tests(home.path(), &fixtures), listener)
        .await
        .unwrap();

    let root_path = root.path().to_path_buf();
    let (project, task) = tokio::task::spawn_blocking(move || {
        let created = call(
            addr,
            "POST",
            "/api/v1/projects",
            Some(json!({
                "name": "Demo",
                "root": root_path,
                "folders": [{ "path": "src", "access": "write" }],
                "agents": [
                    { "name": "Maestro", "cli": "fake", "is_orchestrator": true, "settings": { "fake_scenario": "pusher" } },
                    { "name": "Backend", "cli": "fake", "settings": { "fake_scenario": "basic" } }
                ]
            })),
        );
        let project = created["project"]["id"].as_str().unwrap().to_string();
        let task = call(addr, "POST", "/api/v1/tasks", Some(json!({ "project_id": project, "title": "Publicar", "budget_usd": 2 })));
        (project, task["id"].as_str().unwrap().to_string())
    })
    .await
    .unwrap();

    // The `pusher` scenario parks on an approval: the snapshot then has every kind of entity.
    for _ in 0..800 {
        if !app
            .store
            .list_approvals(Some("pending"))
            .unwrap()
            .is_empty()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let backend = app
        .store
        .list_agents(Some(&project))
        .unwrap()
        .into_iter()
        .find(|agent| agent.name == "Backend")
        .unwrap();
    regente_daemon::mcp::call_tool(
        &app,
        &backend.id,
        "send_message",
        &json!({ "to": "Maestro", "text": "oi" }),
    )
    .unwrap();

    let snapshot = tokio::task::spawn_blocking(move || call(addr, "GET", "/api/v1/snapshot", None))
        .await
        .unwrap();
    schemas.assert_api("Snapshot", &snapshot);
    schemas.assert_api("StreamMessage", &snapshot);
    assert!(!snapshot["approvals"].as_array().unwrap().is_empty());
    assert!(!snapshot["messages"].as_array().unwrap().is_empty());
    assert_eq!(snapshot["tasks"][0]["id"], task.as_str());

    let events = app.store.all_events().unwrap();
    assert!(events.len() >= 5);
    let batch = json!({ "type": "events", "items": events });
    schemas.assert_api("EventsMessage", &batch);
    schemas.assert_api("StreamMessage", &batch);
    schemas.assert_api(
        "UsageMessage",
        &json!({ "type": "usage", "usage": snapshot["usage"] }),
    );
    schemas.assert_api(
        "ApprovalMessage",
        &json!({ "type": "approval", "approval": snapshot["approvals"][0] }),
    );
    // agy has no status command, so no CLI runs here.
    let status = tokio::task::spawn_blocking(move || {
        call(addr, "GET", "/api/v1/accounts/acc_agy/status", None)
    })
    .await
    .unwrap();
    assert_eq!(status["state"], "unknown");
    schemas.assert_api("AccountStatus", &status);

    // The validator itself rejects what it should.
    let mut broken = snapshot.clone();
    broken["agents"][0]["cli"] = json!("emacs");
    let mut errors = Vec::new();
    schemas.check(
        &schemas.api,
        &schemas.api["$defs"]["Snapshot"],
        &broken,
        "Snapshot",
        &mut errors,
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
    let mut missing_live = snapshot.clone();
    missing_live["agents"][0]
        .as_object_mut()
        .unwrap()
        .remove("live");
    let mut errors = Vec::new();
    schemas.check(
        &schemas.api,
        &schemas.api["$defs"]["Snapshot"],
        &missing_live,
        "Snapshot",
        &mut errors,
    );
    assert!(
        errors.iter().any(|error| error.contains("missing live")),
        "{errors:?}"
    );

    let mut errors = Vec::new();
    schemas.check(
        &schemas.api,
        &json!({"$ref": "event.schema.json#/$defs/EventKind"}),
        &json!("invalid"),
        "kind",
        &mut errors,
    );
    assert!(!errors.is_empty());
    let mut errors = Vec::new();
    schemas.check(
        &schemas.api,
        &json!({"anyOf": [{"type": "string"}, {"type": "null"}]}),
        &json!(42),
        "value",
        &mut errors,
    );
    assert!(!errors.is_empty());
    let mut errors = Vec::new();
    schemas.check(
        &schemas.api,
        &json!({"allOf": [{"type": "string"}, {"const": "ok"}]}),
        &json!("wrong"),
        "value",
        &mut errors,
    );
    assert!(!errors.is_empty());
}
