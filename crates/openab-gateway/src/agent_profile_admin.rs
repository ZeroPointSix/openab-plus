use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use openab_core::agent_profile::{AgentProfile, AgentProfileService};
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;

pub fn router<S>(service: Arc<AgentProfileService>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let list_service = service.clone();
    let create_service = service.clone();
    let get_service = service.clone();
    let update_service = service.clone();
    let delete_service = service.clone();
    let validate_service = service.clone();
    let agents_service = service.clone();
    let schema_service = service;

    Router::new()
        .route(
            "/api/v1/agent-profiles",
            get(move |headers| list_profiles(headers, list_service.clone()))
                .post(move |headers, body| create_profile(headers, body, create_service.clone())),
        )
        .route(
            "/api/v1/agent-profiles/{profile_id}",
            get(move |headers, path| get_profile(headers, path, get_service.clone()))
                .put(move |headers, path, body| {
                    update_profile(headers, path, body, update_service.clone())
                })
                .delete(move |headers, path| delete_profile(headers, path, delete_service.clone())),
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
            get(move |headers, path| config_schema(headers, path, schema_service.clone())),
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
) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    match service.delete(&profile_id).await {
        Ok(true) => Json(Deleted { deleted: true }).into_response(),
        Ok(false) => not_found("agent profile not found"),
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
) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    match service.config_schema(&agent).await {
        Ok(schema) => Json(schema).into_response(),
        Err(e) => internal_error(e),
    }
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
