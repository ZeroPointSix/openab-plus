use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use openab_core::cli_config::{self, ApplyRequest};
use openab_core::provider::{api_key_env_name, Provider};
use openab_core::provider_store::ProviderStore;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router_with_store(Arc::new(ProviderStore::from_env()))
}

pub fn router_with_store<S>(providers: Arc<ProviderStore>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let dry_run_providers = providers.clone();
    Router::new()
        .route(
            "/api/v1/agents/{agent}/cli-config/dry-run",
            post(move |headers, path, body| {
                dry_run(headers, path, body, dry_run_providers.clone())
            }),
        )
        .route("/api/v1/agents/{agent}/cli-config/restore", post(restore))
}

#[derive(Debug, Deserialize)]
struct DryRunBody {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    reasoning_effort: Option<String>,
    #[serde(default)]
    provider_id: Option<String>,
}

async fn dry_run(
    headers: HeaderMap,
    Path(agent): Path<String>,
    Json(body): Json<DryRunBody>,
    providers: Arc<ProviderStore>,
) -> Response {
    if let Err(error) = authorize(&headers) {
        return auth_error_response(error);
    }
    let mut request = ApplyRequest {
        agent_type: agent,
        model: body.model,
        reasoning_effort: body.reasoning_effort,
        provider_id: body.provider_id.clone(),
        provider_type: None,
        base_url: None,
        api_key_env: None,
    };
    if let Some(provider_id) = body.provider_id.as_deref() {
        match providers.get(provider_id).await {
            Ok(Some(provider)) => fill_provider_fields(&mut request, &provider),
            Ok(None) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("provider {provider_id} not found") })),
                )
                    .into_response();
            }
            Err(error) => return bad_request(error),
        }
    }
    match cli_config::plan(&request) {
        Ok(report) => Json(report).into_response(),
        Err(error) => bad_request(error),
    }
}

fn fill_provider_fields(request: &mut ApplyRequest, provider: &Provider) {
    request.provider_type = Some(provider.provider_type.clone());
    request.base_url = provider.base_url.clone();
    request.api_key_env = Some(api_key_env_name(&provider.provider_type).to_string());
}

async fn restore(headers: HeaderMap, Path(agent): Path<String>) -> Response {
    if let Err(error) = authorize(&headers) {
        return auth_error_response(error);
    }
    match cli_config::restore(&agent).await {
        Ok(true) => Json(json!({ "restored": true })).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "backup not found", "restored": false })),
        )
            .into_response(),
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

fn bad_request(error: anyhow::Error) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": error.to_string() })),
    )
        .into_response()
}
