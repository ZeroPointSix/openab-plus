# ADR: Ambient Mode

- **Status:** Proposed
- **Date:** 2026-06-26
- **Author:** Pahud Hsieh

## Context

Today, OpenAB only dispatches messages to an agent when the agent is explicitly
mentioned (or the message is in a thread the agent participates in). This means
agents are deaf to surrounding conversation unless invoked — they cannot
proactively contribute context, answer questions addressed to no one in
particular, or notice when a discussion touches their area of expertise.

We want agents to behave more like attentive team members who listen to the room
and speak up when they have something valuable to add, without requiring an
explicit `@mention` every time.

## Prior Art

### OpenClaw — `/activation always`

OpenClaw supports an `always` activation mode as a cross-platform group chat
feature (WhatsApp, Telegram, Discord, Slack, iMessage — configured via
`agents.list[].groupChat.mentionPatterns` and `channels.*.groups`):
- Every message is dispatched to the agent (no mention required).
- Agent returns the sentinel token `NO_REPLY` when it has nothing to add;
  the gateway discards silently.
- Pending messages (up to 50) are accumulated as context and injected as
  `[Chat messages since your last reply - for context]`.
- Per-group toggle via `/activation always` or `/activation mention`.
- **Limitation:** messages are dispatched one-by-one — each message triggers a
  separate LLM invocation, even if it results in `NO_REPLY`. No batching.

### Hermes Agent — `free_response_channels`

Hermes provides `DISCORD_FREE_RESPONSE_CHANNELS` and
`DISCORD_REQUIRE_MENTION=false`:
- The bot responds to **every** message in designated channels without mention.
- Free-response channels skip auto-threading (replies inline) and isolate
  sessions per user (`group_sessions_per_user: true` by default).
- History backfill (`DISCORD_HISTORY_BACKFILL`) recovers missed channel context
  on `@mention` — only triggered when `require_mention: true` and skipped in
  free-response channels and DMs where the transcript is already complete.
  Scans up to 50 messages backwards, stopping at the bot's own last message.
- **Limitation:** no autonomous decision-making — the bot always replies. There
  is no `NO_REPLY` equivalent; it's either "respond to everything" or
  "respond only on mention."

### Research — "Controlling AI Agent Participation in Group Conversations"

(arXiv 2501.17258, Jan 2025) — studies user preferences for AI agent behavior
in group settings. Key finding: users benefited from having the AI in the group,
but disliked when the agent dominated the conversation and desired controls
over its interactive behaviors.

### Gap Our Design Fills

Neither OpenClaw nor Hermes implements **batch flush** — they dispatch per
message. Our design accumulates messages and flushes them as a batch, which:
1. Reduces LLM invocations (one call per batch instead of N).
2. Gives the agent fuller conversational context for better judgment.
3. Provides natural rate-limiting without additional cooldown mechanisms.

## Decision

Introduce an **Ambient Mode** using a **batch flush** strategy.

### Mechanism

```
Discord Channel
────────────────────────────────────────────────────────────────────
  msg1 (t=0s)  │
  msg2 (t=3s)  │  accumulate in buffer
  msg3 (t=8s)  │
  msg4 (t=12s) │
               ▼
         ┌─────────────────────────────┐
         │ Flush trigger fired         │
         │ (60s elapsed OR 10 msgs)    │
         └─────────────┬───────────────┘
                       │
                       ▼
┌──────────────────────────────────────────────────────────────────┐
│  OpenAB Gateway                                                  │
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │ Ambient Dispatch (batch)                                   │  │
│  │                                                            │  │
│  │ • Lock buffer → drain all messages → unlock immediately    │  │
│  │   (new messages enter a fresh buffer cycle)                │  │
│  │ • Prepend: channel history (context_window via API)        │  │
│  │ • Prepend system instruction:                              │  │
│  │   "You are in ambient mode. Below is a batch of recent     │  │
│  │    messages. If you have nothing valuable to add, reply    │  │
│  │    exactly: [NO_REPLY]"                                    │  │
│  │ • Send batch to agent                                      │  │
│  └────────────────────────────┬───────────────────────────────┘  │
│                               │                                  │
│                               ▼                                  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │ Response Router                                            │  │
│  │                                                            │  │
│  │ • Agent replies "[NO_REPLY]" → discard silently            │  │
│  │ • Agent replies with content  → post to Discord            │  │
│  └────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

### Buffer Lifecycle

The `AmbientBuffer` operates as a **swap-and-drain** model:

1. Messages arrive → pushed into the active buffer (under a short lock).
2. Flush triggers → lock the buffer, **swap** it with a fresh empty buffer,
   unlock immediately. The drained batch is processed asynchronously.
3. New messages arriving during flush processing enter the fresh buffer and
   will be part of the **next** flush cycle.

This eliminates race conditions: the lock is held only for the swap operation
(microseconds), and flush processing is fully decoupled from ingestion.

### Flush Triggers

Messages are accumulated in a per-channel buffer and flushed when **any**
condition is met (whichever comes first):

| Trigger | Default | Description |
|---------|---------|-------------|
| Time | `flush_interval_seconds = 60` | Seconds since first buffered message |
| Count | `flush_max_messages = 10` | Max messages to accumulate before flush |
| Hard cap | `flush_hard_cap = 50` | Safety cap — force flush regardless of timer state |
| Mention | immediate | Any message that @mentions the bot triggers instant flush |

**Flush interval jitter:** to prevent thundering herd when many channels flush
simultaneously, the actual interval is `flush_interval_seconds ± 20%` (random
per-channel, recomputed each cycle).

**Immediate flush on @mention:** when a message that `@mentions` the bot enters
the buffer, the buffer flushes immediately. However, the @mention message is
**removed from the batch** and dispatched separately via the normal
mention-triggered path (with full reactions, threading, etc.). The remaining
buffered messages are flushed as a normal ambient batch. This preserves clean
semantics: mention = normal dispatch, ambient = batch dispatch.

**Concurrent reply prevention:** to prevent double-replying to the same channel
when a @mention arrives during ambient processing:
- The ambient consumer holds a per-channel `AtomicBool` lock (`flushing`).
- Normal mention dispatch checks this flag: if the ambient consumer is
  mid-flush, the mention dispatch waits for the ambient flush to complete
  (or cancel) before proceeding.
- Conversely, if a mention dispatch is already in-flight on the same channel
  (tracked via the primary Dispatcher), the ambient consumer skips posting
  its response even if it's not `[NO_REPLY]` — the user already got a direct
  reply.
- This ensures at most one bot response is posted to a channel at any given
  moment from the ambient + mention paths combined.

### Message Filtering for Buffer

Not all messages enter the ambient buffer:

- ✅ **User messages** in ambient-enabled channels (without @mention) → buffer
- ✅ **Bot messages from other bots** (if `allow_bot_messages` permits) → buffer
- ❌ **Own bot messages** → never buffered (prevents echo loops)
- ❌ **Messages that @mention the bot** → bypass buffer, trigger immediate
  flush of existing buffer + normal mention dispatch
- ❌ **Messages in threads created by the bot** → handled by existing
  thread-based session logic, not ambient

### Batch Payload

The flushed batch is formatted as a conversation block:

```
[Ambient context — recent channel history]
[12:00:01] UserC: I pushed the helm fix yesterday
[12:00:02] UserB: cool

[Ambient batch — 4 new messages since last flush]
[12:03:01] UserA: Anyone know how to fix the helm release?
[12:03:04] UserB: Which chart version?
[12:03:11] UserA: 0.8.5
[12:03:15] UserC: Try rolling back first

[End of batch — reply only if you can add meaningful value.
 Otherwise reply exactly: [NO_REPLY]]
```

### Session Strategy

Ambient dispatches use a **dedicated session pool**, separate from the main
mention-triggered pool:

| Aspect | Mention dispatch | Ambient dispatch |
|--------|-----------------|-----------------|
| Session key | `discord:<thread_id>` | `ambient:discord:<channel_id>` |
| Pool | Main pool (`[pool]`) | Ambient pool (`[pool.ambient]`) |
| Lifetime | Long-lived (session_ttl_hours) | Short-lived (ambient_session_ttl_minutes) |
| Cross-flush memory | Full transcript | Rolling window (last N flushes) |
| Reactions | ✅ Full (👀🤔🔥🆗) | ❌ Suppressed |

**Why separate pools:** prevents ambient traffic from exhausting the main pool
and blocking normal @mention responses. The ambient pool has its own
`max_sessions` cap.

**Cross-flush context:** the ambient session retains a rolling window of the
last `ambient_context_flushes` (default: 3) flush interactions, so the agent
has memory of what it said/declined recently. Sessions expire after
`ambient_session_ttl_minutes` (default: 60) of inactivity.

### Configuration

```toml
[discord.ambient]
enabled = false                       # master switch
channels = ["1490282656913559673"]    # required — explicit allowlist, empty = disabled
flush_interval_seconds = 60           # time-based flush trigger (±20% jitter applied)
flush_max_messages = 10               # count-based flush trigger
flush_hard_cap = 50                   # safety cap — force flush at this count
context_window = 20                   # historical messages fetched via Discord API before batch

[pool.ambient]
max_sessions = 5                      # separate pool for ambient dispatches
ambient_session_ttl_minutes = 60      # ambient session inactivity timeout
ambient_context_flushes = 3           # rolling window of retained flush history

[ambient.limits]
max_concurrent_flushes = 3            # max simultaneous LLM calls across all ambient channels
```

**`channels` semantics:** an explicit allowlist is **required**. If `channels`
is empty or omitted while `enabled = true`, ambient mode is **not activated**
for any channel (fail-safe). This prevents accidental global ambient activation.

**`context_window`:** fetches the N most recent messages from the Discord
channel history API (before the batch window) to provide additional context.
This is a Discord API call with standard rate limiting. If fewer than N messages
exist, all available messages are included. These messages are **not** counted
toward `flush_max_messages`.

### Error Handling

| Scenario | Behavior |
|----------|----------|
| LLM timeout / network error | Batch is **discarded** (not retried). Next flush cycle starts fresh. Logged as warning. |
| Agent returns tool calls | Treated as normal response — if final output is not `[NO_REPLY]`, post it. Tool calls execute normally within the ambient session. |
| Agent returns empty response | Treated as `[NO_REPLY]` (discard silently). |
| Buffer grows beyond `flush_hard_cap` | Force flush immediately, regardless of timer state. |
| Discord API rate limit on `context_window` fetch | Skip context window, flush batch without historical context. Log warning. |

### Sentinel Value

The sentinel is `[NO_REPLY]` (case-insensitive, trimmed). Chosen because:
- Unlikely to appear in natural agent output.
- Simple to detect with a string match (no regex needed).
- Easy for any LLM to produce reliably.
- Consistent with OpenClaw's established `NO_REPLY` convention.

## Consequences

### Benefits

- **Token efficient** — one LLM call per batch instead of per message. A
  channel with 10 messages in 60 seconds costs 1 invocation, not 10.
- **Better judgment** — agent sees a complete conversational thread, making
  it far more likely to know when a question was already answered (→ NO_REPLY)
  vs. when it should contribute.
- **Natural rate limiting** — the flush interval + jitter acts as inherent
  rate-limiting. Combined with `max_concurrent_flushes`, prevents cost spikes.
- **Agents behave like real team members** — aware of context, able to
  contribute organically.
- **User-configurable** — operators decide the cost/intelligence trade-off.
- **Fail-safe defaults** — disabled by default, requires explicit channel list,
  separate session pool prevents impact on normal operations.

### Trade-offs

- **Latency** — ambient replies are delayed by up to `flush_interval_seconds`.
  Acceptable because ambient replies are unsolicited; explicitly mentioned
  messages bypass the buffer entirely via the normal dispatch path.
- **Token cost still increases** — even with batching, each flush is an LLM
  call. Mitigations: per-channel opt-in, tunable flush interval/count,
  `max_concurrent_flushes` cap.
- **Potential for noise** — a poorly-tuned prompt or model may reply too
  eagerly. The batch format and explicit instructions mitigate this.
- **No retry on failure** — ambient batches are fire-and-forget. If a flush
  fails, those messages are lost context. Acceptable because ambient is
  best-effort by nature.

## Alternatives Considered

1. **Per-message dispatch (OpenClaw-style)** — dispatch every message
   individually. Rejected because it burns N invocations for N messages,
   most of which return NO_REPLY. Batch flush achieves the same goal with
   ~1/N the cost.
2. **Keyword pre-filter** — only dispatch if the message matches certain
   keywords. Rejected as primary mechanism because it defeats the purpose of
   intelligent, context-aware participation. May be added later as a cost
   optimization layer.
3. **Separate lightweight classifier** — use a small/cheap model to decide
   whether to invoke the main agent. Viable as a future enhancement but adds
   complexity for v1.
4. **Periodic summary mode** — batch N messages and summarize them before
   sending to agent. Rejected because the agent should see raw messages for
   full context; summarization loses nuance.

## Implementation Notes

### Reuse of Existing `Dispatcher` Infrastructure

OpenAB already has a **turn-boundary batching** system (PR #686,
`message_processing_mode` config) with `Dispatcher`, per-thread `mpsc::channel`,
and `consumer_loop`. Ambient Mode should extend this infrastructure rather than
building a parallel buffer system.

**What we reuse:**
- `Dispatcher::submit()` — message ingestion into bounded mpsc channel
- `BufferedMessage` struct — carries prompt, sender_context, attachments
- `consumer_loop` — long-lived task that drains and dispatches
- `dispatch_batch` → `pack_arrival_event` — packing N messages into
  `Vec<ContentBlock>` with repeated `<sender_context>` delimiters
- `ThreadHandle` lifecycle — idle eviction, SendError retry

**What differs for ambient mode:**

| Aspect | Turn-boundary (existing) | Ambient consumer |
|--------|-------------------------|-----------------|
| Drain trigger | Turn completion (greedy drain when agent finishes) | Timer (`flush_interval ± jitter`) OR count (`flush_max_messages`) |
| Key | `(platform, thread_id)` | `ambient:(platform, channel_id)` |
| Prerequisite | Message already passed mention/involved gate | Message has NO mention (new gate path) |
| Response handling | Normal post | `[NO_REPLY]` check before posting |
| Reactions | Full (👀🤔🔥🆗) | Suppressed |
| Session pool | Main pool | Ambient pool (separate `max_sessions`) |

**New `message_processing_mode` value:** extend the enum to include `"ambient"`:

```toml
# Existing modes (unchanged):
message_processing_mode = "per-message"   # 1 msg → 1 turn
message_processing_mode = "per-thread"    # batch at turn boundary
message_processing_mode = "per-lane"      # batch at turn boundary, per-sender

# New mode (this ADR):
# Configured separately in [discord.ambient] — not via message_processing_mode.
# Ambient is a parallel dispatch path, not a replacement for the primary mode.
```

Ambient mode runs as a **separate Dispatcher instance** alongside the primary
one. The primary Dispatcher handles mention-triggered messages (using whatever
`message_processing_mode` is configured). The ambient Dispatcher handles
non-mentioned messages in ambient-enabled channels with a timer-based consumer.

### Ambient Consumer Loop

```rust
// Pseudocode — ambient consumer differs from turn-boundary consumer:
async fn ambient_consumer_loop(rx, config, flush_semaphore, channel_flushing) {
    loop {
        let first = match rx.recv().await {
            Some(msg) => msg,
            None => return,                    // channel closed, exit consumer
        };
        let deadline = Instant::now() + config.flush_interval_jittered();
        let mut batch = vec![first];

        loop {
            let remaining = deadline - Instant::now();
            match timeout(remaining, rx.recv()).await {
                Ok(Some(msg)) => {
                    batch.push(msg);
                    if batch.len() >= config.flush_max_messages { break; }
                    if batch.len() >= config.flush_hard_cap { break; }
                }
                Ok(None) => break,             // channel closed
                Err(_) => break,               // timer expired
            }
        }

        // Acquire global concurrency permit (blocks if max_concurrent_flushes reached)
        let _permit = flush_semaphore.acquire().await;

        // Mark channel as flushing (prevents concurrent mention reply)
        channel_flushing.store(true, Ordering::Release);

        // Flush: dispatch batch with [NO_REPLY] system prompt
        match dispatch_ambient_batch(batch).await {
            Ok(response) => {
                if !response.trim().eq_ignore_ascii_case("[NO_REPLY]") {
                    post_response(response).await;  // no "thinking" msg — post directly
                }
            }
            Err(e) => {
                warn!("ambient flush failed, discarding batch: {e}");
            }
        }

        channel_flushing.store(false, Ordering::Release);
        // _permit dropped here — releases semaphore slot
    }
}
```

### Other Implementation Details

- **No thinking message:** ambient dispatches do NOT send a "..." placeholder
  message. Unlike normal mention dispatch, ambient responses are posted directly
  as a single message (or discarded). This eliminates visual flickering in
  the channel.
- **`[NO_REPLY]` check:** applied after `stream_prompt` completes. If the
  trimmed final content equals `[NO_REPLY]` (case-insensitive), no message is
  posted.
- **Bot-to-bot loop prevention:** the bot's own messages never enter the buffer
  (existing `bot_id` check). Additionally, messages from other bots are gated
  by `allow_bot_messages` config. Even if another bot's reply enters the buffer,
  the existing `MAX_CONSECUTIVE_BOT_TURNS` hard cap (applied at ingest, before
  `submit`) prevents infinite loops. The ambient system prompt also explicitly
  instructs: "Do not reply to other bot messages unless directly relevant to a
  human's question."
- **Mention detection reuse:** the existing `is_mentioned` logic in
  `Handler::message()` (src/discord.rs) fires **before** the buffer push.
  If mentioned, the message takes the normal dispatch path; remaining buffer
  is flushed as ambient.
- **Bot echo prevention:** `msg.author.id == bot_id` check (already exists in
  Handler::message) ensures bot's own messages never enter the buffer.
- **Reactions suppressed:** ambient dispatches skip `StatusReactionController`
  entirely — no 👀🤔🔥 on every channel message.
- **Serialization with normal dispatch:** the ambient session key
  (`ambient:discord:<channel_id>`) is different from mention session keys
  (`discord:<thread_id>`), so they never contend on the same session lock.
  If a normal @mention arrives while an ambient flush is in-flight, both
  proceed independently (different sessions, different pools).
