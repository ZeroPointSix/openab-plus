//! Gateway admin API integration tests (ZER-179).
//!
//! Locks the HTTP contract for sessions, agent profiles, SSE, and admin auth
//! without external Discord/Slack dependencies.

mod common;

use common::{spawn_admin_server, AdminTestEnv};
use openab_core::acp::protocol::{ConfigOption, ConfigOptionValue};
use openab_core::agent_profile::AgentProfile;
use openab_core::session_event::SessionEventKind;
use openab_core::session_snapshot::{SessionSnapshot, SessionStatus};
use serde_json::{json, Value};

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
    pool.seed_session_snapshot_for_test(SessionSnapshot::new(
        "slack:thread-1".into(),
        "codex".into(),
        "/workspace".into(),
        Some("codex-default".into()),
        Some("Codex Default".into()),
        Some("gpt-5".into()),
        None,
    ))
    .await;
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
    assert_eq!(detail["agent"], "codex");

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

    let mut buffer = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut found = None;
    while tokio::time::Instant::now() < deadline {
        if let Some(chunk) = response.chunk().await.expect("sse chunk") {
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            if let Some(event) = common::parse_sse_event(&buffer, "session.created") {
                found = Some(event);
                break;
            }
        } else {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    let event = found.expect("session.created SSE event");
    assert_eq!(event["event"], "session.created");
    assert_eq!(event["snapshot"]["session_id"], "slack:sse-thread");
    assert_eq!(event["snapshot"]["status"], "idle");
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
