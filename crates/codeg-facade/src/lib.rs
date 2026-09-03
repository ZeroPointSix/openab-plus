use axum::{
    body::{Body, Bytes},
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        OriginalUri, Path as AxumPath, State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use futures_util::StreamExt;
use percent_encoding::percent_decode_str;
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    fmt,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;

const CODEG_EVENTS_PROTOCOL: &str = "codeg-events";
const CODEG_TOKEN_PROTOCOL_PREFIX: &str = "codeg-token.";
const READY_MESSAGE: &str = r#"{"channel":"__ready__"}"#;
static CHAT_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub const COLD_START_COMMANDS: [&str; 24] = [
    "automation_list",
    "work_task_list",
    "science_list",
    "science_list_all_install_statuses",
    "experts_list",
    "experts_list_all_install_statuses",
    "officecli_skill_list_all_install_statuses",
    "app_update_status",
    "app_update_state",
    "check_app_update",
    "get_feedback_settings",
    "health",
    "get_system_language_settings",
    "get_system_terminal_settings",
    "list_folder_groups",
    "list_all_folder_details",
    "list_open_folder_details",
    "list_workspace_files",
    "list_opened_tabs",
    "save_opened_tabs",
    "list_all_conversations",
    "create_chat_dir",
    "acp_list_agents",
    "acp_list_agent_skills",
];

#[derive(Clone)]
pub struct Config {
    static_root: Arc<PathBuf>,
    admin_token: Arc<str>,
    chat_root: Arc<PathBuf>,
}

impl Config {
    pub fn new(
        static_root: impl Into<PathBuf>,
        admin_token: impl Into<String>,
    ) -> Result<Self, ConfigError> {
        Self::new_with_chat_root(
            static_root,
            admin_token,
            std::env::temp_dir().join("openab-codeg-chat"),
        )
    }

    pub fn from_env() -> Result<Self, ConfigError> {
        let static_root = std::env::var_os("CODEG_WEB_ROOT")
            .map(PathBuf::from)
            .ok_or(ConfigError::MissingEnvironment("CODEG_WEB_ROOT"))?;
        let admin_token = std::env::var("GATEWAY_ADMIN_TOKEN")
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                std::env::var("OPENAB_ADMIN_TOKEN")
                    .ok()
                    .filter(|value| !value.is_empty())
            })
            .ok_or(ConfigError::MissingEnvironment(
                "GATEWAY_ADMIN_TOKEN or OPENAB_ADMIN_TOKEN",
            ))?;
        let chat_root = std::env::var_os("CODEG_CHAT_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("openab-codeg-chat"));

        Self::new_with_chat_root(static_root, admin_token, chat_root)
    }

    fn new_with_chat_root(
        static_root: impl Into<PathBuf>,
        admin_token: impl Into<String>,
        chat_root: impl Into<PathBuf>,
    ) -> Result<Self, ConfigError> {
        let requested_root = static_root.into();
        let canonical_root =
            std::fs::canonicalize(&requested_root).map_err(|source| ConfigError::StaticRoot {
                path: requested_root.clone(),
                source,
            })?;
        if !canonical_root.is_dir() {
            return Err(ConfigError::StaticRootNotDirectory(canonical_root));
        }

        let admin_token = admin_token.into();
        if admin_token.is_empty() {
            return Err(ConfigError::EmptyAdminToken);
        }

        Ok(Self {
            static_root: Arc::new(canonical_root),
            admin_token: Arc::from(admin_token),
            chat_root: Arc::new(chat_root.into()),
        })
    }
}

#[derive(Debug)]
pub enum ConfigError {
    MissingEnvironment(&'static str),
    EmptyAdminToken,
    StaticRoot {
        path: PathBuf,
        source: std::io::Error,
    },
    StaticRootNotDirectory(PathBuf),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingEnvironment(name) => {
                write!(
                    formatter,
                    "required environment variable is not set: {name}"
                )
            }
            Self::EmptyAdminToken => formatter.write_str("gateway admin token must not be empty"),
            Self::StaticRoot { path, source } => {
                write!(
                    formatter,
                    "failed to resolve Codeg static root {}: {source}",
                    path.display()
                )
            }
            Self::StaticRootNotDirectory(path) => {
                write!(
                    formatter,
                    "Codeg static root is not a directory: {}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::StaticRoot { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub fn router<S>(config: Config) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/api/{command}", post(rpc))
        .route("/ws/events", get(events))
        .fallback(get(static_asset))
        .with_state(config)
}

async fn rpc(
    State(config): State<Config>,
    AxumPath(command): AxumPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !authorized(&headers, &config) {
        return unauthorized();
    }

    if command == "create_chat_dir" {
        return match create_chat_dir(&config).await {
            Ok(path) => Json(json!({ "path": path })).into_response(),
            Err(error) => {
                tracing::error!(%error, "failed to create Codeg chat directory");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "failed to create chat directory" })),
                )
                    .into_response()
            }
        };
    }

    match stub_payload(&command, &json_request_body(&body)) {
        Some(response) => Json(response).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "unknown Codeg command" })),
        )
            .into_response(),
    }
}

fn json_request_body(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(bytes).unwrap_or_else(|_| json!({}))
    }
}

fn server_update_status() -> Value {
    json!({
        "currentVersion": env!("CARGO_PKG_VERSION"),
        "selfUpdateSupported": false,
        "capability": "reexec",
        "runtime": "standalone",
        "restartDelayMs": 0,
        "rollbackAvailable": false,
        "liveProgress": false
    })
}

fn stub_payload(command: &str, payload: &Value) -> Option<Value> {
    Some(match command {
        "automation_list"
        | "work_task_list"
        | "science_list"
        | "science_list_all_install_statuses"
        | "experts_list"
        | "experts_list_all_install_statuses"
        | "officecli_skill_list_all_install_statuses"
        | "list_folder_groups"
        | "list_all_folder_details"
        | "list_open_folder_details"
        | "list_workspace_files"
        | "list_all_conversations"
        | "acp_list_agents" => json!([]),
        "app_update_status" => server_update_status(),
        "app_update_state" => json!({
            "seq": 0,
            "status": "idle"
        }),
        "check_app_update" => {
            let mut status = server_update_status();
            if let Some(object) = status.as_object_mut() {
                object.insert("update".to_owned(), Value::Null);
            }
            status
        }
        "get_feedback_settings" => json!({
            "enabled": false
        }),
        "health" => json!({ "status": "ok" }),
        "get_system_language_settings" => {
            json!({ "mode": "system", "language": "en" })
        }
        "get_system_terminal_settings" => json!({ "default_shell": null }),
        "list_opened_tabs" => json!({ "items": [], "version": 0 }),
        "save_opened_tabs" => {
            let items = payload.get("items").cloned().unwrap_or_else(|| json!([]));
            let expected = payload
                .get("expectedVersion")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            json!({
                "accepted": true,
                "version": expected.saturating_add(1),
                "tabs": items
            })
        }
        "acp_list_agent_skills" => json!({
            "supported": false,
            "message": "Agent skills are not available in phase one",
            "locations": [],
            "skills": []
        }),
        _ => return None,
    })
}

async fn create_chat_dir(config: &Config) -> Result<String, std::io::Error> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sequence = CHAT_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = config
        .chat_root
        .join(format!("chat-{now}-{}-{sequence}", std::process::id()));
    tokio::fs::create_dir_all(&path).await?;
    Ok(path.to_string_lossy().into_owned())
}

async fn events(
    State(config): State<Config>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Response {
    let Some(token) = token_from_websocket_protocols(&headers) else {
        return unauthorized();
    };
    if !tokens_match(&token, config.admin_token.as_bytes()) {
        return unauthorized();
    }

    websocket
        .protocols([CODEG_EVENTS_PROTOCOL])
        .on_upgrade(serve_events_socket)
}

async fn serve_events_socket(mut socket: WebSocket) {
    if socket
        .send(Message::Text(READY_MESSAGE.into()))
        .await
        .is_err()
    {
        return;
    }

    while let Some(message) = socket.next().await {
        match message {
            Ok(Message::Ping(payload)) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

fn token_from_websocket_protocols(headers: &HeaderMap) -> Option<Vec<u8>> {
    let value = headers.get(header::SEC_WEBSOCKET_PROTOCOL)?.to_str().ok()?;
    let mut protocols = value.split(',').map(str::trim);
    if protocols.next()? != CODEG_EVENTS_PROTOCOL {
        return None;
    }

    let encoded = protocols.find_map(|protocol| {
        protocol
            .strip_prefix(CODEG_TOKEN_PROTOCOL_PREFIX)
            .filter(|value| !value.is_empty())
    })?;
    URL_SAFE_NO_PAD.decode(encoded).ok()
}

fn authorized(headers: &HeaderMap, config: &Config) -> bool {
    let provided = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::as_bytes);

    provided
        .map(|token| tokens_match(token, config.admin_token.as_bytes()))
        .unwrap_or(false)
}

fn tokens_match(provided: &[u8], expected: &[u8]) -> bool {
    provided.len() == expected.len() && bool::from(provided.ct_eq(expected))
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "invalid or missing admin token" })),
    )
        .into_response()
}

async fn static_asset(State(config): State<Config>, OriginalUri(uri): OriginalUri) -> Response {
    let Some(relative) = safe_relative_path(&uri) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    match find_static_asset(&config.static_root, &relative).await {
        Ok(Some((path, bytes))) => asset_response(&path, bytes),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(%error, "failed to read Codeg static asset");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn safe_relative_path(uri: &Uri) -> Option<PathBuf> {
    let decoded = percent_decode_str(uri.path()).decode_utf8().ok()?;
    let without_root = decoded.trim_start_matches('/');
    let mut relative = PathBuf::new();

    for component in Path::new(without_root).components() {
        match component {
            Component::Normal(value) => relative.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    Some(relative)
}

async fn find_static_asset(
    root: &Path,
    relative: &Path,
) -> Result<Option<(PathBuf, Vec<u8>)>, std::io::Error> {
    for candidate in candidate_paths(root, relative) {
        let canonical = match tokio::fs::canonicalize(&candidate).await {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !canonical.starts_with(root) {
            continue;
        }
        if !tokio::fs::metadata(&canonical).await?.is_file() {
            continue;
        }

        return Ok(Some((canonical.clone(), tokio::fs::read(canonical).await?)));
    }

    Ok(None)
}

fn candidate_paths(root: &Path, relative: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::with_capacity(4);
    if !relative.as_os_str().is_empty() {
        candidates.push(root.join(relative));

        let mut html_path = OsString::from(relative.as_os_str());
        html_path.push(".html");
        candidates.push(root.join(html_path));

        candidates.push(root.join(relative).join("index.html"));
    }
    candidates.push(root.join("index.html"));
    candidates
}

fn asset_response(path: &Path, bytes: Vec<u8>) -> Response {
    let cache_control = if path.extension().and_then(|value| value.to_str()) == Some("html") {
        "no-cache"
    } else if path
        .components()
        .any(|component| component.as_os_str() == "_next")
    {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=3600"
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type(path))
        .header(
            header::CACHE_CONTROL,
            HeaderValue::from_static(cache_control),
        )
        .body(Body::from(bytes))
        .expect("static asset response must be valid")
}

fn content_type(path: &Path) -> HeaderValue {
    let value = match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json" | "map") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("wasm") => "application/wasm",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };
    HeaderValue::from_static(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use futures_util::StreamExt;
    use http_body_util::BodyExt;
    use serde_json::Value;
    use std::fs;
    use tempfile::TempDir;
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{
            client::IntoClientRequest,
            http::{header::SEC_WEBSOCKET_PROTOCOL, HeaderValue as TungsteniteHeaderValue},
            Message as ClientMessage,
        },
    };
    use tower::ServiceExt;

    fn fixture() -> (TempDir, TempDir, Config) {
        let static_root = tempfile::tempdir().unwrap();
        let chat_root = tempfile::tempdir().unwrap();
        fs::write(static_root.path().join("index.html"), "fallback").unwrap();
        let config =
            Config::new_with_chat_root(static_root.path(), "secret", chat_root.path()).unwrap();
        (static_root, chat_root, config)
    }

    fn post(command: &str, token: Option<&str>) -> Request<Body> {
        post_json(command, token, "{}")
    }

    fn post_json(command: &str, token: Option<&str>, body: &str) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!("/api/{command}"))
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        builder.body(Body::from(body.to_owned())).unwrap()
    }

    async fn json_body(response: Response) -> Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn all_cold_start_commands_return_bare_json() {
        let (_static_root, chat_root, config) = fixture();
        let app = router::<()>(config);

        for command in COLD_START_COMMANDS {
            let response = app
                .clone()
                .oneshot(post(command, Some("secret")))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{command}");
            let body = json_body(response).await;
            assert!(!body.is_null(), "{command}");
            match command {
                "app_update_status"
                | "app_update_state"
                | "check_app_update"
                | "get_feedback_settings"
                | "list_opened_tabs"
                | "save_opened_tabs" => {
                    assert!(body.is_object(), "{command} must return an object: {body}");
                }
                "automation_list"
                | "work_task_list"
                | "science_list"
                | "science_list_all_install_statuses"
                | "experts_list"
                | "experts_list_all_install_statuses"
                | "officecli_skill_list_all_install_statuses"
                | "list_folder_groups"
                | "list_all_folder_details"
                | "list_open_folder_details"
                | "list_workspace_files"
                | "list_all_conversations"
                | "acp_list_agents" => {
                    assert!(body.is_array(), "{command} must return an array: {body}");
                }
                _ => {}
            }
        }

        assert!(fs::read_dir(chat_root.path()).unwrap().next().is_some());
    }

    #[tokio::test]
    async fn phase_one_rpc_shapes_match_pinned_codeg_contracts() {
        let (_static_root, _chat_root, config) = fixture();
        let app = router::<()>(config);
        let expected = [
            (
                "app_update_status",
                "{}",
                stub_payload("app_update_status", &json!({})).unwrap(),
            ),
            (
                "app_update_state",
                "{}",
                stub_payload("app_update_state", &json!({})).unwrap(),
            ),
            (
                "check_app_update",
                "{}",
                stub_payload("check_app_update", &json!({})).unwrap(),
            ),
            (
                "get_feedback_settings",
                "{}",
                stub_payload("get_feedback_settings", &json!({})).unwrap(),
            ),
            ("list_workspace_files", "{}", json!([])),
            (
                "list_opened_tabs",
                "{}",
                json!({ "items": [], "version": 0 }),
            ),
            (
                "save_opened_tabs",
                r#"{"items":[{"conversation_id":7}],"expectedVersion":3,"origin":"hydrate"}"#,
                json!({
                    "accepted": true,
                    "version": 4,
                    "tabs": [{ "conversation_id": 7 }]
                }),
            ),
        ];

        for (command, request_body, expected_body) in expected {
            let response = app
                .clone()
                .oneshot(post_json(command, Some("secret"), request_body))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{command}");
            assert_eq!(json_body(response).await, expected_body, "{command}");
        }
    }

    #[tokio::test]
    async fn rpc_requires_the_existing_admin_bearer_token() {
        let (_static_root, _chat_root, config) = fixture();
        let app = router::<()>(config);

        let missing = app.clone().oneshot(post("health", None)).await.unwrap();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let incorrect = app
            .clone()
            .oneshot(post("health", Some("incorrect")))
            .await
            .unwrap();
        assert_eq!(incorrect.status(), StatusCode::UNAUTHORIZED);

        let unknown = app
            .oneshot(post("not_a_codeg_command", Some("secret")))
            .await
            .unwrap();
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn static_fallback_uses_the_required_lookup_order() {
        let (static_root, _chat_root, config) = fixture();
        fs::write(static_root.path().join("direct"), "direct").unwrap();
        fs::write(static_root.path().join("pretty.html"), "html").unwrap();
        fs::create_dir(static_root.path().join("nested")).unwrap();
        fs::write(
            static_root.path().join("nested").join("index.html"),
            "nested",
        )
        .unwrap();
        let app = router::<()>(config);

        for (uri, expected) in [
            ("/direct", "direct"),
            ("/pretty", "html"),
            ("/nested", "nested"),
            ("/client/route", "fallback"),
        ] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(&bytes[..], expected.as_bytes(), "{uri}");
        }
    }

    #[tokio::test]
    async fn static_fallback_rejects_encoded_parent_traversal() {
        let (_static_root, _chat_root, config) = fixture();
        let response = router::<()>(config)
            .oneshot(
                Request::builder()
                    .uri("/%2e%2e/secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn websocket_negotiates_codeg_protocol_and_sends_ready() {
        let (_static_root, _chat_root, config) = fixture();
        let app = router::<()>(config);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let encoded = URL_SAFE_NO_PAD.encode("secret");
        let mut request = format!("ws://{address}/ws/events")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            SEC_WEBSOCKET_PROTOCOL,
            TungsteniteHeaderValue::from_str(&format!(
                "{CODEG_EVENTS_PROTOCOL}, {CODEG_TOKEN_PROTOCOL_PREFIX}{encoded}"
            ))
            .unwrap(),
        );

        let (mut websocket, response) = connect_async(request).await.unwrap();
        assert_eq!(
            response
                .headers()
                .get(SEC_WEBSOCKET_PROTOCOL)
                .and_then(|value| value.to_str().ok()),
            Some(CODEG_EVENTS_PROTOCOL)
        );
        let message = websocket.next().await.unwrap().unwrap();
        assert_eq!(message, ClientMessage::Text(READY_MESSAGE.into()));

        drop(websocket);
        server.abort();
        let _ = server.await;
    }
}
