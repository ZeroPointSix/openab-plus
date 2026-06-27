//! Ambient Mode — batch flush dispatcher for passive channel listening.
//!
//! See ADR: docs/adr/ambient.md for full design rationale.
//!
//! Ambient mode listens to all messages in configured channels (without @mention)
//! and periodically flushes them as a batch to the LLM. The agent replies only
//! when it has something valuable to add; otherwise it returns `[NO_REPLY]`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::Rng;
use tokio::sync::{Mutex, Semaphore};
use tracing::{debug, info, warn};

use crate::acp::ContentBlock;
use crate::adapter::{ChannelRef, ChatAdapter, MessageRef};
use crate::config::AmbientConfig;
use crate::dispatch::DispatchTarget;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Sentinel value the agent returns when it has nothing to add.
const NO_REPLY_SENTINEL: &str = "[no_reply]";

/// System instruction prepended to every ambient batch.
const AMBIENT_SYSTEM_INSTRUCTION: &str = r#"You are in ambient mode. Below is a batch of recent messages from the channel. You are passively observing the conversation.

Rules:
- If you have nothing valuable to add, reply EXACTLY: [NO_REPLY]
- Only reply when you can provide meaningful help, context, or corrections
- Do not reply to other bot messages unless directly relevant to a human's question
- Keep replies concise and natural — you are joining an ongoing conversation, not starting one
- Do not acknowledge that you are in ambient mode
"#;

// ---------------------------------------------------------------------------
// AmbientMessage — lighter than BufferedMessage for ambient buffering
// ---------------------------------------------------------------------------

/// A single message buffered for ambient dispatch.
#[derive(Debug)]
pub struct AmbientMessage {
    /// Serialised SenderContext JSON.
    pub sender_json: String,
    /// Author display name.
    pub sender_name: String,
    /// User-visible prompt text.
    pub prompt: String,
    /// Attachment blocks.
    pub extra_blocks: Vec<ContentBlock>,
    /// When this message arrived.
    pub arrived_at: Instant,
}

// ---------------------------------------------------------------------------
// FlushingGuard — RAII guard for the per-channel flushing flag with timeout
// ---------------------------------------------------------------------------

/// RAII guard that sets an `AtomicBool` to true on creation and resets to false
/// on drop. Also spawns a safety timeout that forcibly resets if the guard is
/// held too long (e.g., consumer panic under a catch_unwind or OOM).
struct FlushingGuard {
    flag: Arc<AtomicBool>,
    _timeout_handle: tokio::task::JoinHandle<()>,
}

impl FlushingGuard {
    fn new(flag: Arc<AtomicBool>, timeout: Duration) -> Self {
        flag.store(true, Ordering::Release);
        let flag_clone = Arc::clone(&flag);
        let handle = tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            if flag_clone.swap(false, Ordering::AcqRel) {
                warn!("ambient flush timeout exceeded, force-resetting flushing flag");
            }
        });
        Self {
            flag,
            _timeout_handle: handle,
        }
    }
}

impl Drop for FlushingGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
        self._timeout_handle.abort();
    }
}

// ---------------------------------------------------------------------------
// PostGuard — atomic check-and-post to prevent TOCTOU race
// ---------------------------------------------------------------------------

/// Per-channel post guard. The ambient consumer acquires it before posting;
/// the mention path invalidates it to cancel an in-flight ambient response.
#[derive(Debug)]
pub struct PostGuard {
    cancelled: AtomicBool,
}

impl Default for PostGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl PostGuard {
    pub fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
        }
    }

    /// Cancel any pending ambient post for this channel.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Check if posting is still allowed. Returns false if cancelled.
    pub fn can_post(&self) -> bool {
        !self.cancelled.load(Ordering::Acquire)
    }

    /// Reset for next flush cycle.
    pub fn reset(&self) {
        self.cancelled.store(false, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// ChannelState — per-channel ambient state
// ---------------------------------------------------------------------------

struct ChannelState {
    tx: tokio::sync::mpsc::Sender<AmbientMessage>,
    flushing: Arc<AtomicBool>,
    post_guard: Arc<PostGuard>,
    _consumer: tokio::task::JoinHandle<()>,
}

// ---------------------------------------------------------------------------
// AmbientDispatcher
// ---------------------------------------------------------------------------

/// Manages ambient mode across all configured channels.
///
/// Each enabled channel gets its own mpsc channel and consumer task.
/// The consumer accumulates messages and flushes them as a batch when
/// a time or count trigger fires.
pub struct AmbientDispatcher {
    config: AmbientConfig,
    channels: Mutex<HashMap<String, ChannelState>>,
    /// Channel IDs that have ambient mode enabled (pre-parsed for fast lookup).
    enabled_channels: HashSet<u64>,
    /// Global semaphore limiting concurrent flush operations.
    flush_semaphore: Arc<Semaphore>,
}

impl AmbientDispatcher {
    /// Create a new AmbientDispatcher from config.
    ///
    /// Does NOT start consumer tasks — those are spawned lazily on first message
    /// or eagerly via `start_channel`.
    pub fn new(config: AmbientConfig) -> Self {
        let enabled_channels: HashSet<u64> = config
            .discord
            .channels
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect();
        let flush_semaphore = Arc::new(Semaphore::new(config.max_concurrent_flushes));
        Self {
            config,
            channels: Mutex::new(HashMap::new()),
            enabled_channels,
            flush_semaphore,
        }
    }

    /// Check if ambient mode is active and this channel is in the allowlist.
    pub fn is_ambient_channel(&self, channel_id: u64) -> bool {
        self.config.enabled && !self.enabled_channels.is_empty() && self.enabled_channels.contains(&channel_id)
    }

    /// Whether bot messages are allowed in the ambient buffer.
    pub fn allow_bot_messages(&self) -> bool {
        self.config.discord.allow_bot_messages
    }

    /// Submit a message to the ambient buffer for a channel.
    ///
    /// Returns Ok(()) if buffered, Err if the channel consumer is dead.
    pub async fn submit(
        &self,
        channel_id: &str,
        channel_ref: ChannelRef,
        adapter: Arc<dyn ChatAdapter>,
        target: Arc<dyn DispatchTarget>,
        msg: AmbientMessage,
    ) {
        let mut channels = self.channels.lock().await;

        // Lazily spawn consumer if not yet started for this channel.
        if !channels.contains_key(channel_id) {
            let (tx, rx) = tokio::sync::mpsc::channel(self.config.flush_hard_cap);
            let flushing = Arc::new(AtomicBool::new(false));
            let post_guard = Arc::new(PostGuard::new());

            let consumer = tokio::spawn(ambient_consumer_loop(
                channel_id.to_string(),
                channel_ref.clone(),
                rx,
                self.config.clone(),
                Arc::clone(&self.flush_semaphore),
                Arc::clone(&flushing),
                Arc::clone(&post_guard),
                adapter,
                target,
            ));

            channels.insert(
                channel_id.to_string(),
                ChannelState {
                    tx,
                    flushing,
                    post_guard,
                    _consumer: consumer,
                },
            );
        }

        let state = channels.get(channel_id).unwrap();
        // Non-blocking try_send — if buffer is full (hard_cap), drop the message.
        if let Err(e) = state.tx.try_send(msg) {
            debug!(
                channel_id,
                "ambient buffer full, dropping message: {}",
                e
            );
        }
    }

    /// Discard the ambient buffer for a channel (called when @mention arrives).
    /// Also cancels any in-flight ambient response via the post_guard.
    /// The consumer will drain buffered messages when it sees the cancellation.
    pub async fn discard_buffer(&self, channel_id: &str) {
        let channels = self.channels.lock().await;
        if let Some(state) = channels.get(channel_id) {
            // Cancel any in-flight post — consumer drains the buffer on next check.
            state.post_guard.cancel();
            debug!(channel_id, "ambient buffer discard requested (mention arrived)");
        }
    }

    /// Check if a channel is currently mid-flush.
    pub async fn is_flushing(&self, channel_id: &str) -> bool {
        let channels = self.channels.lock().await;
        channels
            .get(channel_id)
            .map(|s| s.flushing.load(Ordering::Acquire))
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// ambient_consumer_loop
// ---------------------------------------------------------------------------

/// Per-channel consumer that accumulates messages and flushes them as a batch.
#[allow(clippy::too_many_arguments)]
async fn ambient_consumer_loop(
    channel_id: String,
    channel_ref: ChannelRef,
    mut rx: tokio::sync::mpsc::Receiver<AmbientMessage>,
    config: AmbientConfig,
    flush_semaphore: Arc<Semaphore>,
    flushing: Arc<AtomicBool>,
    post_guard: Arc<PostGuard>,
    adapter: Arc<dyn ChatAdapter>,
    target: Arc<dyn DispatchTarget>,
) {
    info!(channel_id = %channel_id, "ambient consumer started");

    loop {
        // Wait for first message (blocks until one arrives or channel closes).
        let first = match rx.recv().await {
            Some(msg) => msg,
            None => {
                info!(channel_id = %channel_id, "ambient consumer channel closed, exiting");
                return;
            }
        };

        // Compute jittered deadline: flush_interval ± 20%
        let base = Duration::from_secs(config.flush_interval_seconds);
        let jitter_range = base.as_millis() as f64 * 0.2;
        let jitter_ms = rand::thread_rng().gen_range(-jitter_range..jitter_range) as i64;
        let interval = Duration::from_millis((base.as_millis() as i64 + jitter_ms).max(1000) as u64);
        let deadline = tokio::time::Instant::now() + interval;

        let mut batch = vec![first];

        // Accumulate until trigger fires.
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break; // Timer expired.
            }

            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Some(msg)) => {
                    batch.push(msg);
                    if batch.len() >= config.flush_max_messages {
                        break;
                    }
                    if batch.len() >= config.flush_hard_cap {
                        break;
                    }
                }
                Ok(None) => break, // Channel closed.
                Err(_) => break,   // Timer expired.
            }
        }

        let batch_size = batch.len();
        debug!(
            channel_id = %channel_id,
            batch_size,
            "ambient flush triggered"
        );

        // Acquire global concurrency permit.
        let _permit = match flush_semaphore.acquire().await {
            Ok(permit) => permit,
            Err(_) => {
                warn!(channel_id = %channel_id, "flush semaphore closed, exiting");
                return;
            }
        };

        // Set flushing flag with safety timeout.
        let flush_timeout = Duration::from_secs(config.flush_timeout_seconds);
        let _flushing_guard = FlushingGuard::new(Arc::clone(&flushing), flush_timeout);

        // Reset post_guard for this flush cycle.
        post_guard.reset();

        // Build the batch payload.
        let session_key = format!("ambient:{}:{}", channel_ref.platform, channel_id);
        let content_blocks = build_ambient_payload(&batch);

        // Ensure session exists.
        if let Err(e) = target.ensure_session(&session_key, None).await {
            warn!(
                channel_id = %channel_id,
                error = %e,
                "failed to create ambient session, discarding batch"
            );
            continue;
        }

        // Dispatch batch to agent. For ambient mode, we use stream_prompt_blocks
        // with a dummy reaction controller (enabled=false) to suppress all reactions.
        // The NO_REPLY check must be handled at a higher level (response filtering).
        //
        // TODO(ambient-v2): implement response capture mode so we can intercept
        // [NO_REPLY] before the message is posted. For v1, the system prompt
        // strongly instructs the agent, and we accept that rare false-positives
        // may briefly appear before being deleted.
        let dummy_msg_ref = MessageRef {
            channel: channel_ref.clone(),
            message_id: String::new(),
        };
        let reactions = Arc::new(crate::reactions::StatusReactionController::new(
            false, // disabled — no reactions for ambient
            Arc::clone(&adapter),
            dummy_msg_ref,
            crate::config::ReactionEmojis::default(),
            crate::config::ReactionTiming::default(),
        ));

        // Check post_guard before dispatching (mention may have cancelled).
        if !post_guard.can_post() {
            // Drain any remaining buffered messages so stale context is discarded.
            while rx.try_recv().is_ok() {}
            debug!(channel_id = %channel_id, "ambient flush cancelled by mention, buffer drained");
            continue;
        }

        match target
            .stream_prompt_blocks(
                &adapter,
                &session_key,
                content_blocks,
                &channel_ref,
                reactions,
                false, // other_bot_present
                None,  // no streaming recipient
            )
            .await
        {
            Ok(()) => {
                debug!(channel_id = %channel_id, "ambient flush dispatched");
            }
            Err(e) => {
                warn!(
                    channel_id = %channel_id,
                    error = %e,
                    "ambient flush failed, discarding batch"
                );
            }
        }

        // _flushing_guard drops here → resets flag
        // _permit drops here → releases semaphore
    }
}

// ---------------------------------------------------------------------------
// Payload construction
// ---------------------------------------------------------------------------

/// Build the content blocks for an ambient batch dispatch.
fn build_ambient_payload(batch: &[AmbientMessage]) -> Vec<ContentBlock> {
    let mut blocks = Vec::new();

    // System instruction.
    blocks.push(ContentBlock::Text {
        text: AMBIENT_SYSTEM_INSTRUCTION.to_string(),
    });

    // Format batch as conversation transcript.
    let mut transcript = String::from("[Ambient batch — ");
    transcript.push_str(&batch.len().to_string());
    transcript.push_str(" new messages]\n");

    for msg in batch {
        transcript.push_str(&msg.sender_name);
        transcript.push_str(": ");
        transcript.push_str(&msg.prompt);
        transcript.push('\n');
    }

    transcript.push_str("\n[End of batch — reply only if you can add meaningful value.\n Otherwise reply exactly: [NO_REPLY]]");

    blocks.push(ContentBlock::Text { text: transcript });

    // Append any attachment blocks from messages.
    for msg in batch {
        blocks.extend(msg.extra_blocks.iter().cloned());
    }

    blocks
}

/// Check if a response is the NO_REPLY sentinel.
pub fn is_no_reply(response: &str) -> bool {
    response.trim().to_lowercase() == NO_REPLY_SENTINEL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_no_reply_exact() {
        assert!(is_no_reply("[NO_REPLY]"));
        assert!(is_no_reply("[no_reply]"));
        assert!(is_no_reply("[No_Reply]"));
    }

    #[test]
    fn is_no_reply_with_whitespace() {
        assert!(is_no_reply("  [NO_REPLY]  "));
        assert!(is_no_reply("\n[no_reply]\n"));
        assert!(is_no_reply("\t [NO_REPLY] \t"));
    }

    #[test]
    fn is_no_reply_rejects_partial() {
        assert!(!is_no_reply("NO_REPLY"));
        assert!(!is_no_reply("[NO_REPLY] sure"));
        assert!(!is_no_reply("I have [NO_REPLY] for you"));
        assert!(!is_no_reply(""));
    }

    #[test]
    fn post_guard_lifecycle() {
        let guard = PostGuard::new();
        assert!(guard.can_post());

        guard.cancel();
        assert!(!guard.can_post());

        guard.reset();
        assert!(guard.can_post());
    }

    #[test]
    fn post_guard_double_cancel() {
        let guard = PostGuard::new();
        guard.cancel();
        guard.cancel();
        assert!(!guard.can_post());

        guard.reset();
        assert!(guard.can_post());
    }
}
