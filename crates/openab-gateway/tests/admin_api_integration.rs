//! Gateway admin API integration tests (ZER-179).
//!
//! Locks the HTTP contract for sessions, agent profiles, SSE, and admin auth
//! without external Discord/Slack dependencies.

mod common;

use common::{spawn_admin_server, spawn_admin_server_with, AdminTestEnv};
use openab_core::acp::protocol::{ConfigOption, ConfigOptionValue};
use openab_core::acp::SessionPool;
use openab_core::agent_profile::AgentProfile;
use openab_core::config::AgentConfig;
use openab_core::session_event::SessionEventKind;
use openab_core::session_snapshot::{SessionRuntimeMetadata, SessionSnapshot, SessionStatus};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

async fn read_sse_event(
    response: &mut reqwest::Response,
    event_name: &str,
) -> (Option<String>, Value) {
    let mut buffer = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if let Some(chunk) = response.chunk().await.expect("sse chunk") {
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            if let Some(event) = common::parse_sse_event_with_id(&buffer, event_name) {
                return event;
            }
        } else {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    panic!("{event_name} SSE event not found");
}

fn config_option(id: &str, name: &str, current_value: &str, values: &[&str]) -> ConfigOption {
    ConfigOption {
        id: id.into(),
        name: name.into(),
        description: None,
        category: None,
        option_type: "enum".into(),
        current_value: current_value.into(),
        options: values
            .iter()
            .map(|value| ConfigOptionValue {
                value: (*value).into(),
                name: (*value).into(),
                description: None,
            })
            .collect(),
    }
}

#[tokio::test]
async fn sessions_list_and_detail_happy_path() {
    let env = AdminTestEnv::new().await;
    let pool = env.pool();
    let mut runtime_snapshot = SessionSnapshot::new(
        "slack:thread-1".into(),
        String::new(),
        "/workspace".into(),
        Some("codex-default".into()),
        Some("Codex Default".into()),
        None,
        None,
    );
    runtime_snapshot.replace_runtime_metadata(SessionRuntimeMetadata::acp(
        Some("codex-acp".into()),
        Some("gpt-5".into()),
        Some("high".into()),
    ));
    pool.seed_session_snapshot_for_test(runtime_snapshot).await;
    pool.seed_session_snapshot_for_test(SessionSnapshot::new(
        "discord:thread-2".into(),
        "opencode".into(),
        "/tmp/work".into(),
        None,
        None,
        None,
        None,
    ))
    .await;

    let server = spawn_admin_server(&env).await;
    let client = reqwest::Client::new();

    let list = client
        .get(format!("{}/api/v1/sessions", server.base_url))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("list sessions")
        .json::<Vec<Value>>()
        .await
        .expect("sessions json");
    assert_eq!(list.len(), 2);
    assert!(list.iter().any(|row| row["session_id"] == "slack:thread-1"));
    assert!(list
        .iter()
        .any(|row| row["session_id"] == "discord:thread-2"));

    // Optional query params are accepted (currently ignored by the handler).
    let filtered = client
        .get(format!(
            "{}/api/v1/sessions?status=idle&platform=slack",
            server.base_url
        ))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("filtered list")
        .json::<Vec<Value>>()
        .await
        .expect("filtered json");
    assert_eq!(filtered.len(), 2);

    let detail = client
        .get(format!(
            "{}/api/v1/sessions/{}",
            server.base_url,
            urlencoding::encode("slack:thread-1")
        ))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("get session")
        .json::<Value>()
        .await
        .expect("detail json");
    assert_eq!(detail["session_id"], "slack:thread-1");
    assert_eq!(detail["status"], "idle");
    assert_eq!(detail["agent"], "codex-acp");
    assert_eq!(detail["model"], "gpt-5");
    assert_eq!(detail["reasoning_effort"], "high");
    assert_eq!(detail["metadata_source"], "acp");

    let missing = client
        .get(format!("{}/api/v1/sessions/missing", server.base_url))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("missing session");
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn sessions_auth_rejects_missing_and_invalid_tokens() {
    let env = AdminTestEnv::new().await;
    let server = spawn_admin_server(&env).await;
    let client = reqwest::Client::new();

    let no_token = client
        .get(format!("{}/api/v1/sessions", server.base_url))
        .send()
        .await
        .expect("no token");
    assert_eq!(no_token.status(), reqwest::StatusCode::UNAUTHORIZED);

    let bad_token = client
        .get(format!("{}/api/v1/sessions", server.base_url))
        .bearer_auth("wrong-token")
        .send()
        .await
        .expect("bad token");
    assert_eq!(bad_token.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn session_creation_requires_auth_and_an_enabled_profile() {
    let env = AdminTestEnv::new().await;
    let server = spawn_admin_server(&env).await;
    let client = reqwest::Client::new();

    let unauthenticated = client
        .post(format!("{}/api/v1/sessions", server.base_url))
        .json(&json!({ "profile_id": "missing" }))
        .send()
        .await
        .expect("create without token");
    assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);

    let empty_profile = client
        .post(format!("{}/api/v1/sessions", server.base_url))
        .bearer_auth(&env.token)
        .json(&json!({ "profile_id": " " }))
        .send()
        .await
        .expect("create without profile");
    assert_eq!(empty_profile.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(
        empty_profile
            .json::<Value>()
            .await
            .expect("empty profile error")["error"],
        "profile_id is required"
    );

    let unknown_profile = client
        .post(format!("{}/api/v1/sessions", server.base_url))
        .bearer_auth(&env.token)
        .json(&json!({ "profile_id": "missing" }))
        .send()
        .await
        .expect("create unknown profile");
    assert_eq!(unknown_profile.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(unknown_profile
        .json::<Value>()
        .await
        .expect("unknown profile error")["error"]
        .as_str()
        .is_some_and(|message| message.contains("agent profile not found")));

    let mut disabled = AgentProfile::new("disabled", "Disabled", "codex");
    disabled.enabled = false;
    env.profile_service()
        .upsert(disabled)
        .await
        .expect("save disabled profile");
    let disabled_profile = client
        .post(format!("{}/api/v1/sessions", server.base_url))
        .bearer_auth(&env.token)
        .json(&json!({ "profile_id": "disabled" }))
        .send()
        .await
        .expect("create disabled profile session");
    assert_eq!(disabled_profile.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(disabled_profile
        .json::<Value>()
        .await
        .expect("disabled profile error")["error"]
        .as_str()
        .is_some_and(|message| message.contains("agent profile is disabled")));
}

#[cfg(unix)]
#[tokio::test]
async fn session_creation_starts_selected_non_default_agent_profile() {
    let env = AdminTestEnv::new().await;
    let server = spawn_admin_server(&env).await;
    let client = reqwest::Client::new();
    let agent_dir = tempfile::tempdir().expect("fake agent tempdir");
    let agent_path = agent_dir.path().join("fake_acp_agent.py");
    std::fs::write(
        &agent_path,
        r#"import json
import sys

for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "initialize":
        result = {
            "agentInfo": {"name": "fake-claude-acp"},
            "agentCapabilities": {"loadSession": False},
        }
    elif method == "session/new":
        result = {"sessionId": "fake-claude-session"}
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"#,
    )
    .expect("write fake ACP agent");

    let mut profile = AgentProfile::new("claude-admin", "Claude Admin", "claude");
    profile.command = Some("python3".into());
    profile.args = vec![agent_path.to_string_lossy().into_owned()];
    env.profile_service()
        .upsert(profile)
        .await
        .expect("save selected profile");

    let created = client
        .post(format!("{}/api/v1/sessions", server.base_url))
        .bearer_auth(&env.token)
        .json(&json!({ "profile_id": "claude-admin" }))
        .send()
        .await
        .expect("create selected profile session");
    assert_eq!(created.status(), reqwest::StatusCode::CREATED);
    let snapshot = created.json::<Value>().await.expect("created session json");
    assert_eq!(snapshot["agent"], "claude");
    assert_eq!(snapshot["profile_id"], "claude-admin");
    assert!(snapshot["session_id"]
        .as_str()
        .is_some_and(|id| id.starts_with("admin:")));
}

#[tokio::test]
async fn profiles_crud_default_and_validate() {
    let env = AdminTestEnv::new().await;
    let server = spawn_admin_server(&env).await;
    let client = reqwest::Client::new();

    let invalid = client
        .post(format!("{}/api/v1/agent-profiles", server.base_url))
        .bearer_auth(&env.token)
        .json(&json!({
            "id": "bad profile",
            "name": "",
            "agent_type": "codex"
        }))
        .send()
        .await
        .expect("create invalid");
    assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);
    let body = invalid.json::<Value>().await.expect("validation body");
    assert_eq!(body["validation"]["ok"], false);
    assert!(body["validation"]["errors"]
        .as_array()
        .is_some_and(|e| !e.is_empty()));

    let created = client
        .post(format!("{}/api/v1/agent-profiles", server.base_url))
        .bearer_auth(&env.token)
        .json(&AgentProfile::new("codex-main", "Codex Main", "codex"))
        .send()
        .await
        .expect("create profile");
    assert_eq!(created.status(), reqwest::StatusCode::CREATED);

    let read = client
        .get(format!(
            "{}/api/v1/agent-profiles/codex-main",
            server.base_url
        ))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("read profile")
        .json::<Value>()
        .await
        .expect("profile json");
    assert_eq!(read["id"], "codex-main");
    assert_eq!(read["name"], "Codex Main");

    let updated = client
        .put(format!(
            "{}/api/v1/agent-profiles/codex-main",
            server.base_url
        ))
        .bearer_auth(&env.token)
        .json(&json!({
            "id": "codex-main",
            "name": "Codex Updated",
            "agent_type": "codex",
            "default_model": "gpt-5"
        }))
        .send()
        .await
        .expect("update profile")
        .json::<Value>()
        .await
        .expect("updated json");
    assert_eq!(updated["profiles"][0]["name"], "Codex Updated");

    let validation = client
        .post(format!(
            "{}/api/v1/agent-profiles/codex-main/validate",
            server.base_url
        ))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("validate profile")
        .json::<Value>()
        .await
        .expect("validate json");
    assert_eq!(validation["ok"], true);

    let default_set = client
        .put(format!("{}/api/v1/agent-profiles/default", server.base_url))
        .bearer_auth(&env.token)
        .json(&json!({ "profile_id": "codex-main" }))
        .send()
        .await
        .expect("set default")
        .json::<Value>()
        .await
        .expect("default doc");
    assert_eq!(default_set["default_profile"], "codex-main");

    let default_get = client
        .get(format!("{}/api/v1/agent-profiles/default", server.base_url))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("get default")
        .json::<Value>()
        .await
        .expect("default json");
    assert_eq!(default_get["default_profile"], "codex-main");

    let deleted = client
        .delete(format!(
            "{}/api/v1/agent-profiles/codex-main",
            server.base_url
        ))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("delete profile")
        .json::<Value>()
        .await
        .expect("delete json");
    assert_eq!(deleted["deleted"], true);

    let gone = client
        .get(format!(
            "{}/api/v1/agent-profiles/codex-main",
            server.base_url
        ))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("read deleted");
    assert_eq!(gone.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_profile_marks_existing_session_snapshot_deleted() {
    let env = AdminTestEnv::new().await;
    env.profile_service()
        .upsert(AgentProfile::new("codex-main", "Codex Main", "codex"))
        .await
        .expect("seed profile");
    env.pool()
        .seed_session_snapshot_for_test(SessionSnapshot::new(
            "slack:thread-profile".into(),
            "codex".into(),
            "/workspace".into(),
            Some("codex-main".into()),
            Some("Codex Main".into()),
            Some("gpt-5".into()),
            None,
        ))
        .await;
    let server = spawn_admin_server(&env).await;
    let client = reqwest::Client::new();

    let deleted = client
        .delete(format!(
            "{}/api/v1/agent-profiles/codex-main",
            server.base_url
        ))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("delete profile")
        .json::<Value>()
        .await
        .expect("delete json");
    assert_eq!(deleted["deleted"], true);

    let detail = client
        .get(format!(
            "{}/api/v1/sessions/{}",
            server.base_url,
            urlencoding::encode("slack:thread-profile")
        ))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("get session")
        .json::<Value>()
        .await
        .expect("detail json");
    assert_eq!(detail["profile_id"], "codex-main");
    assert_eq!(detail["profile_name"], "Codex Main");
    assert_eq!(detail["profile_status"], "deleted");

    let sessions = client
        .get(format!("{}/api/v1/sessions", server.base_url))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("list sessions")
        .json::<Vec<Value>>()
        .await
        .expect("sessions json");
    let listed_session = sessions
        .iter()
        .find(|session| session["session_id"] == "slack:thread-profile")
        .expect("listed session");
    assert_eq!(listed_session["profile_id"], "codex-main");
    assert_eq!(listed_session["profile_name"], "Codex Main");
    assert_eq!(listed_session["profile_status"], "deleted");

    let profiles = client
        .get(format!("{}/api/v1/agent-profiles", server.base_url))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("list profiles")
        .json::<Value>()
        .await
        .expect("profiles json");
    if let Some(listed_profiles) = profiles["profiles"].as_array() {
        assert!(
            listed_profiles
                .iter()
                .all(|profile| profile["id"] != "codex-main"),
            "deleted profile should not remain in list: {profiles:?}"
        );
    } else {
        assert!(
            profiles.get("profiles").is_none(),
            "unexpected profiles response: {profiles:?}"
        );
    }

    let base_config = AgentConfig::default();
    let base_options = HashMap::new();
    let err = env
        .profile_service()
        .resolve_for_session(&base_config, &base_options, Some("codex-main"), None)
        .await
        .expect_err("deleted profile should not resolve for new sessions");
    assert!(err.to_string().contains("not found"));
}

#[tokio::test]
async fn profiles_auth_rejects_missing_and_invalid_tokens() {
    let env = AdminTestEnv::new().await;
    let server = spawn_admin_server(&env).await;
    let client = reqwest::Client::new();

    let no_token = client
        .get(format!("{}/api/v1/agent-profiles", server.base_url))
        .send()
        .await
        .expect("profiles no token");
    assert_eq!(no_token.status(), reqwest::StatusCode::UNAUTHORIZED);

    let bad_token = client
        .get(format!("{}/api/v1/agent-profiles", server.base_url))
        .header("x-openab-admin-token", "wrong-token")
        .send()
        .await
        .expect("profiles bad token");
    assert_eq!(bad_token.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn config_schema_prefers_live_agent_options() {
    let env = AdminTestEnv::new().await;
    let pool = env.pool();
    let mut fallback_profile = AgentProfile::new("opencode-static", "OpenCode Static", "opencode");
    fallback_profile.default_model = Some("static-only".into());
    env.profile_service()
        .upsert(fallback_profile)
        .await
        .expect("seed fallback profile");

    let mut snapshot = SessionSnapshot::new(
        "slack:live-opencode".into(),
        "opencode".into(),
        "/workspace".into(),
        None,
        None,
        Some("opencode/latest".into()),
        None,
    );
    snapshot.set_status(SessionStatus::Running);
    pool.seed_session_snapshot_for_test(snapshot).await;
    pool.seed_config_options_for_test(
        "slack:live-opencode",
        vec![
            config_option(
                "model",
                "Model",
                "opencode/latest",
                &["opencode/latest", "opencode/canary"],
            ),
            config_option(
                "reasoning_effort",
                "Reasoning Effort",
                "medium",
                &["low", "medium", "high"],
            ),
        ],
    )
    .await;

    let server = spawn_admin_server(&env).await;
    let client = reqwest::Client::new();
    let schema = client
        .get(format!(
            "{}/api/v1/agents/opencode/config-schema",
            server.base_url
        ))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("config schema")
        .json::<Value>()
        .await
        .expect("schema json");

    assert_eq!(schema["source"], "agent-session-config-options");
    let fields = schema["fields"].as_array().expect("schema fields");
    let model = fields
        .iter()
        .find(|field| field["key"] == "model")
        .expect("model field");
    assert_eq!(model["id"], "model");
    assert_eq!(model["kind"], "enum");
    assert_eq!(model["type"], "enum");
    assert_eq!(model["apply_after_start"], false);
    let model_options = model["options"].as_array().expect("model options");
    assert!(model_options
        .iter()
        .any(|value| value.as_str() == Some("opencode/canary")));
    assert!(!model_options
        .iter()
        .any(|value| value.as_str() == Some("static-only")));
    assert!(fields
        .iter()
        .any(|field| field["key"] == "reasoning_effort" && field["type"] == "enum"));
}

#[tokio::test]
async fn config_schema_falls_back_without_live_agent() {
    let env = AdminTestEnv::new().await;
    let mut profile = AgentProfile::new("codex-main", "Codex Main", "codex");
    profile.default_model = Some("gpt-5".into());
    env.profile_service()
        .upsert(profile)
        .await
        .expect("seed fallback profile");

    let server = spawn_admin_server(&env).await;
    let client = reqwest::Client::new();
    let schema = client
        .get(format!(
            "{}/api/v1/agents/codex/config-schema",
            server.base_url
        ))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("config schema")
        .json::<Value>()
        .await
        .expect("schema json");

    assert_eq!(schema["source"], "profile-store-fallback");
    let fields = schema["fields"].as_array().expect("schema fields");
    let model = fields
        .iter()
        .find(|field| field["key"] == "model")
        .expect("model field");
    assert_eq!(model["id"], "model");
    assert_eq!(model["type"], "enum");
    assert_eq!(model["apply_after_start"], false);
    assert!(model["options"]
        .as_array()
        .expect("model options")
        .iter()
        .any(|value| value.as_str() == Some("gpt-5")));
}

#[tokio::test]
async fn sse_emits_session_created_with_id_and_status() {
    let env = AdminTestEnv::new().await;
    let pool = env.pool();
    let server = spawn_admin_server(&env).await;
    let client = reqwest::Client::new();

    let sse_url = format!("{}/api/v1/sessions/events", server.base_url);
    let mut response = client
        .get(&sse_url)
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("open sse");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/event-stream")));

    let snapshot = SessionSnapshot::new(
        "slack:sse-thread".into(),
        "codex".into(),
        "/workspace".into(),
        None,
        None,
        None,
        None,
    );
    let published = pool
        .session_event_bus()
        .publish(SessionEventKind::SessionCreated, snapshot);

    let (id, event) = read_sse_event(&mut response, "session.created").await;
    assert_eq!(
        id,
        Some(pool.session_event_bus().event_id(published.sequence))
    );
    assert_eq!(event["event"], "session.created");
    assert_eq!(event["snapshot"]["session_id"], "slack:sse-thread");
    assert_eq!(event["snapshot"]["status"], "idle");
    assert_eq!(event["sequence"], published.sequence);
}

#[tokio::test]
async fn sse_replays_missed_event_after_last_event_id() {
    let env = AdminTestEnv::new().await;
    let pool = env.pool();

    let first_snapshot = SessionSnapshot::new(
        "slack:replay-thread".into(),
        "codex".into(),
        "/workspace".into(),
        None,
        None,
        None,
        None,
    );
    let first = pool
        .session_event_bus()
        .publish(SessionEventKind::SessionCreated, first_snapshot);
    let mut missed_snapshot = SessionSnapshot::new(
        "slack:replay-thread".into(),
        "codex".into(),
        "/workspace".into(),
        None,
        None,
        None,
        None,
    );
    missed_snapshot.set_status(SessionStatus::Running);
    let missed = pool
        .session_event_bus()
        .publish(SessionEventKind::StatusChanged, missed_snapshot);

    let server = spawn_admin_server(&env).await;
    let client = reqwest::Client::new();
    let event_bus = pool.session_event_bus();
    let sse_url = format!("{}/api/v1/sessions/events", server.base_url);
    let mut response = client
        .get(&sse_url)
        .bearer_auth(&env.token)
        .header("Last-Event-ID", event_bus.event_id(first.sequence))
        .send()
        .await
        .expect("open replay sse");

    let (id, event) = read_sse_event(&mut response, "status_changed").await;
    assert_eq!(id, Some(event_bus.event_id(missed.sequence)));
    assert_eq!(event["event"], "status_changed");
    assert_eq!(event["snapshot"]["session_id"], "slack:replay-thread");
    assert_eq!(event["snapshot"]["status"], "running");
    assert_eq!(event["sequence"], missed.sequence);
}

#[tokio::test]
async fn sse_resets_cursor_after_gateway_restart_and_delivers_live_events() {
    let env = AdminTestEnv::new().await;
    let previous_pool = env.pool();
    let previous_bus = previous_pool.session_event_bus();
    let previous = previous_bus.publish(
        SessionEventKind::SessionCreated,
        SessionSnapshot::new(
            "slack:previous-gateway".into(),
            "codex".into(),
            "/workspace".into(),
            None,
            None,
            None,
            None,
        ),
    );
    let previous_cursor = previous_bus.event_id(previous.sequence);

    let fresh_pool = Arc::new(
        SessionPool::new(AgentConfig::default(), 4, 120, HashMap::new())
            .with_agent_profile_service(env.profile_service()),
    );
    let fresh_bus = fresh_pool.session_event_bus();
    assert_ne!(previous_bus.generation(), fresh_bus.generation());

    let server = spawn_admin_server_with(fresh_pool, env.profile_service()).await;
    let client = reqwest::Client::new();
    let mut response = client
        .get(format!("{}/api/v1/sessions/events", server.base_url))
        .bearer_auth(&env.token)
        .header("Last-Event-ID", previous_cursor)
        .send()
        .await
        .expect("open restarted gateway sse");

    let (reset_id, reset) = read_sse_event(&mut response, "cursor_reset").await;
    assert_eq!(reset_id, Some(fresh_bus.event_id(0)));
    assert_eq!(reset["error"], "event cursor generation changed");
    assert_eq!(reset["current_generation"], fresh_bus.generation());
    assert_eq!(
        reset["action"],
        "refetch /api/v1/sessions before continuing the stream"
    );

    let published = fresh_bus.publish(
        SessionEventKind::SessionCreated,
        SessionSnapshot::new(
            "slack:fresh-gateway".into(),
            "codex".into(),
            "/workspace".into(),
            None,
            None,
            None,
            None,
        ),
    );
    let (id, event) = read_sse_event(&mut response, "session.created").await;
    assert_eq!(id, Some(fresh_bus.event_id(published.sequence)));
    assert_eq!(event["snapshot"]["session_id"], "slack:fresh-gateway");
    assert_eq!(event["sequence"], published.sequence);
}

#[tokio::test]
async fn unified_router_mounts_session_and_profile_admin_routes() {
    const {
        assert!(
            !openab_gateway::STANDALONE_SESSION_ADMIN_MOUNTED,
            "standalone gateway must not mount session admin routes"
        );
    };

    let env = AdminTestEnv::new().await;
    let server = spawn_admin_server(&env).await;
    let client = reqwest::Client::new();

    for path in [
        "/api/v1/sessions",
        "/api/v1/sessions/events",
        "/api/v1/agent-profiles",
    ] {
        let response = client
            .get(format!("{}{path}", server.base_url))
            .bearer_auth(&env.token)
            .send()
            .await
            .expect("route should exist");
        assert_ne!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND,
            "{path} should be mounted on the unified admin router"
        );
    }
}

#[tokio::test]
async fn transcript_snapshot_supports_full_and_after_replay() {
    let env = AdminTestEnv::new().await;
    let pool = env.pool();
    let session_id = "slack:transcript-thread";
    pool.seed_session_snapshot_for_test(SessionSnapshot::new(
        session_id.into(),
        "codex".into(),
        "/workspace".into(),
        None,
        None,
        None,
        None,
    ))
    .await;

    let transcripts = pool.transcript_store();
    let user = transcripts.record_user_text(session_id, "Summarize this change");
    transcripts.append_assistant_text(session_id, "The change ");
    transcripts.append_assistant_text(session_id, "adds transcript storage.");
    transcripts.finish_assistant_turn(session_id);

    let server = spawn_admin_server(&env).await;
    let client = reqwest::Client::new();
    let encoded_id = urlencoding::encode(session_id);
    let full = client
        .get(format!(
            "{}/api/v1/sessions/{encoded_id}/transcript",
            server.base_url
        ))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("get full transcript")
        .json::<Value>()
        .await
        .expect("full transcript json");

    assert_eq!(full["session_id"], session_id);
    assert_eq!(full["entries"].as_array().unwrap().len(), 2);
    assert_eq!(full["entries"][0]["role"], "user");
    assert_eq!(
        full["entries"][1]["content"],
        "The change adds transcript storage."
    );
    assert_eq!(full["entries"][1]["status"], "completed");

    let incremental = client
        .get(format!(
            "{}/api/v1/sessions/{encoded_id}/transcript?after={}",
            server.base_url, user.entry.sequence
        ))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("get incremental transcript")
        .json::<Value>()
        .await
        .expect("incremental transcript json");
    let entries = incremental["entries"]
        .as_array()
        .expect("incremental entries");
    assert!(!entries.is_empty());
    assert!(entries
        .iter()
        .all(|entry| entry["sequence"].as_u64().unwrap() > user.entry.sequence));
    assert_eq!(incremental["overflowed"], false);

    let missing = client
        .get(format!(
            "{}/api/v1/sessions/missing/transcript",
            server.base_url
        ))
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("missing transcript");
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn sse_emits_and_replays_transcript_events_with_shared_cursor() {
    let env = AdminTestEnv::new().await;
    let pool = env.pool();
    let session_id = "slack:transcript-sse";
    pool.seed_session_snapshot_for_test(SessionSnapshot::new(
        session_id.into(),
        "codex".into(),
        "/workspace".into(),
        None,
        None,
        None,
        None,
    ))
    .await;

    let server = spawn_admin_server(&env).await;
    let client = reqwest::Client::new();
    let sse_url = format!("{}/api/v1/sessions/events", server.base_url);
    let mut response = client
        .get(&sse_url)
        .bearer_auth(&env.token)
        .send()
        .await
        .expect("open transcript sse");

    let first = pool
        .transcript_store()
        .record_user_text(session_id, "first transcript event");
    let (first_id, first_event) = read_sse_event(&mut response, "transcript").await;
    let stream_bus = pool.session_stream_bus();
    assert_eq!(first_id, Some(stream_bus.event_id(first.sequence)));
    assert_eq!(first_event["session_id"], session_id);
    assert_eq!(first_event["entry"]["role"], "user");
    assert_eq!(first_event["entry"]["content"], "first transcript event");

    let second = pool
        .transcript_store()
        .append_assistant_text(session_id, "second transcript event");
    let mut replay = client
        .get(&sse_url)
        .bearer_auth(&env.token)
        .header("Last-Event-ID", stream_bus.event_id(first.sequence))
        .send()
        .await
        .expect("open transcript replay sse");
    let (second_id, second_event) = read_sse_event(&mut replay, "transcript").await;
    assert_eq!(second_id, Some(stream_bus.event_id(second.sequence)));
    assert_eq!(second_event["session_id"], session_id);
    assert_eq!(second_event["entry"]["content"], "second transcript event");
}
