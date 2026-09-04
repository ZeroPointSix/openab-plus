//! Runtime provider abstraction for admin / control-plane session ops.
//!
//! P0 keeps channels and ACP processes in-process via [`LocalRuntime`].
//! [`RemoteRuntime`] is a deliberate stub so ZER-872 can land the WSS path
//! without rewiring every admin router again.

use crate::acp::turn::{run_headless_turn, HeadlessTurnConfig};
use crate::acp::SessionPool;
use crate::agent_profile::{AgentConfigSchema, ProfileSessionOverrides};
use crate::session_event::SessionStreamBus;
use crate::session_snapshot::SessionSnapshot;
use crate::transcript::SessionTranscriptStore;
use anyhow::{bail, Result};
use async_trait::async_trait;
use std::sync::Arc;

/// Which concrete runtime backend an OpenAB hub is talking to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeKind {
    Local,
    Remote,
}

/// Admin-facing session / profile control surface.
///
/// Only the methods currently used by gateway admin routers are abstracted.
/// Channel adapters still hold the concrete [`SessionPool`] in-process.
#[async_trait]
pub trait RuntimeProvider: Send + Sync {
    fn kind(&self) -> RuntimeKind;

    fn transcript_store(&self) -> SessionTranscriptStore;

    fn session_stream_bus(&self) -> SessionStreamBus;

    async fn list_session_snapshots(&self) -> Vec<SessionSnapshot>;

    async fn session_snapshot(&self, session_id: &str) -> Option<SessionSnapshot>;

    async fn get_or_create_with_profile(
        &self,
        thread_id: &str,
        working_dir_override: Option<&str>,
        profile_id: Option<&str>,
        overrides: Option<&ProfileSessionOverrides>,
    ) -> Result<bool>;

    /// Accept a text prompt and run it asynchronously through the current ACP
    /// session. The final result is published through transcript/SSE.
    async fn send_message(&self, session_id: &str, text: String) -> Result<()>;

    /// Best-effort cancellation of the current ACP turn. Idle sessions are a
    /// successful no-op so the control-plane operation stays idempotent.
    async fn cancel_session(&self, session_id: &str) -> Result<()>;

    async fn config_schema_for_agent(&self, agent_type: &str) -> Option<AgentConfigSchema>;

    async fn mark_profile_deleted(&self, profile_id: &str);
}

/// In-process runtime: thin wrapper over the existing facade [`SessionPool`].
#[derive(Clone)]
pub struct LocalRuntime {
    pool: Arc<SessionPool>,
    turn_config: HeadlessTurnConfig,
}

impl LocalRuntime {
    pub fn new(pool: Arc<SessionPool>) -> Self {
        Self::new_with_turn_config_value(pool, HeadlessTurnConfig::default())
    }

    pub fn new_with_turn_config(
        pool: Arc<SessionPool>,
        prompt_hard_timeout_secs: u64,
        liveness_check_secs: u64,
    ) -> Self {
        Self::new_with_turn_config_value(
            pool,
            HeadlessTurnConfig {
                prompt_hard_timeout: std::time::Duration::from_secs(prompt_hard_timeout_secs),
                liveness_check_interval: std::time::Duration::from_secs(liveness_check_secs),
            },
        )
    }

    fn new_with_turn_config_value(pool: Arc<SessionPool>, turn_config: HeadlessTurnConfig) -> Self {
        Self { pool, turn_config }
    }

    /// Escape hatch for channel adapters / tests that still need the concrete pool.
    pub fn pool(&self) -> Arc<SessionPool> {
        self.pool.clone()
    }
}

#[async_trait]
impl RuntimeProvider for LocalRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Local
    }

    fn transcript_store(&self) -> SessionTranscriptStore {
        self.pool.transcript_store()
    }

    fn session_stream_bus(&self) -> SessionStreamBus {
        self.pool.session_stream_bus()
    }

    async fn list_session_snapshots(&self) -> Vec<SessionSnapshot> {
        self.pool.list_session_snapshots().await
    }

    async fn session_snapshot(&self, session_id: &str) -> Option<SessionSnapshot> {
        self.pool.session_snapshot(session_id).await
    }

    async fn get_or_create_with_profile(
        &self,
        thread_id: &str,
        working_dir_override: Option<&str>,
        profile_id: Option<&str>,
        overrides: Option<&ProfileSessionOverrides>,
    ) -> Result<bool> {
        self.pool
            .get_or_create_with_profile(thread_id, working_dir_override, profile_id, overrides)
            .await
    }

    async fn send_message(&self, session_id: &str, text: String) -> Result<()> {
        if self.pool.session_snapshot(session_id).await.is_none() {
            bail!("session not found");
        }

        let turn_guard = self.pool.try_acquire_turn(session_id).await?;
        let pool = self.pool.clone();
        let session_id = session_id.to_string();
        let config = self.turn_config;
        tokio::spawn(async move {
            // Keep the lease until the terminal snapshot is published. Dropping it
            // inside the driver would let a new turn publish `running` before this
            // task publishes its stale `idle` state.
            let result =
                run_headless_turn(pool.clone(), session_id.clone(), text, config, &turn_guard)
                    .await;
            match result {
                Ok(turn) if turn.response_error.is_none() && !turn.status_failure_recorded => {
                    pool.mark_session_status(
                        &session_id,
                        crate::session_snapshot::SessionStatus::Idle,
                    )
                    .await;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::error!(
                        session_id = %session_id,
                        error = %error,
                        "headless ACP turn failed"
                    );
                    pool.mark_session_error(&session_id, error.to_string())
                        .await;
                }
            }
        });
        Ok(())
    }

    async fn cancel_session(&self, session_id: &str) -> Result<()> {
        if self.pool.session_snapshot(session_id).await.is_none() {
            bail!("session not found");
        }
        if let Err(error) = self.pool.cancel_session(session_id).await {
            // Cancellation is deliberately best-effort. The snapshot still
            // exists, so an already-dead agent must not turn an accepted
            // control-plane cancellation into a 500 response.
            tracing::warn!(
                session_id,
                error = %error,
                "best-effort session cancellation failed"
            );
        }
        Ok(())
    }

    async fn config_schema_for_agent(&self, agent_type: &str) -> Option<AgentConfigSchema> {
        self.pool.config_schema_for_agent(agent_type).await
    }

    async fn mark_profile_deleted(&self, profile_id: &str) {
        self.pool.mark_profile_deleted(profile_id).await;
    }
}

/// Placeholder for the future out-of-process / WSS runtime (ZER-872).
#[derive(Debug, Default, Clone, Copy)]
pub struct RemoteRuntime;

#[async_trait]
impl RuntimeProvider for RemoteRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Remote
    }

    fn transcript_store(&self) -> SessionTranscriptStore {
        unimplemented!("remote runtime is not implemented")
    }

    fn session_stream_bus(&self) -> SessionStreamBus {
        unimplemented!("remote runtime is not implemented")
    }

    async fn list_session_snapshots(&self) -> Vec<SessionSnapshot> {
        Vec::new()
    }

    async fn session_snapshot(&self, _session_id: &str) -> Option<SessionSnapshot> {
        None
    }

    async fn get_or_create_with_profile(
        &self,
        _thread_id: &str,
        _working_dir_override: Option<&str>,
        _profile_id: Option<&str>,
        _overrides: Option<&ProfileSessionOverrides>,
    ) -> Result<bool> {
        bail!("remote runtime is not implemented")
    }

    async fn send_message(&self, _session_id: &str, _text: String) -> Result<()> {
        bail!("remote runtime is not implemented")
    }

    async fn cancel_session(&self, _session_id: &str) -> Result<()> {
        bail!("remote runtime is not implemented")
    }

    async fn config_schema_for_agent(&self, _agent_type: &str) -> Option<AgentConfigSchema> {
        None
    }

    async fn mark_profile_deleted(&self, _profile_id: &str) {
        // No-op stub: remote runtime is not wired yet.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentConfig;
    use crate::session_snapshot::{ProfileStatus, SessionSnapshot};
    use std::collections::HashMap;

    fn test_pool() -> Arc<SessionPool> {
        Arc::new(SessionPool::new(
            AgentConfig::default(),
            2,
            120,
            HashMap::new(),
        ))
    }

    #[tokio::test]
    async fn local_runtime_delegates_snapshot_ops() {
        let pool = test_pool();
        let runtime: Arc<dyn RuntimeProvider> = Arc::new(LocalRuntime::new(pool.clone()));

        assert_eq!(runtime.kind(), RuntimeKind::Local);
        assert!(runtime.list_session_snapshots().await.is_empty());
        assert!(runtime.session_snapshot("missing").await.is_none());

        let snapshot = SessionSnapshot::new(
            "admin:test".into(),
            "opencode".into(),
            "/tmp/work".into(),
            Some("profile-1".into()),
            Some("Profile One".into()),
            None,
            None,
        );
        pool.seed_session_snapshot_for_test(snapshot).await;

        let listed = runtime.list_session_snapshots().await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "admin:test");
        assert_eq!(
            runtime
                .session_snapshot("admin:test")
                .await
                .map(|s| s.session_id),
            Some("admin:test".into())
        );
        assert!(runtime.config_schema_for_agent("opencode").await.is_none());

        runtime.mark_profile_deleted("profile-1").await;
        let after = runtime
            .session_snapshot("admin:test")
            .await
            .expect("snapshot");
        assert_eq!(after.profile_status, Some(ProfileStatus::Deleted));
    }

    #[tokio::test]
    async fn remote_runtime_refuses_session_create() {
        let runtime = RemoteRuntime;
        assert_eq!(runtime.kind(), RuntimeKind::Remote);
        let err = runtime
            .get_or_create_with_profile("admin:x", None, Some("p"), None)
            .await
            .expect_err("remote must fail");
        assert!(err.to_string().contains("not implemented"));
        assert!(runtime.list_session_snapshots().await.is_empty());
    }
}
