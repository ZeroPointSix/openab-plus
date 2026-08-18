use crate::presentation::PresentationOverrides;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::warn;

const DEFAULT_CHANNEL_PROFILE_PATH: &str = "config/channel-profiles.toml";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChannelProfileDocument {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<ChannelProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChannelProfile {
    pub platform: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(flatten)]
    pub presentation: PresentationOverrides,
}

impl ChannelProfile {
    fn layer_name(&self) -> String {
        match (&self.workspace_id, &self.channel_id) {
            (None, None) => format!("platform:{}", self.platform),
            (Some(workspace), None) => format!("workspace:{workspace}"),
            (_, Some(channel)) => format!("channel:{channel}"),
        }
    }

    fn matches(
        &self,
        platform: &str,
        workspace_id: Option<&str>,
        channel_id: Option<&str>,
    ) -> bool {
        self.platform == platform
            && self.workspace_id.as_deref() == workspace_id
            && self.channel_id.as_deref() == channel_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedChannelProfile {
    pub presentation: PresentationOverrides,
    pub applied_layers: Vec<String>,
}

impl ChannelProfileDocument {
    pub fn validate(&self) -> Result<()> {
        let mut scopes = HashSet::new();
        for profile in &self.profiles {
            if profile.platform.trim().is_empty() {
                return Err(anyhow!("channel profile platform must not be empty"));
            }
            if profile.channel_id.is_some() && profile.workspace_id.is_none() {
                return Err(anyhow!(
                    "channel profile {} requires workspace_id",
                    profile.layer_name()
                ));
            }
            let scope = (
                profile.platform.as_str(),
                profile.workspace_id.as_deref(),
                profile.channel_id.as_deref(),
            );
            if !scopes.insert(scope) {
                return Err(anyhow!("duplicate channel profile scope: {scope:?}"));
            }
        }
        Ok(())
    }

    pub fn resolve(
        &self,
        platform: &str,
        workspace_id: Option<&str>,
        channel_id: Option<&str>,
        base: &PresentationOverrides,
    ) -> Result<ResolvedChannelProfile> {
        self.validate()?;
        let mut presentation = base.clone();
        let mut applied_layers = Vec::new();
        for (workspace, channel) in [
            (None, None),
            (workspace_id, None),
            (workspace_id, channel_id),
        ] {
            if channel.is_some() && workspace.is_none() {
                continue;
            }
            if let Some(profile) = self
                .profiles
                .iter()
                .find(|profile| profile.matches(platform, workspace, channel))
            {
                presentation.merge_from(&profile.presentation);
                applied_layers.push(profile.layer_name());
            }
        }
        Ok(ResolvedChannelProfile {
            presentation,
            applied_layers,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ChannelProfileStore {
    path: PathBuf,
}

impl ChannelProfileStore {
    pub fn from_env() -> Self {
        let path = std::env::var("OPENAB_CHANNEL_PROFILES_PATH")
            .unwrap_or_else(|_| DEFAULT_CHANNEL_PROFILE_PATH.to_string());
        Self { path: path.into() }
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn load(&self) -> Result<ChannelProfileDocument> {
        match self.try_load_path(&self.path).await {
            Ok(document) => Ok(document),
            Err(error) => {
                if tokio::fs::try_exists(self.backup_path())
                    .await
                    .unwrap_or(false)
                {
                    warn!(path = %self.path.display(), error = %error, "channel profile file is invalid, loading backup");
                    self.try_load_path(&self.backup_path()).await
                } else if matches!(
                    error
                        .downcast_ref::<std::io::Error>()
                        .map(std::io::Error::kind),
                    Some(std::io::ErrorKind::NotFound)
                ) {
                    Ok(ChannelProfileDocument::default())
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn save_atomic(&self, document: &ChannelProfileDocument) -> Result<()> {
        document.validate()?;
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

    fn backup_path(&self) -> PathBuf {
        let mut backup = self.path.clone();
        let ext = self
            .path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| format!("{value}.bak"))
            .unwrap_or_else(|| "bak".to_string());
        backup.set_extension(ext);
        backup
    }

    async fn try_load_path(&self, path: &Path) -> Result<ChannelProfileDocument> {
        let raw = tokio::fs::read_to_string(path).await?;
        if raw.trim().is_empty() {
            return Ok(ChannelProfileDocument::default());
        }
        let document: ChannelProfileDocument = toml::from_str(&raw)?;
        document.validate()?;
        Ok(document)
    }
}

#[derive(Debug, Clone)]
pub struct ChannelProfileService {
    store: ChannelProfileStore,
}

impl ChannelProfileService {
    pub fn from_env() -> Self {
        Self::new(ChannelProfileStore::from_env())
    }

    pub fn new(store: ChannelProfileStore) -> Self {
        Self { store }
    }

    pub async fn list(&self) -> Result<ChannelProfileDocument> {
        self.store.load().await
    }

    pub async fn replace(&self, document: &ChannelProfileDocument) -> Result<()> {
        self.store.save_atomic(document).await
    }

    pub async fn resolve(
        &self,
        platform: &str,
        workspace_id: Option<&str>,
        channel_id: Option<&str>,
        base: &PresentationOverrides,
    ) -> Result<ResolvedChannelProfile> {
        self.store
            .load()
            .await?
            .resolve(platform, workspace_id, channel_id, base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(
        platform: &str,
        workspace: Option<&str>,
        channel: Option<&str>,
        narration: Option<bool>,
    ) -> ChannelProfile {
        ChannelProfile {
            platform: platform.into(),
            workspace_id: workspace.map(str::to_owned),
            channel_id: channel.map(str::to_owned),
            presentation: PresentationOverrides {
                narration,
                ..Default::default()
            },
        }
    }

    #[test]
    fn resolves_platform_workspace_channel_in_order() {
        let document = ChannelProfileDocument {
            profiles: vec![
                profile("slack", None, None, Some(true)),
                profile("slack", Some("T1"), None, Some(false)),
                profile("slack", Some("T1"), Some("C1"), Some(true)),
            ],
        };
        let resolved = document
            .resolve("slack", Some("T1"), Some("C1"), &Default::default())
            .unwrap();
        assert_eq!(resolved.presentation.narration, Some(true));
        assert_eq!(
            resolved.applied_layers,
            ["platform:slack", "workspace:T1", "channel:C1"]
        );
    }

    #[test]
    fn invalid_and_duplicate_scopes_are_rejected() {
        let invalid = ChannelProfileDocument {
            profiles: vec![profile("slack", None, Some("C1"), None)],
        };
        assert!(invalid.validate().is_err());
        let duplicate = ChannelProfileDocument {
            profiles: vec![
                profile("slack", None, None, None),
                profile("slack", None, None, None),
            ],
        };
        assert!(duplicate.validate().is_err());
    }

    #[tokio::test]
    async fn missing_file_is_an_empty_document() {
        let dir = tempfile::tempdir().unwrap();
        let store = ChannelProfileStore::new(dir.path().join("channel-profiles.toml"));
        assert!(store.load().await.unwrap().profiles.is_empty());
    }
}
