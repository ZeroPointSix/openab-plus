use crate::config::{parse_s3_uri, OnFailure, PreSeedConfig};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Instant;
use tracing::{error, info, warn};

/// Maximum number of sources allowed.
const MAX_SOURCES: usize = 5;

/// Default max extracted (uncompressed) size: 500 MiB.
const DEFAULT_MAX_EXTRACTED_BYTES: u64 = 500 * 1024 * 1024;

/// Default max file count per zip.
const DEFAULT_MAX_FILE_COUNT: usize = 10_000;

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

        let deadline = Instant::now() + std::time::Duration::from_secs(cfg.timeout_seconds);

        let result =
            download_and_extract(&s3, source, &target, expected_sha, cfg.max_bytes, deadline).await;

        let outcome = match result {
            Ok(()) => {
                info!(layer, "hooks.pre_seed: layer extracted successfully");
                continue;
            }
            Err(e) => e,
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
/// The deadline is enforced cooperatively inside the blocking task.
async fn download_and_extract(
    s3: &aws_sdk_s3::Client,
    uri: &str,
    target: &Path,
    expected_sha: Option<&str>,
    max_bytes: u64,
    deadline: Instant,
) -> anyhow::Result<()> {
    let (bucket, key) = parse_s3_uri(uri)?;

    // Check deadline before S3 call
    if Instant::now() >= deadline {
        anyhow::bail!("hooks.pre_seed: timed out before download for {uri}");
    }

    let resp = s3
        .get_object()
        .bucket(&bucket)
        .key(&key)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("S3 GetObject failed for {uri}: {e}"))?;

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

    if Instant::now() >= deadline {
        anyhow::bail!("hooks.pre_seed: timed out after download for {uri}");
    }

    info!(
        uri,
        bytes = bytes.len(),
        "hooks.pre_seed: downloaded, extracting"
    );

    // Extract and move in a blocking task with cooperative deadline checking.
    let target = target.to_path_buf();
    // Bytes is Arc-backed, Clone is zero-copy (ref-count bump only)
    tokio::task::spawn_blocking(move || extract_and_apply(&bytes, &target, deadline))
        .await
        .map_err(|e| anyhow::anyhow!("hooks.pre_seed: extract task panicked: {e}"))??;

    Ok(())
}

/// Extract zip to a temp directory with budget enforcement, then move into target.
/// Checks deadline cooperatively before each file operation.
fn extract_and_apply(data: &[u8], target: &Path, deadline: Instant) -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir_in(target.parent().unwrap_or(target))?;

    extract_zip_with_limits(data, temp_dir.path(), deadline)?;

    // Check deadline before applying to target
    if Instant::now() >= deadline {
        // temp_dir drops and cleans up automatically
        anyhow::bail!("hooks.pre_seed: timed out before applying to target");
    }

    move_recursive(temp_dir.path(), target, deadline)?;
    Ok(())
}

/// Extract a zip archive with cooperative deadline checks and extraction budget.
fn extract_zip_with_limits(data: &[u8], dest: &Path, deadline: Instant) -> anyhow::Result<()> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)?;

    let file_count = archive.len();
    if file_count > DEFAULT_MAX_FILE_COUNT {
        anyhow::bail!(
            "hooks.pre_seed: zip contains too many entries ({file_count}, max {DEFAULT_MAX_FILE_COUNT})"
        );
    }

    let mut total_extracted: u64 = 0;

    for i in 0..file_count {
        // Cooperative deadline check per file
        if i % 100 == 0 && Instant::now() >= deadline {
            anyhow::bail!("hooks.pre_seed: timed out during extraction at entry {i}");
        }

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

            // Check extracted size budget before writing
            let uncompressed = file.size();
            total_extracted += uncompressed;
            if total_extracted > DEFAULT_MAX_EXTRACTED_BYTES {
                anyhow::bail!(
                    "hooks.pre_seed: extracted size exceeds limit ({total_extracted} > {DEFAULT_MAX_EXTRACTED_BYTES})"
                );
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
/// Checks deadline cooperatively.
fn move_recursive(src: &Path, dst: &Path, deadline: Instant) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(src)? {
        if Instant::now() >= deadline {
            anyhow::bail!("hooks.pre_seed: timed out during move to target");
        }

        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            std::fs::create_dir_all(&dst_path)?;
            move_recursive(&src_path, &dst_path, deadline)?;
        } else {
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
        let deadline = Instant::now() + std::time::Duration::from_secs(60);

        let buf = Vec::new();
        let cursor = std::io::Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("hello.txt", options).unwrap();
        writer.write_all(b"world").unwrap();
        writer.start_file("sub/nested.txt", options).unwrap();
        writer.write_all(b"nested content").unwrap();
        let cursor = writer.finish().unwrap();

        extract_zip_with_limits(cursor.get_ref(), dir.path(), deadline).unwrap();

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
    fn extract_and_apply_atomic() {
        use std::io::Write;
        let target = tempfile::tempdir().unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(60);

        std::fs::write(target.path().join("existing.txt"), "keep").unwrap();

        let buf = Vec::new();
        let cursor = std::io::Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("new.txt", options).unwrap();
        writer.write_all(b"added").unwrap();
        let cursor = writer.finish().unwrap();

        extract_and_apply(cursor.get_ref(), target.path(), deadline).unwrap();

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
    fn extract_respects_expired_deadline() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        // Already expired deadline
        let deadline = Instant::now() - std::time::Duration::from_secs(1);

        let buf = Vec::new();
        let cursor = std::io::Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("a.txt", options).unwrap();
        writer.write_all(b"data").unwrap();
        let cursor = writer.finish().unwrap();

        // extract_and_apply should fail due to expired deadline
        let result = extract_and_apply(cursor.get_ref(), dir.path(), deadline);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[test]
    fn extract_zip_overwrites() {
        use std::io::Write;
        let target = tempfile::tempdir().unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(60);
        std::fs::write(target.path().join("hello.txt"), "original").unwrap();

        let buf = Vec::new();
        let cursor = std::io::Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("hello.txt", options).unwrap();
        writer.write_all(b"overwritten").unwrap();
        let cursor = writer.finish().unwrap();

        extract_and_apply(cursor.get_ref(), target.path(), deadline).unwrap();

        assert_eq!(
            std::fs::read_to_string(target.path().join("hello.txt")).unwrap(),
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

    #[test]
    fn move_respects_deadline() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("f.txt"), "x").unwrap();

        let expired = Instant::now() - std::time::Duration::from_secs(1);
        let result = move_recursive(src.path(), dst.path(), expired);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }
}
