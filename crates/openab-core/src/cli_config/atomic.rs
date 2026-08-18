use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn openab_bak_path(path: &Path) -> PathBuf {
    let mut bak = path.as_os_str().to_os_string();
    bak.push(".openab.bak");
    PathBuf::from(bak)
}

pub async fn ensure_openab_bak(path: &Path) -> Result<Option<PathBuf>> {
    if !tokio::fs::try_exists(path).await.unwrap_or(false) {
        return Ok(None);
    }
    let bak = openab_bak_path(path);
    if tokio::fs::try_exists(&bak).await.unwrap_or(false) {
        return Ok(Some(bak));
    }
    if let Some(parent) = bak.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::copy(path, &bak).await?;
    Ok(Some(bak))
}

pub async fn atomic_write(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = {
        let mut name = path
            .file_name()
            .map(|s| s.to_os_string())
            .unwrap_or_else(|| "config".into());
        name.push(".tmp");
        path.with_file_name(name)
    };
    tokio::fs::write(&tmp, contents).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

#[cfg(unix)]
pub async fn atomic_write_private(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    atomic_write(path, contents).await?;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
    Ok(())
}

#[cfg(not(unix))]
pub async fn atomic_write_private(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    atomic_write(path, contents).await
}

pub async fn restore_from_openab_bak(path: &Path) -> Result<bool> {
    let bak = openab_bak_path(path);
    if !tokio::fs::try_exists(&bak).await.unwrap_or(false) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::copy(&bak, path).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn backup_created_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        tokio::fs::write(&path, "a = 1\n").await.unwrap();
        let bak = ensure_openab_bak(&path).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read_to_string(&bak).await.unwrap(), "a = 1\n");
        tokio::fs::write(&path, "a = 2\n").await.unwrap();
        ensure_openab_bak(&path).await.unwrap();
        assert_eq!(tokio::fs::read_to_string(&bak).await.unwrap(), "a = 1\n");
    }
}
