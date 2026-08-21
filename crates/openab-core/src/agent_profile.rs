use crate::config::AgentConfig;
use crate::profile_store::ProfileStore;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkdirStrategy {
    #[default]
    SystemDefault,
    ProfileDefault,
    EphemeralPerSession,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStrategy {
    None,
    RestartProcess,
    #[default]
    ResumeSession,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub agent_type: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default)]
    pub workdir_strategy: WorkdirStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env_refs: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inherit_env: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub recovery_strategy: RecoveryStrategy,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub config_options: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

fn default_enabled() -> bool {
    true
}

impl AgentProfile {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        agent_type: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            agent_type: agent_type.into(),
            enabled: true,
            command: None,
            args: Vec::new(),
            default_model: None,
            reasoning_effort: None,
            provider: None,
            workdir_strategy: WorkdirStrategy::default(),
            working_dir: None,
            env_refs: HashMap::new(),
            inherit_env: Vec::new(),
            timeout_secs: None,
            recovery_strategy: RecoveryStrategy::default(),
            config_options: HashMap::new(),
            created_at: None,
            updated_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentProfileDocument {
    #[serde(default = "default_profile_schema_version")]
    pub schema_version: u32,
    /// Legacy global default retained for backward-compatible reads. New writes use
    /// `default_profiles`, which scopes a default to its Agent type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub default_profiles: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<AgentProfile>,
}

impl Default for AgentProfileDocument {
    fn default() -> Self {
        Self {
            schema_version: default_profile_schema_version(),
            default_profile: None,
            default_profiles: BTreeMap::new(),
            profiles: Vec::new(),
        }
    }
}

impl AgentProfileDocument {
    pub fn default_profile_for_agent(&self, agent_type: &str) -> Option<&str> {
        self.default_profiles
            .get(agent_type)
            .map(String::as_str)
            .or_else(|| {
                self.default_profiles
                    .is_empty()
                    .then_some(self.default_profile.as_deref())
                    .flatten()
            })
    }

    pub fn compatibility_default_profile(&self) -> Option<String> {
        match self.default_profiles.len() {
            0 => self.default_profile.clone(),
            1 => self.default_profiles.values().next().cloned(),
            _ => None,
        }
    }
}

fn default_profile_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileValidationError {
    pub path: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileValidationResult {
    pub ok: bool,
    pub errors: Vec<ProfileValidationError>,
}

impl ProfileValidationResult {
    pub fn ok() -> Self {
        Self {
            ok: true,
            errors: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileSessionOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub config_options: HashMap<String, String>,
}

/// Strip process-level secrets before persisting session overrides to disk.
pub(crate) fn overrides_for_persistence(
    overrides: &ProfileSessionOverrides,
) -> Option<ProfileSessionOverrides> {
    let sanitized = ProfileSessionOverrides {
        model: overrides.model.clone(),
        reasoning_effort: overrides.reasoning_effort.clone(),
        config_options: overrides.config_options.clone(),
        ..ProfileSessionOverrides::default()
    };
    let has_overrides = sanitized.model.is_some()
        || sanitized.reasoning_effort.is_some()
        || !sanitized.config_options.is_empty();
    has_overrides.then_some(sanitized)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentProfileSnapshot {
    pub id: String,
    pub name: String,
    pub agent_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct ResolvedAgentProfile {
    pub pool_key: String,
    pub profile: Option<AgentProfileSnapshot>,
    pub config: AgentConfig,
    pub config_options: HashMap<String, String>,
    pub timeout_secs: Option<u64>,
    pub recovery_strategy: RecoveryStrategy,
    pub applied_layers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSummary {
    pub agent_type: String,
    pub profile_count: usize,
    pub enabled_profile_count: usize,
    pub default_profile: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct AgentConfigField {
    #[serde(alias = "key")]
    pub id: String,
    pub label: String,
    #[serde(alias = "type")]
    pub kind: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub dynamic: bool,
}

impl Serialize for AgentConfigField {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let field_count = if self.options.is_empty() { 7 } else { 8 };
        let mut state = serializer.serialize_struct("AgentConfigField", field_count)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("key", &self.id)?;
        state.serialize_field("label", &self.label)?;
        state.serialize_field("kind", &self.kind)?;
        state.serialize_field("type", &self.kind)?;
        if !self.options.is_empty() {
            state.serialize_field("options", &self.options)?;
        }
        state.serialize_field("dynamic", &self.dynamic)?;
        state.serialize_field("apply_after_start", &false)?;
        state.end()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentConfigSchema {
    pub agent_type: String,
    pub source: String,
    pub generated_at: DateTime<Utc>,
    pub fields: Vec<AgentConfigField>,
}

#[derive(Clone)]
pub struct AgentProfileService {
    store: ProfileStore,
}

impl AgentProfileService {
    pub fn from_env() -> Self {
        Self::new(ProfileStore::from_env())
    }

    pub fn new(store: ProfileStore) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &ProfileStore {
        &self.store
    }

    pub async fn list(&self) -> Result<AgentProfileDocument> {
        self.store.load().await
    }

    pub async fn get(&self, id: &str) -> Result<Option<AgentProfile>> {
        Ok(self
            .store
            .load()
            .await?
            .profiles
            .into_iter()
            .find(|profile| profile.id == id))
    }

    pub async fn upsert(&self, mut profile: AgentProfile) -> Result<AgentProfileDocument> {
        let _guard = self.store.write_lock().await;
        let mut doc = self.store.load_unlocked().await?;
        let now = Utc::now();
        match doc
            .profiles
            .iter_mut()
            .find(|current| current.id == profile.id)
        {
            Some(current) => {
                profile.created_at = current.created_at.or(Some(now));
                profile.updated_at = Some(now);
                *current = profile;
            }
            None => {
                profile.created_at = Some(now);
                profile.updated_at = Some(now);
                doc.profiles.push(profile);
            }
        }
        sort_profiles(&mut doc);
        let validation = validate_document(&doc);
        if !validation.ok {
            return Err(anyhow!(
                "invalid agent profile document: {:?}",
                validation.errors
            ));
        }
        self.store.save_atomic_unlocked(&doc).await?;
        Ok(doc)
    }

    pub async fn delete(&self, id: &str) -> Result<bool> {
        let _guard = self.store.write_lock().await;
        let mut doc = self.store.load_unlocked().await?;
        let before = doc.profiles.len();
        doc.profiles.retain(|profile| profile.id != id);
        let deleted = doc.profiles.len() != before;
        if doc.default_profile.as_deref() == Some(id) {
            doc.default_profile = None;
        }
        doc.default_profiles
            .retain(|_, default_id| default_id != id);
        if deleted {
            self.store.save_atomic_unlocked(&doc).await?;
        }
        Ok(deleted)
    }

    /// Set a default for the selected profile's Agent type. The legacy global field
    /// is retained as a compatibility hint, but session/UI selection uses the map.
    pub async fn set_default(&self, id: Option<String>) -> Result<AgentProfileDocument> {
        let _guard = self.store.write_lock().await;
        let mut doc = self.store.load_unlocked().await?;
        match id {
            Some(id) => {
                let profile = find_enabled_profile(&doc, &id)?
                    .ok_or_else(|| anyhow!("default profile not found: {id}"))?;
                doc.default_profiles
                    .insert(profile.agent_type.clone(), id.clone());
                doc.default_profile = doc.compatibility_default_profile();
            }
            None => {
                doc.default_profile = None;
                doc.default_profiles.clear();
            }
        }
        let validation = validate_document(&doc);
        if !validation.ok {
            return Err(anyhow!("invalid default profile: {:?}", validation.errors));
        }
        self.store.save_atomic_unlocked(&doc).await?;
        Ok(doc)
    }

    pub async fn set_default_for_agent(
        &self,
        agent_type: &str,
        id: Option<String>,
    ) -> Result<AgentProfileDocument> {
        let _guard = self.store.write_lock().await;
        let mut doc = self.store.load_unlocked().await?;
        match id {
            Some(id) => {
                let profile = find_enabled_profile(&doc, &id)?
                    .ok_or_else(|| anyhow!("default profile not found: {id}"))?;
                if profile.agent_type != agent_type {
                    return Err(anyhow!(
                        "profile {id} belongs to {}, not {agent_type}",
                        profile.agent_type
                    ));
                }
                doc.default_profiles
                    .insert(agent_type.to_string(), id.clone());
                doc.default_profile = doc.compatibility_default_profile();
            }
            None => {
                doc.default_profiles.remove(agent_type);
                doc.default_profile = doc.compatibility_default_profile();
            }
        }
        let validation = validate_document(&doc);
        if !validation.ok {
            return Err(anyhow!("invalid default profile: {:?}", validation.errors));
        }
        self.store.save_atomic_unlocked(&doc).await?;
        Ok(doc)
    }

    pub fn validate_profile(&self, profile: &AgentProfile) -> ProfileValidationResult {
        validate_profiles(None, &BTreeMap::new(), std::slice::from_ref(profile))
    }

    pub async fn validate_existing(&self, id: &str) -> Result<ProfileValidationResult> {
        match self.get(id).await? {
            Some(profile) => Ok(self.validate_profile(&profile)),
            None => Err(anyhow!("agent profile not found: {id}")),
        }
    }

    pub async fn validate_all(&self) -> Result<ProfileValidationResult> {
        Ok(validate_document(&self.store.load().await?))
    }

    pub async fn resolve_for_session(
        &self,
        base_config: &AgentConfig,
        base_options: &HashMap<String, String>,
        specified_profile_id: Option<&str>,
        overrides: Option<&ProfileSessionOverrides>,
    ) -> Result<ResolvedAgentProfile> {
        let doc = self.store.load().await?;
        let validation = validate_document(&doc);
        if !validation.ok {
            return Err(anyhow!(
                "invalid agent profile document: {:?}",
                validation.errors
            ));
        }

        let mut config = clone_agent_config(base_config);
        let mut config_options = base_options.clone();
        let mut applied_layers = vec!["system_default".to_string()];
        let mut profile_snapshot = None;
        let mut timeout_secs = None;
        let mut recovery_strategy = RecoveryStrategy::default();

        let specified_profile = match specified_profile_id {
            Some(profile_id) => Some(
                find_enabled_profile(&doc, profile_id)?
                    .ok_or_else(|| anyhow!("specified profile is not enabled: {profile_id}"))?,
            ),
            None => None,
        };
        // A selected Profile provides the Agent type needed to resolve its scoped
        // default. With no selection, retain the legacy global fallback; when a
        // single scoped default exists it is also unambiguous to apply.
        let default_id = if let Some(profile) = specified_profile {
            doc.default_profile_for_agent(&profile.agent_type)
        } else if doc.default_profiles.is_empty() {
            doc.default_profile.as_deref()
        } else if doc.default_profiles.len() == 1 {
            doc.default_profiles.values().next().map(String::as_str)
        } else {
            None
        };
        if let Some(default_id) = default_id {
            if let Some(default_profile) = find_enabled_profile(&doc, default_id)? {
                apply_profile(
                    &mut config,
                    &mut config_options,
                    default_profile,
                    &mut timeout_secs,
                    &mut recovery_strategy,
                );
                applied_layers.push(format!("default_profile:{default_id}"));
                profile_snapshot = Some(snapshot(default_profile));
            }
        }

        if let Some(profile) = specified_profile {
            let profile_id = &profile.id;
            apply_profile(
                &mut config,
                &mut config_options,
                profile,
                &mut timeout_secs,
                &mut recovery_strategy,
            );
            applied_layers.push(format!("specified_profile:{profile_id}"));
            profile_snapshot = Some(snapshot(profile));
        }

        if let Some(overrides) = overrides {
            apply_overrides(&mut config, &mut config_options, overrides);
            applied_layers.push("entry_override".to_string());
        }

        apply_agent_startup_options(
            profile_snapshot
                .as_ref()
                .map(|profile| profile.agent_type.as_str()),
            &mut config,
            &mut config_options,
        );

        let pool_key = profile_pool_key(
            profile_snapshot.as_ref(),
            &config,
            &config_options,
            timeout_secs,
            &recovery_strategy,
        );

        Ok(ResolvedAgentProfile {
            pool_key,
            profile: profile_snapshot,
            config,
            config_options,
            timeout_secs,
            recovery_strategy,
            applied_layers,
        })
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentSummary>> {
        let doc = self.store.load().await?;
        let mut agent_types = BTreeSet::new();
        for profile in &doc.profiles {
            agent_types.insert(profile.agent_type.clone());
        }
        if agent_types.is_empty() {
            agent_types.insert("system-default".to_string());
        }

        Ok(agent_types
            .into_iter()
            .map(|agent_type| {
                let profiles: Vec<_> = doc
                    .profiles
                    .iter()
                    .filter(|profile| profile.agent_type == agent_type)
                    .collect();
                AgentSummary {
                    default_profile: doc
                        .default_profile_for_agent(&agent_type)
                        .map(str::to_string),
                    agent_type,
                    profile_count: profiles.len(),
                    enabled_profile_count: profiles
                        .iter()
                        .filter(|profile| profile.enabled)
                        .count(),
                }
            })
            .collect())
    }

    pub async fn config_schema(&self, agent_type: &str) -> Result<AgentConfigSchema> {
        let doc = self.store.load().await?;
        Ok(AgentCapabilityResolver::new(doc).config_schema(agent_type))
    }
}

pub struct AgentCapabilityResolver {
    document: AgentProfileDocument,
}

impl AgentCapabilityResolver {
    pub fn new(document: AgentProfileDocument) -> Self {
        Self { document }
    }

    pub fn config_schema_from_options(
        agent_type: &str,
        options: &[crate::acp::protocol::ConfigOption],
    ) -> AgentConfigSchema {
        let fields = options
            .iter()
            .map(|option| AgentConfigField {
                id: option.id.clone(),
                label: option.name.clone(),
                kind: option.option_type.clone(),
                options: option
                    .options
                    .iter()
                    .map(|value| value.value.clone())
                    .collect(),
                dynamic: true,
            })
            .collect();

        AgentConfigSchema {
            agent_type: agent_type.to_string(),
            source: "agent-session-config-options".into(),
            generated_at: Utc::now(),
            fields,
        }
    }

    pub fn config_schema(&self, agent_type: &str) -> AgentConfigSchema {
        let mut models = BTreeSet::new();
        let mut efforts = BTreeSet::new();
        let mut options = BTreeSet::new();

        for profile in self
            .document
            .profiles
            .iter()
            .filter(|profile| profile.agent_type == agent_type)
        {
            if let Some(model) = profile.default_model.as_deref() {
                models.insert(model.to_string());
            }
            if let Some(reasoning_effort) = profile.reasoning_effort.as_deref() {
                efforts.insert(reasoning_effort.to_string());
            }
            for key in profile.config_options.keys() {
                options.insert(key.to_string());
            }
        }

        let mut fields = vec![
            AgentConfigField {
                id: "command".into(),
                label: "Startup command".into(),
                kind: "string".into(),
                options: Vec::new(),
                dynamic: false,
            },
            AgentConfigField {
                id: "working_dir".into(),
                label: "Working directory".into(),
                kind: "string".into(),
                options: Vec::new(),
                dynamic: false,
            },
            AgentConfigField {
                id: "model".into(),
                label: "Model".into(),
                kind: "enum".into(),
                options: models.into_iter().collect(),
                dynamic: true,
            },
            AgentConfigField {
                id: "reasoning_effort".into(),
                label: "Reasoning effort".into(),
                kind: "enum".into(),
                options: efforts.into_iter().collect(),
                dynamic: true,
            },
        ];

        for option in options {
            if option != "model" && option != "reasoning_effort" {
                fields.push(AgentConfigField {
                    id: option.clone(),
                    label: option,
                    kind: "string".into(),
                    options: Vec::new(),
                    dynamic: true,
                });
            }
        }

        AgentConfigSchema {
            agent_type: agent_type.to_string(),
            source: "profile-store-fallback".into(),
            generated_at: Utc::now(),
            fields,
        }
    }
}

pub fn validate_document(document: &AgentProfileDocument) -> ProfileValidationResult {
    validate_profiles(
        document.default_profile.as_deref(),
        &document.default_profiles,
        &document.profiles,
    )
}

fn validate_profiles(
    default_profile: Option<&str>,
    default_profiles: &BTreeMap<String, String>,
    profiles: &[AgentProfile],
) -> ProfileValidationResult {
    let mut errors = Vec::new();
    let mut ids = BTreeSet::new();

    for (index, profile) in profiles.iter().enumerate() {
        let base = format!("profiles[{index}]");
        if profile.id.trim().is_empty() {
            push_error(
                &mut errors,
                format!("{base}.id"),
                "required",
                "profile id is required",
            );
        } else if !profile
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            push_error(
                &mut errors,
                format!("{base}.id"),
                "invalid_id",
                "profile id may contain only ascii letters, numbers, dash, underscore, or dot",
            );
        }
        if !ids.insert(profile.id.clone()) {
            push_error(
                &mut errors,
                format!("{base}.id"),
                "duplicate_id",
                "profile id must be unique",
            );
        }
        if profile.name.trim().is_empty() {
            push_error(
                &mut errors,
                format!("{base}.name"),
                "required",
                "profile name is required",
            );
        }
        if profile.agent_type.trim().is_empty() {
            push_error(
                &mut errors,
                format!("{base}.agent_type"),
                "required",
                "agent type is required",
            );
        } else if !is_safe_profile_agent_type(&profile.agent_type) {
            push_error(
                &mut errors,
                format!("{base}.agent_type"),
                "invalid_agent_type",
                "agent type may contain only ascii letters, numbers, dash, or underscore",
            );
        }
        if profile
            .command
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            push_error(
                &mut errors,
                format!("{base}.command"),
                "empty_command",
                "startup command must not be empty",
            );
        }
        if profile
            .default_model
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            push_error(
                &mut errors,
                format!("{base}.default_model"),
                "empty_model",
                "default model must not be empty",
            );
        }
        if profile
            .reasoning_effort
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            push_error(
                &mut errors,
                format!("{base}.reasoning_effort"),
                "empty_reasoning_effort",
                "reasoning effort must not be empty",
            );
        }
        if let Some(timeout) = profile.timeout_secs {
            if !(5..=86_400).contains(&timeout) {
                push_error(
                    &mut errors,
                    format!("{base}.timeout_secs"),
                    "invalid_timeout",
                    "timeout must be between 5 and 86400 seconds",
                );
            }
        }
        for (key, value) in &profile.env_refs {
            if key.trim().is_empty() {
                push_error(
                    &mut errors,
                    format!("{base}.env_refs"),
                    "empty_env_key",
                    "environment variable key must not be empty",
                );
            }
            if !is_external_secret_ref(value) {
                push_error(
                    &mut errors,
                    format!("{base}.env_refs.{key}"),
                    "plain_secret_rejected",
                    "environment values must reference an external secret or environment variable",
                );
            }
        }
        for (key, value) in &profile.config_options {
            if key.trim().is_empty() || value.trim().is_empty() {
                push_error(
                    &mut errors,
                    format!("{base}.config_options"),
                    "empty_config_option",
                    "config option keys and values must not be empty",
                );
            }
        }
        for (idx, value) in profile.inherit_env.iter().enumerate() {
            if value.trim().is_empty() {
                push_error(
                    &mut errors,
                    format!("{base}.inherit_env[{idx}]"),
                    "empty_inherit_env",
                    "inherited environment names must not be empty",
                );
            }
        }
    }

    if let Some(default_id) = default_profile {
        match profiles.iter().find(|profile| profile.id == default_id) {
            Some(profile) if profile.enabled => {}
            Some(_) => push_error(
                &mut errors,
                "default_profile",
                "default_disabled",
                "default profile must be enabled",
            ),
            None => push_error(
                &mut errors,
                "default_profile",
                "default_missing",
                "default profile must refer to an existing profile",
            ),
        }
    }

    for (agent_type, default_id) in default_profiles {
        match profiles.iter().find(|profile| profile.id == *default_id) {
            Some(profile) if !profile.enabled => push_error(
                &mut errors,
                format!("default_profiles.{agent_type}"),
                "default_disabled",
                "default profile must be enabled",
            ),
            Some(profile) if profile.agent_type != *agent_type => push_error(
                &mut errors,
                format!("default_profiles.{agent_type}"),
                "agent_mismatch",
                "default profile must belong to its Agent type",
            ),
            Some(_) => {}
            None => push_error(
                &mut errors,
                format!("default_profiles.{agent_type}"),
                "default_missing",
                "default profile must refer to an existing profile",
            ),
        }
    }

    ProfileValidationResult {
        ok: errors.is_empty(),
        errors,
    }
}

fn push_error(
    errors: &mut Vec<ProfileValidationError>,
    path: impl Into<String>,
    code: impl Into<String>,
    message: impl Into<String>,
) {
    errors.push(ProfileValidationError {
        path: path.into(),
        code: code.into(),
        message: message.into(),
    });
}

fn sort_profiles(document: &mut AgentProfileDocument) {
    document.profiles.sort_by(|a, b| a.id.cmp(&b.id));
}

fn find_enabled_profile<'a>(
    doc: &'a AgentProfileDocument,
    id: &str,
) -> Result<Option<&'a AgentProfile>> {
    let profile = doc.profiles.iter().find(|profile| profile.id == id);
    if let Some(profile) = profile {
        if profile.enabled {
            Ok(Some(profile))
        } else {
            Err(anyhow!("agent profile is disabled: {id}"))
        }
    } else {
        Err(anyhow!("agent profile not found: {id}"))
    }
}

fn snapshot(profile: &AgentProfile) -> AgentProfileSnapshot {
    AgentProfileSnapshot {
        id: profile.id.clone(),
        name: profile.name.clone(),
        agent_type: profile.agent_type.clone(),
        provider: profile.provider.clone(),
        updated_at: profile.updated_at,
    }
}

fn clone_agent_config(config: &AgentConfig) -> AgentConfig {
    AgentConfig {
        command: config.command.clone(),
        args: config.args.clone(),
        working_dir: config.working_dir.clone(),
        env: config.env.clone(),
        inherit_env: config.inherit_env.clone(),
        images: config.images,
        command_explicit: config.command_explicit,
    }
}

fn apply_profile(
    config: &mut AgentConfig,
    config_options: &mut HashMap<String, String>,
    profile: &AgentProfile,
    timeout_secs: &mut Option<u64>,
    recovery_strategy: &mut RecoveryStrategy,
) {
    if let Some(command) = profile.command.as_deref() {
        apply_startup_command(config, command, &profile.args);
    } else if !profile.args.is_empty() {
        config.args = profile.args.clone();
    }
    if matches!(
        profile.workdir_strategy,
        WorkdirStrategy::ProfileDefault | WorkdirStrategy::EphemeralPerSession
    ) {
        if let Some(working_dir) = profile.working_dir.as_deref() {
            config.working_dir = working_dir.to_string();
        }
    }
    for (key, value) in &profile.env_refs {
        config.env.insert(key.clone(), resolve_secret_ref(value));
    }
    for key in &profile.inherit_env {
        if !config.inherit_env.iter().any(|existing| existing == key) {
            config.inherit_env.push(key.clone());
        }
    }
    if let Some(model) = profile.default_model.as_deref() {
        config_options.insert("model".to_string(), model.to_string());
    }
    if let Some(reasoning_effort) = profile.reasoning_effort.as_deref() {
        config_options.insert("reasoning_effort".to_string(), reasoning_effort.to_string());
    }
    for (key, value) in &profile.config_options {
        config_options.insert(key.clone(), value.clone());
    }
    if profile.timeout_secs.is_some() {
        *timeout_secs = profile.timeout_secs;
    }
    *recovery_strategy = profile.recovery_strategy.clone();
}

fn apply_overrides(
    config: &mut AgentConfig,
    config_options: &mut HashMap<String, String>,
    overrides: &ProfileSessionOverrides,
) {
    if let Some(command) = overrides.command.as_deref() {
        apply_startup_command(config, command, &overrides.args);
    } else if !overrides.args.is_empty() {
        config.args = overrides.args.clone();
    }
    if let Some(working_dir) = overrides.working_dir.as_deref() {
        config.working_dir = working_dir.to_string();
    }
    if let Some(model) = overrides.model.as_deref() {
        config_options.insert("model".to_string(), model.to_string());
    }
    if let Some(reasoning_effort) = overrides.reasoning_effort.as_deref() {
        config_options.insert("reasoning_effort".to_string(), reasoning_effort.to_string());
    }
    for (key, value) in &overrides.env {
        config.env.insert(key.clone(), value.clone());
    }
    for (key, value) in &overrides.config_options {
        config_options.insert(key.clone(), value.clone());
    }
}

fn apply_agent_startup_options(
    agent_type: Option<&str>,
    config: &mut AgentConfig,
    config_options: &mut HashMap<String, String>,
) {
    if agent_type != Some("claude") {
        return;
    }

    if let Some(model) = config_options.remove("model") {
        config.env.insert("ANTHROPIC_MODEL".to_string(), model);
    }

    let supported_effort = config_options
        .get("reasoning_effort")
        .is_some_and(|value| matches!(value.as_str(), "low" | "medium" | "high" | "xhigh" | "max"));
    if supported_effort {
        if let Some(effort) = config_options.remove("reasoning_effort") {
            config
                .env
                .insert("CLAUDE_CODE_EFFORT_LEVEL".to_string(), effort);
        }
    }
}

fn apply_startup_command(config: &mut AgentConfig, command: &str, args: &[String]) {
    if args.is_empty() {
        let mut parts = command.split_whitespace();
        if let Some(program) = parts.next() {
            config.command = program.to_string();
            config.args = parts.map(ToString::to_string).collect();
            config.command_explicit = true;
        }
    } else {
        config.command = command.to_string();
        config.args = args.to_vec();
        config.command_explicit = true;
    }
}

pub(crate) fn is_safe_profile_agent_type(agent_type: &str) -> bool {
    agent_type
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn is_external_secret_ref(value: &str) -> bool {
    if crate::provider::is_env_only_secret_ref(value) {
        return true;
    }
    let value = value.trim();
    value.starts_with("aws-sm://")
        || value.starts_with("vault://")
        || value.starts_with("gcp-sm://")
        || value.starts_with("azure-kv://")
        || value.starts_with("exec://")
}

fn resolve_secret_ref(value: &str) -> String {
    let value = value.trim();
    if let Some(name) = value.strip_prefix("${").and_then(|v| v.strip_suffix('}')) {
        let name = name.strip_prefix("env:").unwrap_or(name);
        std::env::var(name).unwrap_or_else(|_| value.to_string())
    } else if let Some(name) = value.strip_prefix("env://") {
        std::env::var(name).unwrap_or_else(|_| value.to_string())
    } else {
        value.to_string()
    }
}

fn profile_pool_key(
    profile: Option<&AgentProfileSnapshot>,
    config: &AgentConfig,
    options: &HashMap<String, String>,
    timeout_secs: Option<u64>,
    recovery_strategy: &RecoveryStrategy,
) -> String {
    let mut digest = Sha256::new();
    digest.update(config.command.as_bytes());
    digest.update(b"\0");
    for arg in &config.args {
        digest.update(arg.as_bytes());
        digest.update(b"\0");
    }
    digest.update(config.working_dir.as_bytes());
    digest.update(b"\0");
    for (key, value) in sorted_pairs(&config.env) {
        digest.update(key.as_bytes());
        digest.update(b"=");
        digest.update(value.as_bytes());
        digest.update(b"\0");
    }
    for key in &config.inherit_env {
        digest.update(key.as_bytes());
        digest.update(b"\0");
    }
    for (key, value) in sorted_pairs(options) {
        digest.update(key.as_bytes());
        digest.update(b"=");
        digest.update(value.as_bytes());
        digest.update(b"\0");
    }
    if let Some(timeout) = timeout_secs {
        digest.update(timeout.to_string().as_bytes());
    }
    digest.update(format!("{recovery_strategy:?}").as_bytes());
    let prefix = profile
        .map(|profile| format!("profile:{}", profile.id))
        .unwrap_or_else(|| "system".to_string());
    format!("{prefix}:{:x}", digest.finalize())
}

fn sorted_pairs(map: &HashMap<String, String>) -> Vec<(&String, &String)> {
    let mut pairs: Vec<_> = map.iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn base_config() -> AgentConfig {
        AgentConfig {
            command: "kiro-cli".into(),
            args: vec!["acp".into()],
            working_dir: "/workspace".into(),
            env: HashMap::new(),
            inherit_env: Vec::new(),
            images: crate::config::ImageHandling::default(),
            command_explicit: false,
        }
    }

    #[test]
    fn rejects_plain_secret_values() {
        let mut profile = AgentProfile::new("codex", "Codex", "codex");
        profile
            .env_refs
            .insert("OPENAI_API_KEY".into(), "plain".into());

        let result = validate_profiles(None, &BTreeMap::new(), &[profile]);

        assert!(!result.ok);
        assert_eq!(result.errors[0].code, "plain_secret_rejected");
    }

    #[test]
    fn validates_default_profile_integrity() {
        let doc = AgentProfileDocument {
            default_profile: Some("missing".into()),
            profiles: vec![AgentProfile::new("codex", "Codex", "codex")],
            ..Default::default()
        };

        let result = validate_document(&doc);

        assert!(!result.ok);
        assert_eq!(result.errors[0].code, "default_missing");
    }

    #[tokio::test]
    async fn resolves_precedence_chain() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path().join("profiles.toml"));
        let mut default = AgentProfile::new("default", "Default", "codex");
        default.default_model = Some("gpt-5".into());
        default
            .config_options
            .insert("mode".into(), "standard".into());
        let mut specified = AgentProfile::new("deep", "Deep", "codex");
        specified.reasoning_effort = Some("high".into());
        specified
            .env_refs
            .insert("OPENAI_API_KEY".into(), "${OPENAI_API_KEY}".into());
        store
            .save_atomic(&AgentProfileDocument {
                default_profile: Some("default".into()),
                profiles: vec![default, specified],
                ..Default::default()
            })
            .await
            .unwrap();
        let service = AgentProfileService::new(store);
        let overrides = ProfileSessionOverrides {
            model: Some("gpt-5.1".into()),
            ..Default::default()
        };

        let resolved = service
            .resolve_for_session(
                &base_config(),
                &HashMap::new(),
                Some("deep"),
                Some(&overrides),
            )
            .await
            .unwrap();

        assert_eq!(resolved.config.command, "kiro-cli");
        assert_eq!(resolved.config_options.get("mode").unwrap(), "standard");
        assert_eq!(resolved.config_options.get("model").unwrap(), "gpt-5.1");
        assert_eq!(
            resolved.config_options.get("reasoning_effort").unwrap(),
            "high"
        );
        assert!(resolved
            .applied_layers
            .contains(&"default_profile:default".to_string()));
        assert!(resolved
            .applied_layers
            .contains(&"specified_profile:deep".to_string()));
    }

    #[tokio::test]
    async fn keeps_independent_defaults_for_multiple_agent_types() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path().join("profiles.toml"));
        let codex = AgentProfile::new("codex-default", "Codex", "codex");
        let claude = AgentProfile::new("claude-default", "Claude", "claude");
        let service = AgentProfileService::new(store);
        service.upsert(codex).await.unwrap();
        service.upsert(claude).await.unwrap();
        service
            .set_default_for_agent("codex", Some("codex-default".into()))
            .await
            .unwrap();
        let document = service
            .set_default_for_agent("claude", Some("claude-default".into()))
            .await
            .unwrap();

        assert_eq!(
            document.default_profile_for_agent("codex"),
            Some("codex-default")
        );
        assert_eq!(
            document.default_profile_for_agent("claude"),
            Some("claude-default")
        );
        assert_eq!(
            service.list_agents().await.unwrap()[0].default_profile,
            Some("claude-default".into())
        );
    }

    #[tokio::test]
    async fn concurrent_profile_upserts_do_not_lose_updates() {
        let dir = tempfile::tempdir().unwrap();
        let service = Arc::new(AgentProfileService::new(ProfileStore::new(
            dir.path().join("profiles.toml"),
        )));
        let mut tasks = Vec::new();
        for index in 0..8 {
            let service = service.clone();
            tasks.push(tokio::spawn(async move {
                service
                    .upsert(AgentProfile::new(
                        format!("profile-{index}"),
                        format!("Profile {index}"),
                        "codex",
                    ))
                    .await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }

        let document = service.list().await.unwrap();
        assert_eq!(document.profiles.len(), 8);
    }

    #[tokio::test]
    async fn selected_non_default_agent_profile_uses_its_own_acp_command() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path().join("profiles.toml"));
        let mut claude = AgentProfile::new("claude-research", "Claude Research", "claude");
        claude.command = Some("claude-agent-acp".into());
        store
            .save_atomic(&AgentProfileDocument {
                default_profile: None,
                profiles: vec![claude],
                ..Default::default()
            })
            .await
            .unwrap();
        let service = AgentProfileService::new(store);

        let resolved = service
            .resolve_for_session(
                &base_config(),
                &HashMap::new(),
                Some("claude-research"),
                None,
            )
            .await
            .unwrap();

        assert_eq!(resolved.config.command, "claude-agent-acp");
        assert!(resolved.config.args.is_empty());
        assert!(resolved.config.command_explicit);
        assert_eq!(
            resolved
                .profile
                .expect("selected profile snapshot")
                .agent_type,
            "claude"
        );
    }

    #[tokio::test]
    async fn claude_profile_applies_model_and_effort_before_process_start() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path().join("profiles.toml"));
        let mut claude = AgentProfile::new("claude-deep", "Claude Deep", "claude");
        claude.command = Some("claude-agent-acp".into());
        claude.default_model = Some("claude-opus-4-6".into());
        claude.reasoning_effort = Some("high".into());
        store
            .save_atomic(&AgentProfileDocument {
                default_profile: None,
                profiles: vec![claude],
                ..Default::default()
            })
            .await
            .unwrap();
        let service = AgentProfileService::new(store);

        let resolved = service
            .resolve_for_session(&base_config(), &HashMap::new(), Some("claude-deep"), None)
            .await
            .unwrap();

        assert_eq!(
            resolved
                .config
                .env
                .get("ANTHROPIC_MODEL")
                .map(String::as_str),
            Some("claude-opus-4-6")
        );
        assert_eq!(
            resolved
                .config
                .env
                .get("CLAUDE_CODE_EFFORT_LEVEL")
                .map(String::as_str),
            Some("high")
        );
        assert!(!resolved.config_options.contains_key("model"));
        assert!(!resolved.config_options.contains_key("reasoning_effort"));
    }

    #[tokio::test]
    async fn claude_profile_keeps_unsupported_effort_for_strict_error_reporting() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path().join("profiles.toml"));
        let mut claude = AgentProfile::new("claude-off", "Claude Off", "claude");
        claude.reasoning_effort = Some("off".into());
        store
            .save_atomic(&AgentProfileDocument {
                default_profile: None,
                profiles: vec![claude],
                ..Default::default()
            })
            .await
            .unwrap();
        let service = AgentProfileService::new(store);

        let resolved = service
            .resolve_for_session(&base_config(), &HashMap::new(), Some("claude-off"), None)
            .await
            .unwrap();

        assert!(!resolved.config.env.contains_key("CLAUDE_CODE_EFFORT_LEVEL"));
        assert_eq!(
            resolved
                .config_options
                .get("reasoning_effort")
                .map(String::as_str),
            Some("off")
        );
    }

    #[test]
    fn startup_command_can_be_exact_command_line() {
        let mut config = base_config();
        apply_startup_command(&mut config, "opencode acp", &[]);

        assert_eq!(config.command, "opencode");
        assert_eq!(config.args, vec!["acp"]);
        assert!(config.command_explicit);
    }

    #[test]
    fn capability_schema_can_reflect_live_config_options() {
        let options = vec![crate::acp::protocol::ConfigOption {
            id: "model".into(),
            name: "Model".into(),
            description: Some("AI model selection".into()),
            category: Some("model".into()),
            option_type: "enum".into(),
            current_value: "gpt-5".into(),
            options: vec![crate::acp::protocol::ConfigOptionValue {
                value: "gpt-5".into(),
                name: "GPT-5".into(),
                description: None,
            }],
        }];

        let schema = AgentCapabilityResolver::config_schema_from_options("codex", &options);

        assert_eq!(schema.source, "agent-session-config-options");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].id, "model");
        assert_eq!(schema.fields[0].label, "Model");
        assert_eq!(schema.fields[0].kind, "enum");
        assert_eq!(schema.fields[0].options, vec!["gpt-5".to_string()]);
        assert!(schema.fields[0].dynamic);
    }

    #[test]
    fn config_field_serializes_legacy_and_gateway_schema_names() {
        let field = AgentConfigField {
            id: "reasoning_effort".into(),
            label: "Reasoning Effort".into(),
            kind: "select".into(),
            options: vec!["low".into(), "medium".into(), "high".into()],
            dynamic: true,
        };

        let value = serde_json::to_value(field).expect("serialize field");

        assert_eq!(value["id"], "reasoning_effort");
        assert_eq!(value["key"], "reasoning_effort");
        assert_eq!(value["kind"], "select");
        assert_eq!(value["type"], "select");
        assert_eq!(value["apply_after_start"], false);
        assert_eq!(value["options"], json_vec(["low", "medium", "high"]));
    }

    #[test]
    fn capability_schema_is_profile_derived() {
        let mut profile = AgentProfile::new("codex", "Codex", "codex");
        profile.default_model = Some("gpt-5".into());
        profile.reasoning_effort = Some("medium".into());
        profile
            .config_options
            .insert("approval_policy".into(), "never".into());
        let resolver = AgentCapabilityResolver::new(AgentProfileDocument {
            default_profile: Some("codex".into()),
            profiles: vec![profile],
            ..Default::default()
        });

        let schema = resolver.config_schema("codex");

        assert_eq!(schema.source, "profile-store-fallback");
        assert!(schema.fields.iter().any(|field| field.id == "model"
            && field.options == vec!["gpt-5".to_string()]
            && field.dynamic));
        assert!(schema
            .fields
            .iter()
            .any(|field| field.id == "approval_policy" && field.dynamic));
    }

    fn json_vec(values: [&str; 3]) -> serde_json::Value {
        serde_json::Value::Array(
            values
                .into_iter()
                .map(|value| serde_json::Value::String(value.to_string()))
                .collect(),
        )
    }
}
