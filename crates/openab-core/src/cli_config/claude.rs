use super::atomic::{
    atomic_write_private, ensure_openab_bak, openab_bak_path, restore_from_openab_bak,
};
use super::home::claude_settings_path;
use super::merge::merge_json_owned_keys;
use super::thinking::{claude_effort_value, is_supported};
use super::{ApplyRequest, DryRunFile, DryRunReport};
use anyhow::{anyhow, Result};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

pub fn plan(request: &ApplyRequest) -> Result<DryRunReport> {
    let path = claude_settings_path()?;
    let existing = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let owned = owned_keys(request)?;
    let (_merged, changes) = merge_json_owned_keys(&existing, &owned)?;
    Ok(DryRunReport {
        agent_type: "claude".into(),
        unsupported_thinking: unsupported_thinking(request),
        files: vec![DryRunFile {
            path: path.display().to_string(),
            backup_path: openab_bak_path(&path).display().to_string(),
            fields: changes,
        }],
        errors: Vec::new(),
    })
}

pub async fn apply(request: &ApplyRequest) -> Result<DryRunReport> {
    let path = claude_settings_path()?;
    let existing = if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        tokio::fs::read_to_string(&path).await?
    } else {
        String::new()
    };
    let owned = owned_keys(request)?;
    let (merged, changes) = merge_json_owned_keys(&existing, &owned)?;
    let bak = ensure_openab_bak(&path).await?;
    atomic_write_private(&path, merged).await?;
    Ok(DryRunReport {
        agent_type: "claude".into(),
        unsupported_thinking: unsupported_thinking(request),
        files: vec![DryRunFile {
            path: path.display().to_string(),
            backup_path: bak
                .unwrap_or_else(|| openab_bak_path(&path))
                .display()
                .to_string(),
            fields: changes,
        }],
        errors: Vec::new(),
    })
}

pub async fn restore() -> Result<bool> {
    restore_from_openab_bak(&claude_settings_path()?).await
}

fn unsupported_thinking(request: &ApplyRequest) -> Option<String> {
    request.reasoning_effort.as_ref().and_then(|level| {
        if is_supported("claude", request.model.as_deref(), level) {
            None
        } else {
            Some(level.clone())
        }
    })
}

fn owned_keys(request: &ApplyRequest) -> Result<BTreeMap<String, JsonValue>> {
    let mut owned = BTreeMap::new();
    if let Some(model) = request.model.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        owned.insert("model".into(), JsonValue::String(model.to_string()));
    }
    if let Some(level) = request
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let value = claude_effort_value(level)
            .ok_or_else(|| anyhow!("unsupported thinking level for claude: {level}"))?;
        owned.insert("effortLevel".into(), JsonValue::String(value.to_string()));
    }
    if let Some(base_url) = request
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let key = match request.provider_type.as_deref() {
            Some("openai_compatible") | Some("openai") => "OPENAI_BASE_URL",
            _ => "ANTHROPIC_BASE_URL",
        };
        let mut env = serde_json::Map::new();
        env.insert(key.into(), JsonValue::String(base_url.to_string()));
        owned.insert("env".into(), JsonValue::Object(env));
    }
    // API keys stay in process env (request.api_key_env); never write secret values.
    let _ = &request.api_key_env;
    Ok(owned)
}
