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

## Decision

Introduce an **Ambient Mode** that can be enabled per-agent per-channel.

### Mechanism

```
┌────────────────────────────────────────────────────────┐
│  Discord Channel                                       │
│                                                        │
│  User A: "Anyone know how to fix the helm release?"    │
│                                                        │
└──────────────────────────┬─────────────────────────────┘
                           │  (all messages dispatched)
                           ▼
┌──────────────────────────────────────────────────────────┐
│  OpenAB Gateway                                          │
│                                                          │
│  ┌─────────────────────────────────────────────────────┐ │
│  │ Ambient Dispatch                                    │ │
│  │                                                     │ │
│  │ • Forward message to agent                          │ │
│  │ • Prepend system instruction:                       │ │
│  │   "You are in ambient mode. If you have nothing     │ │
│  │    valuable to add, reply exactly: [NO_REPLY]"      │ │
│  └──────────────────────────────┬──────────────────────┘ │
│                                 │                        │
│                                 ▼                        │
│  ┌──────────────────────────────────────────────────────┐│
│  │ Response Router                                      ││
│  │                                                      ││
│  │ • Agent replies "[NO_REPLY]" → discard silently      ││
│  │ • Agent replies with content  → post to Discord      ││
│  └──────────────────────────────────────────────────────┘│
└──────────────────────────────────────────────────────────┘
```

1. When ambient mode is enabled for a channel, **every message** in that channel
   is dispatched to the agent — regardless of mentions.
2. The dispatch payload includes a system-level instruction telling the agent:
   *"You are in ambient listening mode. Only reply if you have something
   genuinely useful to add. Otherwise respond exactly with `[NO_REPLY]`."*
3. If the agent's response is the literal sentinel `[NO_REPLY]`, OpenAB
   discards it and sends nothing back to Discord.
4. Otherwise the response is posted normally.

### Configuration

```toml
[discord]
bot_token = "${DISCORD_BOT_TOKEN}"

[discord.ambient]
enabled = true
channels = ["1490282656913559673"]   # channels where ambient mode is active
cooldown_seconds = 300                # min gap between unsolicited replies
context_window = 20                   # recent messages to include as context
```

- `enabled` — master switch (default: `false`).
- `channels` — allowlist of channel IDs. Empty = all allowed channels.
- `cooldown_seconds` — after the agent posts an ambient reply, suppress further
  ambient replies for this many seconds (prevents chattiness). Explicitly
  mentioned messages bypass cooldown.
- `context_window` — number of recent messages to include for context when
  dispatching.

### Sentinel Value

The sentinel is `[NO_REPLY]` (case-insensitive, trimmed). Chosen because:
- Unlikely to appear in natural agent output.
- Simple to detect with a string match (no regex needed).
- Easy for any LLM to produce reliably.

## Consequences

### Benefits

- Agents behave like real team members — aware of context, able to contribute
  organically.
- Zero additional infrastructure; the mechanism reuses existing dispatch and
  response paths.
- User-configurable: operators decide the cost/intelligence trade-off themselves.

### Trade-offs

- **Token cost increases** — every channel message triggers an LLM invocation
  (even if the result is `[NO_REPLY]`). Mitigations:
  - Per-channel opt-in keeps blast radius small.
  - Cooldown prevents runaway cost from high-traffic channels.
  - Future: add a lightweight keyword/semantic pre-filter before dispatching.
- **Potential for noise** — a poorly-tuned prompt or model may reply too eagerly.
  Cooldown + explicit prompt instructions + the ability to disable per-channel
  mitigate this.
- **Session management** — ambient messages should not create long-lived
  sessions. They should use a short-lived or stateless invocation so the session
  pool isn't exhausted.

## Alternatives Considered

1. **Keyword pre-filter** — only dispatch if the message matches certain
   keywords. Rejected as primary mechanism because it defeats the purpose of
   intelligent, context-aware participation. May be added later as a cost
   optimization layer.
2. **Periodic summary mode** — batch N messages and summarize them to the agent
   periodically. Rejected because it loses real-time interactivity.
3. **Separate lightweight classifier** — use a small/cheap model to decide
   whether to invoke the main agent. Viable as a future enhancement but adds
   complexity for v1.

## Implementation Notes

- The `allow_bot_messages` and `allow_user_messages` config options already
  provide some dispatch filtering logic in `src/discord.rs`. Ambient mode can
  be implemented as a new dispatch path that coexists with the existing
  mention-based path.
- The `[NO_REPLY]` check should be applied in the response router
  (`src/adapter.rs`) before calling `send_message`.
- Reactions (👀, 🤔, etc.) should be suppressed for ambient dispatches to avoid
  spamming the channel with status indicators on every single message.
