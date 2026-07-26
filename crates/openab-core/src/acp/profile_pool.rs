use super::connection::AcpConnection;
use super::pool;
use super::protocol::ConfigOption;
use crate::agent_profile::{AgentProfileService, ProfileSessionOverrides};
use crate::config::AgentConfig;
use crate::session_event::{SessionEventBus, SessionEventKind};
use crate::session_snapshot::{SessionSnapshot, SessionStatus};
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::collections::HashMap;
use std::env;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

type PoolHandle = Arc<pool::SessionPool>;

pub struct SessionPool {
    base_config: AgentConfig,
    max_sessions: usize,
    hung_threshold_secs: u64,
    default_config_options: HashMap<String, String>,
    profile_service: Arc<AgentProfileService>,
    pools: RwLock<HashMap<String, PoolHandle>>,
    thread_pools: RwLock<HashMap<String, String>>,
    thread_gates: RwLock<HashMap<String, Arc<Mutex<()>>>>,
    session_events: SessionEventBus,
    snapshots: RwLock<HashMap<String, SessionSnapshot>>,
    external_base_url: Option<String>,
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
        Self {
            base_config: config,
            max_sessions,
            hung_threshold_secs,
            default_config_options,
            profile_service: Arc::new(AgentProfileService::from_env()),
            pools: RwLock::new(pools),
            thread_pools: RwLock::new(HashMap::new()),
            thread_gates: RwLock::new(HashMap::new()),
            session_events: SessionEventBus::default(),
            snapshots: RwLock::new(HashMap::new()),
            external_base_url,
            #[cfg(any(test, feature = "test-support"))]
            config_options_for_test: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_agent_profile_service(mut self, profile_service: Arc<AgentProfileService>) -> Self {
        self.profile_service = profile_service;
        self
    }

    pub fn profile_service(&self) -> Arc<AgentProfileService> {
        self.profile_service.clone()
    }

    pub fn session_event_bus(&self) -> SessionEventBus {
        self.session_events.clone()
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
        let gate = self.thread_gate(thread_id).await;
        let _guard = gate.lock().await;

        if let Some(pool) = self.existing_pool(thread_id).await {
            let result = pool.get_or_create(thread_id, working_dir_override).await;
            if let Err(err) = &result {
                self.mark_session_error(thread_id, err.to_string()).await;
            }
            return result;
        }

        let resolved = self
            .profile_service
            .resolve_for_session(
                &self.base_config,
                &self.default_config_options,
                profile_id,
                overrides,
            )
            .await?;
        let pool_key = if resolved.profile.is_none() && overrides.is_none() {
            "system".to_string()
        } else {
            resolved.pool_key.clone()
        };
        let profile_id = resolved.profile.as_ref().map(|profile| profile.id.clone());
        let profile_name = resolved
            .profile
            .as_ref()
            .map(|profile| profile.name.clone());
        let agent = resolved
            .profile
            .as_ref()
            .map(|profile| profile.agent_type.clone())
            .unwrap_or_else(|| resolved.config.command.clone());
        let workdir = working_dir_override
            .unwrap_or(&resolved.config.working_dir)
            .to_string();
        let model = resolved.config_options.get("model").cloned();
        let pool = self
            .pool_for_key(&pool_key, resolved.config, resolved.config_options)
            .await;

        let result = pool.get_or_create(thread_id, working_dir_override).await;
        match result {
            Ok(created) => {
                self.thread_pools
                    .write()
                    .await
                    .insert(thread_id.to_string(), pool_key.clone());
                if created {
                    let snapshot = SessionSnapshot::new(
                        thread_id.to_string(),
                        agent,
                        workdir,
                        profile_id,
                        profile_name,
                        model,
                        self.external_base_url.as_deref(),
                    );
                    self.record_session_created(snapshot).await;
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
                self.record_session_config_update(thread_id, &options).await;
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
                self.thread_pools.write().await.remove(thread_id);
                self.mark_session_exited(thread_id, None).await;
            }
            return result;
        }
        for (_, pool) in self.pools_snapshot().await {
            if pool.reset_session(thread_id).await.is_ok() {
                self.thread_pools.write().await.remove(thread_id);
                self.mark_session_exited(thread_id, None).await;
                return Ok(());
            }
        }
        Err(anyhow!("no session for thread {thread_id}"))
    }

    pub async fn cleanup_idle(&self, ttl_secs: u64) {
        for (pool_key, pool) in self.pools_snapshot().await {
            pool.cleanup_idle(ttl_secs).await;
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
        self.update_snapshot(thread_id, SessionEventKind::Exited, |snapshot| {
            snapshot.set_exited(error);
        })
        .await;
    }

    pub async fn record_session_config_update(&self, thread_id: &str, options: &[ConfigOption]) {
        let model = model_from_options(options);
        self.update_snapshot(thread_id, SessionEventKind::ConfigChanged, |snapshot| {
            if model.is_some() {
                snapshot.set_model(model);
            } else {
                snapshot.updated_at = Utc::now();
            }
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
                    self.thread_pools.write().await.remove(&session_id);
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
            self.thread_pools.write().await.remove(thread_id);
            None
        }
    }

    async fn pool_for_key(
        &self,
        key: &str,
        config: AgentConfig,
        config_options: HashMap<String, String>,
    ) -> PoolHandle {
        if let Some(pool) = self.pools.read().await.get(key).cloned() {
            return pool;
        }
        let mut pools = self.pools.write().await;
        pools
            .entry(key.to_string())
            .or_insert_with(|| {
                Arc::new(pool::SessionPool::new(
                    config,
                    self.max_sessions,
                    self.hung_threshold_secs,
                    config_options,
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
        command_explicit: config.command_explicit,
    }
}

fn model_from_options(options: &[ConfigOption]) -> Option<String> {
    options
        .iter()
        .find(|option| option.id == "model")
        .map(|option| option.current_value.trim())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
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
    use crate::session_snapshot::ProfileStatus;

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
    fn extracts_model_from_config_options() {
        let options = vec![
            config_option("mode", "default"),
            config_option("model", "gpt-5"),
        ];

        assert_eq!(model_from_options(&options).as_deref(), Some("gpt-5"));
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

        assert_eq!(model_from_options(&options), None);
    }
}
