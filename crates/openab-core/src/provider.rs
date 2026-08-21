use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

pub const PROVIDER_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Must be `${VAR}` or `env://VAR`. Never store plaintext secrets.
    pub api_key_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderDocument {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<Provider>,
}

fn default_schema_version() -> u32 {
    PROVIDER_SCHEMA_VERSION
}

impl Default for ProviderDocument {
    fn default() -> Self {
        Self {
            schema_version: PROVIDER_SCHEMA_VERSION,
            providers: Vec::new(),
        }
    }
}

impl Provider {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        provider_type: impl Into<String>,
        api_key_ref: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            provider_type: provider_type.into(),
            base_url: None,
            api_key_ref: api_key_ref.into(),
        }
    }
}

pub fn env_ref_variable_name(value: &str) -> Option<&str> {
    let value = value.trim();
    if let Some(name) = value
        .strip_prefix("${")
        .and_then(|v| v.strip_suffix('}'))
    {
        let name = name.strip_prefix("env:").unwrap_or(name).trim();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    } else if let Some(name) = value.strip_prefix("env://") {
        let name = name.trim();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    } else {
        None
    }
}

pub fn is_env_only_secret_ref(value: &str) -> bool {
    env_ref_variable_name(value).is_some()
}

pub fn validate_provider(provider: &Provider) -> Result<()> {
    if provider.id.trim().is_empty() {
        return Err(anyhow!("provider.id must not be empty"));
    }
    if !provider
        .id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        return Err(anyhow!(
            "provider.id may only contain ascii alphanumerics and -_./"
        ));
    }
    if provider.name.trim().is_empty() {
        return Err(anyhow!("provider.name must not be empty"));
    }
    if provider.provider_type.trim().is_empty() {
        return Err(anyhow!("provider.provider_type must not be empty"));
    }
    if !is_env_only_secret_ref(&provider.api_key_ref) {
        return Err(anyhow!(
            "provider.api_key_ref must be ${{VAR}} or env://VAR (plaintext rejected)"
        ));
    }
    if let Some(url) = provider.base_url.as_deref() {
        if url.trim().is_empty() {
            return Err(anyhow!("provider.base_url must not be empty when set"));
        }
    }
    Ok(())
}

pub fn validate_document(document: &ProviderDocument) -> Result<()> {
    if document.schema_version != PROVIDER_SCHEMA_VERSION {
        return Err(anyhow!(
            "unsupported providers schema_version {}",
            document.schema_version
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for provider in &document.providers {
        validate_provider(provider)?;
        if !seen.insert(provider.id.clone()) {
            return Err(anyhow!("duplicate provider id {}", provider.id));
        }
    }
    Ok(())
}

/// Resolve env-only refs for process injection. Does not accept other schemes.
pub fn resolve_env_secret_ref(value: &str) -> Result<String> {
    let name = env_ref_variable_name(value)
        .ok_or_else(|| anyhow!("secret ref must be ${{VAR}} or env://VAR with a non-empty name"))?;
    std::env::var(name).map_err(|_| anyhow!("environment variable {name} is not set"))
}

pub fn api_key_env_name(provider_type: &str) -> &'static str {
    match provider_type {
        "anthropic" | "anthropic_compatible" => "ANTHROPIC_API_KEY",
        _ => "OPENAI_API_KEY",
    }
}

pub fn base_url_env_name(provider_type: &str) -> &'static str {
    match provider_type {
        "anthropic" | "anthropic_compatible" => "ANTHROPIC_BASE_URL",
        _ => "OPENAI_BASE_URL",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_env_ref() {
        let err = validate_provider(&Provider::new("p1", "P", "openai_compatible", "${}"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("plaintext") || err.contains("non-empty"));
        let err = validate_provider(&Provider::new("p1", "P", "openai_compatible", "env://"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("plaintext") || err.contains("non-empty"));
    }

    #[test]
    fn rejects_plaintext_api_key_ref() {
        let provider = Provider::new("p1", "P", "openai_compatible", "sk-secret");
        let err = validate_provider(&provider).unwrap_err().to_string();
        assert!(err.contains("plaintext"));
    }

    #[test]
    fn accepts_env_refs() {
        validate_provider(&Provider::new(
            "p1",
            "P",
            "openai_compatible",
            "env://OPENAI_API_KEY",
        ))
        .unwrap();
        validate_provider(&Provider::new(
            "p2",
            "P",
            "anthropic",
            "${ANTHROPIC_API_KEY}",
        ))
        .unwrap();
    }
}
