use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use openab_core::agent_profile::AgentProfileService;
use openab_core::provider::Provider;
use openab_core::provider_store::ProviderStore;
use serde_json::json;
use std::sync::Arc;

pub fn router<S>(providers: Arc<ProviderStore>, profiles: Arc<AgentProfileService>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let list_providers = providers.clone();
    let create_providers = providers.clone();
    let get_providers = providers.clone();
    let update_providers = providers.clone();
    let delete_providers = providers.clone();
    let delete_profiles = profiles;

    Router::new()
        .route(
            "/api/v1/providers",
            get(move |headers| list(headers, list_providers.clone()))
                .post(move |headers, body| create(headers, body, create_providers.clone())),
        )
        .route(
            "/api/v1/providers/{provider_id}",
            get(move |headers, path| get_one(headers, path, get_providers.clone()))
                .put(move |headers, path, body| {
                    update(headers, path, body, update_providers.clone())
                })
                .delete(move |headers, path| {
                    delete(
                        headers,
                        path,
                        delete_providers.clone(),
                        delete_profiles.clone(),
                    )
                }),
        )
}

async fn list(headers: HeaderMap, store: Arc<ProviderStore>) -> Response {
    if let Err(error) = authorize(&headers) {
        return auth_error_response(error);
    }
    match store.list().await {
        Ok(providers) => Json(json!({ "providers": providers })).into_response(),
        Err(error) => internal_error(error),
    }
}

async fn get_one(
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    store: Arc<ProviderStore>,
) -> Response {
    if let Err(error) = authorize(&headers) {
        return auth_error_response(error);
    }
    match store.get(&provider_id).await {
        Ok(Some(provider)) => Json(provider).into_response(),
        Ok(None) => not_found("provider not found"),
        Err(error) => internal_error(error),
    }
}

async fn create(
    headers: HeaderMap,
    Json(provider): Json<Provider>,
    store: Arc<ProviderStore>,
) -> Response {
    if let Err(error) = authorize(&headers) {
        return auth_error_response(error);
    }
    match store.upsert(provider).await {
        Ok(provider) => (StatusCode::CREATED, Json(provider)).into_response(),
        Err(error) => bad_request(error),
    }
}

async fn update(
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    Json(mut provider): Json<Provider>,
    store: Arc<ProviderStore>,
) -> Response {
    if let Err(error) = authorize(&headers) {
        return auth_error_response(error);
    }
    provider.id = provider_id;
    match store.upsert(provider).await {
        Ok(provider) => Json(provider).into_response(),
        Err(error) => bad_request(error),
    }
}

async fn delete(
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    store: Arc<ProviderStore>,
    profiles: Arc<AgentProfileService>,
) -> Response {
    if let Err(error) = authorize(&headers) {
        return auth_error_response(error);
    }
    let referenced = match profiles.list().await {
        Ok(document) => document
            .profiles
            .into_iter()
            .filter(|profile| profile.provider.as_deref() == Some(provider_id.as_str()))
            .map(|profile| profile.id)
            .collect::<Vec<_>>(),
        Err(error) => return internal_error(error),
    };
    match store
        .delete_unless_referenced(&provider_id, &referenced)
        .await
    {
        Ok(true) => Json(json!({ "deleted": true })).into_response(),
        Ok(false) => not_found("provider not found"),
        Err(error) => bad_request(error),
    }
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

fn bad_request(error: anyhow::Error) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": error.to_string() })),
    )
        .into_response()
}

fn internal_error(error: anyhow::Error) -> Response {
    tracing::error!(error = %error, "provider admin error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": error.to_string() })),
    )
        .into_response()
}
