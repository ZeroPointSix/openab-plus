use super::connection::AcpConnection;
use super::pool;
use super::protocol::ConfigOption;
use crate::agent_profile::{AgentProfileService, ProfileSessionOverrides};
use crate::config::AgentConfig;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
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
        Self {
            base_config: config,
            max_sessions,
            hung_threshold_secs,
            default_config_options,
            profile_service: Arc::new(AgentProfileService::from_env()),
            pools: RwLock::new(pools),
            thread_pools: RwLock::new(HashMap::new()),
            thread_gates: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_agent_profile_service(mut self, profile_service: Arc<AgentProfileService>) -> Self {
        self.profile_service = profile_service;
        self
    }

    pub fn profile_service(&self) -> Arc<AgentProfileService> {
        self.profile_service.clone()
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
            return pool.get_or_create(thread_id, working_dir_override).await;
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
        let pool = self
            .pool_for_key(&pool_key, resolved.config, resolved.config_options)
            .await;

        let result = pool.get_or_create(thread_id, working_dir_override).await;
        if result.is_ok() {
            self.thread_pools
                .write()
                .await
                .insert(thread_id.to_string(), pool_key);
        }
        result
    }

    pub async fn has_active_session(&self, thread_id: &str) -> bool {
        if let Some(pool) = self.existing_pool(thread_id).await {
            return pool.has_active_session(thread_id).await;
        }
        for pool in self.pools_snapshot().await {
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
        match self.existing_pool(thread_id).await {
            Some(pool) => pool.get_config_options(thread_id).await,
            None => Vec::new(),
        }
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
        pool.set_config_option(thread_id, config_id, value).await
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
        for pool in self.pools_snapshot().await {
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
            }
            return result;
        }
        for pool in self.pools_snapshot().await {
            if pool.reset_session(thread_id).await.is_ok() {
                self.thread_pools.write().await.remove(thread_id);
                return Ok(());
            }
        }
        Err(anyhow!("no session for thread {thread_id}"))
    }

    pub async fn cleanup_idle(&self, ttl_secs: u64) {
        for pool in self.pools_snapshot().await {
            pool.cleanup_idle(ttl_secs).await;
        }
    }

    pub async fn shutdown(&self) {
        for pool in self.pools_snapshot().await {
            pool.shutdown().await;
        }
    }

    async fn thread_gate(&self, thread_id: &str) -> Arc<Mutex<()>> {
        let mut gates = self.thread_gates.write().await;
        gates
            .entry(thread_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
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

    async fn pools_snapshot(&self) -> Vec<PoolHandle> {
        self.pools.read().await.values().cloned().collect()
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
