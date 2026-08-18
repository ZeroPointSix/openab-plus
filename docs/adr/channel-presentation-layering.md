# ADR: Channel Presentation Layering (Control Plane vs IM Presentation)

- **Status:** Proposed (discussion deliverable)
- **Date:** 2026-08-18
- **Tracking issues:** ZER-569 (parent ZER-565); related ZER-414 (Slack thread presentation), ZER-525 (hide Slack thinking chain), ZER-506 (MetaMCP boundary), ZER-554 (Ask User / Webhook HITL)
- **Supersedes:** nothing. Extends `docs/adr/multi-platform-adapters.md` (ChatAdapter / AdapterRouter) with an explicit presentation-vs-control-plane boundary.

---

## 1. Context

`docs/adr/multi-platform-adapters.md` established that the "front door" is pluggable: `ChatAdapter` implementations own platform I/O, while `AdapterRouter` + `SessionPool` own the turn lifecycle. Since then the trait has grown a second, different kind of method: **presentation decisions**.

Today `crates/openab-core/src/adapter.rs` mixes both kinds in one trait:

- transport capability: `send_message`, `edit_message`, `add_reaction`, `create_thread`, `stream_begin` / `stream_append` / `stream_finish`, `set_status`
- presentation policy: `exposes_intermediate_text`, `uses_tool_progress_message`, `session_link_label`, `renders_native_tables`, `use_streaming`, `show_streaming_placeholder`, `uses_assistant_status`

The second group answers product questions ("should this channel show the thinking chain?", "should tool titles be visible?", "is a session deep link appropriate here?"), not protocol questions. Because they are hard-coded per adapter, every presentation change for one IM (Slack) lands as a code change inside that IM's adapter, and there is standing pressure to express IM-specific wishes by widening shared state - in the worst case by forking the agent Profile schema per channel.

ZER-569 asks for the boundary to be written down before more channels (Feishu, Telegram, LINE, WeCom, Google Chat, MS Teams, Admin Web, ACP) land, so that the front end is not welded to a single channel.

## 2. Decision

Three layers, with a fixed responsibility split. A layer may not reach into another layer's column.

| Layer | Owns | Explicitly does NOT own |
|-------|------|--------------------------|
| **Channel adapter layer** (`discord.rs`, `slack.rs`, `gateway.rs` + `unified_adapter.rs`) | Platform auth and connection, message shape / chunking bounds, thread UX model, reactions vs status API, emoji and Markdown dialect, presentation policy *evaluation* | Profile source of truth, session lifecycle, config persistence |
| **Control plane / upstream** (`AdapterRouter`, `SessionPool`, `profile_store.rs`, `agent_profile.rs`, `config.rs`, `transcript.rs`, `session_snapshot.rs`, SSE / Admin Web APIs) | Session identity and lifecycle, Profile and Config as single source of truth, ACP turn loop, transcript / snapshot / observability feed, trust gate | Per-IM copywriting, per-IM emoji choices, per-IM message layout |
| **Agent runtime** (ACP connection, agents, tools) | Reasoning and tool use | Channel protocol, channel-specific rendering |

**Core rule:** a channel is a *renderer plus an ingress*, never a second source of truth. Presentation differences are expressed as **data** (a presentation policy resolved from config, with defaults preserving today's behaviour), not as new shared schema fields and not as new branches inside control-plane code.

## 3. Interface inventory: channel vs control plane

This is the ZER-569 acceptance deliverable. Classification of the current `ChatAdapter` surface in `crates/openab-core/src/adapter.rs`:

### 3.1 Channel-owned - transport capability (stays on `ChatAdapter`)

| Method | Why it is channel-owned |
|--------|-------------------------|
| `platform()` | session-key namespace + logging |
| `message_limit()` | platform hard limit for `format::split_message` |
| `send_message`, `send_message_with_reply`, `edit_message`, `delete_message` | platform write API |
| `create_thread`, `rename_thread` | platform threading model |
| `add_reaction`, `remove_reaction`, `set_status` | platform status primitives |
| `stream_begin`, `stream_append`, `stream_finish` | native streaming API, if any |
| `uses_native_streaming`, `uses_assistant_status` | capability probes: does the API exist at all |

### 3.2 Channel-owned - presentation policy (should become resolved data, not hard-coded per adapter)

| Method today | Product question it answers | Target home |
|--------------|-----------------------------|-------------|
| `exposes_intermediate_text` | may raw intermediate agent text be published live? | `PresentationPolicy.expose_intermediate_text` |
| `uses_tool_progress_message` | one editable progress message vs inline tool lines | `PresentationPolicy.tool_progress` |
| `session_link_label` | append a session deep link, and with what label | `PresentationPolicy.session_link_label` |
| `renders_native_tables` | skip the table -> code/bullets pre-pass | `PresentationPolicy.native_tables` |
| `use_streaming`, `show_streaming_placeholder` | cosmetic streaming vs send-once, placeholder or not | `PresentationPolicy.streaming` |
| `[reactions] tool_display`, `narration_display` (already config) | how much tool / narration detail is visible | already policy - reuse the same shape |

### 3.3 Control-plane-owned (a channel must never reimplement these)

| Concern | Where it lives |
|---------|----------------|
| Session key namespacing `{platform}:{thread_id}` | `AdapterRouter::handle_message` |
| Session create / reuse / TTL / status | `SessionPool`, `session_snapshot.rs` |
| Profile selection and per-session overrides | `agent_profile.rs`, `profile_store.rs`, `profile_session_from_metadata` |
| ACP turn loop, liveness, hard timeout, abandon | `stream_prompt_blocks` |
| Prompt packing and `<sender_context>` envelope | `AdapterRouter::pack_arrival_event` |
| Transcript / SSE observability feed | `transcript.rs` |
| Trust gate (L2 scope + L3 identity) | `AdapterRouter::gate_incoming`, `trust.rs` |
| Directive parsing (`[[reply_to:...]]`, `[[ws:...]]`, profile directives) | `directives.rs`, `adapter.rs` |
| Config load / validation / defaults | `config.rs` |

### 3.4 Proposed shape (sketch, not implemented in this PR)

```rust
/// Resolved per-channel presentation policy. Every field defaults to the
/// value that reproduces today's behaviour for that channel.
pub struct PresentationPolicy {
    pub expose_intermediate_text: bool,
    pub streaming: StreamingMode,        // Auto | Edit | SendOnce
    pub show_streaming_placeholder: bool,
    pub tool_display: ToolDisplay,       // None | Compact | Full
    pub tool_progress: bool,
    pub narration_display: bool,
    pub native_tables: bool,
    pub session_link_label: Option<String>,
}
```

Resolution order (each step optional, later steps override earlier ones):

1. adapter default (current hard-coded value, so behaviour is unchanged when nothing is configured)
2. global `[reactions]` / display config
3. `[channel.<name>.presentation]` in gateway config

The router keeps reading a single resolved `PresentationPolicy`; adapters keep only capability probes.

## 4. Where does channel presentation config live?

**Decision: inside the existing gateway/platform config tree, as a `presentation` sub-table of the channel's own section - not as a standalone per-channel config service, and not in the agent Profile.**

```toml
[slack]
bot_token = "${SLACK_BOT_TOKEN}"
app_token = "${SLACK_APP_TOKEN}"

[slack.presentation]
expose_intermediate_text = false   # default for Slack today
tool_display = "none"
session_link_label = "Open in OpenAB Plus"
```

Rationale:

- one config load path, one validation path, one `docs/config-reference.md` section family
- a second config backend per channel is an explicit non-goal of ZER-569
- presentation is deployment-scoped, while a Profile is agent-scoped; mixing them is what produces schema forks

## 5. Invariant: Slack presentation changes must not fork the Profile schema

Operational form of the ZER-569 acceptance criterion, checkable in review:

1. No `slack_*` / `discord_*` / per-channel-named field may be added to `agent_profile.rs` or `profile_store.rs`. Channel-conditional presentation belongs to `PresentationPolicy`.
2. Profile answers "which agent, which model, which reasoning effort, which ACP config options". It never answers "how visible is the thinking chain in channel X".
3. A Slack-only presentation change is acceptable only if it is expressible as (a) a Slack adapter capability implementation, or (b) a presentation-policy default/value. If neither works, the change is a control-plane change and needs its own ADR.
4. Admin Web, Slack, and any future channel read the *same* session + Profile API. Divergence is allowed in rendering and in auth scope, never in schema.

## 6. New channel onboarding checklist

A new channel should be "adapter + presentation policy" only.

**Add:**

- [ ] a `ChatAdapter` implementation (or reuse `UnifiedGatewayAdapter` when the channel speaks the gateway protocol)
- [ ] `platform()` string, and confirm session keys become `{platform}:{thread_id}`
- [ ] `message_limit()` from the platform's documented cap, with a safety margin
- [ ] threading model mapping into `ChannelRef` (thread-as-reply-chain vs thread-as-child-channel)
- [ ] capability probes: native streaming, status API, native Markdown tables, edit support
- [ ] a presentation policy default block that reproduces the channel's intended UX
- [ ] a `[<channel>]` config section with secure-by-default allowlists (empty = deny all) and a trust-gate entry
- [ ] `docs/config-reference.md` entries and a `docs/<channel>.md` page
- [ ] adapter tests plus a router test through `MockAdapter`

**Must NOT be needed:**

- [ ] no change to `SessionPool`, session keying, or TTL semantics
- [ ] no new Profile schema field
- [ ] no new config backend or duplicated config store
- [ ] no channel-specific branch inside `AdapterRouter::stream_prompt_blocks` (policy value instead)
- [ ] no channel-specific transcript / SSE shape

If a checklist item in the second list is unavoidable, that is the signal to write a follow-up ADR rather than to special-case the router.

## 7. Boundary with Ask User / Webhook HITL (ZER-554)

Human-in-the-loop is a **control-plane** capability: the request, its pending state, and its resolution belong to the session, so a request raised in one channel remains valid if it is answered from Admin Web. A channel only renders the prompt and collects the answer with its native affordance (Slack modal, Discord buttons, web form). No channel may hold HITL state privately.

## 8. Non-goals

- No per-channel duplicate configuration backend.
- The browser / Admin Web does not masquerade as a full `ChatAdapter` in Phase 1; it consumes the session and transcript APIs.
- No refactor of `ChatAdapter` in this PR. This ADR only fixes the boundary and the target shape.
- No change to existing runtime behaviour: every field described here defaults to today's value.

## 9. Open questions

1. Should `PresentationPolicy` be per channel, or per (channel, workspace) pair for multi-tenant deployments?
2. Does `tool_display` need a per-thread override via control directives, or is deployment scope enough?
3. Should capability probes and policy be split into two traits (`ChatAdapter` + `ChannelPresentation`), or is one trait with a documented split sufficient?
4. When a policy asks for something the platform cannot do (native streaming where the API is absent), is the correct behaviour a startup validation error or a silent documented downgrade?

## 10. Acceptance mapping (ZER-569)

| ZER-569 criterion | Section |
|-------------------|---------|
| Channel vs control-plane interface inventory | 3 |
| Slack presentation changes must not fork the Profile schema | 5 |
| New-channel checklist limited to adapter + presentation policy | 6 |
