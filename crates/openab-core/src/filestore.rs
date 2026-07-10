//! S3/R2-compatible object store for uploading large text file attachments
//! and returning presigned GET URLs.

use crate::config::FilestoreConfig;
use std::time::Duration;
use tracing::{error, info};

/// Manages uploads to an S3-compatible object store and generates presigned
/// GET URLs for retrieval without authentication.
pub struct Filestore {
    client: aws_sdk_s3::Client,
    bucket: String,
    prefix: String,
    presigned_ttl: Duration,
}

impl Filestore {
    /// Initialize a new Filestore from the given configuration.
    ///
    /// Builds an S3 client with optional custom endpoint and explicit credentials.
    /// Falls back to the standard AWS provider chain when credentials are not
    /// specified in config.
    pub async fn new(config: &FilestoreConfig) -> Self {
        let mut sdk_config_loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(config.region.clone()));

        if let (Some(access_key), Some(secret_key)) =
            (&config.access_key_id, &config.secret_access_key)
        {
            let creds = aws_sdk_s3::config::Credentials::new(
                access_key.clone(),
                secret_key.clone(),
                None,
                None,
                "filestore-config",
            );
            sdk_config_loader = sdk_config_loader.credentials_provider(creds);
        }

        let sdk_config = sdk_config_loader.load().await;

        let mut s3_config_builder = aws_sdk_s3::config::Builder::from(&sdk_config);

        if let Some(endpoint) = &config.endpoint {
            // Path-style access is required for most S3-compatible services
            // (R2, MinIO) but deprecated by AWS S3 itself.
            s3_config_builder = s3_config_builder
                .endpoint_url(endpoint.clone())
                .force_path_style(true);
        }

        let client = aws_sdk_s3::Client::from_conf(s3_config_builder.build());

        // Cap presigned TTL at 7 days to prevent excessively long-lived URLs.
        const MAX_PRESIGNED_TTL: u64 = 7 * 24 * 60 * 60; // 7 days
        let ttl_secs = config.presigned_ttl.min(MAX_PRESIGNED_TTL);
        if config.presigned_ttl > MAX_PRESIGNED_TTL {
            tracing::warn!(
                configured = config.presigned_ttl,
                capped = MAX_PRESIGNED_TTL,
                "presigned_ttl exceeds 7-day maximum, capping"
            );
        }

        Self {
            client,
            bucket: config.bucket.clone(),
            prefix: config.prefix.clone(),
            presigned_ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Upload a file to S3 and return a presigned GET URL.
    ///
    /// The object key is `{prefix}{uuid}_{filename}`. On success returns the
    /// presigned URL as a String. On failure logs the error and returns Err.
    pub async fn upload_and_presign(
        &self,
        filename: &str,
        data: &[u8],
    ) -> anyhow::Result<String> {
        // Sanitize filename: strip path separators, traversal sequences, and
        // non-ASCII chars. Limit length to prevent excessively long S3 keys.
        let safe_name: String = filename
            .replace(['/', '\\', '\0'], "_")
            .replace("..", "_")
            .chars()
            .filter(|c| c.is_ascii_graphic() || *c == ' ')
            .take(200)
            .collect();
        let safe_name = if safe_name.is_empty() { "unnamed" } else { &safe_name };
        let key = format!(
            "{}{}_{}",
            self.prefix,
            uuid::Uuid::new_v4(),
            safe_name
        );

        // Upload the object
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .content_type("text/plain; charset=utf-8")
            .body(aws_sdk_s3::primitives::ByteStream::from(data.to_vec()))
            .send()
            .await
            .map_err(|e| {
                error!(bucket = %self.bucket, key = %key, error = %e, "filestore upload failed");
                anyhow::anyhow!("filestore upload failed: {e}")
            })?;

        info!(bucket = %self.bucket, key = %key, size = data.len(), "filestore upload complete");

        // Generate presigned GET URL
        let presigning_config =
            aws_sdk_s3::presigning::PresigningConfig::expires_in(self.presigned_ttl)
                .map_err(|e| anyhow::anyhow!("presigning config error: {e}"))?;

        let presigned = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .presigned(presigning_config)
            .await
            .map_err(|e| {
                error!(bucket = %self.bucket, key = %key, error = %e, "presigned URL generation failed");
                anyhow::anyhow!("presigned URL generation failed: {e}")
            })?;

        Ok(presigned.uri().to_string())
    }

    /// Return the configured presigned TTL in seconds.
    pub fn presigned_ttl_secs(&self) -> u64 {
        self.presigned_ttl.as_secs()
    }
}

/// Format the hint block returned to the agent when a large file is uploaded
/// to the filestore instead of being inlined.
pub fn format_filestore_hint(filename: &str, size_bytes: u64, presigned_url: &str, ttl_secs: u64) -> String {
    let size_kb = size_bytes / 1024;
    let ttl_minutes = ttl_secs / 60;
    format!(
        "[File: {filename}]\n\
         This file ({size_kb} KB) exceeds the 512 KB inline limit. \
         It has been uploaded to temporary storage. \
         Fetch the contents using the URL below:\n\
         {presigned_url}\n\
         Note: this URL expires in {ttl_minutes} minutes."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filestore_config_deserializes_with_defaults() {
        let toml_str = r#"
bucket = "my-oab-files"
region = "us-west-2"
"#;
        let config: FilestoreConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.bucket, "my-oab-files");
        assert_eq!(config.region, "us-west-2");
        assert!(config.endpoint.is_none());
        assert_eq!(config.prefix, "incoming/");
        assert_eq!(config.presigned_ttl, 3600);
        assert!(config.access_key_id.is_none());
        assert!(config.secret_access_key.is_none());
    }

    #[test]
    fn filestore_config_deserializes_full() {
        let toml_str = r#"
bucket = "my-bucket"
region = "eu-west-1"
endpoint = "https://abc123.r2.cloudflarestorage.com"
prefix = "uploads/"
presigned_ttl = 7200
access_key_id = "AKID"
secret_access_key = "SECRET"
"#;
        let config: FilestoreConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.bucket, "my-bucket");
        assert_eq!(config.region, "eu-west-1");
        assert_eq!(
            config.endpoint.as_deref(),
            Some("https://abc123.r2.cloudflarestorage.com")
        );
        assert_eq!(config.prefix, "uploads/");
        assert_eq!(config.presigned_ttl, 7200);
        assert_eq!(config.access_key_id.as_deref(), Some("AKID"));
        assert_eq!(config.secret_access_key.as_deref(), Some("SECRET"));
    }

    #[test]
    fn format_filestore_hint_produces_expected_output() {
        let hint = format_filestore_hint(
            "big-log.txt",
            1_048_576, // 1 MB
            "https://bucket.s3.amazonaws.com/incoming/uuid_big-log.txt?X-Amz-...",
            3600,
        );
        assert!(hint.contains("[File: big-log.txt]"));
        assert!(hint.contains("1024 KB"));
        assert!(hint.contains("exceeds the 512 KB inline limit"));
        assert!(hint.contains("https://bucket.s3.amazonaws.com/incoming/uuid_big-log.txt?X-Amz-..."));
        assert!(hint.contains("expires in 60 minutes"));
    }

    #[test]
    fn format_filestore_hint_short_ttl() {
        let hint = format_filestore_hint("data.csv", 600_000, "https://example.com/file", 900);
        assert!(hint.contains("585 KB"));
        assert!(hint.contains("expires in 15 minutes"));
    }
}
