use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use openab_core::agent_profile::{AgentConfigSchema, AgentProfile, AgentProfileService};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

type CoreSessionPool = Arc<openab_core::acp::SessionPool>;
type LiveConfigSchemaResolver = Arc<
    dyn Fn(String) -> Pin<Box<dyn Future<Output = Option<AgentConfigSchema>> + Send>> + Send + Sync,
>;

pub fn router<S>(service: Arc<AgentProfileService>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router_with_live_schema_resolver(service, None, None)
}

pub fn router_with_pool<S>(service: Arc<AgentProfileService>, pool: CoreSessionPool) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let session_pool = pool.clone();
    let resolver: LiveConfigSchemaResolver = Arc::new(move |agent: String| {
        let pool = pool.clone();
        Box::pin(async move { pool.config_schema_for_agent(&agent).await })
    });
    router_with_live_schema_resolver(service, Some(resolver), Some(session_pool))
}

pub fn router_with_live_schema_resolver<S>(
    service: Arc<AgentProfileService>,
    live_schema: Option<LiveConfigSchemaResolver>,
    session_pool: Option<CoreSessionPool>,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let list_service = service.clone();
    let create_service = service.clone();
    let get_service = service.clone();
    let update_service = service.clone();
    let delete_service = service.clone();
    let delete_pool = session_pool;
    let validate_service = service.clone();
    let get_default_service = service.clone();
    let set_default_service = service.clone();
    let set_default_path_service = service.clone();
    let clear_default_service = service.clone();
    let agents_service = service.clone();
    let schema_service = service;
    let schema_live_resolver = live_schema;

    Router::new()
        .route(
            "/api/v1/agent-profiles",
            get(move |headers| list_profiles(headers, list_service.clone()))
                .post(move |headers, body| create_profile(headers, body, create_service.clone())),
        )
        .route(
            "/api/v1/agent-profiles/default",
            get(move |headers| get_default_profile(headers, get_default_service.clone()))
                .put(move |headers, body| {
                    set_default_profile(headers, body, set_default_service.clone())
                })
                .delete(move |headers| {
                    clear_default_profile(headers, clear_default_service.clone())
                }),
        )
        .route(
            "/api/v1/agent-profiles/default/{profile_id}",
            axum::routing::put(move |headers, path| {
                set_default_profile_path(headers, path, set_default_path_service.clone())
            }),
        )
        .route(
            "/api/v1/agent-profiles/{profile_id}",
            get(move |headers, path| get_profile(headers, path, get_service.clone()))
                .put(move |headers, path, body| {
                    update_profile(headers, path, body, update_service.clone())
                })
                .delete(move |headers, path| {
                    delete_profile(headers, path, delete_service.clone(), delete_pool.clone())
                }),
        )
        .route(
            "/api/v1/agent-profiles/{profile_id}/validate",
            post(move |headers, path| validate_profile(headers, path, validate_service.clone())),
        )
        .route(
            "/api/v1/agents",
            get(move |headers| list_agents(headers, agents_service.clone())),
        )
        .route(
            "/api/v1/agents/{agent}/config-schema",
            get(move |headers, path| {
                config_schema(
                    headers,
                    path,
                    schema_service.clone(),
                    schema_live_resolver.clone(),
                )
            }),
        )
}

async fn list_profiles(headers: HeaderMap, service: Arc<AgentProfileService>) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    match service.list().await {
        Ok(document) => Json(document).into_response(),
        Err(e) => internal_error(e),
    }
}

async fn create_profile(
    headers: HeaderMap,
    Json(profile): Json<AgentProfile>,
    service: Arc<AgentProfileService>,
) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    if !service.validate_profile(&profile).ok {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "validation": service.validate_profile(&profile) })),
        )
            .into_response();
    }
    match service.upsert(profile).await {
        Ok(document) => (StatusCode::CREATED, Json(document)).into_response(),
        Err(e) => internal_error(e),
    }
}

async fn get_profile(
    headers: HeaderMap,
    Path(profile_id): Path<String>,
    service: Arc<AgentProfileService>,
) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    match service.get(&profile_id).await {
        Ok(Some(profile)) => Json(profile).into_response(),
        Ok(None) => not_found("agent profile not found"),
        Err(e) => internal_error(e),
    }
}

async fn update_profile(
    headers: HeaderMap,
    Path(profile_id): Path<String>,
    Json(mut profile): Json<AgentProfile>,
    service: Arc<AgentProfileService>,
) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    profile.id = profile_id;
    let validation = service.validate_profile(&profile);
    if !validation.ok {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "validation": validation })),
        )
            .into_response();
    }
    match service.upsert(profile).await {
        Ok(document) => Json(document).into_response(),
        Err(e) => internal_error(e),
    }
}

async fn delete_profile(
    headers: HeaderMap,
    Path(profile_id): Path<String>,
    service: Arc<AgentProfileService>,
    session_pool: Option<CoreSessionPool>,
) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    match service.delete(&profile_id).await {
        Ok(true) => {
            if let Some(pool) = session_pool {
                pool.mark_profile_deleted(&profile_id).await;
            }
            Json(Deleted { deleted: true }).into_response()
        }
        Ok(false) => not_found("agent profile not found"),
        Err(e) => internal_error(e),
    }
}

async fn get_default_profile(headers: HeaderMap, service: Arc<AgentProfileService>) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    match service.list().await {
        Ok(document) => Json(DefaultProfile {
            default_profile: document.default_profile,
        })
        .into_response(),
        Err(e) => internal_error(e),
    }
}

async fn set_default_profile(
    headers: HeaderMap,
    Json(request): Json<SetDefaultRequest>,
    service: Arc<AgentProfileService>,
) -> Response {
    set_default_common(headers, request.profile_id, service).await
}

async fn set_default_profile_path(
    headers: HeaderMap,
    Path(profile_id): Path<String>,
    service: Arc<AgentProfileService>,
) -> Response {
    set_default_common(headers, Some(profile_id), service).await
}

async fn clear_default_profile(headers: HeaderMap, service: Arc<AgentProfileService>) -> Response {
    set_default_common(headers, None, service).await
}

async fn set_default_common(
    headers: HeaderMap,
    profile_id: Option<String>,
    service: Arc<AgentProfileService>,
) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    let profile_id = profile_id.and_then(|id| {
        let id = id.trim().to_string();
        if id.is_empty() {
            None
        } else {
            Some(id)
        }
    });
    match service.set_default(profile_id).await {
        Ok(document) => Json(document).into_response(),
        Err(e) if e.to_string().contains("invalid default profile") => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
        Err(e) => internal_error(e),
    }
}

async fn validate_profile(
    headers: HeaderMap,
    Path(profile_id): Path<String>,
    service: Arc<AgentProfileService>,
) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    match service.validate_existing(&profile_id).await {
        Ok(validation) => Json(validation).into_response(),
        Err(e) if e.to_string().contains("not found") => not_found("agent profile not found"),
        Err(e) => internal_error(e),
    }
}

async fn list_agents(headers: HeaderMap, service: Arc<AgentProfileService>) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    match service.list_agents().await {
        Ok(agents) => Json(agents).into_response(),
        Err(e) => internal_error(e),
    }
}

async fn config_schema(
    headers: HeaderMap,
    Path(agent): Path<String>,
    service: Arc<AgentProfileService>,
    live_schema: Option<LiveConfigSchemaResolver>,
) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    if let Some(resolve_live_schema) = live_schema {
        if let Some(schema) = resolve_live_schema(agent.clone()).await {
            return Json(schema).into_response();
        }
    }
    match service.config_schema(&agent).await {
        Ok(schema) => Json(schema).into_response(),
        Err(e) => internal_error(e),
    }
}

#[derive(Deserialize)]
struct SetDefaultRequest {
    profile_id: Option<String>,
}

#[derive(Serialize)]
struct DefaultProfile {
    default_profile: Option<String>,
}

#[derive(Serialize)]
struct Deleted {
    deleted: bool,
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

fn not_found(message: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": message }))).into_response()
}

fn internal_error(error: anyhow::Error) -> Response {
    tracing::error!(error = %error, "agent profile admin error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": error.to_string() })),
    )
        .into_response()
}
