mod atomic;
mod claude;
mod codex;
mod home;
mod merge;
mod thinking;

pub use atomic::{atomic_write_private, atomic_write_private_sync};

pub use home::{claude_settings_path, cli_home_dir, codex_config_path};
pub use merge::FieldChange;
pub use thinking::{disabled_levels, is_supported, supported_levels, THINKING_LEVELS};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::Mutex as AsyncMutex;

/// When a successful native-config write becomes visible to agent processes.
///
/// OpenAB only guarantees that **sessions started after** `apply` / `apply_unlocked`
/// read the updated vendor files. There is no per-turn or live-process reload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveOn {
    /// Fresh ACP / CLI processes started after the write.
    #[default]
    NewSession,
}

#[derive(Debug, Clone, Default)]
pub struct ApplyRequest {
    pub agent_type: String,
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

impl ApplyRequest {
    /// Native file writes are observed by processes started after apply, not by
    /// already-running sessions (see [`EffectiveOn::NewSession`]).
    pub const EFFECTIVE_ON: EffectiveOn = EffectiveOn::NewSession;
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
    /// Contract marker: apply only writes files; new sessions pick them up.
    #[serde(default)]
    pub effective_on: EffectiveOn,
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

/// Per-agent-type mutex shared by CLI file apply and spawn.
pub async fn lock_for(agent_type: &str) -> Arc<AsyncMutex<()>> {
    let mut map = apply_locks().lock().expect("apply lock map poisoned");
    map.entry(agent_type.to_string())
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
///
/// Writes vendor files only; effectiveness is [`EffectiveOn::NewSession`] (no
/// live-process / per-turn reload). Caller must already hold [`lock_for`] for
/// the same agent type when pairing with spawn.
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
    let lock = lock_for(&request.agent_type).await;
    let _guard = lock.lock().await;
    apply_unlocked(request).await
}

pub async fn restore(agent_type: &str) -> Result<bool> {
    if !supports_file_renderer(agent_type) {
        return Err(anyhow!("no CLI file renderer for agent type {agent_type}"));
    }
    let lock = lock_for(agent_type).await;
    let _guard = lock.lock().await;
    match agent_type {
        "codex" => codex::restore().await,
        "claude" => claude::restore().await,
        other => Err(anyhow!("no CLI file renderer for agent type {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn apply_codex_writes_config_under_test_home() {
        let _guard = home::test_home_env_lock().lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("OPENAB_TEST_HOME", dir.path());
        let report = apply(&ApplyRequest {
            agent_type: "codex".into(),
            model: Some("gpt-5".into()),
            reasoning_effort: Some("high".into()),
            provider_id: Some("newapi".into()),
            provider_type: Some("openai_compatible".into()),
            base_url: Some("https://api.example.com/v1".into()),
            api_key_env: Some("OPENAI_API_KEY".into()),
        })
        .await
        .unwrap();
        let path = dir.path().join(".codex/config.toml");
        let body = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(body.contains("gpt-5"));
        assert!(body.contains("model_reasoning_effort"));
        assert!(body.contains("openai_base_url"));
        assert!(body.contains("api.example.com"));
        assert!(!report.files.is_empty());
        assert_eq!(report.effective_on, EffectiveOn::NewSession);
        assert_eq!(ApplyRequest::EFFECTIVE_ON, EffectiveOn::NewSession);
        std::env::remove_var("OPENAB_TEST_HOME");
    }

    /// Contract: apply mutates vendor files only — it does not touch running
    /// processes, re-read config per turn, or pull images.
    #[tokio::test]
    async fn apply_only_writes_files_new_session_contract() {
        let _guard = home::test_home_env_lock().lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("OPENAB_TEST_HOME", dir.path());
        let before = std::fs::read_dir(dir.path())
            .map(|entries| entries.count())
            .unwrap_or(0);

        let report = apply(&ApplyRequest {
            agent_type: "claude".into(),
            model: Some("claude-opus-4-6".into()),
            reasoning_effort: Some("high".into()),
            provider_id: None,
            provider_type: Some("anthropic".into()),
            base_url: None,
            api_key_env: Some("ANTHROPIC_API_KEY".into()),
        })
        .await
        .unwrap();

        assert_eq!(report.effective_on, EffectiveOn::NewSession);
        assert_eq!(report.agent_type, "claude");
        assert_eq!(report.files.len(), 1);
        let written = dir.path().join(".claude/settings.json");
        assert_eq!(
            report.files[0].path,
            written.display().to_string(),
            "apply reports the file it wrote, not a process handle"
        );
        assert!(
            tokio::fs::try_exists(&written).await.unwrap(),
            "apply must create/update the vendor settings file"
        );
        let body = tokio::fs::read_to_string(&written).await.unwrap();
        assert!(body.contains("claude-opus-4-6"));
        // No sidecar “reload” or process-control artifacts under test home.
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .collect();
        assert!(
            names.iter().any(|n| n == ".claude"),
            "expected .claude under test home; entries before={before} after={names:?}"
        );
        assert!(
            names
                .iter()
                .all(|n| n != "pid" && n != "reload" && n != "hot_reload"),
            "apply must not create process-control artifacts: {names:?}"
        );
        std::env::remove_var("OPENAB_TEST_HOME");
    }
}
