use super::atomic::{atomic_write, ensure_openab_bak, openab_bak_path, restore_from_openab_bak};
use super::home::codex_config_path;
use super::merge::{merge_toml_owned_keys, FieldChange};
use super::thinking::{codex_effort_value, is_supported};
use super::{ApplyRequest, DryRunFile, DryRunReport};
use anyhow::{anyhow, Result};
use std::collections::BTreeMap;

pub fn plan(request: &ApplyRequest) -> Result<DryRunReport> {
    let path = codex_config_path()?;
    let existing = if path.exists() {
        std::fs::read_to_string(&path).unwrap_or_default()
    } else {
        String::new()
    };
    let owned = owned_keys(request)?;
    let (_merged, changes) = merge_toml_owned_keys(&existing, &owned)?;
    Ok(DryRunReport {
        agent_type: "codex".into(),
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
    let path = codex_config_path()?;
    let existing = if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        tokio::fs::read_to_string(&path).await.unwrap_or_default()
    } else {
        String::new()
    };
    let owned = owned_keys(request)?;
    let (merged, changes) = merge_toml_owned_keys(&existing, &owned)?;
    let bak = ensure_openab_bak(&path).await?;
    atomic_write(&path, merged).await?;
    Ok(DryRunReport {
        agent_type: "codex".into(),
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
    restore_from_openab_bak(&codex_config_path()?).await
}

fn unsupported_thinking(request: &ApplyRequest) -> Option<String> {
    request.reasoning_effort.as_ref().and_then(|level| {
        if is_supported("codex", request.model.as_deref(), level) {
            None
        } else {
            Some(level.clone())
        }
    })
}

fn owned_keys(request: &ApplyRequest) -> Result<BTreeMap<String, toml::Value>> {
    let mut owned = BTreeMap::new();
    if let Some(model) = request.model.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        owned.insert("model".into(), toml::Value::String(model.to_string()));
    }
    if let Some(level) = request
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let value = codex_effort_value(level)
            .ok_or_else(|| anyhow!("unsupported thinking level for codex: {level}"))?;
        owned.insert(
            "model_reasoning_effort".into(),
            toml::Value::String(value.to_string()),
        );
    }
    if let Some(base_url) = request
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        owned.insert(
            "openai_base_url".into(),
            toml::Value::String(base_url.to_string()),
        );
    }
    // API keys stay in process env (request.api_key_env); never write secret values.
    let _ = (&request.api_key_env, FieldChange { from: None, to: None });
    Ok(owned)
}
