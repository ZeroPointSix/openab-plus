use super::connection::{runtime_metadata_from_options, AcpConnection};
use super::pool;
use super::protocol::ConfigOption;
use crate::agent_profile::{
    overrides_for_persistence, AgentProfileService, ProfileSessionOverrides, RecoveryStrategy,
};
use crate::cli_config::atomic_write_private_sync;
use crate::config::AgentConfig;
use crate::provider::{api_key_env_name, base_url_env_name, resolve_env_secret_ref};
use crate::provider_store::ProviderStore;
use crate::session_event::{SessionEventBus, SessionEventKind, SessionStreamBus};
use crate::session_snapshot::{SessionRuntimeMetadata, SessionSnapshot, SessionStatus};
use crate::transcript::SessionTranscriptStore;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::warn;

type PoolHandle = Arc<pool::SessionPool>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ThreadProfilePolicy {
    timeout_secs: Option<u64>,
    recovery_strategy: RecoveryStrategy,
    profile_id: Option<String>,
    overrides: Option<ProfileSessionOverrides>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedThreadSession {
    pool_key: String,
    policy: ThreadProfilePolicy,
}

pub struct SessionPool {
    base_config: AgentConfig,
    max_sessions: usize,
    hung_threshold_secs: u64,
    default_config_options: HashMap<String, String>,
    profile_service: Arc<AgentProfileService>,
    provider_store: Arc<ProviderStore>,
    pools: RwLock<HashMap<String, PoolHandle>>,
    thread_pools: RwLock<HashMap<String, String>>,
    thread_policies: RwLock<HashMap<String, ThreadProfilePolicy>>,
    thread_gates: RwLock<HashMap<String, Arc<Mutex<()>>>>,
    session_events: SessionEventBus,
    session_stream: SessionStreamBus,
    transcripts: SessionTranscriptStore,
    snapshots: RwLock<HashMap<String, SessionSnapshot>>,
    external_base_url: Option<String>,
    thread_profile_path: PathBuf,
    thread_profile_flush_lock: Mutex<()>,
    #[cfg(any(test, feature = "test-support"))]
    config_options_for_test: RwLock<HashMap<String, Vec<ConfigOption>>>,
}

impl SessionPool {
    pub fn new(
        config: AgentConfig,
        max_sessions: usize,
        hung_threshold_secs: u64,
        default_config_options: HashMap<String, String>,
    ) -> Self {
        let system_pool = Arc::new(pool::SessionPool::new(
            clone_agent_config(&config),
            max_sessions,
            hung_threshold_secs,
            default_config_options.clone(),
        ));
        let mut pools = HashMap::new();
        pools.insert("system".to_string(), system_pool);
        let external_base_url = session_external_base_url_from_env();
        let transcript_capacity = SessionTranscriptStore::capacity_from_env();
        // Transcript retention is configurable independently from the existing
        // lifecycle event history. Keep the lifecycle/SSE replay buffer at its
        // established default rather than changing it with transcript tuning.
        let session_stream = SessionStreamBus::default();
        let session_events = SessionEventBus::new_with_stream(session_stream.clone());
        let transcripts = SessionTranscriptStore::new(transcript_capacity, session_stream.clone());
        let openab_dir = openab_data_dir();
        let _ = std::fs::create_dir_all(&openab_dir);
        let thread_profile_path = openab_dir.join("thread_profile_context.json");
        let persisted_sessions = load_thread_sessions(&thread_profile_path);
        let mut thread_pools = HashMap::new();
        let mut thread_policies = HashMap::new();
        for (thread_id, mut entry) in persisted_sessions {
            entry.policy = policy_for_persistence(&entry.policy);
            thread_pools.insert(thread_id.clone(), entry.pool_key);
            thread_policies.insert(thread_id, entry.policy);
        }
        Self {
            base_config: config,
            max_sessions,
            hung_threshold_secs,
            default_config_options,
            profile_service: Arc::new(AgentProfileService::from_env()),
            provider_store: Arc::new(ProviderStore::from_env()),
            pools: RwLock::new(pools),
            thread_pools: RwLock::new(thread_pools),
            thread_policies: RwLock::new(thread_policies),
            thread_gates: RwLock::new(HashMap::new()),
            session_events,
            session_stream,
            transcripts,
            snapshots: RwLock::new(HashMap::new()),
            external_base_url,
            thread_profile_path,
            thread_profile_flush_lock: Mutex::new(()),
            #[cfg(any(test, feature = "test-support"))]
            config_options_for_test: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_agent_profile_service(mut self, profile_service: Arc<AgentProfileService>) -> Self {
        self.profile_service = profile_service;
        self
    }

    pub fn with_provider_store(mut self, provider_store: Arc<ProviderStore>) -> Self {
        self.provider_store = provider_store;
        self
    }

    pub fn profile_service(&self) -> Arc<AgentProfileService> {
        self.profile_service.clone()
    }

    pub fn session_event_bus(&self) -> SessionEventBus {
        self.session_events.clone()
    }

    /// Unified, read-only cursor source for status and transcript SSE events.
    pub fn session_stream_bus(&self) -> SessionStreamBus {
        self.session_stream.clone()
    }

    /// Independent per-session ring buffers for ACP transcript data.
    pub fn transcript_store(&self) -> SessionTranscriptStore {
        self.transcripts.clone()
    }

    /// Seed a session snapshot and emit `session.created` for integration tests.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn seed_session_snapshot_for_test(&self, snapshot: SessionSnapshot) {
        self.record_session_created(snapshot).await;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn seed_config_options_for_test(&self, thread_id: &str, options: Vec<ConfigOption>) {
        self.config_options_for_test
            .write()
            .await
            .insert(thread_id.to_string(), options);
    }

    pub async fn list_session_snapshots(&self) -> Vec<SessionSnapshot> {
        let mut snapshots: Vec<_> = self.snapshots.read().await.values().cloned().collect();
        snapshots.sort_by_key(|snapshot| std::cmp::Reverse(snapshot.updated_at));
        snapshots
    }

    pub async fn session_snapshot(&self, session_id: &str) -> Option<SessionSnapshot> {
        self.snapshots.read().await.get(session_id).cloned()
    }

    pub async fn get_or_create(
        &self,
        thread_id: &str,
        working_dir_override: Option<&str>,
    ) -> Result<bool> {
        self.get_or_create_with_profile(thread_id, working_dir_override, None, None)
            .await
    }

    pub async fn get_or_create_with_profile(
        &self,
        thread_id: &str,
        working_dir_override: Option<&str>,
        profile_id: Option<&str>,
        overrides: Option<&ProfileSessionOverrides>,
    ) -> Result<bool> {
        self.get_or_create_with_profile_and_source(
            thread_id,
            working_dir_override,
            profile_id,
            overrides,
            None,
        )
        .await
    }

    pub async fn get_or_create_with_profile_and_source(
        &self,
        thread_id: &str,
        working_dir_override: Option<&str>,
        profile_id: Option<&str>,
        overrides: Option<&ProfileSessionOverrides>,
        source_permalink: Option<&str>,
    ) -> Result<bool> {
        let gate = self.thread_gate(thread_id).await;
        let _guard = gate.lock().await;

        let (effective_profile_id, effective_overrides) = self
            .effective_profile_context(thread_id, profile_id, overrides)
            .await;

        if let Some(pool) = self.existing_pool(thread_id).await {
            if pool.has_live_active_connection(thread_id).await {
                let result = pool.get_or_create(thread_id, working_dir_override).await;
                return match result {
                    Ok(outcome) => {
                        self.backfill_source_permalink(thread_id, source_permalink)
                            .await;
                        Ok(self.apply_ensure_outcome(thread_id, outcome).await)
                    }
                    Err(err) => {
                        self.mark_session_error(thread_id, err.to_string()).await;
                        Err(err)
                    }
                };
            }
        }

        let resolved = self
            .profile_service
            .resolve_for_session(
                &self.base_config,
                &self.default_config_options,
                effective_profile_id.as_deref(),
                effective_overrides.as_ref(),
            )
            .await?;
        let mut resolved = resolved;
        let model_for_metadata = configured_model(&resolved.config, &resolved.config_options);
        let reasoning_for_metadata =
            configured_reasoning_effort(&resolved.config, &resolved.config_options);
        let renderer_agent = resolved
            .profile
            .as_ref()
            .map(|profile| profile.agent_type.clone())
            .filter(|agent| crate::cli_config::supports_file_renderer(agent));
        let apply_lock = match renderer_agent.as_deref() {
            Some(agent) => Some(crate::cli_config::lock_for(agent).await),
            None => None,
        };
        let _apply_guard = match apply_lock.as_ref() {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };

        if let Some(profile) = resolved.profile.clone() {
            if let Some(provider_id) = profile.provider.as_deref() {
                let provider = self
                    .provider_store
                    .get(provider_id)
                    .await?
                    .ok_or_else(|| anyhow!("provider {provider_id} not found"))?;
                let api_key_env = api_key_env_name(&provider.provider_type).to_string();
                let base_url_env = base_url_env_name(&provider.provider_type).to_string();
                let api_key_value = resolve_env_secret_ref(&provider.api_key_ref)?;
                resolved
                    .config
                    .env
                    .insert(api_key_env.clone(), api_key_value);
                if let Some(base_url) = provider
                    .base_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    resolved
                        .config
                        .env
                        .insert(base_url_env, base_url.to_string());
                }
            }

            if crate::cli_config::supports_file_renderer(&profile.agent_type) {
                let model = configured_model(&resolved.config, &resolved.config_options);
                let reasoning_effort =
                    configured_reasoning_effort(&resolved.config, &resolved.config_options);
                let mut request = crate::cli_config::ApplyRequest {
                    agent_type: profile.agent_type.clone(),
                    model,
                    reasoning_effort,
                    provider_id: profile.provider.clone(),
                    provider_type: None,
                    base_url: None,
                    api_key_env: None,
                };
                if let Some(provider_id) = profile.provider.as_deref() {
                    let provider = self
                        .provider_store
                        .get(provider_id)
                        .await?
                        .ok_or_else(|| anyhow!("provider {provider_id} not found"))?;
                    let api_key_env = api_key_env_name(&provider.provider_type).to_string();
                    request.provider_type = Some(provider.provider_type.clone());
                    request.api_key_env = Some(api_key_env);
                    if let Some(base_url) = provider
                        .base_url
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        request.base_url = Some(base_url.to_string());
                    }
                }
                // Native CLI files only; already-running sessions are not hot-reloaded.
                crate::cli_config::apply_unlocked(&request).await?;
                // File renderer owns model/thinking for codex/claude.
                resolved.config_options.remove("model");
                resolved.config_options.remove("reasoning_effort");
            }
        }
        let configured_metadata =
            SessionRuntimeMetadata::configured(model_for_metadata, reasoning_for_metadata);
        let applied_provider = resolved
            .profile
            .as_ref()
            .and_then(|profile| profile.provider.clone());
        let pool_key = if resolved.profile.is_none() && effective_overrides.is_none() {
            "system".to_string()
        } else {
            resolved.pool_key.clone()
        };
        let profile_id = resolved.profile.as_ref().map(|profile| profile.id.clone());
        let profile_name = resolved
            .profile
            .as_ref()
            .map(|profile| profile.name.clone());
        let workdir = working_dir_override
            .unwrap_or(&resolved.config.working_dir)
            .to_string();
        let policy = ThreadProfilePolicy {
            timeout_secs: resolved.timeout_secs,
            recovery_strategy: resolved.recovery_strategy.clone(),
            profile_id: effective_profile_id.clone(),
            overrides: effective_overrides.clone(),
        };
        let pool = self
            .pool_for_key(
                &pool_key,
                resolved.config,
                resolved.config_options,
                policy.clone(),
            )
            .await;

        // Hold the per-agent apply lock through spawn so concurrent sessions cannot
        // interleave CLI file writes with process start. New sessions then read the
        // files written above; live processes are not notified.
        let result = pool.get_or_create(thread_id, working_dir_override).await;
        match result {
            Ok(outcome) => {
                self.record_thread_session(thread_id, &pool_key, policy)
                    .await;
                let profile_config_errors = outcome.profile_config_errors.clone();
                let runtime_metadata =
                    merge_runtime_metadata(outcome.runtime_metadata.clone(), configured_metadata);
                let created = self.apply_ensure_outcome(thread_id, outcome).await;
                if created {
                    let mut snapshot = SessionSnapshot::new(
                        thread_id.to_string(),
                        String::new(),
                        workdir,
                        profile_id,
                        profile_name,
                        None,
                        self.external_base_url.as_deref(),
                    );
                    snapshot.replace_runtime_metadata(runtime_metadata);
                    snapshot.set_applied_provider(applied_provider.clone());
                    snapshot.set_source_permalink(source_permalink);
                    if !profile_config_errors.is_empty() {
                        snapshot.set_profile_config_errors(profile_config_errors);
                    }
                    self.record_session_created(snapshot).await;
                } else if applied_provider.is_some() {
                    self.update_snapshot(thread_id, SessionEventKind::ConfigChanged, |snapshot| {
                        snapshot.set_applied_provider(applied_provider.clone());
                    })
                    .await;
                }
                self.sync_pool_snapshot_statuses(&pool_key, &pool).await;
                Ok(created)
            }
            Err(err) => {
                self.mark_session_error(thread_id, err.to_string()).await;
                Err(err)
            }
        }
    }

    pub async fn has_active_session(&self, thread_id: &str) -> bool {
        if let Some(pool) = self.existing_pool(thread_id).await {
            return pool.has_active_session(thread_id).await;
        }
        for (_, pool) in self.pools_snapshot().await {
            if pool.has_active_session(thread_id).await {
                return true;
            }
        }
        false
    }

    pub async fn with_connection<F, R>(&self, thread_id: &str, f: F) -> Result<R>
    where
        F: for<'a> FnOnce(
            &'a mut AcpConnection,
        ) -> Pin<Box<dyn Future<Output = Result<R>> + Send + 'a>>,
    {
        let pool = self
            .existing_pool(thread_id)
            .await
            .ok_or_else(|| anyhow!("no connection for thread {thread_id}"))?;
        pool.with_connection(thread_id, f).await
    }

    pub async fn get_config_options(&self, thread_id: &str) -> Vec<ConfigOption> {
        #[cfg(any(test, feature = "test-support"))]
        {
            if let Some(options) = self
                .config_options_for_test
                .read()
                .await
                .get(thread_id)
                .cloned()
            {
                return options;
            }
        }

        match self.existing_pool(thread_id).await {
            Some(pool) => pool.get_config_options(thread_id).await,
            None => Vec::new(),
        }
    }

    pub async fn config_schema_for_agent(
        &self,
        agent_type: &str,
    ) -> Option<crate::agent_profile::AgentConfigSchema> {
        for snapshot in self
            .list_session_snapshots()
            .await
            .into_iter()
            .filter(|snapshot| {
                snapshot.agent == agent_type
                    && matches!(
                        snapshot.status,
                        SessionStatus::Starting | SessionStatus::Idle | SessionStatus::Running
                    )
            })
        {
            if let Some(schema) = self
                .config_schema_for_thread(&snapshot.session_id, agent_type)
                .await
            {
                return Some(schema);
            }
        }

        None
    }

    pub async fn config_schema_for_thread(
        &self,
        thread_id: &str,
        agent_type: &str,
    ) -> Option<crate::agent_profile::AgentConfigSchema> {
        let options = self.get_config_options(thread_id).await;
        if options.is_empty() {
            None
        } else {
            Some(
                crate::agent_profile::AgentCapabilityResolver::config_schema_from_options(
                    agent_type, &options,
                ),
            )
        }
    }

    pub async fn set_config_option(
        &self,
        thread_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<Vec<ConfigOption>> {
        let pool = self
            .existing_pool(thread_id)
            .await
            .ok_or_else(|| anyhow!("no connection for thread {thread_id}"))?;
        match pool.set_config_option(thread_id, config_id, value).await {
            Ok(options) => {
                let runtime_metadata = pool.runtime_metadata(thread_id).await.unwrap_or_default();
                self.update_snapshot(thread_id, SessionEventKind::ConfigChanged, |snapshot| {
                    snapshot.replace_runtime_metadata(runtime_metadata);
                })
                .await;
                Ok(options)
            }
            Err(err) => {
                self.mark_session_error(thread_id, err.to_string()).await;
                Err(err)
            }
        }
    }

    pub async fn get_usage(&self, thread_id: &str) -> Result<crate::acp::protocol::UsageReport> {
        let pool = self
            .existing_pool(thread_id)
            .await
            .ok_or_else(|| anyhow!("no connection for thread {thread_id}"))?;
        pool.get_usage(thread_id).await
    }

    pub async fn cancel_session(&self, thread_id: &str) -> Result<()> {
        if let Some(pool) = self.existing_pool(thread_id).await {
            return pool.cancel_session(thread_id).await;
        }
        for (_, pool) in self.pools_snapshot().await {
            if pool.cancel_session(thread_id).await.is_ok() {
                return Ok(());
            }
        }
        Err(anyhow!("no session for thread {thread_id}"))
    }

    pub async fn reset_session(&self, thread_id: &str) -> Result<()> {
        if let Some(pool) = self.existing_pool(thread_id).await {
            let result = pool.reset_session(thread_id).await;
            if result.is_ok() {
                self.clear_thread_session(thread_id).await;
                self.mark_session_exited(thread_id, None).await;
            }
            return result;
        }
        for (_, pool) in self.pools_snapshot().await {
            if pool.reset_session(thread_id).await.is_ok() {
                self.clear_thread_session(thread_id).await;
                self.mark_session_exited(thread_id, None).await;
                return Ok(());
            }
        }
        Err(anyhow!("no session for thread {thread_id}"))
    }

    pub async fn cleanup_idle(&self, ttl_secs: u64) {
        for (pool_key, pool) in self.pools_snapshot().await {
            let failures = pool.cleanup_idle(ttl_secs).await;
            for failure in failures {
                self.clear_thread_session(&failure.thread_id).await;
                self.mark_session_error(&failure.thread_id, failure.error)
                    .await;
            }
            self.sync_pool_snapshot_statuses(&pool_key, &pool).await;
        }
    }

    pub async fn shutdown(&self) {
        for (_, pool) in self.pools_snapshot().await {
            pool.shutdown().await;
        }
        let session_ids: Vec<_> = self.snapshots.read().await.keys().cloned().collect();
        for session_id in session_ids {
            self.mark_session_exited(&session_id, None).await;
        }
    }

    pub async fn mark_session_status(&self, thread_id: &str, status: SessionStatus) {
        self.update_snapshot(thread_id, SessionEventKind::StatusChanged, |snapshot| {
            snapshot.set_status(status);
        })
        .await;
    }

    pub async fn mark_session_error(&self, thread_id: &str, error: impl Into<String>) {
        let error = error.into();
        self.update_snapshot(thread_id, SessionEventKind::Error, |snapshot| {
            snapshot.set_error(error);
        })
        .await;
    }

    pub async fn mark_session_exited(&self, thread_id: &str, error: Option<String>) {
        if let Some(error) = error {
            if self
                .thread_policy(thread_id)
                .await
                .is_some_and(|policy| matches!(policy.recovery_strategy, RecoveryStrategy::None))
            {
                self.mark_session_error(thread_id, error).await;
                return;
            }
            self.update_snapshot(thread_id, SessionEventKind::Exited, |snapshot| {
                snapshot.set_exited(Some(error));
            })
            .await;
            return;
        }

        self.update_snapshot(thread_id, SessionEventKind::Exited, |snapshot| {
            snapshot.set_exited(None);
        })
        .await;
    }

    pub async fn record_session_config_update(&self, thread_id: &str, options: &[ConfigOption]) {
        let runtime_metadata = runtime_metadata_from_options(None, options);
        self.update_snapshot(thread_id, SessionEventKind::ConfigChanged, |snapshot| {
            snapshot.update_runtime_config_metadata(runtime_metadata);
        })
        .await;
    }

    pub async fn mark_profile_deleted(&self, profile_id: &str) {
        let updated: Vec<SessionSnapshot> = {
            let mut snapshots = self.snapshots.write().await;
            snapshots
                .values_mut()
                .filter_map(|snapshot| {
                    if snapshot.mark_profile_deleted(profile_id) {
                        Some(snapshot.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };

        for snapshot in updated {
            self.session_events
                .publish(SessionEventKind::ProfileChanged, snapshot);
        }
    }

    async fn record_session_created(&self, snapshot: SessionSnapshot) {
        self.snapshots
            .write()
            .await
            .insert(snapshot.session_id.clone(), snapshot.clone());
        self.session_events
            .publish(SessionEventKind::SessionCreated, snapshot);
    }

    async fn backfill_source_permalink(&self, thread_id: &str, permalink: Option<&str>) {
        let snapshot = {
            let mut snapshots = self.snapshots.write().await;
            let Some(snapshot) = snapshots.get_mut(thread_id) else {
                return;
            };
            if !snapshot.set_source_permalink(permalink) {
                return;
            }
            snapshot.clone()
        };
        self.session_events
            .publish(SessionEventKind::SourceChanged, snapshot);
    }

    async fn update_snapshot<F>(&self, thread_id: &str, kind: SessionEventKind, apply: F)
    where
        F: FnOnce(&mut SessionSnapshot),
    {
        let snapshot = {
            let mut snapshots = self.snapshots.write().await;
            let Some(snapshot) = snapshots.get_mut(thread_id) else {
                return;
            };
            apply(snapshot);
            snapshot.clone()
        };
        self.session_events.publish(kind, snapshot);
    }

    async fn apply_ensure_outcome(
        &self,
        thread_id: &str,
        outcome: pool::SessionEnsureOutcome,
    ) -> bool {
        let created = outcome.created;
        let recovered = outcome.recovered;
        let profile_config_errors = outcome.profile_config_errors;
        let runtime_metadata = outcome.runtime_metadata;

        if recovered {
            self.update_snapshot(
                thread_id,
                SessionEventKind::StatusChanged,
                move |snapshot| {
                    snapshot.set_status(SessionStatus::Idle);
                    snapshot.replace_runtime_metadata(runtime_metadata);
                    if !profile_config_errors.is_empty() {
                        snapshot.set_profile_config_errors(profile_config_errors);
                    }
                },
            )
            .await;
        } else if !profile_config_errors.is_empty() {
            self.update_snapshot(
                thread_id,
                SessionEventKind::ConfigChanged,
                move |snapshot| {
                    snapshot.set_profile_config_errors(profile_config_errors);
                },
            )
            .await;
        }

        created
    }

    async fn thread_policy(&self, thread_id: &str) -> Option<ThreadProfilePolicy> {
        self.thread_policies.read().await.get(thread_id).cloned()
    }

    /// Reuse the session's original profile selection when follow-up messages do not
    /// repeat [[profile:...]] directives (suspended/recovery paths).
    async fn effective_profile_context(
        &self,
        thread_id: &str,
        profile_id: Option<&str>,
        overrides: Option<&ProfileSessionOverrides>,
    ) -> (Option<String>, Option<ProfileSessionOverrides>) {
        let (stored_profile_id, stored_overrides) = self.stored_profile_context(thread_id).await;

        if profile_id.is_none() && overrides.is_none() {
            return (stored_profile_id, stored_overrides);
        }

        let effective_profile_id = profile_id.map(str::to_string).or(stored_profile_id);

        let effective_overrides = match (overrides, stored_overrides) {
            (Some(request), Some(stored)) => Some(merge_profile_overrides(&stored, request)),
            (Some(request), None) => Some(request.clone()),
            (None, Some(stored)) => Some(stored),
            (None, None) => None,
        };

        (effective_profile_id, effective_overrides)
    }

    async fn stored_profile_context(
        &self,
        thread_id: &str,
    ) -> (Option<String>, Option<ProfileSessionOverrides>) {
        if let Some(policy) = self.thread_policies.read().await.get(thread_id) {
            if policy.profile_id.is_some() || policy.overrides.is_some() {
                return (policy.profile_id.clone(), policy.overrides.clone());
            }
        }

        if let Some(snapshot) = self.snapshots.read().await.get(thread_id) {
            let snapshot_overrides = profile_overrides_from_snapshot(snapshot);
            if snapshot.profile_id.is_some() || snapshot_overrides.is_some() {
                return (snapshot.profile_id.clone(), snapshot_overrides);
            }
        }

        (None, None)
    }

    async fn record_thread_session(
        &self,
        thread_id: &str,
        pool_key: &str,
        policy: ThreadProfilePolicy,
    ) {
        self.thread_pools
            .write()
            .await
            .insert(thread_id.to_string(), pool_key.to_string());
        self.thread_policies
            .write()
            .await
            .insert(thread_id.to_string(), policy);
        self.flush_thread_sessions().await;
    }

    async fn clear_thread_session(&self, thread_id: &str) {
        self.thread_pools.write().await.remove(thread_id);
        self.thread_policies.write().await.remove(thread_id);
        self.flush_thread_sessions().await;
    }

    async fn flush_thread_sessions(&self) {
        let _guard = self.thread_profile_flush_lock.lock().await;
        let sessions = {
            let pools = self.thread_pools.read().await;
            let policies = self.thread_policies.read().await;
            pools
                .iter()
                .filter_map(|(thread_id, pool_key)| {
                    policies.get(thread_id).map(|policy| {
                        (
                            thread_id.clone(),
                            PersistedThreadSession {
                                pool_key: pool_key.clone(),
                                policy: policy_for_persistence(policy),
                            },
                        )
                    })
                })
                .collect::<HashMap<_, _>>()
        };
        let path = self.thread_profile_path.clone();
        let _ = tokio::task::spawn_blocking(move || save_thread_sessions(&path, &sessions)).await;
    }

    async fn thread_gate(&self, thread_id: &str) -> Arc<Mutex<()>> {
        let mut gates = self.thread_gates.write().await;
        gates
            .entry(thread_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn sync_pool_snapshot_statuses(&self, pool_key: &str, pool: &PoolHandle) {
        let session_ids: Vec<String> = {
            let mappings = self.thread_pools.read().await;
            let snapshots = self.snapshots.read().await;
            snapshots
                .keys()
                .filter(|session_id| {
                    mappings
                        .get(*session_id)
                        .is_some_and(|mapped_pool| mapped_pool == pool_key)
                })
                .cloned()
                .collect()
        };

        for session_id in session_ids {
            match pool.session_entry_status(&session_id).await {
                pool::SessionEntryStatus::Active => {}
                pool::SessionEntryStatus::Suspended => {
                    self.mark_session_status(&session_id, SessionStatus::Suspended)
                        .await;
                }
                pool::SessionEntryStatus::Missing => {
                    self.clear_thread_session(&session_id).await;
                    self.mark_session_exited(&session_id, None).await;
                }
            }
        }
    }

    async fn existing_pool(&self, thread_id: &str) -> Option<PoolHandle> {
        let pool_key = {
            let mapping = self.thread_pools.read().await;
            mapping.get(thread_id).cloned()
        }?;
        let pool = {
            let pools = self.pools.read().await;
            pools.get(&pool_key).cloned()
        }?;
        if pool.has_active_session(thread_id).await {
            Some(pool)
        } else {
            self.clear_thread_session(thread_id).await;
            None
        }
    }

    async fn pool_for_key(
        &self,
        key: &str,
        config: AgentConfig,
        config_options: HashMap<String, String>,
        policy: ThreadProfilePolicy,
    ) -> PoolHandle {
        if let Some(pool) = self.pools.read().await.get(key).cloned() {
            return pool;
        }
        let mut pools = self.pools.write().await;
        pools
            .entry(key.to_string())
            .or_insert_with(|| {
                Arc::new(pool::SessionPool::new_with_policy(
                    config,
                    self.max_sessions,
                    self.hung_threshold_secs,
                    config_options,
                    policy.timeout_secs,
                    policy.recovery_strategy.clone(),
                ))
            })
            .clone()
    }

    async fn pools_snapshot(&self) -> Vec<(String, PoolHandle)> {
        self.pools
            .read()
            .await
            .iter()
            .map(|(key, pool)| (key.clone(), pool.clone()))
            .collect()
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

fn configured_model(
    config: &AgentConfig,
    config_options: &HashMap<String, String>,
) -> Option<String> {
    config_options
        .get("model")
        .and_then(|value| non_empty_string(value))
        .or_else(|| model_from_agent_env(config))
}

fn configured_reasoning_effort(
    config: &AgentConfig,
    config_options: &HashMap<String, String>,
) -> Option<String> {
    config_options
        .get("reasoning_effort")
        .and_then(|value| non_empty_string(value))
        .or_else(|| agent_env_value(config, "CLAUDE_CODE_EFFORT_LEVEL"))
}

fn merge_runtime_metadata(
    mut runtime: SessionRuntimeMetadata,
    configured: SessionRuntimeMetadata,
) -> SessionRuntimeMetadata {
    let mut used_configured = false;
    if runtime.model.is_none() {
        runtime.model = configured.model;
        used_configured |= runtime.model.is_some();
    }
    if runtime.reasoning_effort.is_none() {
        runtime.reasoning_effort = configured.reasoning_effort;
        used_configured |= runtime.reasoning_effort.is_some();
    }
    if used_configured {
        runtime.metadata_source = configured.metadata_source;
    }
    runtime
}

fn model_from_agent_env(config: &AgentConfig) -> Option<String> {
    for key in ["ANTHROPIC_MODEL", "ANTHROPIC_DEFAULT_MODEL", "CLAUDE_MODEL"] {
        if let Some(value) = agent_env_value(config, key) {
            return Some(value);
        }
    }

    agent_env_value(config, "CLAUDE_MODEL_CONFIG")
        .and_then(|value| model_from_claude_model_config(&value))
}

fn agent_env_value(config: &AgentConfig, key: &str) -> Option<String> {
    config
        .env
        .get(key)
        .and_then(|value| non_empty_string(value))
        .or_else(|| {
            config
                .inherit_env
                .iter()
                .any(|inherited| inherited == key)
                .then(|| env::var(key).ok())
                .flatten()
                .and_then(|value| non_empty_string(&value))
        })
}

fn model_from_claude_model_config(value: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(value).ok()?;
    let models = parsed.get("availableModels")?.as_array()?;

    models.iter().find_map(|model| match model {
        serde_json::Value::String(value) => non_empty_string(value),
        serde_json::Value::Object(model) => ["modelId", "id", "value"]
            .iter()
            .find_map(|key| model.get(*key).and_then(|value| value.as_str()))
            .and_then(non_empty_string),
        _ => None,
    })
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn profile_overrides_from_snapshot(snapshot: &SessionSnapshot) -> Option<ProfileSessionOverrides> {
    let overrides = ProfileSessionOverrides {
        model: snapshot.model.clone(),
        reasoning_effort: snapshot.reasoning_effort.clone(),
        ..ProfileSessionOverrides::default()
    };
    let has_overrides = overrides.model.is_some() || overrides.reasoning_effort.is_some();
    has_overrides.then_some(overrides)
}

fn merge_profile_overrides(
    base: &ProfileSessionOverrides,
    patch: &ProfileSessionOverrides,
) -> ProfileSessionOverrides {
    let mut merged = base.clone();
    if let Some(command) = patch.command.as_ref() {
        merged.command = Some(command.clone());
    }
    if !patch.args.is_empty() {
        merged.args = patch.args.clone();
    }
    if let Some(working_dir) = patch.working_dir.as_ref() {
        merged.working_dir = Some(working_dir.clone());
    }
    if let Some(model) = patch.model.as_ref() {
        merged.model = Some(model.clone());
    }
    if let Some(reasoning_effort) = patch.reasoning_effort.as_ref() {
        merged.reasoning_effort = Some(reasoning_effort.clone());
    }
    for (key, value) in &patch.env {
        merged.env.insert(key.clone(), value.clone());
    }
    for (key, value) in &patch.config_options {
        merged.config_options.insert(key.clone(), value.clone());
    }
    merged
}

fn policy_for_persistence(policy: &ThreadProfilePolicy) -> ThreadProfilePolicy {
    ThreadProfilePolicy {
        timeout_secs: policy.timeout_secs,
        recovery_strategy: policy.recovery_strategy.clone(),
        profile_id: policy.profile_id.clone(),
        overrides: policy
            .overrides
            .as_ref()
            .and_then(overrides_for_persistence),
    }
}

fn openab_data_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .join(".openab")
}

fn load_thread_sessions(path: &Path) -> HashMap<String, PersistedThreadSession> {
    let sessions: HashMap<String, PersistedThreadSession> = match std::fs::read_to_string(path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_else(|e| {
            warn!(path = %path.display(), error = %e, "corrupt thread profile context, starting fresh");
            HashMap::new()
        }),
        Err(_) => HashMap::new(),
    };
    sessions
        .into_iter()
        .map(|(thread_id, mut entry)| {
            entry.policy = policy_for_persistence(&entry.policy);
            (thread_id, entry)
        })
        .collect()
}

fn save_thread_sessions(path: &Path, sessions: &HashMap<String, PersistedThreadSession>) {
    let data = match serde_json::to_string_pretty(sessions) {
        Ok(data) => data,
        Err(e) => {
            warn!(error = %e, "failed to serialize thread profile context");
            return;
        }
    };
    if let Err(e) = atomic_write_private_sync(path, data.as_bytes()) {
        warn!(path = %path.display(), error = %e, "failed to persist thread profile context");
    }
}

fn session_external_base_url_from_env() -> Option<String> {
    [
        "OPENAB_SESSION_PUBLIC_BASE_URL",
        "OPENAB_PUBLIC_BASE_URL",
        "GATEWAY_PUBLIC_URL",
        "PUBLIC_BASE_URL",
    ]
    .into_iter()
    .filter_map(|key| env::var(key).ok())
    .map(|value| value.trim().to_string())
    .find(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::super::protocol::ConfigOptionValue;
    use super::*;
    use crate::session_snapshot::{
        ProfileConfigError, ProfileStatus, SessionMetadataSource, SessionRuntimeMetadata,
    };

    fn config_option(id: &str, current_value: &str) -> ConfigOption {
        ConfigOption {
            id: id.into(),
            name: id.into(),
            description: None,
            category: None,
            option_type: "string".into(),
            current_value: current_value.into(),
            options: Vec::<ConfigOptionValue>::new(),
        }
    }

    fn enum_config_option(
        id: &str,
        name: &str,
        current_value: &str,
        values: &[&str],
    ) -> ConfigOption {
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

    #[test]
    fn extracts_runtime_metadata_from_config_options() {
        let options = vec![
            config_option("mode", "default"),
            config_option("model", "gpt-5"),
            config_option("reasoning_effort", "high"),
        ];

        let metadata = runtime_metadata_from_options(Some("Codex ACP"), &options);
        assert_eq!(metadata.agent.as_deref(), Some("Codex ACP"));
        assert_eq!(metadata.model.as_deref(), Some("gpt-5"));
        assert_eq!(metadata.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(metadata.metadata_source, Some(SessionMetadataSource::Acp));
    }

    #[test]
    fn configured_model_prefers_runtime_option_over_agent_env() {
        let mut config = AgentConfig::default();
        config.env.insert(
            "ANTHROPIC_MODEL".into(),
            "deepseek/deepseek-v4-flash".into(),
        );
        let options = HashMap::from([("model".into(), "claude-sonnet-4".into())]);

        assert_eq!(
            configured_model(&config, &options).as_deref(),
            Some("claude-sonnet-4")
        );
    }

    #[test]
    fn configured_model_falls_back_to_anthropic_agent_env() {
        let mut config = AgentConfig::default();
        config.env.insert(
            "ANTHROPIC_MODEL".into(),
            " deepseek/deepseek-v4-flash ".into(),
        );

        assert_eq!(
            configured_model(&config, &HashMap::new()).as_deref(),
            Some("deepseek/deepseek-v4-flash")
        );
    }

    #[test]
    fn configured_model_falls_back_to_claude_model_config() {
        let mut config = AgentConfig::default();
        config.env.insert(
            "CLAUDE_MODEL_CONFIG".into(),
            r#"{"availableModels":["deepseek/deepseek-v4-flash"]}"#.into(),
        );

        assert_eq!(
            configured_model(&config, &HashMap::new()).as_deref(),
            Some("deepseek/deepseek-v4-flash")
        );
    }

    #[test]
    fn configured_reasoning_effort_falls_back_to_claude_startup_env() {
        let mut config = AgentConfig::default();
        config
            .env
            .insert("CLAUDE_CODE_EFFORT_LEVEL".into(), " high ".into());

        assert_eq!(
            configured_reasoning_effort(&config, &HashMap::new()).as_deref(),
            Some("high")
        );
    }

    #[test]
    fn configured_values_fill_missing_acp_metadata() {
        let runtime = SessionRuntimeMetadata::acp(Some("claude-agent-acp".into()), None, None);
        let configured =
            SessionRuntimeMetadata::configured(Some("claude-opus-4-6".into()), Some("high".into()));

        let merged = merge_runtime_metadata(runtime, configured);

        assert_eq!(merged.agent.as_deref(), Some("claude-agent-acp"));
        assert_eq!(merged.model.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(merged.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(
            merged.metadata_source,
            Some(SessionMetadataSource::Configured)
        );
    }

    #[tokio::test]
    async fn effective_profile_context_reuses_thread_policy_without_request_directives() {
        let outer = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());
        outer.thread_policies.write().await.insert(
            "slack:thread".into(),
            ThreadProfilePolicy {
                profile_id: Some("deep".into()),
                overrides: Some(ProfileSessionOverrides {
                    model: Some("gpt-5.1".into()),
                    reasoning_effort: Some("high".into()),
                    ..ProfileSessionOverrides::default()
                }),
                ..ThreadProfilePolicy::default()
            },
        );

        let (profile_id, overrides) = outer
            .effective_profile_context("slack:thread", None, None)
            .await;
        let overrides = overrides.expect("expected stored overrides");

        assert_eq!(profile_id.as_deref(), Some("deep"));
        assert_eq!(overrides.model.as_deref(), Some("gpt-5.1"));
        assert_eq!(overrides.reasoning_effort.as_deref(), Some("high"));
    }

    #[tokio::test]
    async fn effective_profile_context_prefers_explicit_request_directives() {
        let outer = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());
        outer.thread_policies.write().await.insert(
            "slack:thread".into(),
            ThreadProfilePolicy {
                profile_id: Some("deep".into()),
                ..ThreadProfilePolicy::default()
            },
        );
        let request_overrides = ProfileSessionOverrides {
            model: Some("claude-opus-4".into()),
            ..ProfileSessionOverrides::default()
        };

        let (profile_id, overrides) = outer
            .effective_profile_context("slack:thread", Some("other"), Some(&request_overrides))
            .await;

        assert_eq!(profile_id.as_deref(), Some("other"));
        assert_eq!(
            overrides.and_then(|value| value.model),
            Some("claude-opus-4".into())
        );
    }

    #[test]
    fn configured_metadata_captured_before_file_renderer_strips_options() {
        let config = AgentConfig::default();
        let mut config_options = HashMap::from([
            ("model".into(), "gpt-5".into()),
            ("reasoning_effort".into(), "high".into()),
        ]);
        let metadata = SessionRuntimeMetadata::configured(
            configured_model(&config, &config_options),
            configured_reasoning_effort(&config, &config_options),
        );
        config_options.remove("model");
        config_options.remove("reasoning_effort");

        assert_eq!(metadata.model.as_deref(), Some("gpt-5"));
        assert_eq!(metadata.reasoning_effort.as_deref(), Some("high"));
        assert!(!config_options.contains_key("model"));
    }

    #[tokio::test]
    async fn effective_profile_context_merges_partial_override_with_stored_profile() {
        let outer = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());
        outer.thread_policies.write().await.insert(
            "slack:thread".into(),
            ThreadProfilePolicy {
                profile_id: Some("deep".into()),
                ..ThreadProfilePolicy::default()
            },
        );
        let request_overrides = ProfileSessionOverrides {
            model: Some("gpt-5.1".into()),
            ..ProfileSessionOverrides::default()
        };

        let (profile_id, overrides) = outer
            .effective_profile_context("slack:thread", None, Some(&request_overrides))
            .await;
        let overrides = overrides.expect("expected merged overrides");

        assert_eq!(profile_id.as_deref(), Some("deep"));
        assert_eq!(overrides.model.as_deref(), Some("gpt-5.1"));
    }

    #[tokio::test]
    async fn thread_profile_context_persists_across_pool_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _home_guard = HomeEnvGuard::set(dir.path());

        {
            let outer = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());
            outer
                .record_thread_session(
                    "slack:thread",
                    "profile:deep",
                    ThreadProfilePolicy {
                        profile_id: Some("deep".into()),
                        overrides: Some(ProfileSessionOverrides {
                            model: Some("gpt-5.1".into()),
                            reasoning_effort: Some("high".into()),
                            env: HashMap::from([("SECRET".into(), "plain-text".into())]),
                            ..ProfileSessionOverrides::default()
                        }),
                        ..ThreadProfilePolicy::default()
                    },
                )
                .await;
        }

        let persisted =
            std::fs::read_to_string(dir.path().join(".openab/thread_profile_context.json"))
                .expect("persisted context");
        assert!(!persisted.contains("plain-text"));
        assert!(!persisted.contains("SECRET"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(".openab/thread_profile_context.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }

        let reloaded = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());
        let (profile_id, overrides) = reloaded
            .effective_profile_context("slack:thread", None, None)
            .await;
        let overrides = overrides.expect("expected persisted overrides");

        assert_eq!(profile_id.as_deref(), Some("deep"));
        assert_eq!(overrides.model.as_deref(), Some("gpt-5.1"));
        assert_eq!(overrides.reasoning_effort.as_deref(), Some("high"));
        assert!(overrides.env.is_empty());
        assert_eq!(
            reloaded
                .thread_pools
                .read()
                .await
                .get("slack:thread")
                .map(String::as_str),
            Some("profile:deep")
        );
    }

    struct HomeEnvGuard {
        previous: Option<String>,
    }

    impl HomeEnvGuard {
        fn set(path: &std::path::Path) -> Self {
            let previous = std::env::var("HOME").ok();
            std::env::set_var("HOME", path);
            Self { previous }
        }
    }

    impl Drop for HomeEnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    #[test]
    fn merge_profile_overrides_keeps_profile_and_patches_model() {
        let base = ProfileSessionOverrides {
            reasoning_effort: Some("medium".into()),
            ..ProfileSessionOverrides::default()
        };
        let patch = ProfileSessionOverrides {
            model: Some("gpt-5.1".into()),
            ..ProfileSessionOverrides::default()
        };

        let merged = merge_profile_overrides(&base, &patch);

        assert_eq!(merged.model.as_deref(), Some("gpt-5.1"));
        assert_eq!(merged.reasoning_effort.as_deref(), Some("medium"));
    }

    #[tokio::test]
    async fn pool_for_key_applies_profile_policy() {
        let outer = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());
        let policy = ThreadProfilePolicy {
            timeout_secs: Some(60),
            recovery_strategy: RecoveryStrategy::None,
            ..ThreadProfilePolicy::default()
        };

        let pool = outer
            .pool_for_key(
                "profile:timeout",
                AgentConfig::default(),
                HashMap::new(),
                policy,
            )
            .await;

        assert_eq!(pool.timeout_secs_for_test(), Some(60));
        assert_eq!(pool.recovery_strategy_for_test(), RecoveryStrategy::None);
    }

    #[tokio::test]
    async fn exited_error_without_recovery_marks_error() {
        let outer = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());
        outer
            .seed_session_snapshot_for_test(SessionSnapshot::new(
                "thread".into(),
                "codex".into(),
                "/workspace".into(),
                None,
                None,
                None,
                None,
            ))
            .await;
        outer.thread_policies.write().await.insert(
            "thread".into(),
            ThreadProfilePolicy {
                timeout_secs: None,
                recovery_strategy: RecoveryStrategy::None,
                ..ThreadProfilePolicy::default()
            },
        );
        let mut events = outer.session_event_bus().subscribe();

        outer
            .mark_session_exited("thread", Some("Agent process died".into()))
            .await;

        let snapshot = outer.session_snapshot("thread").await.expect("snapshot");
        assert_eq!(snapshot.status, SessionStatus::Error);
        assert_eq!(snapshot.last_error.as_deref(), Some("Agent process died"));
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("event should arrive")
            .expect("event channel should stay open");
        assert_eq!(event.event, SessionEventKind::Error);
    }

    #[tokio::test]
    async fn recovered_session_marks_snapshot_idle() {
        let outer = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());
        let mut snapshot = SessionSnapshot::new(
            "thread".into(),
            "codex".into(),
            "/workspace".into(),
            None,
            None,
            None,
            None,
        );
        snapshot.set_exited(Some("Agent process died".into()));
        outer.seed_session_snapshot_for_test(snapshot).await;
        let mut events = outer.session_event_bus().subscribe();

        let created = outer
            .apply_ensure_outcome(
                "thread",
                pool::SessionEnsureOutcome {
                    created: false,
                    recovered: true,
                    profile_config_errors: Vec::new(),
                    runtime_metadata: SessionRuntimeMetadata::acp(
                        Some("Codex ACP".into()),
                        Some("gpt-5".into()),
                        Some("high".into()),
                    ),
                },
            )
            .await;

        assert!(!created);
        let snapshot = outer.session_snapshot("thread").await.expect("snapshot");
        assert_eq!(snapshot.status, SessionStatus::Idle);
        assert_eq!(snapshot.last_error, None);
        assert_eq!(snapshot.agent, "Codex ACP");
        assert_eq!(snapshot.model.as_deref(), Some("gpt-5"));
        assert_eq!(snapshot.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(snapshot.metadata_source, Some(SessionMetadataSource::Acp));
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("event should arrive")
            .expect("event channel should stay open");
        assert_eq!(event.event, SessionEventKind::StatusChanged);
    }

    #[tokio::test]
    async fn runtime_metadata_stays_session_specific_within_one_profile() {
        let outer = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());
        for thread in ["slack:first", "slack:second"] {
            outer
                .seed_session_snapshot_for_test(SessionSnapshot::new(
                    thread.into(),
                    String::new(),
                    "/workspace".into(),
                    Some("shared-profile".into()),
                    Some("Shared Profile".into()),
                    None,
                    None,
                ))
                .await;
        }

        for (thread, model, effort) in [
            ("slack:first", "gpt-5", "high"),
            ("slack:second", "claude-sonnet-4", "medium"),
        ] {
            outer
                .apply_ensure_outcome(
                    thread,
                    pool::SessionEnsureOutcome {
                        created: false,
                        recovered: true,
                        profile_config_errors: Vec::new(),
                        runtime_metadata: SessionRuntimeMetadata::acp(
                            Some("Codex ACP".into()),
                            Some(model.into()),
                            Some(effort.into()),
                        ),
                    },
                )
                .await;
        }

        let first = outer
            .session_snapshot("slack:first")
            .await
            .expect("first snapshot");
        let second = outer
            .session_snapshot("slack:second")
            .await
            .expect("second snapshot");
        assert_eq!(first.profile_id, second.profile_id);
        assert_eq!(first.model.as_deref(), Some("gpt-5"));
        assert_eq!(second.model.as_deref(), Some("claude-sonnet-4"));
        assert_eq!(first.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(second.reasoning_effort.as_deref(), Some("medium"));
    }

    #[tokio::test]
    async fn config_update_clears_stale_runtime_model_and_thinking() {
        let outer = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());
        let mut snapshot = SessionSnapshot::new(
            "slack:thread".into(),
            String::new(),
            "/workspace".into(),
            None,
            None,
            None,
            None,
        );
        snapshot.replace_runtime_metadata(SessionRuntimeMetadata::acp(
            Some("Codex ACP".into()),
            Some("gpt-5".into()),
            Some("high".into()),
        ));
        outer.seed_session_snapshot_for_test(snapshot).await;

        outer
            .record_session_config_update("slack:thread", &[])
            .await;

        let snapshot = outer
            .session_snapshot("slack:thread")
            .await
            .expect("snapshot");
        assert_eq!(snapshot.agent, "Codex ACP");
        assert_eq!(snapshot.model, None);
        assert_eq!(snapshot.reasoning_effort, None);
        assert_eq!(snapshot.metadata_source, Some(SessionMetadataSource::Acp));
    }

    #[tokio::test]
    async fn recovered_session_records_profile_config_errors() {
        let outer = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());
        let mut snapshot = SessionSnapshot::new(
            "thread".into(),
            "codex".into(),
            "/workspace".into(),
            None,
            None,
            None,
            None,
        );
        snapshot.set_exited(Some("Agent process died".into()));
        outer.seed_session_snapshot_for_test(snapshot).await;
        let mut events = outer.session_event_bus().subscribe();

        let created = outer
            .apply_ensure_outcome(
                "thread",
                pool::SessionEnsureOutcome {
                    created: false,
                    recovered: true,
                    profile_config_errors: vec![ProfileConfigError::new("model", "unsupported")],
                    runtime_metadata: SessionRuntimeMetadata::default(),
                },
            )
            .await;

        assert!(!created);
        let snapshot = outer.session_snapshot("thread").await.expect("snapshot");
        assert_eq!(snapshot.status, SessionStatus::Idle);
        assert_eq!(snapshot.profile_config_errors.len(), 1);
        assert_eq!(snapshot.profile_config_errors[0].config_id, "model");
        assert_eq!(snapshot.profile_config_errors[0].error, "unsupported");
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("event should arrive")
            .expect("event channel should stay open");
        assert_eq!(event.event, SessionEventKind::StatusChanged);
        assert_eq!(
            event.snapshot.profile_config_errors,
            snapshot.profile_config_errors
        );
    }

    #[tokio::test]
    async fn sync_marks_suspended_snapshots() {
        let pool = Arc::new(pool::SessionPool::new(
            AgentConfig::default(),
            2,
            120,
            HashMap::new(),
        ));
        let outer = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());
        let snapshot = SessionSnapshot::new(
            "thread".into(),
            "codex".into(),
            "/workspace".into(),
            None,
            None,
            None,
            None,
        );
        outer
            .snapshots
            .write()
            .await
            .insert("thread".into(), snapshot);
        outer
            .thread_pools
            .write()
            .await
            .insert("thread".into(), "system".into());
        pool.insert_suspended_for_test("thread", "sid").await;

        outer.sync_pool_snapshot_statuses("system", &pool).await;

        let snapshot = outer.session_snapshot("thread").await.expect("snapshot");
        assert_eq!(snapshot.status, SessionStatus::Suspended);
    }

    #[tokio::test]
    async fn mark_profile_deleted_preserves_session_profile_snapshot() {
        let outer = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());
        let profile_id = Some("profile-1".to_string());
        let profile_name = Some("Deleted Profile".to_string());
        outer
            .seed_session_snapshot_for_test(SessionSnapshot::new(
                "slack:thread".into(),
                "codex".into(),
                "/workspace".into(),
                profile_id.clone(),
                profile_name.clone(),
                None,
                None,
            ))
            .await;

        outer.mark_profile_deleted("profile-1").await;

        let snapshot = outer
            .session_snapshot("slack:thread")
            .await
            .expect("snapshot");
        assert_eq!(snapshot.profile_id, profile_id);
        assert_eq!(snapshot.profile_name, profile_name);
        assert_eq!(snapshot.profile_status, Some(ProfileStatus::Deleted));
    }

    #[tokio::test]
    async fn config_schema_for_agent_uses_live_options_from_matching_session() {
        let outer = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());
        let mut snapshot = SessionSnapshot::new(
            "slack:live-opencode".into(),
            "opencode".into(),
            "/workspace".into(),
            None,
            None,
            None,
            None,
        );
        snapshot.set_status(SessionStatus::Running);
        outer.seed_session_snapshot_for_test(snapshot).await;
        outer
            .seed_config_options_for_test(
                "slack:live-opencode",
                vec![enum_config_option(
                    "model",
                    "Model",
                    "opencode/latest",
                    &["opencode/latest", "opencode/canary"],
                )],
            )
            .await;

        let schema = outer
            .config_schema_for_agent("opencode")
            .await
            .expect("live schema");

        assert_eq!(schema.source, "agent-session-config-options");
        assert!(schema.fields.iter().any(|field| {
            field.id == "model" && field.options.contains(&"opencode/canary".to_string())
        }));
    }

    #[tokio::test]
    async fn config_schema_for_agent_ignores_other_agent_snapshots() {
        let outer = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());
        outer
            .seed_session_snapshot_for_test(SessionSnapshot::new(
                "slack:codex".into(),
                "codex".into(),
                "/workspace".into(),
                None,
                None,
                None,
                None,
            ))
            .await;
        outer
            .seed_config_options_for_test(
                "slack:codex",
                vec![enum_config_option("model", "Model", "gpt-5", &["gpt-5"])],
            )
            .await;

        assert!(outer.config_schema_for_agent("opencode").await.is_none());
    }

    #[test]
    fn ignores_empty_model_config_value() {
        let options = vec![config_option("model", "   ")];

        assert_eq!(runtime_metadata_from_options(None, &options).model, None);
    }
}
