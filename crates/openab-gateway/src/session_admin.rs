use axum::extract::Path;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{
    sse::{Event, KeepAlive, Sse},
    IntoResponse, Response,
};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::stream;
use openab_core::session_event::SessionEvent;
use serde_json::json;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::broadcast;

type CoreSessionPool = Arc<openab_core::acp::SessionPool>;

pub fn router<S>(pool: CoreSessionPool) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route(
            "/api/v1/sessions",
            get({
                let pool = pool.clone();
                move |headers: HeaderMap| {
                    let pool = pool.clone();
                    async move { list_sessions(headers, pool).await }
                }
            }),
        )
        .route(
            "/api/v1/sessions/events",
            get({
                let pool = pool.clone();
                move |headers: HeaderMap| {
                    let pool = pool.clone();
                    async move { stream_session_events(headers, pool).await }
                }
            }),
        )
        .route(
            "/api/v1/sessions/{session_id}",
            get({
                let pool = pool.clone();
                move |headers: HeaderMap, Path(session_id): Path<String>| {
                    let pool = pool.clone();
                    async move { get_session(headers, session_id, pool).await }
                }
            }),
        )
}

async fn list_sessions(headers: HeaderMap, pool: CoreSessionPool) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    Json(pool.list_session_snapshots().await).into_response()
}

async fn get_session(headers: HeaderMap, session_id: String, pool: CoreSessionPool) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    match pool.session_snapshot(&session_id).await {
        Some(snapshot) => Json(snapshot).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "session not found" })),
        )
            .into_response(),
    }
}

async fn stream_session_events(headers: HeaderMap, pool: CoreSessionPool) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }

    let receiver = pool.session_event_bus().subscribe();
    let events = stream::unfold(receiver, |mut receiver| async move {
        match receiver.recv().await {
            Ok(event) => Some((session_event_sse(event), receiver)),
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                Some((lagged_event_sse(skipped), receiver))
            }
            Err(broadcast::error::RecvError::Closed) => None,
        }
    });

    Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn session_event_sse(event: SessionEvent) -> Result<Event, Infallible> {
    let event_name = event.event.as_sse_event();
    let id = event.sequence.to_string();
    let data = serde_json::to_string(&event).unwrap_or_else(|err| {
        json!({
            "error": "failed to serialize session event",
            "message": err.to_string(),
        })
        .to_string()
    });
    Ok(Event::default().event(event_name).id(id).data(data))
}

fn lagged_event_sse(skipped: u64) -> Result<Event, Infallible> {
    Ok(Event::default().event("error").data(
        json!({
            "error": "event stream lagged",
            "skipped": skipped,
        })
        .to_string(),
    ))
}

#[derive(Debug)]
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

    authorize_with_expected(headers, &expected)
}

fn authorize_with_expected(headers: &HeaderMap, expected: &str) -> Result<(), AuthError> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let header_token = headers
        .get("x-openab-admin-token")
        .and_then(|value| value.to_str().ok());

    if bearer == Some(expected) || header_token == Some(expected) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_accepts_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer test-token".parse().unwrap());

        assert!(authorize_with_expected(&headers, "test-token").is_ok());
    }

    #[test]
    fn authorize_accepts_admin_header_token() {
        let mut headers = HeaderMap::new();
        headers.insert("x-openab-admin-token", "test-token".parse().unwrap());

        assert!(authorize_with_expected(&headers, "test-token").is_ok());
    }

    #[test]
    fn authorize_rejects_missing_token() {
        let headers = HeaderMap::new();

        assert!(matches!(
            authorize_with_expected(&headers, "test-token"),
            Err(AuthError::Unauthorized)
        ));
    }
}
