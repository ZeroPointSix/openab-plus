use anyhow::Result;
use std::path::{Path, PathBuf};

/// Marker written when the target file did not exist before OpenAB first applied.
pub const NO_PREEXISTING_MARKER: &[u8] = b"# openab:no-preexisting\n";

pub fn openab_bak_path(path: &Path) -> PathBuf {
    let mut bak = path.as_os_str().to_os_string();
    bak.push(".openab.bak");
    PathBuf::from(bak)
}

pub async fn ensure_openab_bak(path: &Path) -> Result<Option<PathBuf>> {
    let bak = openab_bak_path(path);
    if tokio::fs::try_exists(&bak).await.unwrap_or(false) {
        return Ok(Some(bak));
    }
    if let Some(parent) = bak.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if tokio::fs::try_exists(path).await.unwrap_or(false) {
        tokio::fs::copy(path, &bak).await?;
        return Ok(Some(bak));
    }
    tokio::fs::write(&bak, NO_PREEXISTING_MARKER).await?;
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

/// Write via a temp file then enforce owner-only permissions on the final path.
pub async fn atomic_write_private(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    atomic_write(path, contents).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(path).await?.permissions();
        perms.set_mode(0o600);
        tokio::fs::set_permissions(path, perms).await?;
    }
    Ok(())
}

pub async fn restore_from_openab_bak(path: &Path) -> Result<bool> {
    let bak = openab_bak_path(path);
    if !tokio::fs::try_exists(&bak).await.unwrap_or(false) {
        return Ok(false);
    }
    let marker = tokio::fs::read(&bak).await?;
    if marker == NO_PREEXISTING_MARKER {
        if tokio::fs::try_exists(path).await.unwrap_or(false) {
            tokio::fs::remove_file(path).await?;
        }
        tokio::fs::remove_file(&bak).await?;
        return Ok(true);
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::copy(&bak, path).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = tokio::fs::metadata(&bak).await {
            let mode = meta.permissions().mode() & 0o777;
            if mode != 0 {
                let mut perms = tokio::fs::metadata(path).await?.permissions();
                perms.set_mode(mode);
                tokio::fs::set_permissions(path, perms).await?;
            }
        }
    }
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

    #[tokio::test]
    async fn restore_deletes_file_created_from_no_preexisting_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let bak = ensure_openab_bak(&path).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(&bak).await.unwrap(), NO_PREEXISTING_MARKER);
        tokio::fs::write(&path, "a = 1\n").await.unwrap();
        assert!(restore_from_openab_bak(&path).await.unwrap());
        assert!(!tokio::fs::try_exists(&path).await.unwrap());
        assert!(!tokio::fs::try_exists(&bak).await.unwrap());
    }

    #[tokio::test]
    async fn atomic_write_private_sets_mode_600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        atomic_write_private(&path, "secret = true\n").await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = tokio::fs::metadata(&path)
                .await
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
