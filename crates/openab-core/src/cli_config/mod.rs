mod atomic;
mod claude;
mod codex;
mod home;
mod merge;
mod thinking;

pub use atomic::{atomic_write_private, atomic_write_private_sync};

pub use home::{
    build_spawn_home_and_cli_env, claude_settings_path, claude_settings_path_for, cli_config_dir,
    cli_home_dir, cli_isolation_env, codex_config_path, codex_config_path_for,
    ensure_cli_config_dir, openab_home_dir, real_home_dir, sanitize_profile_segment,
};
pub use merge::FieldChange;
pub use thinking::{disabled_levels, is_supported, supported_levels, THINKING_LEVELS};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::Mutex as AsyncMutex;

#[derive(Debug, Clone, Default)]
pub struct ApplyRequest {
    pub agent_type: String,
    /// Optional profile id for per-profile CLI config isolation (ZER-888).
    pub profile_id: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub provider_id: Option<String>,
    /// Provider type from total ledger (e.g. openai_compatible / anthropic).
    pub provider_type: Option<String>,
    /// Optional OpenAI/Anthropic compatible base URL to render into CLI config / env.
    pub base_url: Option<String>,
    /// Environment variable name that carries the API key (never the secret value).
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DryRunFile {
    pub path: String,
    pub backup_path: String,
    pub fields: BTreeMap<String, FieldChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DryRunReport {
    pub agent_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsupported_thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<DryRunFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

pub fn supports_file_renderer(agent_type: &str) -> bool {
    matches!(agent_type, "codex" | "claude")
}

fn apply_locks() -> &'static Mutex<HashMap<String, Arc<AsyncMutex<()>>>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_key(agent_type: &str, profile_id: Option<&str>) -> String {
    format!(
        "{agent_type}::{}",
        home::sanitize_profile_segment(profile_id)
    )
}

/// Per-agent-type mutex shared by CLI file apply and spawn.
/// Prefer [`lock_for_profile`] when a profile id is known.
pub async fn lock_for(agent_type: &str) -> Arc<AsyncMutex<()>> {
    lock_for_profile(agent_type, None).await
}

/// Per agent_type + profile mutex so concurrent profiles do not serialize
/// unnecessarily, while still serializing apply+spawn for the same target dir.
pub async fn lock_for_profile(agent_type: &str, profile_id: Option<&str>) -> Arc<AsyncMutex<()>> {
    let key = lock_key(agent_type, profile_id);
    let mut map = apply_locks().lock().expect("apply lock map poisoned");
    map.entry(key)
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

pub fn plan(request: &ApplyRequest) -> Result<DryRunReport> {
    match request.agent_type.as_str() {
        "codex" => codex::plan(request),
        "claude" => claude::plan(request),
        other => Err(anyhow!("no CLI file renderer for agent type {other}")),
    }
}

/// Apply CLI config without taking the per-agent lock.
/// Caller must already hold [`lock_for`] for the same agent type when pairing with spawn.
pub async fn apply_unlocked(request: &ApplyRequest) -> Result<DryRunReport> {
    if !supports_file_renderer(&request.agent_type) {
        return Err(anyhow!(
            "no CLI file renderer for agent type {}",
            request.agent_type
        ));
    }
    match request.agent_type.as_str() {
        "codex" => codex::apply(request).await,
        "claude" => claude::apply(request).await,
        other => Err(anyhow!("no CLI file renderer for agent type {other}")),
    }
}

pub async fn apply(request: &ApplyRequest) -> Result<DryRunReport> {
    let lock = lock_for_profile(&request.agent_type, request.profile_id.as_deref()).await;
    let _guard = lock.lock().await;
    apply_unlocked(request).await
}

pub async fn restore(agent_type: &str) -> Result<bool> {
    restore_for_profile(agent_type, None).await
}

pub async fn restore_for_profile(agent_type: &str, profile_id: Option<&str>) -> Result<bool> {
    if !supports_file_renderer(agent_type) {
        return Err(anyhow!("no CLI file renderer for agent type {agent_type}"));
    }
    let lock = lock_for_profile(agent_type, profile_id).await;
    let _guard = lock.lock().await;
    match agent_type {
        "codex" => codex::restore(profile_id).await,
        "claude" => claude::restore(profile_id).await,
        other => Err(anyhow!("no CLI file renderer for agent type {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn apply_codex_writes_config_under_test_home() {
        let _guard = home::test_home_env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("OPENAB_TEST_HOME", dir.path());
        let report = apply(&ApplyRequest {
            agent_type: "codex".into(),
            profile_id: None,
            model: Some("gpt-5".into()),
            reasoning_effort: Some("high".into()),
            provider_id: Some("newapi".into()),
            provider_type: Some("openai_compatible".into()),
            base_url: Some("https://api.example.com/v1".into()),
            api_key_env: Some("OPENAI_API_KEY".into()),
        })
        .await
        .unwrap();
        // OPENAB_TEST_HOME compat: writers land under $OPENAB_TEST_HOME/cli/...
        let path = dir.path().join("cli/codex/system/config.toml");
        let body = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(body.contains("gpt-5"));
        assert!(body.contains("model_reasoning_effort"));
        assert!(body.contains("openai_base_url"));
        assert!(body.contains("api.example.com"));
        assert!(!report.files.is_empty());
        std::env::remove_var("OPENAB_TEST_HOME");
    }

    #[tokio::test]
    async fn dual_profile_claude_settings_do_not_overwrite_and_spawn_env_keeps_real_home() {
        let _guard = home::test_home_env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("OPENAB_TEST_HOME", dir.path());

        let report_a = apply(&ApplyRequest {
            agent_type: "claude".into(),
            profile_id: Some("alpha".into()),
            model: Some("claude-alpha".into()),
            reasoning_effort: Some("high".into()),
            provider_id: None,
            provider_type: None,
            base_url: Some("https://alpha.example/v1".into()),
            api_key_env: None,
        })
        .await
        .unwrap();
        let report_b = apply(&ApplyRequest {
            agent_type: "claude".into(),
            profile_id: Some("beta".into()),
            model: Some("claude-beta".into()),
            reasoning_effort: Some("low".into()),
            provider_id: None,
            provider_type: None,
            base_url: Some("https://beta.example/v1".into()),
            api_key_env: None,
        })
        .await
        .unwrap();

        let path_a = dir.path().join("cli/claude/alpha/settings.json");
        let path_b = dir.path().join("cli/claude/beta/settings.json");
        assert_ne!(path_a, path_b);
        assert_eq!(report_a.files[0].path, path_a.display().to_string());
        assert_eq!(report_b.files[0].path, path_b.display().to_string());

        let body_a = tokio::fs::read_to_string(&path_a).await.unwrap();
        let body_b = tokio::fs::read_to_string(&path_b).await.unwrap();
        assert!(body_a.contains("claude-alpha"));
        assert!(body_a.contains("alpha.example"));
        assert!(!body_a.contains("claude-beta"));
        assert!(body_b.contains("claude-beta"));
        assert!(body_b.contains("beta.example"));
        assert!(!body_b.contains("claude-alpha"));

        let (home, extra) = build_spawn_home_and_cli_env(Some("claude"), Some("alpha")).unwrap();
        assert_eq!(home, dir.path().display().to_string());
        assert!(!home.contains("/cli/claude/"));
        assert_eq!(
            extra,
            vec![(
                "CLAUDE_CONFIG_DIR".into(),
                dir.path().join("cli/claude/alpha").display().to_string()
            )]
        );

        std::env::remove_var("OPENAB_TEST_HOME");
    }
}
