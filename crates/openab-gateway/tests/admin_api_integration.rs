//! Gateway admin API integration tests (ZER-179).
//!
//! Locks the HTTP contract for sessions, agent profiles, SSE, and admin auth
//! without external Discord/Slack dependencies.

mod common;

use common::{spawn_admin_server, AdminTestEnv};
use openab_core::agent_profile::AgentProfile;
use openab_core::session_event::SessionEventKind;
use openab_core::session_snapshot::SessionSnapshot;
use serde_json::{json, Value};

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
    assert!(list.iter().any(|row| row["session_id"] == "discord:thread-2"));

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
    assert!(body["validation"]["errors"].as_array().is_some_and(|e| !e.is_empty()));

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
    assert!(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/event-stream"))
    );

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
    assert!(
        !openab_gateway::STANDALONE_SESSION_ADMIN_MOUNTED,
        "standalone gateway must not mount session admin routes"
    );

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
