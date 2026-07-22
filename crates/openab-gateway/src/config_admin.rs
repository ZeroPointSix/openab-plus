use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

const DEFAULT_CONFIG_PATH: &str = "config/gateway.toml";
const MASKED_SECRET: &str = "********";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplyPolicy {
    Runtime,
    NewSession,
    RestartRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldMetadata {
    pub path: String,
    pub apply_policy: ApplyPolicy,
    pub secret: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigDocument {
    pub values: Value,
    pub metadata: Vec<FieldMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub path: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub ok: bool,
    pub errors: Vec<ValidationError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigStatus {
    pub config_path: String,
    pub last_saved_at: Option<DateTime<Utc>>,
    pub last_loaded_hash: Option<String>,
    pub pending_restart: Vec<String>,
    pub rollback_available: bool,
    pub last_validation: Option<ValidationResult>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateConfigRequest {
    pub values: Value,
}

#[derive(Debug, Serialize)]
pub struct UpdateConfigResponse {
    pub validation: ValidationResult,
    pub status: ConfigStatus,
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn from_env() -> Self {
        let path = std::env::var("GATEWAY_CONFIG_PATH")
            .unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string());
        Self { path: path.into() }
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn load(&self) -> anyhow::Result<Value> {
        let raw = match tokio::fs::read_to_string(&self.path).await {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        if raw.trim().is_empty() {
            return Ok(Value::Object(Default::default()));
        }
        parse_toml_json(&raw)
    }

    pub async fn save_atomic(&self, values: &Value) -> anyhow::Result<()> {
        let raw = toml::to_string_pretty(values)?;
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        if tokio::fs::try_exists(&self.path).await.unwrap_or(false) {
            tokio::fs::copy(&self.path, self.backup_path()).await?;
        }

        let tmp = self.path.with_extension("tmp");
        tokio::fs::write(&tmp, raw).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        Ok(())
    }

    pub async fn rollback_available(&self) -> bool {
        tokio::fs::try_exists(self.backup_path()).await.unwrap_or(false)
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
}

#[derive(Debug)]
struct ManagerState {
    last_saved_at: Option<DateTime<Utc>>,
    last_loaded_hash: Option<String>,
    pending_restart: BTreeSet<String>,
    last_validation: Option<ValidationResult>,
}

#[derive(Debug)]
pub struct ConfigManager {
    store: ConfigStore,
    state: Mutex<ManagerState>,
}

impl ConfigManager {
    pub fn from_env() -> Arc<Self> {
        Arc::new(Self::new(ConfigStore::from_env()))
    }

    pub fn new(store: ConfigStore) -> Self {
        Self {
            store,
            state: Mutex::new(ManagerState {
                last_saved_at: None,
                last_loaded_hash: None,
                pending_restart: BTreeSet::new(),
                last_validation: None,
            }),
        }
    }

    pub async fn read(&self) -> anyhow::Result<ConfigDocument> {
        let values = self.store.load().await?;
        let hash = hash_value(&values)?;
        self.state.lock().await.last_loaded_hash = Some(hash);
        Ok(ConfigDocument {
            values: mask_secrets(values),
            metadata: field_metadata(),
        })
    }

    pub async fn validate_values(&self, values: &Value) -> ValidationResult {
        validate_config(values)
    }

    pub async fn save(&self, values: Value) -> anyhow::Result<UpdateConfigResponse> {
        let validation = self.validate_values(&values).await;
        if !validation.ok {
            self.state.lock().await.last_validation = Some(validation.clone());
            return Ok(UpdateConfigResponse {
                validation,
                status: self.status().await,
            });
        }

        self.store.save_atomic(&values).await?;

        let pending_restart = restart_required_paths(&values);
        let hash = hash_value(&values)?;
        {
            let mut state = self.state.lock().await;
            state.last_saved_at = Some(Utc::now());
            state.last_loaded_hash = Some(hash);
            state.pending_restart = pending_restart.into_iter().collect();
            state.last_validation = Some(validation.clone());
        }

        Ok(UpdateConfigResponse {
            validation,
            status: self.status().await,
        })
    }

    pub async fn mark_reloaded(&self) -> ConfigStatus {
        self.state.lock().await.pending_restart.clear();
        self.status().await
    }

    pub async fn status(&self) -> ConfigStatus {
        let state = self.state.lock().await;
        ConfigStatus {
            config_path: self.store.path().display().to_string(),
            last_saved_at: state.last_saved_at,
            last_loaded_hash: state.last_loaded_hash.clone(),
            pending_restart: state.pending_restart.iter().cloned().collect(),
            rollback_available: futures_util::future::ready(()).await; self.store.rollback_available().await,
            last_validation: state.last_validation.clone(),
        }
    }
}

pub fn router<S>(manager: Arc<ConfigManager>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let get_manager = manager.clone();
    let put_manager = manager.clone();
    let validate_manager = manager.clone();
    let reload_manager = manager.clone();
    let status_manager = manager;

    Router::new()
        .route(
            "/api/v1/config",
            get(move |headers| get_config(headers, get_manager.clone()))
                .put(move |headers, body| put_config(headers, body, put_manager.clone())),
        )
        .route(
            "/api/v1/config/validate",
            post(move |headers, body| validate_config_handler(headers, body, validate_manager.clone())),
        )
        .route(
            "/api/v1/config/reload",
            post(move |headers| reload_config(headers, reload_manager.clone())),
        )
        .route(
            "/api/v1/config/status",
            get(move |headers| get_status(headers, status_manager.clone())),
        )
}

async fn get_config(headers: HeaderMap, manager: Arc<ConfigManager>) -> Response {
    if let Err(resp) = authorize(&headers) {
        return resp;
    }
    match manager.read().await {
        Ok(doc) => Json(doc).into_response(),
        Err(e) => internal_error(e),
    }
}

async fn put_config(
    headers: HeaderMap,
    Json(req): Json<UpdateConfigRequest>,
    manager: Arc<ConfigManager>,
) -> Response {
    if let Err(resp) = authorize(&headers) {
        return resp;
    }
    match manager.save(req.values).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => internal_error(e),
    }
}

async fn validate_config_handler(
    headers: HeaderMap,
    Json(req): Json<UpdateConfigRequest>,
    manager: Arc<ConfigManager>,
) -> Response {
    if let Err(resp) = authorize(&headers) {
        return resp;
    }
    Json(manager.validate_values(&req.values).await).into_response()
}

async fn reload_config(headers: HeaderMap, manager: Arc<ConfigManager>) -> Response {
    if let Err(resp) = authorize(&headers) {
        return resp;
    }
    Json(manager.mark_reloaded().await).into_response()
}

async fn get_status(headers: HeaderMap, manager: Arc<ConfigManager>) -> Response {
    if let Err(resp) = authorize(&headers) {
        return resp;
    }
    Json(manager.status().await).into_response()
}

fn authorize(headers: &HeaderMap) -> Result<(), Response> {
    let expected = std::env::var("GATEWAY_ADMIN_TOKEN")
        .or_else(|_| std::env::var("OPENAB_ADMIN_TOKEN"))
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "admin token is not configured" })),
            )
                .into_response()
        })?;

    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let header_token = headers
        .get("x-openab-admin-token")
        .and_then(|v| v.to_str().ok());

    if bearer == Some(expected.as_str()) || header_token == Some(expected.as_str()) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid or missing admin token" })),
        )
            .into_response())
    }
}

fn internal_error(error: anyhow::Error) -> Response {
    tracing::error!(error = %error, "gateway config admin error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": error.to_string() })),
    )
        .into_response()
}

fn parse_toml_json(raw: &str) -> anyhow::Result<Value> {
    let value: toml::Value = toml::from_str(raw)?;
    Ok(serde_json::to_value(value)?)
}

fn hash_value(value: &Value) -> anyhow::Result<String> {
    let bytes = serde_json::to_vec(value)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{digest:x}"))
}

fn validate_config(values: &Value) -> ValidationResult {
    let mut errors = Vec::new();

    if toml::to_string_pretty(values).is_err() {
        errors.push(ValidationError {
            path: "$".into(),
            code: "invalid_toml_shape".into(),
            message: "configuration cannot be represented as TOML".into(),
        });
    }

    validate_webhook_path(values, &["telegram", "webhook_path"], &mut errors);
    validate_webhook_path(values, &["line", "webhook_path"], &mut errors);
    validate_webhook_path(values, &["wecom", "webhook_path"], &mut errors);
    validate_webhook_path(values, &["googlechat", "webhook_path"], &mut errors);
    validate_webhook_path(values, &["teams", "webhook_path"], &mut errors);
    validate_webhook_path(values, &["feishu", "webhook_path"], &mut errors);

    if let Some(url) = get_string(values, &["gateway", "url"]) {
        if !(url.starts_with("ws://") || url.starts_with("wss://")) {
            errors.push(ValidationError {
                path: "gateway.url".into(),
                code: "invalid_ws_url".into(),
                message: "gateway.url must start with ws:// or wss://".into(),
            });
        }
    }

    for section in ["telegram", "line", "wecom", "googlechat", "teams", "feishu"] {
        validate_allowlist(values, section, &mut errors);
    }

    ValidationResult {
        ok: errors.is_empty(),
        errors,
    }
}

fn validate_webhook_path(values: &Value, path: &[&str], errors: &mut Vec<ValidationError>) {
    if let Some(path_value) = get_string(values, path) {
        if !path_value.starts_with('/') || path_value.contains("//") {
            errors.push(ValidationError {
                path: path.join("."),
                code: "invalid_webhook_path".into(),
                message: "webhook path must start with / and must not contain //".into(),
            });
        }
    }
}

fn validate_allowlist(values: &Value, section: &str, errors: &mut Vec<ValidationError>) {
    let allow_all = get_bool(values, &[section, "allow_all_users"]);
    let users = get_array(values, &[section, "allowed_users"]);
    if allow_all == Some(false) && users.is_some_and(|v| v.is_empty()) {
        errors.push(ValidationError {
            path: format!("{section}.allowed_users"),
            code: "empty_allowlist".into(),
            message: "allow_all_users=false requires at least one allowed user".into(),
        });
    }
}

fn restart_required_paths(values: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    for path in [
        "gateway.url",
        "gateway.platform",
        "telegram.webhook_path",
        "line.webhook_path",
        "wecom.webhook_path",
        "googlechat.webhook_path",
        "teams.webhook_path",
        "feishu.webhook_path",
    ] {
        if contains_path(values, &path.split('.').collect::<Vec<_>>()) {
            paths.push(path.to_string());
        }
    }
    paths
}

fn field_metadata() -> Vec<FieldMetadata> {
    let mut metadata = Vec::new();
    for path in [
        "telegram.bot_token",
        "telegram.secret_token",
        "line.channel_secret",
        "line.channel_access_token",
        "wecom.secret",
        "wecom.token",
        "wecom.encoding_aes_key",
        "googlechat.sa_key_json",
        "googlechat.access_token",
        "teams.app_secret",
        "feishu.app_secret",
        "feishu.verification_token",
        "feishu.encrypt_key",
        "gateway.token",
    ] {
        metadata.push(FieldMetadata {
            path: path.into(),
            apply_policy: ApplyPolicy::RestartRequired,
            secret: true,
        });
    }
    for path in [
        "telegram.rich_messages",
        "telegram.trusted_source_only",
        "telegram.streaming",
        "wecom.streaming_enabled",
        "wecom.debounce_secs",
    ] {
        metadata.push(FieldMetadata {
            path: path.into(),
            apply_policy: ApplyPolicy::Runtime,
            secret: false,
        });
    }
    metadata
}

fn mask_secrets(mut value: Value) -> Value {
    mask_value(&mut value, None);
    value
}

fn mask_value(value: &mut Value, key: Option<&str>) {
    if key.is_some_and(is_secret_key) && value.is_string() {
        *value = Value::String(MASKED_SECRET.into());
        return;
    }

    match value {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                mask_value(v, Some(k));
            }
        }
        Value::Array(values) => {
            for v in values {
                mask_value(v, key);
            }
        }
        _ => {}
    }
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    ["token", "secret", "password", "credential", "key"]
        .iter()
        .any(|needle| key.contains(needle))
}

fn get_string<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(value, |v, key| v.get(*key))?.as_str()
}

fn get_bool(value: &Value, path: &[&str]) -> Option<bool> {
    path.iter().try_fold(value, |v, key| v.get(*key))?.as_bool()
}

fn get_array<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Vec<Value>> {
    path.iter().try_fold(value, |v, key| v.get(*key))?.as_array()
}

fn contains_path(value: &Value, path: &[&str]) -> bool {
    path.iter().try_fold(value, |v, key| v.get(*key)).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_secret_fields_recursively() {
        let value = json!({
            "telegram": { "bot_token": "abc", "rich_messages": true },
            "nested": [{ "password": "pw" }]
        });
        let masked = mask_secrets(value);
        assert_eq!(masked["telegram"]["bot_token"], MASKED_SECRET);
        assert_eq!(masked["telegram"]["rich_messages"], true);
        assert_eq!(masked["nested"][0]["password"], MASKED_SECRET);
    }

    #[test]
    fn validates_gateway_url_and_webhook_paths() {
        let result = validate_config(&json!({
            "gateway": { "url": "http://bad" },
            "line": { "webhook_path": "bad" }
        }));
        assert!(!result.ok);
        assert_eq!(result.errors.len(), 2);
    }

    #[test]
    fn allowlist_false_requires_users() {
        let result = validate_config(&json!({
            "telegram": { "allow_all_users": false, "allowed_users": [] }
        }));
        assert!(!result.ok);
        assert_eq!(result.errors[0].code, "empty_allowlist");
    }

    #[tokio::test]
    async fn store_round_trips_toml_with_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gateway.toml");
        let store = ConfigStore::new(&path);
        let values = json!({ "line": { "webhook_path": "/hook/line" } });
        store.save_atomic(&values).await.unwrap();
        let loaded = store.load().await.unwrap();
        assert_eq!(loaded["line"]["webhook_path"], "/hook/line");

        store.save_atomic(&json!({ "telegram": { "webhook_path": "/hook/tg" } }))
            .await
            .unwrap();
        assert!(store.rollback_available().await);
    }
}
