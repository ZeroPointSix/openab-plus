use super::connection::{AcpConnection, ContentBlock};
use super::profile_pool::{SessionPool, SessionTurnGuard};
use super::protocol::{classify_notification, parse_turn_result, AcpEvent, TurnResult};
use crate::error_display::format_coded_error;
use crate::session_snapshot::SessionStatus;
use crate::transcript::{SessionTranscriptStore, ToolTranscriptUpdate};
use anyhow::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Async event handling whose future borrows the handler, not the event.
///
/// ACP events are owned by the driver and are consumed after each callback.
/// Tying the callback future to an `&AcpEvent` would prevent presentation code
/// from mutating its turn-local buffers across an await point.
pub(crate) trait AcpTurnEventHandler: Send {
    fn handle<'a>(&'a mut self, event: AcpEvent) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

struct NoopTurnEventHandler;

impl AcpTurnEventHandler for NoopTurnEventHandler {
    fn handle<'a>(&'a mut self, _event: AcpEvent) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }
}

/// Timeout and polling policy for one ACP prompt turn.
#[derive(Debug, Clone, Copy)]
pub struct HeadlessTurnConfig {
    pub prompt_hard_timeout: std::time::Duration,
    pub liveness_check_interval: std::time::Duration,
}

impl Default for HeadlessTurnConfig {
    fn default() -> Self {
        Self {
            prompt_hard_timeout: std::time::Duration::from_secs(
                crate::config::default_prompt_hard_timeout_secs(),
            ),
            liveness_check_interval: std::time::Duration::from_secs(
                crate::config::default_liveness_check_secs(),
            ),
        }
    }
}

/// Result of the shared ACP prompt driver.
///
/// A JSON-RPC agent error is represented in the result rather than returned as
/// a transport error because the caller may still need to finish presentation
/// or other post-turn cleanup. The driver has already published the matching
/// session error event in that case.
#[derive(Debug, Default)]
pub struct AcpTurnResult {
    pub response_error: Option<String>,
    pub turn_result: TurnResult,
    pub status_failure_recorded: bool,
}

/// Drive one ACP prompt while allowing callers to observe classified events.
///
/// This is the single shared prompt/notification loop for channel adapters and
/// control-plane messages. It owns transcript persistence, session status
/// transitions, liveness checks, and stale response filtering; presentation
/// code is supplied through `handler` and never parses ACP messages itself.
pub(crate) async fn drive_acp_turn<H>(
    conn: &mut AcpConnection,
    pool: &SessionPool,
    session_id: &str,
    content_blocks: Vec<ContentBlock>,
    config: HeadlessTurnConfig,
    handler: &mut H,
) -> Result<AcpTurnResult>
where
    H: AcpTurnEventHandler,
{
    pool.mark_session_status(session_id, SessionStatus::Running)
        .await;
    record_prompt_transcript(pool, session_id, &content_blocks);

    let (mut rx, request_id) = match conn.session_prompt(content_blocks).await {
        Ok(value) => value,
        Err(err) => {
            conn.prompt_done().await;
            pool.mark_session_error(session_id, err.to_string()).await;
            return Err(err);
        }
    };

    let mut response_error: Option<String> = None;
    let mut status_failure_recorded = false;
    let mut turn_result = TurnResult::default();
    let prompt_start = tokio::time::Instant::now();

    loop {
        let notification = tokio::select! {
            msg = rx.recv() => match msg {
                Some(msg) => msg,
                None => {
                    if response_error.is_none() {
                        let err = "Agent process exited unexpectedly".to_string();
                        response_error = Some(err.clone());
                        pool.mark_session_exited(session_id, Some(err)).await;
                        status_failure_recorded = true;
                    }
                    break;
                }
            },
            _ = tokio::time::sleep(config.liveness_check_interval) => {
                if !conn.alive() {
                    let err = "Agent process died".to_string();
                    response_error = Some(err.clone());
                    pool.mark_session_exited(session_id, Some(err)).await;
                    status_failure_recorded = true;
                    conn.abandon_request(request_id).await;
                    break;
                }
                if prompt_start.elapsed() > config.prompt_hard_timeout {
                    let err = format!(
                        "Agent exceeded hard timeout ({}s)",
                        config.prompt_hard_timeout.as_secs(),
                    );
                    response_error = Some(err.clone());
                    pool.mark_session_error(session_id, err).await;
                    status_failure_recorded = true;
                    conn.abandon_request(request_id).await;
                    break;
                }
                continue;
            }
        };

        if let Some(notification_id) = notification.id {
            if notification_id != request_id {
                // A late response from an abandoned request must not terminate
                // the current turn or leak into its transcript.
                continue;
            }
            if let Some(err) = notification.error.as_ref() {
                let formatted = format_coded_error(err.code, &err.message, err.data_message());
                response_error = Some(formatted.clone());
                pool.mark_session_error(session_id, formatted).await;
                status_failure_recorded = true;
            }
            if let Some(result) = notification.result.as_ref() {
                turn_result = parse_turn_result(result);
            }
            break;
        }

        if let Some(event) = classify_notification(&notification) {
            let transcript_store = pool.transcript_store();
            record_acp_event_transcript(&transcript_store, session_id, &event);
            if let AcpEvent::ConfigUpdate { options } = &event {
                pool.record_session_config_update(session_id, options).await;
                conn.replace_config_options_from_acp(options.clone());
            }
            handler.handle(event).await;
        }
    }

    conn.prompt_done().await;
    Ok(AcpTurnResult {
        response_error,
        turn_result,
        status_failure_recorded,
    })
}

/// Run a text-only control-plane turn after the caller has acquired the
/// session turn lease. The HTTP layer can therefore return 202 without holding
/// an ACP connection or waiting for the final response.
pub(crate) async fn run_headless_turn(
    pool: Arc<SessionPool>,
    session_id: String,
    text: String,
    config: HeadlessTurnConfig,
    _turn_guard: &SessionTurnGuard,
) -> Result<AcpTurnResult> {
    let driver_pool = pool.clone();
    let driver_session_id = session_id.clone();
    pool.with_connection(&session_id, move |conn| {
        Box::pin(async move {
            let mut handler = NoopTurnEventHandler;
            let result = drive_acp_turn(
                conn,
                &driver_pool,
                &driver_session_id,
                vec![ContentBlock::Text { text }],
                config,
                &mut handler,
            )
            .await;
            if result.is_ok() {
                driver_pool
                    .transcript_store()
                    .finish_assistant_turn(&driver_session_id);
            }
            result
        })
    })
    .await
}

/// Keep only user-visible text in the transcript. Internal sender context is
/// useful to an agent but is not part of the Codeg conversation view.
pub(crate) fn record_prompt_transcript(
    pool: &SessionPool,
    session_id: &str,
    blocks: &[ContentBlock],
) {
    let content = blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } if !text.starts_with("<sender_context>\n") => {
                (!text.trim().is_empty()).then_some(text.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !content.is_empty() {
        pool.transcript_store()
            .record_user_text(session_id, content);
    }
}

/// Persist every classified ACP activity before presentation-specific handling.
pub(crate) fn record_acp_event_transcript(
    store: &SessionTranscriptStore,
    session_id: &str,
    event: &AcpEvent,
) {
    match event {
        AcpEvent::Text(content) => {
            store.append_assistant_text(session_id, content);
        }
        AcpEvent::Thinking { content } => {
            store.append_thinking(session_id, content);
        }
        AcpEvent::ToolStart { id, title, payload } => {
            store.finish_assistant_turn(session_id);
            store.upsert_tool_call(
                session_id,
                ToolTranscriptUpdate {
                    tool_call_id: id.clone(),
                    title: title.clone(),
                    status: payload
                        .get("status")
                        .and_then(|value| value.as_str())
                        .map(String::from)
                        .or_else(|| {
                            (payload
                                .get("sessionUpdate")
                                .and_then(|value| value.as_str())
                                == Some("tool_call"))
                            .then(|| "running".to_string())
                        }),
                    completed: false,
                    payload: payload.clone(),
                },
            );
        }
        AcpEvent::ToolDone {
            id,
            title,
            status,
            payload,
        } => {
            store.upsert_tool_call(
                session_id,
                ToolTranscriptUpdate {
                    tool_call_id: id.clone(),
                    title: title.clone(),
                    status: Some(status.clone()),
                    completed: true,
                    payload: payload.clone(),
                },
            );
        }
        AcpEvent::Plan { content } => {
            store.record_system_text(session_id, content, "plan");
        }
        AcpEvent::ConfigUpdate { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_headless_turn_config_matches_pool_defaults() {
        let config = HeadlessTurnConfig::default();
        assert_eq!(
            config.prompt_hard_timeout,
            std::time::Duration::from_secs(crate::config::default_prompt_hard_timeout_secs())
        );
        assert_eq!(
            config.liveness_check_interval,
            std::time::Duration::from_secs(crate::config::default_liveness_check_secs())
        );
    }

    #[test]
    fn prompt_transcript_excludes_sender_context() {
        use crate::config::AgentConfig;
        use std::collections::HashMap;

        let pool = SessionPool::new(AgentConfig::default(), 1, 120, HashMap::new());
        let blocks = vec![
            ContentBlock::Text {
                text: "<sender_context>\n{}\n</sender_context>".into(),
            },
            ContentBlock::Text {
                text: "hello".into(),
            },
        ];

        record_prompt_transcript(&pool, "session", &blocks);

        let snapshot = pool.transcript_store().snapshot("session", None);
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].content.as_deref(), Some("hello"));
    }

    #[test]
    fn empty_prompt_transcript_is_not_created() {
        use crate::config::AgentConfig;
        use std::collections::HashMap;

        let pool = SessionPool::new(AgentConfig::default(), 1, 120, HashMap::new());
        record_prompt_transcript(
            &pool,
            "session",
            &[ContentBlock::Text { text: "  ".into() }],
        );

        assert_eq!(
            pool.transcript_store()
                .snapshot("session", None)
                .entries
                .len(),
            0
        );
    }
}
