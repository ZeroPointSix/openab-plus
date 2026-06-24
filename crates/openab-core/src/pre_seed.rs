use crate::config::{OnFailure, PreSeedConfig};
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
            "pre_seed: too many sources ({}, max {})",
            cfg.sources.len(),
            MAX_SOURCES
        );
    }

    let target = match &cfg.target {
        Some(t) => std::path::PathBuf::from(t),
        None => dirs_home(),
    };

    info!(
        sources = cfg.sources.len(),
        target = %target.display(),
        "pre_seed: starting"
    );

    let aws_cfg = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let s3 = aws_sdk_s3::Client::new(&aws_cfg);

    for (i, source) in cfg.sources.iter().enumerate() {
        let layer = i + 1;
        info!(layer, source = source.as_str(), "pre_seed: downloading");

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(cfg.timeout_seconds),
            download_and_extract(&s3, source, &target),
        )
        .await;

        let outcome = match result {
            Ok(Ok(())) => {
                info!(layer, "pre_seed: layer extracted successfully");
                continue;
            }
            Ok(Err(e)) => e,
            Err(_) => anyhow::anyhow!(
                "pre_seed: layer {layer} timed out after {}s",
                cfg.timeout_seconds
            ),
        };

        match cfg.on_failure {
            OnFailure::Abort => {
                error!(layer, error = %outcome, "pre_seed failed (on_failure=abort)");
                return Err(outcome);
            }
            OnFailure::Warn => {
                warn!(layer, error = %outcome, "pre_seed failed (on_failure=warn), continuing");
            }
        }
    }

    info!("pre_seed: complete");
    Ok(())
}

/// Parse s3://bucket/key, download the object, and extract the zip to target.
async fn download_and_extract(
    s3: &aws_sdk_s3::Client,
    uri: &str,
    target: &Path,
) -> anyhow::Result<()> {
    let (bucket, key) = parse_s3_uri(uri)?;

    let resp = s3
        .get_object()
        .bucket(&bucket)
        .key(&key)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("S3 GetObject failed for {uri}: {e}"))?;

    let body = resp
        .body
        .collect()
        .await
        .map_err(|e| anyhow::anyhow!("failed to read S3 body for {uri}: {e}"))?;
    let bytes = body.into_bytes();

    info!(uri, bytes = bytes.len(), "pre_seed: downloaded, extracting");

    let target = target.to_path_buf();
    let bytes_vec = bytes.to_vec();
    tokio::task::spawn_blocking(move || extract_zip(&bytes_vec, &target))
        .await
        .map_err(|e| anyhow::anyhow!("pre_seed: extract task panicked: {e}"))??;

    Ok(())
}

/// Extract a zip archive from memory into the target directory.
fn extract_zip(data: &[u8], target: &Path) -> anyhow::Result<()> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file
            .enclosed_name()
            .ok_or_else(|| anyhow::anyhow!("pre_seed: invalid zip entry name at index {i}"))?;
        let out_path = target.join(name);

        if file.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&out_path)?;
            std::io::copy(&mut file, &mut out)?;

            // Preserve unix permissions
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

/// Parse an s3://bucket/key URI into (bucket, key).
fn parse_s3_uri(uri: &str) -> anyhow::Result<(String, String)> {
    let stripped = uri
        .strip_prefix("s3://")
        .ok_or_else(|| anyhow::anyhow!("pre_seed: source must start with s3://, got: {uri}"))?;
    let (bucket, key) = stripped
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("pre_seed: invalid S3 URI (no key): {uri}"))?;
    if bucket.is_empty() || key.is_empty() {
        anyhow::bail!("pre_seed: empty bucket or key in URI: {uri}");
    }
    Ok((bucket.to_string(), key.to_string()))
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
    fn parse_s3_uri_valid() {
        let (b, k) = parse_s3_uri("s3://my-bucket/path/to/file.zip").unwrap();
        assert_eq!(b, "my-bucket");
        assert_eq!(k, "path/to/file.zip");
    }

    #[test]
    fn parse_s3_uri_no_prefix() {
        assert!(parse_s3_uri("https://example.com/file.zip").is_err());
    }

    #[test]
    fn parse_s3_uri_no_key() {
        assert!(parse_s3_uri("s3://bucket-only").is_err());
    }

    #[test]
    fn parse_s3_uri_empty_key() {
        assert!(parse_s3_uri("s3://bucket/").is_err());
    }

    #[test]
    fn parse_s3_uri_empty_bucket() {
        assert!(parse_s3_uri("s3:///key").is_err());
    }

    #[test]
    fn extract_zip_basic() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();

        // Create a zip in memory
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
    fn extract_zip_overwrites() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();

        // Write an initial file
        std::fs::write(dir.path().join("hello.txt"), "original").unwrap();

        // Create a zip that overwrites it
        let buf = Vec::new();
        let cursor = std::io::Cursor::new(buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("hello.txt", options).unwrap();
        writer.write_all(b"overwritten").unwrap();
        let cursor = writer.finish().unwrap();

        extract_zip(cursor.get_ref(), dir.path()).unwrap();

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
}
