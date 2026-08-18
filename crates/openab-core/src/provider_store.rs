use crate::provider::{validate_document, Provider, ProviderDocument, PROVIDER_SCHEMA_VERSION};
use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use tracing::warn;

const DEFAULT_PROVIDERS_PATH: &str = "config/providers.toml";

#[derive(Debug, Clone)]
pub struct ProviderStore {
    path: PathBuf,
}

impl ProviderStore {
    pub fn from_env() -> Self {
        let path = std::env::var("OPENAB_PROVIDERS_PATH")
            .unwrap_or_else(|_| DEFAULT_PROVIDERS_PATH.to_string());
        Self { path: path.into() }
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn load(&self) -> Result<ProviderDocument> {
        match self.try_load_path(&self.path).await {
            Ok(document) => {
                validate_document(&document)?;
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
                        "providers file is invalid, loading backup"
                    );
                    let document = self.try_load_path(&self.backup_path()).await?;
                    validate_document(&document)?;
                    Ok(document)
                } else if matches!(
                    error
                        .downcast_ref::<std::io::Error>()
                        .map(std::io::Error::kind),
                    Some(std::io::ErrorKind::NotFound)
                ) {
                    Ok(ProviderDocument::default())
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn save_atomic(&self, document: &ProviderDocument) -> Result<()> {
        validate_document(document)?;
        let mut document = document.clone();
        document.schema_version = PROVIDER_SCHEMA_VERSION;
        let raw = toml::to_string_pretty(&document)?;
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

    pub async fn list(&self) -> Result<Vec<Provider>> {
        Ok(self.load().await?.providers)
    }

    pub async fn get(&self, id: &str) -> Result<Option<Provider>> {
        Ok(self
            .load()
            .await?
            .providers
            .into_iter()
            .find(|provider| provider.id == id))
    }

    pub async fn upsert(&self, provider: Provider) -> Result<Provider> {
        crate::provider::validate_provider(&provider)?;
        let mut document = self.load().await?;
        if let Some(existing) = document
            .providers
            .iter_mut()
            .find(|item| item.id == provider.id)
        {
            *existing = provider.clone();
        } else {
            document.providers.push(provider.clone());
        }
        self.save_atomic(&document).await?;
        Ok(provider)
    }

    pub async fn delete(&self, id: &str) -> Result<bool> {
        let mut document = self.load().await?;
        let before = document.providers.len();
        document.providers.retain(|provider| provider.id != id);
        if document.providers.len() == before {
            return Ok(false);
        }
        self.save_atomic(&document).await?;
        Ok(true)
    }

    pub async fn delete_unless_referenced(
        &self,
        id: &str,
        referenced_by: &[String],
    ) -> Result<bool> {
        if !referenced_by.is_empty() {
            return Err(anyhow!(
                "provider {id} is still referenced by profiles: {}",
                referenced_by.join(", ")
            ));
        }
        self.delete(id).await
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

    async fn try_load_path(&self, path: &Path) -> Result<ProviderDocument> {
        let raw = tokio::fs::read_to_string(path).await?;
        if raw.trim().is_empty() {
            return Ok(ProviderDocument::default());
        }
        Ok(toml::from_str(&raw)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trip_providers() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProviderStore::new(dir.path().join("providers.toml"));
        store
            .upsert(Provider::new(
                "newapi",
                "NewAPI",
                "openai_compatible",
                "env://OPENAI_API_KEY",
            ))
            .await
            .unwrap();
        let loaded = store.get("newapi").await.unwrap().unwrap();
        assert_eq!(loaded.name, "NewAPI");
    }
}
