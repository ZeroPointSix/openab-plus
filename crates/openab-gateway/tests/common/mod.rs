//! Shared harness for gateway admin API integration tests.

use axum::routing::get;
use axum::Router;
use openab_core::acp::SessionPool;
use openab_core::agent_profile::AgentProfileService;
use openab_core::config::AgentConfig;
use openab_core::profile_store::ProfileStore;
use openab_gateway::{agent_profile_admin, session_admin};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tempfile::TempDir;
use tokio::sync::{Mutex, MutexGuard};

/// Serializes env mutation across integration tests. The guard must live for the
/// whole `AdminTestEnv` lifetime so concurrent tests cannot restore env vars
/// while another test's HTTP server is still handling requests.
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> &'static Mutex<()> {
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

pub struct AdminTestEnv {
    _profile_dir: TempDir,
    _env_lock: MutexGuard<'static, ()>,
    previous_admin_token: Option<String>,
    previous_profiles_path: Option<String>,
    pool: Arc<SessionPool>,
    profile_service: Arc<AgentProfileService>,
    pub token: String,
}

impl Drop for AdminTestEnv {
    fn drop(&mut self) {
        restore_env(
            "GATEWAY_ADMIN_TOKEN",
            self.previous_admin_token.take(),
        );
        restore_env(
            "OPENAB_AGENT_PROFILES_PATH",
            self.previous_profiles_path.take(),
        );
    }
}

fn restore_env(key: &str, value: Option<String>) {
    match value {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }
}

impl AdminTestEnv {
    pub async fn new() -> Self {
        let env_lock = env_lock().lock().await;
        let previous_admin_token = std::env::var("GATEWAY_ADMIN_TOKEN").ok();
        let previous_profiles_path = std::env::var("OPENAB_AGENT_PROFILES_PATH").ok();

        let profile_dir = tempfile::tempdir().expect("profile tempdir");
        let profiles_path = profile_dir.path().join("agent-profiles.toml");
        let token = "integration-test-admin-token".to_string();
        std::env::set_var("GATEWAY_ADMIN_TOKEN", &token);
        std::env::set_var("OPENAB_AGENT_PROFILES_PATH", &profiles_path);

        let profile_service = Arc::new(AgentProfileService::new(ProfileStore::new(&profiles_path)));
        let pool = Arc::new(
            SessionPool::new(AgentConfig::default(), 4, 120, HashMap::new())
                .with_agent_profile_service(profile_service.clone()),
        );

        Self {
            _profile_dir: profile_dir,
            _env_lock: env_lock,
            previous_admin_token,
            previous_profiles_path,
            pool,
            profile_service,
            token,
        }
    }

    pub fn pool(&self) -> Arc<SessionPool> {
        self.pool.clone()
    }

    pub fn profile_service(&self) -> Arc<AgentProfileService> {
        self.profile_service.clone()
    }
}

pub struct TestServer {
    pub base_url: String,
    _handle: tokio::task::JoinHandle<()>,
}

pub fn unified_admin_router(
    pool: Arc<SessionPool>,
    profile_service: Arc<AgentProfileService>,
) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(session_admin::router(pool))
        .merge(agent_profile_admin::router(profile_service))
}

pub async fn spawn_admin_server(env: &AdminTestEnv) -> TestServer {
    let app = unified_admin_router(env.pool(), env.profile_service());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("listener addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve admin test router");
    });
    TestServer {
        base_url: format!("http://{addr}"),
        _handle: handle,
    }
}

pub fn parse_sse_event(buffer: &str, event_name: &str) -> Option<Value> {
    let normalized = buffer.replace("\r\n", "\n");
    for chunk in normalized.split("\n\n") {
        let mut event = String::new();
        let mut data_lines = Vec::new();
        for line in chunk.lines() {
            if let Some(value) = line.strip_prefix("event:") {
                event = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("data:") {
                data_lines.push(value.trim_start().to_string());
            }
        }
        if event == event_name {
            let data = data_lines.join("\n");
            return serde_json::from_str(&data).ok();
        }
    }
    None
}
