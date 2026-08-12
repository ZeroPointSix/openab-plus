use axum::extract::{Path, Query};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{
    sse::{Event, KeepAlive, Sse},
    IntoResponse, Response,
};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::{stream, StreamExt};
use openab_core::session_event::{SessionStreamBus, SessionStreamEvent, SessionStreamReplay};
use serde::Deserialize;
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
            "/api/v1/sessions/{session_id}/transcript",
            get({
                let pool = pool.clone();
                move |headers: HeaderMap,
                      Path(session_id): Path<String>,
                      Query(query): Query<TranscriptQuery>| {
                    let pool = pool.clone();
                    async move { get_transcript(headers, session_id, query, pool).await }
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

#[derive(Debug, Deserialize)]
struct TranscriptQuery {
    after: Option<u64>,
}

async fn get_transcript(
    headers: HeaderMap,
    session_id: String,
    query: TranscriptQuery,
    pool: CoreSessionPool,
) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }
    if pool.session_snapshot(&session_id).await.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "session not found" })),
        )
            .into_response();
    }

    Json(pool.transcript_store().snapshot(&session_id, query.after)).into_response()
}

async fn stream_session_events(headers: HeaderMap, pool: CoreSessionPool) -> Response {
    if let Err(err) = authorize(&headers) {
        return auth_error_response(err);
    }

    let stream_bus = pool.session_stream_bus();
    let cursor = last_event_cursor(&headers);
    let last_sequence = replay_cursor_sequence(cursor.as_ref(), &stream_bus);
    let subscription = stream_bus.subscribe_after(last_sequence);
    let last_replayed_sequence = subscription
        .replay
        .events
        .last()
        .map(SessionStreamEvent::sequence)
        .or(last_sequence)
        .unwrap_or_default();
    let replay_events = replay_events_sse(
        cursor.as_ref(),
        last_sequence,
        &stream_bus,
        subscription.replay,
    );

    let live_events = stream::unfold(
        (subscription.receiver, last_replayed_sequence, stream_bus),
        |(mut receiver, last_replayed_sequence, stream_bus)| async move {
            loop {
                match receiver.recv().await {
                    Ok(event) if event.sequence() <= last_replayed_sequence => continue,
                    Ok(event) => {
                        let sequence = event.sequence();
                        return Some((
                            session_stream_event_sse(&stream_bus, event),
                            (receiver, sequence, stream_bus),
                        ));
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        return Some((
                            lagged_event_sse(skipped),
                            (receiver, last_replayed_sequence, stream_bus),
                        ));
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    );

    Sse::new(stream::iter(replay_events).chain(live_events))
        .keep_alive(KeepAlive::default())
        .into_response()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionEventCursor {
    generation: Option<String>,
    sequence: u64,
}

fn last_event_cursor(headers: &HeaderMap) -> Option<SessionEventCursor> {
    headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_last_event_id)
}

fn parse_last_event_id(value: &str) -> Option<SessionEventCursor> {
    let value = value.trim();
    if let Some((generation, sequence)) = value.rsplit_once(':') {
        if !generation.is_empty() {
            return sequence.parse().ok().map(|sequence| SessionEventCursor {
                generation: Some(generation.to_string()),
                sequence,
            });
        }
    }

    value.parse().ok().map(|sequence| SessionEventCursor {
        generation: None,
        sequence,
    })
}

fn replay_cursor_sequence(
    cursor: Option<&SessionEventCursor>,
    stream_bus: &SessionStreamBus,
) -> Option<u64> {
    match cursor {
        Some(cursor) if cursor.generation.as_deref() == Some(stream_bus.generation()) => {
            Some(cursor.sequence)
        }
        Some(_) => None,
        // A fresh admin page has no browser cursor yet. Replay from zero so the
        // globally-owned connection seeds detail timelines before it goes live.
        None => Some(0),
    }
}

fn replay_events_sse(
    cursor: Option<&SessionEventCursor>,
    last_sequence: Option<u64>,
    stream_bus: &SessionStreamBus,
    replay: SessionStreamReplay,
) -> Vec<Result<Event, Infallible>> {
    let mut events = Vec::new();
    if let Some(cursor) = cursor {
        if last_sequence.is_none() {
            events.push(cursor_reset_event_sse(cursor, stream_bus));
            return events;
        }
    }
    if replay.overflowed {
        events.push(history_unavailable_event_sse(
            last_sequence.unwrap_or_default(),
            &replay,
        ));
    }
    events.extend(
        replay
            .events
            .into_iter()
            .map(|event| session_stream_event_sse(stream_bus, event)),
    );
    events
}

fn session_stream_event_sse(
    stream_bus: &SessionStreamBus,
    event: SessionStreamEvent,
) -> Result<Event, Infallible> {
    let event_name = event.as_sse_event();
    let id = stream_bus.event_id(event.sequence());
    let data = serde_json::to_string(&event).unwrap_or_else(|err| {
        json!({
            "error": "failed to serialize session stream event",
            "message": err.to_string(),
        })
        .to_string()
    });
    Ok(Event::default().event(event_name).id(id).data(data))
}

fn cursor_reset_event_sse(
    cursor: &SessionEventCursor,
    stream_bus: &SessionStreamBus,
) -> Result<Event, Infallible> {
    Ok(Event::default()
        .event("cursor_reset")
        .id(stream_bus.event_id(0))
        .data(
            json!({
                "error": "event cursor generation changed",
                "last_event_generation": cursor.generation.as_deref(),
                "last_sequence": cursor.sequence,
                "current_generation": stream_bus.generation(),
                "action": "refetch /api/v1/sessions before continuing the stream",
            })
            .to_string(),
        ))
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

fn history_unavailable_event_sse(
    last_event_id: u64,
    replay: &SessionStreamReplay,
) -> Result<Event, Infallible> {
    Ok(Event::default().event("error").data(
        json!({
            "error": "event history unavailable",
            "last_event_id": last_event_id,
            "oldest_sequence": replay.oldest_sequence,
            "next_sequence": replay.next_sequence,
            "action": "refetch /api/v1/sessions before continuing the stream",
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

    #[test]
    fn parses_generation_qualified_last_event_id() {
        assert_eq!(
            parse_last_event_id("generation-1:42"),
            Some(SessionEventCursor {
                generation: Some("generation-1".into()),
                sequence: 42,
            })
        );
    }

    #[test]
    fn parses_numeric_last_event_id_as_legacy_cursor() {
        assert_eq!(
            parse_last_event_id("42"),
            Some(SessionEventCursor {
                generation: None,
                sequence: 42,
            })
        );
    }

    #[test]
    fn fresh_stream_replays_history_from_zero() {
        let stream_bus = SessionStreamBus::new(8);

        assert_eq!(replay_cursor_sequence(None, &stream_bus), Some(0));
    }

    #[test]
    fn stale_stream_cursor_requests_a_reset() {
        let stream_bus = SessionStreamBus::new(8);
        let stale = SessionEventCursor {
            generation: Some("previous-generation".into()),
            sequence: 42,
        };

        assert_eq!(replay_cursor_sequence(Some(&stale), &stream_bus), None);
    }
}
