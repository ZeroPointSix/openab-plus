use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
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

#[derive(Debug, Clone, Serialize)]
pub struct AppliedRuntimeConfig {
    pub applied_paths: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ReloadConfigResponse {
    pub validation: ValidationResult,
    pub runtime: AppliedRuntimeConfig,
    pub status: ConfigStatus,
}

#[derive(Clone)]
pub struct RuntimeConfigApplier {
    state: Arc<crate::AppState>,
    baseline: crate::RuntimeGatewayConfig,
}

impl std::fmt::Debug for RuntimeConfigApplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeConfigApplier")
            .field("baseline", &self.baseline)
            .finish_non_exhaustive()
    }
}

impl RuntimeConfigApplier {
    pub fn new(state: Arc<crate::AppState>) -> Self {
        let baseline = crate::RuntimeGatewayConfig {
            telegram_rich_messages: state.telegram_rich_messages,
            telegram_trusted_source_only: state.telegram_trusted_source_only,
            telegram_streaming: state.telegram_streaming,
        };
        Self { state, baseline }
    }

    pub async fn apply(&self, values: &Value) -> anyhow::Result<AppliedRuntimeConfig> {
        let mut next = self.baseline.clone();
        let mut applied_paths = Vec::new();

        if let Some(value) = get_bool(values, &["telegram", "rich_messages"]) {
            next.telegram_rich_messages = value;
            applied_paths.push("telegram.rich_messages".to_string());
        }
        if let Some(value) = get_bool(values, &["telegram", "trusted_source_only"]) {
            next.telegram_trusted_source_only = value;
            applied_paths.push("telegram.trusted_source_only".to_string());
        }
        if let Some(value) = get_bool(values, &["telegram", "streaming"]) {
            next.telegram_streaming = Some(value);
            applied_paths.push("telegram.streaming".to_string());
        }

        *self.state.runtime_config.write().await = next;
        Ok(AppliedRuntimeConfig { applied_paths })
    }
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
    runtime_applier: Mutex<Option<RuntimeConfigApplier>>,
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
            runtime_applier: Mutex::new(None),
        }
    }

    pub async fn set_runtime_applier(&self, applier: RuntimeConfigApplier) {
        *self.runtime_applier.lock().await = Some(applier);
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

    pub async fn save(&self, mut values: Value) -> anyhow::Result<UpdateConfigResponse> {
        let current = self.store.load().await?;
        if contains_masked_secret(&values) {
            preserve_masked_secrets(&mut values, &current);
        }

        let validation = self.validate_values(&values).await;
        if !validation.ok {
            self.state.lock().await.last_validation = Some(validation.clone());
            return Ok(UpdateConfigResponse {
                validation,
                status: self.status().await,
            });
        }

        self.store.save_atomic(&values).await?;

        let pending_restart = changed_restart_required_paths(&current, &values);
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

    pub async fn reload(&self) -> anyhow::Result<ReloadConfigResponse> {
        let values = self.store.load().await?;
        let validation = self.validate_values(&values).await;

        if !validation.ok {
            self.state.lock().await.last_validation = Some(validation.clone());
            return Ok(ReloadConfigResponse {
                validation,
                runtime: AppliedRuntimeConfig {
                    applied_paths: Vec::new(),
                },
                status: self.status().await,
            });
        }

        let applier = self.runtime_applier.lock().await.clone();
        let runtime = if let Some(applier) = applier {
            applier.apply(&values).await?
        } else {
            AppliedRuntimeConfig {
                applied_paths: Vec::new(),
            }
        };

        let hash = hash_value(&values)?;
        {
            let mut state = self.state.lock().await;
            state.last_loaded_hash = Some(hash);
            state.last_validation = Some(validation.clone());
        }

        Ok(ReloadConfigResponse {
            validation,
            runtime,
            status: self.status().await,
        })
    }

    pub async fn status(&self) -> ConfigStatus {
        let rollback_available = self.store.rollback_available().await;
        let state = self.state.lock().await;
        ConfigStatus {
            config_path: self.store.path().display().to_string(),
            last_saved_at: state.last_saved_at,
            last_loaded_hash: state.last_loaded_hash.clone(),
            pending_restart: state.pending_restart.iter().cloned().collect(),
            rollback_available,
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
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
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
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
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
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    Json(manager.validate_values(&req.values).await).into_response()
}

async fn reload_config(headers: HeaderMap, manager: Arc<ConfigManager>) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    match manager.reload().await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => internal_error(e),
    }
}

async fn get_status(headers: HeaderMap, manager: Arc<ConfigManager>) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    Json(manager.status().await).into_response()
}

enum AuthError {
    TokenNotConfigured,
    Unauthorized,
}

fn authorize(headers: &HeaderMap) -> Result<(), AuthError> {
    let expected = std::env::var("GATEWAY_ADMIN_TOKEN")
        .or_else(|_| std::env::var("OPENAB_ADMIN_TOKEN"))
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or(AuthError::TokenNotConfigured)?;

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
        Err(AuthError::Unauthorized)
    }
}

fn auth_error_response(error: AuthError) -> Response {
    match error {
        AuthError::TokenNotConfigured => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "admin token is not configured" })),
        )
            .into_response(),
        AuthError::Unauthorized => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid or missing admin token" })),
        )
            .into_response(),
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
    if allow_all == Some(false) && users.is_none_or(|v| v.is_empty()) {
        errors.push(ValidationError {
            path: format!("{section}.allowed_users"),
            code: "empty_allowlist".into(),
            message: "allow_all_users=false requires at least one allowed user".into(),
        });
    }
}

fn changed_restart_required_paths(previous: &Value, next: &Value) -> Vec<String> {
    let mut configured_paths = [
        "gateway.url",
        "gateway.platform",
        "telegram.webhook_path",
        "line.webhook_path",
        "wecom.webhook_path",
        "googlechat.webhook_path",
        "teams.webhook_path",
        "feishu.webhook_path",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<BTreeSet<_>>();

    configured_paths.extend(
        field_metadata()
            .into_iter()
            .filter(|field| matches!(field.apply_policy, ApplyPolicy::RestartRequired))
            .map(|field| field.path),
    );

    configured_paths
        .into_iter()
        .filter(|path| {
            let segments = path.split('.').collect::<Vec<_>>();
            value_at_path(previous, &segments) != value_at_path(next, &segments)
        })
        .collect()
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
    for path in ["wecom.streaming_enabled", "wecom.debounce_secs"] {
        metadata.push(FieldMetadata {
            path: path.into(),
            apply_policy: ApplyPolicy::RestartRequired,
            secret: false,
        });
    }
    for path in [
        "telegram.rich_messages",
        "telegram.trusted_source_only",
        "telegram.streaming",
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

fn contains_masked_secret(value: &Value) -> bool {
    contains_masked_secret_value(value, None)
}

fn contains_masked_secret_value(value: &Value, key: Option<&str>) -> bool {
    if key.is_some_and(is_secret_key) && value.as_str() == Some(MASKED_SECRET) {
        return true;
    }

    match value {
        Value::Object(map) => map
            .iter()
            .any(|(k, v)| contains_masked_secret_value(v, Some(k))),
        Value::Array(values) => values
            .iter()
            .any(|v| contains_masked_secret_value(v, key)),
        _ => false,
    }
}

fn preserve_masked_secrets(value: &mut Value, current: &Value) {
    preserve_masked_secret_value(value, current, None);
}

fn preserve_masked_secret_value(value: &mut Value, current: &Value, key: Option<&str>) {
    if key.is_some_and(is_secret_key) && value.as_str() == Some(MASKED_SECRET) {
        if !current.is_null() {
            *value = current.clone();
        }
        return;
    }

    match (value, current) {
        (Value::Object(map), Value::Object(current_map)) => {
            for (k, v) in map.iter_mut() {
                if let Some(current_value) = current_map.get(k) {
                    preserve_masked_secret_value(v, current_value, Some(k));
                }
            }
        }
        (Value::Array(values), Value::Array(current_values)) => {
            for (idx, v) in values.iter_mut().enumerate() {
                if let Some(current_value) = current_values.get(idx) {
                    preserve_masked_secret_value(v, current_value, key);
                }
            }
        }
        _ => {}
    }
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

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |v, key| v.get(*key))
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

    #[tokio::test]
    async fn save_preserves_masked_secret_values_from_read_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gateway.toml");
        let store = ConfigStore::new(&path);
        store
            .save_atomic(&json!({
                "telegram": {
                    "bot_token": "real-bot-token",
                    "secret_token": "real-secret-token",
                    "rich_messages": true
                },
                "nested": { "password": "real-password" }
            }))
            .await
            .unwrap();

        let manager = ConfigManager::new(store.clone());
        let doc = manager.read().await.unwrap();
        assert_eq!(doc.values["telegram"]["bot_token"], MASKED_SECRET);
        assert_eq!(doc.values["telegram"]["secret_token"], MASKED_SECRET);
        assert_eq!(doc.values["nested"]["password"], MASKED_SECRET);

        let mut values = doc.values;
        values["telegram"]["rich_messages"] = json!(false);
        let resp = manager.save(values).await.unwrap();

        assert!(resp.validation.ok);
        let saved = store.load().await.unwrap();
        assert_eq!(saved["telegram"]["bot_token"], "real-bot-token");
        assert_eq!(saved["telegram"]["secret_token"], "real-secret-token");
        assert_eq!(saved["nested"]["password"], "real-password");
        assert_eq!(saved["telegram"]["rich_messages"], false);
    }

    #[test]
    fn wecom_controls_are_restart_required_but_not_secret() {
        let metadata = field_metadata();

        for path in ["wecom.streaming_enabled", "wecom.debounce_secs"] {
            let field = metadata
                .iter()
                .find(|field| field.path == path)
                .expect("wecom field metadata should exist");
            assert_eq!(field.apply_policy, ApplyPolicy::RestartRequired);
            assert!(!field.secret);
        }
    }

    #[test]
    fn secret_metadata_changes_require_restart() {
        let paths = changed_restart_required_paths(
            &json!({ "telegram": { "bot_token": "old-token" } }),
            &json!({ "telegram": { "bot_token": "new-token" } }),
        );

        assert_eq!(paths, vec!["telegram.bot_token"]);
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

        let result = validate_config(&json!({
            "telegram": { "allow_all_users": false }
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

    #[tokio::test]
    async fn runtime_only_save_does_not_create_restart_pending() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gateway.toml");
        let store = ConfigStore::new(&path);
        store
            .save_atomic(&json!({
                "telegram": { "rich_messages": false },
                "line": { "webhook_path": "/hook/line" },
                "wecom": {
                    "streaming_enabled": true,
                    "debounce_secs": 7
                }
            }))
            .await
            .unwrap();

        let manager = ConfigManager::new(store);
        let response = manager
            .save(json!({
                "telegram": { "rich_messages": true },
                "line": { "webhook_path": "/hook/line" },
                "wecom": {
                    "streaming_enabled": true,
                    "debounce_secs": 7
                }
            }))
            .await
            .unwrap();

        assert!(response.validation.ok);
        assert!(response.status.pending_restart.is_empty());
    }

    #[tokio::test]
    async fn save_tracks_changed_restart_fields_and_reload_preserves_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gateway.toml");
        let store = ConfigStore::new(&path);
        store
            .save_atomic(&json!({
                "telegram": {
                    "rich_messages": false,
                    "trusted_source_only": false,
                    "streaming": true
                },
                "line": { "webhook_path": "/hook/old" },
                "wecom": {
                    "streaming_enabled": true,
                    "debounce_secs": 7
                }
            }))
            .await
            .unwrap();

        let manager = ConfigManager::new(store);
        let save_response = manager
            .save(json!({
                "telegram": {
                    "rich_messages": true,
                    "trusted_source_only": true,
                    "streaming": false
                },
                "line": { "webhook_path": "/hook/line" },
                "wecom": {
                    "streaming_enabled": true,
                    "debounce_secs": 7
                }
            }))
            .await
            .unwrap();

        assert_eq!(
            save_response.status.pending_restart,
            vec!["line.webhook_path"]
        );

        let (tx, _rx) = tokio::sync::broadcast::channel(4);
        let state = Arc::new(crate::AppState::test_default(tx));
        manager
            .set_runtime_applier(RuntimeConfigApplier::new(state.clone()))
            .await;

        let response = manager.reload().await.unwrap();

        assert!(response.validation.ok);
        assert_eq!(
            response.runtime.applied_paths,
            vec![
                "telegram.rich_messages",
                "telegram.trusted_source_only",
                "telegram.streaming"
            ]
        );
        assert_eq!(
            response.status.pending_restart,
            vec!["line.webhook_path"]
        );
        assert!(state.telegram_rich_messages().await);
        assert!(state.telegram_trusted_source_only().await);
        assert_eq!(
            state.runtime_config.read().await.telegram_streaming,
            Some(false)
        );
    }
}
