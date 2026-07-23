use crate::agent_profile::AgentProfileDocument;
use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::warn;

const DEFAULT_PROFILE_PATH: &str = "config/agent-profiles.toml";

#[derive(Debug, Clone)]
pub struct ProfileStore {
    path: PathBuf,
}

impl ProfileStore {
    pub fn from_env() -> Self {
        let path = std::env::var("OPENAB_AGENT_PROFILES_PATH")
            .or_else(|_| std::env::var("AGENT_PROFILES_PATH"))
            .unwrap_or_else(|_| DEFAULT_PROFILE_PATH.to_string());
        Self { path: path.into() }
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn load(&self) -> Result<AgentProfileDocument> {
        match self.try_load_path(&self.path).await {
            Ok(document) => Ok(document),
            Err(error) => {
                if tokio::fs::try_exists(self.backup_path())
                    .await
                    .unwrap_or(false)
                {
                    warn!(
                        path = %self.path.display(),
                        backup = %self.backup_path().display(),
                        error = %error,
                        "agent profile file is invalid, loading backup"
                    );
                    self.try_load_path(&self.backup_path()).await
                } else if matches!(
                    error
                        .downcast_ref::<std::io::Error>()
                        .map(std::io::Error::kind),
                    Some(std::io::ErrorKind::NotFound)
                ) {
                    Ok(AgentProfileDocument::default())
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn save_atomic(&self, document: &AgentProfileDocument) -> Result<()> {
        let raw = toml::to_string_pretty(document)?;
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        if tokio::fs::try_exists(&self.path).await.unwrap_or(false) {
            tokio::fs::copy(&self.path, self.backup_path()).await?;
        }
        let tmp = self.path.with_extension("toml.tmp");
        tokio::fs::write(&tmp, raw).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        Ok(())
    }

    pub async fn rollback_available(&self) -> bool {
        tokio::fs::try_exists(self.backup_path())
            .await
            .unwrap_or(false)
    }

    fn backup_path(&self) -> PathBuf {
        let mut backup = self.path.clone();
        let ext = self
            .path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| format!("{s}.bak"))
            .unwrap_or_else(|| "bak".to_string());
        backup.set_extension(ext);
        backup
    }

    async fn try_load_path(&self, path: &Path) -> Result<AgentProfileDocument> {
        let raw = tokio::fs::read_to_string(path).await?;
        if raw.trim().is_empty() {
            return Ok(AgentProfileDocument::default());
        }
        Ok(toml::from_str(&raw)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_profile::AgentProfile;

    #[tokio::test]
    async fn missing_file_loads_empty_document() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path().join("agent-profiles.toml"));

        let document = store.load().await.unwrap();

        assert!(document.profiles.is_empty());
    }

    #[tokio::test]
    async fn save_creates_backup_for_last_good_copy() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path().join("agent-profiles.toml"));
        store
            .save_atomic(&AgentProfileDocument {
                default_profile: Some("codex".into()),
                profiles: vec![AgentProfile::new("codex", "Codex", "codex")],
            })
            .await
            .unwrap();
        store
            .save_atomic(&AgentProfileDocument {
                default_profile: None,
                profiles: vec![AgentProfile::new("opencode", "OpenCode", "opencode")],
            })
            .await
            .unwrap();

        assert!(store.rollback_available().await);
    }

    #[tokio::test]
    async fn corrupt_current_file_falls_back_to_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-profiles.toml");
        let store = ProfileStore::new(&path);
        store
            .save_atomic(&AgentProfileDocument {
                default_profile: Some("codex".into()),
                profiles: vec![AgentProfile::new("codex", "Codex", "codex")],
            })
            .await
            .unwrap();
        store
            .save_atomic(&AgentProfileDocument {
                default_profile: Some("opencode".into()),
                profiles: vec![AgentProfile::new("opencode", "OpenCode", "opencode")],
            })
            .await
            .unwrap();
        tokio::fs::write(&path, "not = [valid").await.unwrap();

        let document = store.load().await.unwrap();

        assert_eq!(document.default_profile.as_deref(), Some("codex"));
    }
}
