//! Shared resolution for `spec.secrets` values.
//!
//! Values can be either a Secrets Manager reference in ECS-native
//! `valueFrom` format directly (a full ARN, optionally suffixed with
//! `:<jsonKey>::` to extract one field of a JSON secret — an ECS-only
//! convention; the Secrets Manager API itself has no knowledge of it), or
//! the same `aws-sm://<secret-id>#<json-key>` shorthand openab itself uses
//! for in-app secret refs (see `crates/openab-core/src/secrets.rs`) — kept
//! identical here so a manifest author can write one convention across both
//! `spec.secrets` (consumed by ECS at container launch) and `config.toml`
//! (consumed by openab itself at runtime).

use anyhow::{Context, Result};

/// Parse `aws-sm://<secret-id>#<json-key>` into `(secret_id, json_key)`.
/// Returns `None` if `value` doesn't use the `aws-sm://` scheme.
fn parse_aws_sm_uri(value: &str) -> Option<Result<(&str, &str)>> {
    let rest = value.strip_prefix("aws-sm://")?;
    Some(match rest.rsplit_once('#') {
        Some((secret_id, json_key)) if !secret_id.is_empty() && !json_key.is_empty() => {
            Ok((secret_id, json_key))
        }
        _ => Err(anyhow::anyhow!(
            "invalid aws-sm:// secret ref '{value}' — expected aws-sm://<secret-id>#<json-key>"
        )),
    })
}

/// Resolve a `spec.secrets` value into the ECS-native `valueFrom` format ECS
/// actually requires. ECS's `valueFrom` requires the *full* ARN (not just a
/// secret name) whenever a JSON-key suffix is present, so an `aws-sm://`
/// secret-id that isn't already an ARN is resolved to its ARN via
/// `DescribeSecret` first. Values already in ECS-native format are passed
/// through unchanged.
pub async fn resolve_value_from(
    sm: &aws_sdk_secretsmanager::Client,
    value: &str,
) -> Result<String> {
    let Some(parsed) = parse_aws_sm_uri(value) else {
        return Ok(value.to_string());
    };
    let (secret_id, json_key) = parsed?;

    let arn = if secret_id.starts_with("arn:") {
        secret_id.to_string()
    } else {
        sm.describe_secret()
            .secret_id(secret_id)
            .send()
            .await
            .with_context(|| format!("failed to resolve secret '{secret_id}' to an ARN"))?
            .arn()
            .with_context(|| format!("secret '{secret_id}' has no ARN"))?
            .to_string()
    };
    Ok(format!("{arn}:{json_key}::"))
}

/// Split an ECS-native `valueFrom` value into its base secret ARN/name and
/// an optional JSON key, if it carries the `:<jsonKey>::` suffix ECS uses to
/// extract one field of a JSON secret. Only applies to values already
/// shaped like an ARN (`arn:...:secret:<name>-<suffix>:<jsonKey>::`) — a
/// bare secret name is never split, since ECS only recognizes the suffix on
/// a full ARN. Returns `(value, None)` unchanged if there's no such suffix.
fn split_ecs_json_key_suffix(value: &str) -> (&str, Option<&str>) {
    if !value.starts_with("arn:") {
        return (value, None);
    }
    // `<base-arn>:<jsonKey>::` — strip the trailing empty segment (from the
    // final `::`), then split off exactly one more `:<jsonKey>` segment. A
    // bare secret ARN has none of this, so `rest` won't match the pattern
    // and falls through to the unchanged case.
    let Some(without_trailing) = value.strip_suffix("::") else {
        return (value, None);
    };
    let Some((base, json_key)) = without_trailing.rsplit_once(':') else {
        return (value, None);
    };
    if json_key.is_empty() || !base.starts_with("arn:") {
        return (value, None);
    }
    (base, Some(json_key))
}

/// Resolve a `spec.secrets` value to its plain string content, for callers
/// that need the actual secret value in-process (e.g. calling a third-party
/// API on the caller's behalf) rather than an ECS `valueFrom` reference.
/// Supports the same two forms as [`resolve_value_from`]: `aws-sm://...#...`
/// (fetched and JSON-key-extracted here), or a plain/ECS-native Secrets
/// Manager ARN — including one already carrying a `:<jsonKey>::` suffix.
/// That suffix is an ECS-only convention (resolved by ECS itself at
/// container launch, via `register_task_definition`'s `valueFrom` field) —
/// the Secrets Manager `GetSecretValue` API has no knowledge of it and
/// rejects it as an invalid secret ID, so it's stripped and the JSON key
/// extracted manually here, the same way the `aws-sm://` form is.
pub async fn resolve_string(sm: &aws_sdk_secretsmanager::Client, value: &str) -> Result<String> {
    let (secret_id, json_key) = match parse_aws_sm_uri(value) {
        Some(parsed) => {
            let (id, key) = parsed?;
            (id, Some(key))
        }
        None => split_ecs_json_key_suffix(value),
    };

    let secret_string = sm
        .get_secret_value()
        .secret_id(secret_id)
        .send()
        .await
        .with_context(|| format!("failed to fetch secret '{secret_id}' from Secrets Manager"))?
        .secret_string()
        .with_context(|| format!("secret '{secret_id}' has no string value"))?
        .to_string();

    let Some(json_key) = json_key else {
        return Ok(secret_string);
    };
    let json: serde_json::Value = serde_json::from_str(&secret_string)
        .with_context(|| format!("secret '{secret_id}' is not valid JSON"))?;
    json.get(json_key)
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .with_context(|| format!("JSON key '{json_key}' not found in secret '{secret_id}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_ecs_json_key_suffix_extracts_key_from_real_world_arn() {
        // The exact shape that surfaced this bug: an ECS-native valueFrom
        // ARN with a JSON-key suffix, passed to resolve_string (which used
        // to hand this straight to GetSecretValue and fail, since that API
        // has no knowledge of the trailing `:<jsonKey>::` ECS convention).
        let (base, key) = split_ecs_json_key_suffix(
            "arn:aws:secretsmanager:us-east-1:903779448426:secret:oab/telegram/pahudxbot-AC80TP:TELEGRAM_BOT_TOKEN::",
        );
        assert_eq!(base, "arn:aws:secretsmanager:us-east-1:903779448426:secret:oab/telegram/pahudxbot-AC80TP");
        assert_eq!(key, Some("TELEGRAM_BOT_TOKEN"));
    }

    #[test]
    fn split_ecs_json_key_suffix_unchanged_for_plain_arn() {
        let (base, key) = split_ecs_json_key_suffix(
            "arn:aws:secretsmanager:us-east-1:903779448426:secret:oab/telegram/pahudxbot-AC80TP",
        );
        assert_eq!(base, "arn:aws:secretsmanager:us-east-1:903779448426:secret:oab/telegram/pahudxbot-AC80TP");
        assert_eq!(key, None);
    }

    #[test]
    fn split_ecs_json_key_suffix_unchanged_for_bare_secret_name() {
        let (base, key) = split_ecs_json_key_suffix("plain-secret-name");
        assert_eq!(base, "plain-secret-name");
        assert_eq!(key, None);
    }

    #[test]
    fn parse_aws_sm_uri_extracts_id_and_key() {
        let (id, key) = parse_aws_sm_uri("aws-sm://oab/telegram/pahudxbot#TELEGRAM_BOT_TOKEN")
            .unwrap()
            .unwrap();
        assert_eq!(id, "oab/telegram/pahudxbot");
        assert_eq!(key, "TELEGRAM_BOT_TOKEN");
    }

    #[test]
    fn parse_aws_sm_uri_rejects_missing_hash() {
        assert!(parse_aws_sm_uri("aws-sm://oab/telegram/pahudxbot").unwrap().is_err());
    }

    #[test]
    fn parse_aws_sm_uri_rejects_empty_parts() {
        assert!(parse_aws_sm_uri("aws-sm://#key").unwrap().is_err());
        assert!(parse_aws_sm_uri("aws-sm://secret-id#").unwrap().is_err());
    }

    #[test]
    fn parse_aws_sm_uri_returns_none_for_other_schemes() {
        assert!(parse_aws_sm_uri("arn:aws:secretsmanager:us-east-1:123:secret:oab/x-AbCdEf").is_none());
        assert!(parse_aws_sm_uri("plain-secret-name").is_none());
    }
}
