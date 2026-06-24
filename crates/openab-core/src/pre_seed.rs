use crate::config::{parse_s3_uri, OnFailure, PreSeedConfig};
use sha2::{Digest, Sha256};
use std::path::Path;
use tracing::{error, info, warn};

/// Maximum number of sources allowed.
const MAX_SOURCES: usize = 5;

/// Run the pre_seed phase: download zip archives from S3 and extract them in order.
pub async fn run(cfg: &PreSeedConfig) -> anyhow::Result<()> {
    if cfg.sources.is_empty() {
        return Ok(());
    }
    if cfg.sources.len() > MAX_SOURCES {
        anyhow::bail!(
            "hooks.pre_seed: too many sources ({}, max {})",
            cfg.sources.len(),
            MAX_SOURCES
        );
    }
    if !cfg.sha256s.is_empty() && cfg.sha256s.len() != cfg.sources.len() {
        anyhow::bail!(
            "hooks.pre_seed: sha256s length ({}) must match sources length ({})",
            cfg.sha256s.len(),
            cfg.sources.len()
        );
    }

    let target = match &cfg.target {
        Some(t) => std::path::PathBuf::from(t),
        None => dirs_home(),
    };

    info!(
        sources = cfg.sources.len(),
        target = %target.display(),
        "hooks.pre_seed: starting"
    );

    let mut s3_config_loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
    if let Some(ref region) = cfg.region {
        s3_config_loader = s3_config_loader.region(aws_config::Region::new(region.clone()));
    }
    if let Some(ref endpoint) = cfg.endpoint_url {
        s3_config_loader = s3_config_loader.endpoint_url(endpoint);
    }
    let aws_cfg = s3_config_loader.load().await;
    let s3 = aws_sdk_s3::Client::new(&aws_cfg);

    for (i, source) in cfg.sources.iter().enumerate() {
        let layer = i + 1;
        let expected_sha = cfg.sha256s.get(i).map(|s| s.as_str());
        info!(
            layer,
            source = source.as_str(),
            "hooks.pre_seed: downloading"
        );

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(cfg.timeout_seconds),
            download_and_extract(&s3, source, &target, expected_sha, cfg.max_bytes),
        )
        .await;

        let outcome = match result {
            Ok(Ok(())) => {
                info!(layer, "hooks.pre_seed: layer extracted successfully");
                continue;
            }
            Ok(Err(e)) => e,
            Err(_) => anyhow::anyhow!(
                "hooks.pre_seed: layer {layer} timed out after {}s",
                cfg.timeout_seconds
            ),
        };

        match cfg.on_failure {
            OnFailure::Abort => {
                error!(layer, error = %outcome, "hooks.pre_seed failed (on_failure=abort)");
                return Err(outcome);
            }
            OnFailure::Warn => {
                warn!(layer, error = %outcome, "hooks.pre_seed failed (on_failure=warn), continuing");
            }
        }
    }

    info!("hooks.pre_seed: complete");
    Ok(())
}

/// Download zip from S3, verify integrity, extract to a temp dir, then move into target.
async fn download_and_extract(
    s3: &aws_sdk_s3::Client,
    uri: &str,
    target: &Path,
    expected_sha: Option<&str>,
    max_bytes: u64,
) -> anyhow::Result<()> {
    let (bucket, key) = parse_s3_uri(uri)?;

    let resp = s3
        .get_object()
        .bucket(&bucket)
        .key(&key)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("S3 GetObject failed for {uri}: {e}"))?;

    // Check content length before downloading body
    if let Some(len) = resp.content_length() {
        if len as u64 > max_bytes {
            anyhow::bail!("hooks.pre_seed: {uri} too large ({len} bytes, max {max_bytes})");
        }
    }

    let body = resp
        .body
        .collect()
        .await
        .map_err(|e| anyhow::anyhow!("failed to read S3 body for {uri}: {e}"))?;
    let bytes = body.into_bytes();

    if bytes.len() as u64 > max_bytes {
        anyhow::bail!(
            "hooks.pre_seed: {uri} too large ({} bytes, max {max_bytes})",
            bytes.len()
        );
    }

    // SHA-256 verification
    if let Some(expected) = expected_sha {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let actual = format!("{:x}", hasher.finalize());
        if actual != expected.to_lowercase() {
            anyhow::bail!(
                "hooks.pre_seed: SHA-256 mismatch for {uri}: expected {expected}, got {actual}"
            );
        }
        info!(uri, "hooks.pre_seed: SHA-256 verified");
    }

    info!(
        uri,
        bytes = bytes.len(),
        "hooks.pre_seed: downloaded, extracting"
    );

    // Extract to a temp dir first, then move files into target.
    // This ensures that if extraction fails or times out, target is not corrupted.
    let target = target.to_path_buf();
    tokio::task::spawn_blocking(move || extract_to_target(&bytes, &target))
        .await
        .map_err(|e| anyhow::anyhow!("hooks.pre_seed: extract task panicked: {e}"))??;

    Ok(())
}

/// Extract zip to a temp directory, then move all files into target atomically.
fn extract_to_target(data: &[u8], target: &Path) -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir_in(target.parent().unwrap_or(target))?;
    extract_zip(data, temp_dir.path())?;

    // Move extracted files from temp dir into target
    move_recursive(temp_dir.path(), target)?;

    Ok(())
}

/// Extract a zip archive from memory into the given directory.
fn extract_zip(data: &[u8], dest: &Path) -> anyhow::Result<()> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.enclosed_name().ok_or_else(|| {
            anyhow::anyhow!("hooks.pre_seed: invalid zip entry name at index {i}")
        })?;
        let out_path = dest.join(name);

        if file.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&out_path)?;
            std::io::copy(&mut file, &mut out)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = file.unix_mode() {
                    std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode))?;
                }
            }
        }
    }

    Ok(())
}

/// Recursively move files from src directory into dst directory.
fn move_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            std::fs::create_dir_all(&dst_path)?;
            move_recursive(&src_path, &dst_path)?;
        } else {
            // rename is atomic on same filesystem; fallback to copy+remove
            if std::fs::rename(&src_path, &dst_path).is_err() {
                std::fs::copy(&src_path, &dst_path)?;
                std::fs::remove_file(&src_path)?;
            }
        }
    }
    Ok(())
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/home/agent"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_zip_basic() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();

        let buf = Vec::new();
        let cursor = std::io::Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("hello.txt", options).unwrap();
        writer.write_all(b"world").unwrap();
        writer.start_file("sub/nested.txt", options).unwrap();
        writer.write_all(b"nested content").unwrap();
        let cursor = writer.finish().unwrap();

        extract_zip(cursor.get_ref(), dir.path()).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
            "world"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("sub/nested.txt")).unwrap(),
            "nested content"
        );
    }

    #[test]
    fn extract_to_target_atomic() {
        use std::io::Write;
        let target = tempfile::tempdir().unwrap();

        // Pre-existing file that should survive
        std::fs::write(target.path().join("existing.txt"), "keep").unwrap();

        let buf = Vec::new();
        let cursor = std::io::Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("new.txt", options).unwrap();
        writer.write_all(b"added").unwrap();
        let cursor = writer.finish().unwrap();

        extract_to_target(cursor.get_ref(), target.path()).unwrap();

        assert_eq!(
            std::fs::read_to_string(target.path().join("existing.txt")).unwrap(),
            "keep"
        );
        assert_eq!(
            std::fs::read_to_string(target.path().join("new.txt")).unwrap(),
            "added"
        );
    }

    #[test]
    fn extract_zip_overwrites() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "original").unwrap();

        let buf = Vec::new();
        let cursor = std::io::Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("hello.txt", options).unwrap();
        writer.write_all(b"overwritten").unwrap();
        let cursor = writer.finish().unwrap();

        extract_to_target(cursor.get_ref(), dir.path()).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
            "overwritten"
        );
    }

    #[tokio::test]
    async fn run_empty_sources() {
        let cfg = PreSeedConfig::default();
        assert!(run(&cfg).await.is_ok());
    }

    #[tokio::test]
    async fn run_too_many_sources() {
        let cfg = PreSeedConfig {
            sources: vec!["s3://b/k.zip".into(); 6],
            ..Default::default()
        };
        assert!(run(&cfg).await.is_err());
    }

    #[tokio::test]
    async fn run_sha256s_length_mismatch() {
        let cfg = PreSeedConfig {
            sources: vec!["s3://b/k.zip".into()],
            sha256s: vec!["abc".into(), "def".into()],
            ..Default::default()
        };
        assert!(run(&cfg).await.is_err());
    }

    #[test]
    fn default_has_correct_values() {
        let cfg = PreSeedConfig::default();
        assert_eq!(cfg.timeout_seconds, 300);
        assert_eq!(cfg.max_bytes, 100 * 1024 * 1024);
        assert_eq!(cfg.on_failure, OnFailure::Abort);
        assert!(cfg.sources.is_empty());
    }
}
