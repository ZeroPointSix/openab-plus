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

OpenClaw supports an `always` activation mode for group chats:
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
- History backfill (`DISCORD_HISTORY_BACKFILL`) recovers missed context when
  the bot is later @mentioned.
- **Limitation:** no autonomous decision-making — the bot always replies. There
  is no `NO_REPLY` equivalent; it's either "respond to everything" or
  "respond only on mention."

### Research — "Controlling AI Agent Participation in Group Conversations"

(arXiv 2501.17258) — studies user preferences for AI agent behavior in group
settings. Key finding: users disliked agents that dominated the conversation
and preferred controls over when/how the agent participates.

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
│  │ • Collect buffered messages as conversation context        │  │
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

### Flush Triggers

Messages are accumulated in a per-channel buffer and flushed when **either**
condition is met (whichever comes first):

| Trigger | Default | Description |
|---------|---------|-------------|
| Time | `flush_interval_seconds = 60` | Seconds since first buffered message |
| Count | `flush_max_messages = 10` | Max messages to accumulate before flush |

**Immediate flush override:** if any message in the buffer explicitly
`@mentions` the bot, the buffer is flushed immediately (no waiting) — users
should not have to wait 60 seconds when they directly address the agent.

### Batch Payload

The flushed batch is formatted as a conversation block:

```
[Ambient batch — 4 messages, channel: #general]

[12:00:01] UserA: Anyone know how to fix the helm release?
[12:00:04] UserB: Which chart version?
[12:00:11] UserA: 0.8.5
[12:00:15] UserC: Try rolling back first

[End of batch — reply only if you can add value. Otherwise reply exactly: [NO_REPLY]]
```

### Configuration

```toml
[discord.ambient]
enabled = true
channels = ["1490282656913559673"]   # channels where ambient mode is active
flush_interval_seconds = 60           # time-based flush trigger
flush_max_messages = 10               # count-based flush trigger
context_window = 20                   # additional history before the batch
```

- `enabled` — master switch (default: `false`).
- `channels` — allowlist of channel IDs. Empty = all allowed channels.
- `flush_interval_seconds` — max time to hold messages before flushing
  (timer starts when the first message enters an empty buffer).
- `flush_max_messages` — max messages to buffer before flushing regardless
  of time elapsed.
- `context_window` — number of historical messages (before the batch) to
  include for additional context.

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
- **Natural rate limiting** — the flush interval acts as an inherent cooldown.
  No separate cooldown mechanism needed.
- **Agents behave like real team members** — aware of context, able to
  contribute organically.
- **User-configurable** — operators decide the cost/intelligence trade-off.

### Trade-offs

- **Latency** — ambient replies are delayed by up to `flush_interval_seconds`.
  Acceptable because ambient replies are unsolicited; explicitly mentioned
  messages bypass the buffer and get immediate dispatch via the normal path.
- **Token cost still increases** — even with batching, each flush is an LLM
  call. Mitigations: per-channel opt-in, tunable flush interval/count.
- **Potential for noise** — a poorly-tuned prompt or model may reply too
  eagerly. The batch format and explicit instructions mitigate this.
- **Session management** — ambient dispatches should use short-lived or
  stateless sessions so the pool isn't exhausted.

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

- A new `AmbientBuffer` struct per channel holds pending messages and a
  flush timer. On flush, it formats the batch and sends to the agent via
  the existing ACP dispatch path.
- The `[NO_REPLY]` check should be applied in the response router
  (`src/adapter.rs`) before calling `send_message`.
- Reactions (👀, 🤔, etc.) should be suppressed for ambient dispatches to
  avoid spamming the channel with status indicators.
- When a message in the buffer contains an explicit `@mention` of the bot,
  the buffer should flush immediately and the dispatch should be marked as
  "mention-triggered" (not ambient) so normal reply behavior applies.
- The `allow_bot_messages` and `allow_user_messages` config options already
  provide dispatch filtering logic in `src/discord.rs`. Ambient mode adds a
  new dispatch path that coexists with the existing mention-based path.
