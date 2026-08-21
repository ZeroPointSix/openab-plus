use crate::agent_profile::{is_safe_profile_agent_type, AgentProfile, AgentProfileDocument};
use crate::cli_config::atomic_write_private;
use anyhow::{anyhow, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, MutexGuard};
use tracing::{info, warn};

const DEFAULT_PROFILE_PATH: &str = "config/agent-profiles.toml";
const DEFAULT_PROFILES_DIR: &str = "config/profiles.d";
const PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct ProfileStore {
    path: PathBuf,
    dir: PathBuf,
    // The Profile service can be constructed by more than one gateway component.
    // Share a process-wide lock so every load-modify-save transaction is serialized.
    write_lock: Arc<Mutex<()>>,
}

fn shared_profile_write_lock() -> Arc<Mutex<()>> {
    static LOCK: OnceLock<Arc<Mutex<()>>> = OnceLock::new();
    LOCK.get_or_init(|| Arc::new(Mutex::new(()))).clone()
}

impl ProfileStore {
    pub fn from_env() -> Self {
        let path = std::env::var("OPENAB_AGENT_PROFILES_PATH")
            .or_else(|_| std::env::var("AGENT_PROFILES_PATH"))
            .unwrap_or_else(|_| DEFAULT_PROFILE_PATH.to_string());
        let dir = std::env::var("OPENAB_AGENT_PROFILES_DIR")
            .unwrap_or_else(|_| DEFAULT_PROFILES_DIR.to_string());
        Self {
            path: path.into(),
            dir: dir.into(),
            write_lock: shared_profile_write_lock(),
        }
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let dir = path
            .parent()
            .map(|parent| parent.join("profiles.d"))
            .unwrap_or_else(|| PathBuf::from(DEFAULT_PROFILES_DIR));
        Self {
            path,
            dir,
            write_lock: shared_profile_write_lock(),
        }
    }

    pub fn with_dir(path: impl Into<PathBuf>, dir: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            dir: dir.into(),
            write_lock: shared_profile_write_lock(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) async fn write_lock(&self) -> MutexGuard<'_, ()> {
        self.write_lock.lock().await
    }

    pub async fn load(&self) -> Result<AgentProfileDocument> {
        let _guard = self.write_lock().await;
        self.load_unlocked().await
    }

    pub(crate) async fn load_unlocked(&self) -> Result<AgentProfileDocument> {
        self.maybe_migrate_from_legacy().await?;
        if self.directory_mode_active().await {
            return self.load_directory().await;
        }
        match self.try_load_path(&self.path).await {
            Ok(mut document) => {
                self.ensure_schema(&mut document)?;
                Ok(document)
            }
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
                    let mut document = self.try_load_path(&self.backup_path()).await?;
                    self.ensure_schema(&mut document)?;
                    Ok(document)
                } else if matches!(
                    error
                        .downcast_ref::<std::io::Error>()
                        .map(std::io::Error::kind),
                    Some(std::io::ErrorKind::NotFound)
                ) {
                    Ok(AgentProfileDocument {
                        schema_version: PROFILE_SCHEMA_VERSION,
                        ..AgentProfileDocument::default()
                    })
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn save_atomic(&self, document: &AgentProfileDocument) -> Result<()> {
        let _guard = self.write_lock().await;
        self.save_atomic_unlocked(document).await
    }

    pub(crate) async fn save_atomic_unlocked(&self, document: &AgentProfileDocument) -> Result<()> {
        let mut document = document.clone();
        self.ensure_schema(&mut document)?;
        // Prefer directory persistence once migrated or when explicitly configured.
        if self.directory_mode_active().await
            || std::env::var("OPENAB_AGENT_PROFILES_DIR").is_ok()
            || !self.path_exists().await
        {
            return self.save_directory(&document).await;
        }
        let raw = toml::to_string_pretty(&document)?;
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        if tokio::fs::try_exists(&self.path).await.unwrap_or(false) {
            tokio::fs::copy(&self.path, self.backup_path()).await?;
        }
        atomic_write_private(&self.path, raw).await?;
        Ok(())
    }

    pub async fn rollback_available(&self) -> bool {
        if self.directory_mode_active().await {
            return false;
        }
        tokio::fs::try_exists(self.backup_path())
            .await
            .unwrap_or(false)
    }

    async fn directory_mode_active(&self) -> bool {
        let Ok(mut entries) = tokio::fs::read_dir(&self.dir).await else {
            return false;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".toml") && !name.contains(".migrated") {
                return true;
            }
        }
        false
    }

    async fn path_exists(&self) -> bool {
        tokio::fs::try_exists(&self.path).await.unwrap_or(false)
    }

    async fn maybe_migrate_from_legacy(&self) -> Result<()> {
        if self.directory_mode_active().await || !self.path_exists().await {
            return Ok(());
        }
        let migrated = PathBuf::from(format!("{}.migrated", self.path.display()));
        if tokio::fs::try_exists(&migrated).await.unwrap_or(false) {
            return Ok(());
        }
        let Ok(document) = self.try_load_path(&self.path).await else {
            return Ok(());
        };
        if document.profiles.is_empty() {
            return Ok(());
        }
        info!(
            from = %self.path.display(),
            to = %self.dir.display(),
            "migrating agent-profiles.toml into profiles.d"
        );
        self.save_directory(&document).await?;
        tokio::fs::rename(&self.path, &migrated).await?;
        Ok(())
    }

    async fn load_directory(&self) -> Result<AgentProfileDocument> {
        let mut profiles = Vec::new();
        let mut default_profile = None;
        let mut default_profiles = BTreeMap::new();
        let mut ids = BTreeSet::new();
        let mut entries = tokio::fs::read_dir(&self.dir).await?;
        let mut files = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if name.ends_with(".toml") && !name.contains(".migrated") && !name.starts_with('_') {
                files.push(path);
            }
        }
        files.sort();
        for path in files {
            let mut document = self.try_load_path(&path).await?;
            self.ensure_schema(&mut document)?;
            let legacy_default = document.default_profile.clone();
            let file_defaults = std::mem::take(&mut document.default_profiles);
            if default_profile.is_none() {
                default_profile = legacy_default.clone();
            }
            for (agent_type, default_id) in file_defaults {
                if default_profiles
                    .insert(agent_type.clone(), default_id)
                    .is_some()
                {
                    return Err(anyhow!(
                        "duplicate default for agent type {agent_type} while loading {}",
                        path.display()
                    ));
                }
            }
            if let Some(default_id) = legacy_default {
                if !default_profiles.values().any(|id| id == &default_id) {
                    if let Some(profile) = document
                        .profiles
                        .iter()
                        .find(|profile| profile.id == default_id)
                    {
                        default_profiles.insert(profile.agent_type.clone(), default_id);
                    }
                }
            }
            for profile in document.profiles {
                if !ids.insert(profile.id.clone()) {
                    return Err(anyhow!(
                        "duplicate profile id {} while loading {}",
                        profile.id,
                        path.display()
                    ));
                }
                profiles.push(profile);
            }
        }
        Ok(AgentProfileDocument {
            schema_version: PROFILE_SCHEMA_VERSION,
            default_profile,
            default_profiles,
            profiles,
        })
    }

    async fn save_directory(&self, document: &AgentProfileDocument) -> Result<()> {
        tokio::fs::create_dir_all(&self.dir).await?;
        let mut by_agent: BTreeMap<String, Vec<AgentProfile>> = BTreeMap::new();
        for profile in &document.profiles {
            by_agent
                .entry(profile.agent_type.clone())
                .or_default()
                .push(profile.clone());
        }
        let mut written = BTreeSet::new();
        for (agent_type, profiles) in by_agent {
            if !is_safe_profile_agent_type(&agent_type) {
                return Err(anyhow!(
                    "invalid agent_type for profile file: {agent_type} (path traversal rejected)"
                ));
            }
            let file_name = format!("{agent_type}.toml");
            let path = self.dir.join(&file_name);
            written.insert(file_name);
            let default_profile = document
                .default_profile_for_agent(&agent_type)
                .filter(|id| profiles.iter().any(|profile| profile.id == *id))
                .map(str::to_string);
            let mut default_profiles = BTreeMap::new();
            if let Some(default_id) = default_profile.clone() {
                default_profiles.insert(agent_type.clone(), default_id);
            }
            let part = AgentProfileDocument {
                schema_version: PROFILE_SCHEMA_VERSION,
                default_profile,
                default_profiles,
                profiles,
            };
            let raw = toml::to_string_pretty(&part)?;
            atomic_write_private(&path, raw).await?;
        }
        if let Ok(mut entries) = tokio::fs::read_dir(&self.dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                if name.ends_with(".toml")
                    && !name.contains(".migrated")
                    && !name.starts_with('_')
                    && !written.contains(&name)
                {
                    let _ = tokio::fs::remove_file(path).await;
                }
            }
        }
        Ok(())
    }

    fn ensure_schema(&self, document: &mut AgentProfileDocument) -> Result<()> {
        if document.schema_version == 0 {
            document.schema_version = PROFILE_SCHEMA_VERSION;
        }
        if document.schema_version != PROFILE_SCHEMA_VERSION {
            return Err(anyhow!(
                "unsupported agent profile schema_version {}",
                document.schema_version
            ));
        }
        Ok(())
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
            return Ok(AgentProfileDocument {
                schema_version: PROFILE_SCHEMA_VERSION,
                ..AgentProfileDocument::default()
            });
        }
        Ok(toml::from_str(&raw)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_file_loads_empty_document() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::with_dir(
            dir.path().join("agent-profiles.toml"),
            dir.path().join("profiles.d"),
        );
        let document = store.load().await.unwrap();
        assert!(document.profiles.is_empty());
        assert_eq!(document.schema_version, 1);
    }

    #[tokio::test]
    async fn save_directory_groups_by_agent_type() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::with_dir(
            dir.path().join("agent-profiles.toml"),
            dir.path().join("profiles.d"),
        );
        std::env::set_var(
            "OPENAB_AGENT_PROFILES_DIR",
            dir.path().join("profiles.d").as_os_str(),
        );
        store
            .save_atomic(&AgentProfileDocument {
                schema_version: 1,
                default_profile: Some("codex".into()),
                default_profiles: BTreeMap::from([
                    ("codex".into(), "codex".into()),
                    ("claude".into(), "claude".into()),
                ]),
                profiles: vec![
                    AgentProfile::new("codex", "Codex", "codex"),
                    AgentProfile::new("claude", "Claude", "claude"),
                ],
            })
            .await
            .unwrap();
        assert!(dir.path().join("profiles.d/codex.toml").exists());
        assert!(dir.path().join("profiles.d/claude.toml").exists());
        let loaded = store.load().await.unwrap();
        assert_eq!(loaded.profiles.len(), 2);
        assert_eq!(loaded.default_profile_for_agent("codex"), Some("codex"));
        assert_eq!(loaded.default_profile_for_agent("claude"), Some("claude"));
        std::env::remove_var("OPENAB_AGENT_PROFILES_DIR");
    }
}
