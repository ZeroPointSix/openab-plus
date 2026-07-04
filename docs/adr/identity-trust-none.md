# ADR: Identity Trust-None Default & Trust Pyramid

- **Status:** Proposed (v2 — revised per PR #1263 review feedback)
- **Date:** 2026-06-30 (revised 2026-07-04)
- **Author:** @chaodu-agent
- **Reviewers:** @pahud, @howie
- **Tracking issues:** #1262
- **Depends on:** [First-Class Per-Platform Configuration](first-class-platform-config.md) — per-platform `allowed_users` live in the first-class `[platform]` sections defined there.

---

## 1. Context & Decision

Flip the default trust model from **allow-all** to **identity trust-none**: when a
platform's `allowed_users` is empty and `allow_all_users` is not explicitly set to
`true`, deny all incoming messages and echo the sender their own ID so they can
request access.

Trust is enforced at a **dedicated ingress layer** — the Trust Gate — that sits
between the platform Receiver and the per-platform Handler. This is a structural
guarantee: no event reaches any Handler (or the Dispatcher / Agent) without passing
through the gate. The gate is **not** inside any adapter — it is an independent
layer that all adapters are wired through.

## 2. Motivation: trust-all default is insecure

All adapters currently auto-detect: empty `allowed_users` → `allow_all_users = true`.
A fresh deployment trusts **everyone** by default. For publicly discoverable bots
(e.g. anyone can DM a Telegram bot), this means any stranger can drive the agent.

Additionally, trust checks are currently **scattered** across adapters — each one
implements its own variant (`is_denied_user()` in Discord, `should_skip_event()` in
Gateway, inline allowlist in Slack). This means:
- Different implementations doing the same thing
- A new adapter forgetting the check = fully open bot
- No architectural guarantee that trust is enforced

## 3. Trust Pyramid (Defense in Depth)

Three layers with **clearly separated responsibilities** — only L1 and L3 are
security boundaries. L2 is operator scoping, not authorization.

```
                          ▲
                         ╱ ╲
                        ╱   ╲
                       ╱ L3  ╲         🔒 Layer 3: Identity Trust Control  (SECURITY)
                      ╱       ╲        allowed_users per platform — default DENY-ALL
                     ╱ sender  ╲       "Is THIS IDENTITY allowed?"  covers every path incl. DMs
                    ╱  allowed? ╲
                   ╱─────────────╲
                  ╱               ╲
                 ╱      L2         ╲    🔓 Layer 2: Channel/Group Scope Control  (NOT security)
                ╱                   ╲   allowed_channels, allowed_groups, allow_dm — default OPEN
               ╱  surface open?      ╲  "Which CONVERSATION SURFACES does the bot engage in?"
              ╱  (channel/group/DM)   ╲  optional operator scoping (noise/cost), not authorization
             ╱─────────────────────────╲
            ╱                           ╲
           ╱           L1                ╲   🔒 Layer 1: Platform Authentication  (SECURITY)
          ╱                               ╲  "Is this request REALLY from the platform?"
         ╱   webhook signature / JWT /     ╲
        ╱    secret token / IP range        ╲
       ╱─────────────────────────────────────╲
```

**Default posture:** L1 always on (edge) · **L2 open** unless explicitly disabled · **L3 deny-all** unless explicitly allowed.

### Layer 1: Platform Authentication (gateway layer — transport)

Verifies the request is genuinely from the platform, not spoofed. The **only**
security check at the gateway level.

| Platform | Auth Mechanism | How it works |
|----------|---------------|--------------|
| **Telegram** | Secret Token + IP Range | `X-Telegram-Bot-Api-Secret-Token` header; source IP in Telegram subnet (149.154.160.0/20, 91.108.4.0/22) |
| **LINE** | HMAC-SHA256 Signature | `X-Line-Signature` = HMAC(channel_secret, request_body) |
| **Feishu** | SHA256 Signature + Encrypt Key | SHA256(timestamp + nonce + encrypt_key + body) |
| **WeCom** | Token Signature + AES Decrypt | SHA1(sort(token, timestamp, nonce, encrypt)); AES-256-CBC body decryption |
| **Google Chat** | JWT (RS256) | Bearer token verified via Google JWKS; email claim = `chat@system.gserviceaccount.com` |
| **MS Teams** | JWT (OpenID Connect) | RS256 JWT verified via Bot Framework OpenID metadata + JWKS |
| **Slack** | Socket Mode WebSocket | App-Level Token (xapp-...) authenticates WS connection |
| **Discord** | Gateway WebSocket | Bot Token authenticates WS connection |

### Layer 2: Channel/Group Scope Control (core layer) — NOT a security boundary

Controls **which conversation surfaces** the bot engages in — channels, groups,
and DMs (`allow_dm`). Already implemented.

This is **operator scoping, not authorization**. The platform itself already
guarantees the bot only receives events from channels/groups it is a member of
with read permission — you cannot receive a message from a channel you were never
added to. So `allowed_channels` does not defend against "unauthorized channels"
(L1/the platform already does); it only narrows an over-permissioned bot to the
surfaces an operator wants it active in. Its value is noise/cost control.

**Default: OPEN** (`allow_all_channels = true`, `allow_dm = true`). Operators
*disable* surfaces only for hard scoping (e.g. a group-only bot sets
`allow_dm = false`).

**DMs are an L2 surface with a critical asymmetry:** unlike groups, a DM has **no
platform membership gate** — anyone can open a DM with a public bot. So when
`allow_dm = true`, the **only** protection on that path is L3. Enabling the DM
surface is an L2 decision; guarding who may use it is L3.

### Layer 3: Identity Trust Control (core layer) ← This ADR — the SECURITY gate

Controls which individual senders can trigger agent actions. Currently defaults
to allow-all; this ADR flips it to **deny-all**. This is the one authorization
boundary at the policy layer, and it covers **every** ingress path — including
DMs, where it is the sole protection.

**Why L2 must stay open for the deny UX to work:** the "echo your UID so you can
request access" reply only fires if an untrusted sender's message actually
*reaches* L3. If L2 defaulted closed (e.g. `allow_dm = false`), a new user would
be silently dropped at the scope layer with no path to onboard. L2-open + L3-deny
gives the intended self-service flow:

```
stranger messages the bot
  → L1 ✅ authentic platform request
  → L2 ✅ surface open by default (channel / DM)
  → L3 ❌ identity not in allowed_users
  → echo "⚠️ You're not trusted. Your ID: 123456789. Ask the admin to add you."
  → drop — no agent action
```

This flips **only L3** from today's allow-all to deny-all; L2 stays open. Minimal
breaking surface, maximal safety: nothing acts for an untrusted identity, yet
strangers still get a way to request access.

## 4. Decision

### 4.1 Trust-none default (identity layer)

```
Current:  empty allowed_users → allow_all_users = true  (TRUST ALL)
Proposed: empty allowed_users → allow_all_users = false (TRUST NONE)
```

When a message arrives from an untrusted sender:
1. Log the event (sender ID, platform, timestamp)
2. Reply with an echo message showing the sender their own ID
3. Do NOT dispatch to any agent

**Semantics of `allowed_users`:**
- Missing key = empty list = deny-all (unless `allow_all_users = true`)
- Empty string sender_id = always denied (fail-closed, regardless of `allow_all_users`)
- Startup validation: warn when a platform section has neither `allowed_users` nor
  explicit `allow_all_users = true` — helps operators catch misconfiguration.

### 4.2 Three-layer adapter architecture (Receiver → Trust Gate → Handler)

Trust enforcement happens in a **dedicated ingress layer** — the Trust Gate —
that is structurally between the Receiver and the Handler. This is NOT inside
any adapter. It is an independent layer that every platform flows through.

**Architecture: Receiver → Trust Gate → Handler**

Each adapter is split into two components with the Trust Gate in between:

| Layer | Responsibility | Per-platform? |
|-------|---------------|---------------|
| **Receiver** | Connect, listen, L1 verify, normalize to `InboundEvent` | Yes |
| **Trust Gate** | L2 scope check + L3 identity check (`decide()`) | **No — unified** |
| **Handler** | Platform-specific interaction logic + dispatch | Yes |

**Why this order:**
- Trust Gate is upstream of Handler — Handler never sees untrusted events
- Slash commands (`/reset`, `/cancel`) are in the Handler — they are gated
- No adapter can bypass the gate — it is architecturally mandatory
- New platform = write Receiver + Handler; trust is automatic

**Type-level enforcement (compile-time guarantee):**
The "impossible to bypass" property is enforced via Rust's type system, not just
calling convention. The Trust Gate consumes `InboundEvent` and produces a
**different type** — `GatedEvent` — which is the only type Handler accepts:

```rust
/// Receiver produces this (untrusted).
pub struct InboundEvent { /* ... */ }

/// Trust Gate produces this (trusted). Only constructible by the gate.
pub struct GatedEvent {
    pub(crate) inner: InboundEvent,  // pub(crate) — Handler cannot forge this
}

/// Handler signature — cannot accept InboundEvent directly.
async fn handle(&self, event: GatedEvent) { /* ... */ }
```

A Handler that tries to accept `InboundEvent` directly will not compile.
A Receiver that tries to construct `GatedEvent` will fail (private field).
This makes the bypass impossible at compile time, not just by convention.

**Trust lookup key:** The gate uses the **per-event platform** from
`InboundEvent.platform` (which maps to `ChannelRef.platform`), NOT
`adapter.platform()`. This correctly handles unified mode where a single
`UnifiedGatewayAdapter` (whose `platform()` returns `"unified"`) multiplexes
events from multiple real platforms.

## 5. Architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│                         Platform Sources                                   │
├──────────────┬───────────────┬────────────────┬─────────────────────────┤
│   Discord    │    Slack      │    Gateway     │         Cron            │
│  (WebSocket) │ (Socket Mode) │  (TG/LINE/..) │      (timer)            │
└──────┬───────┴───────┬───────┴───────┬────────┴────────────┬────────────┘
       │               │               │                     │
       ▼               ▼               ▼                     ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                Receivers (per-platform transport)                          │
│                                                                          │
│  ┌──────────┐  ┌──────────┐  ┌──────────────┐  ┌──────────┐            │
│  │ Discord  │  │  Slack   │  │   Gateway    │  │   Cron   │            │
│  │ Receiver │  │ Receiver │  │  Receiver    │  │ Receiver │            │
│  └──────────┘  └──────────┘  └──────────────┘  └──────────┘            │
│                                                                          │
│  Responsibilities:                                                       │
│  • Connect & listen (WebSocket / HTTP webhook / timer)                   │
│  • L1 authentication (verify signature / JWT / token)                    │
│  • Normalize → InboundEvent { platform, sender_id, channel_id, is_dm }  │
│                                                                          │
│  Does NOT:                                                               │
│  • ❌ Check allowed_users                                                │
│  • ❌ Handle slash commands                                              │
│  • ❌ Evaluate @mention / multibot / role logic                          │
└──────────────────────────────────┬───────────────────────────────────────┘
                                   │
                                   │  InboundEvent (unified format)
                                   ▼
┌──────────────────────────────────────────────────────────────────────────┐
│           🔒  TRUST GATE (L2 scope + L3 identity, unified)  🔒            │
│                                                                          │
│  PlatformTrustConfigs::decide(                                           │
│      event.platform,      // per-event platform (not adapter.platform()) │
│      event.channel_id,                                                   │
│      event.is_dm,                                                        │
│      event.sender_id,                                                    │
│  ) → Decision { Allow | DenyScope | DenyIdentity }                       │
│                                                                          │
│  L2 (scope):    surface_allowed(channel_id, is_dm) — default OPEN        │
│  L3 (identity): identity_allowed(sender_id)        — default DENY-ALL    │
│                                                                          │
│  On DenyIdentity: echo sender ID (with rate-limit)                       │
│  On DenyScope:    silent drop                                            │
│  On Allow:        pass event to Handler ↓                                │
│                                                                          │
│  Bot messages:    if is_bot → skip L3 (bot admission stays in Handler)   │
│                                                                          │
│  🔑 Architectural guarantee: Handler never receives untrusted events      │
└──────────────────────────────────┬───────────────────────────────────────┘
                                   │
                                   │  Only Allow events reach here
                                   ▼
┌──────────────────────────────────────────────────────────────────────────┐
│              Handlers (per-platform interaction logic)                     │
│                                                                          │
│  ┌────────────┐  ┌────────────┐  ┌─────────────┐  ┌───────────┐        │
│  │  Discord   │  │   Slack    │  │   Gateway   │  │   Cron    │        │
│  │  Handler   │  │  Handler   │  │   Handler   │  │  Handler  │        │
│  │            │  │            │  │             │  │           │        │
│  │ • @mention │  │ • thread   │  │ • /reset    │  │ • format  │        │
│  │ • role     │  │ • assist   │  │ • /cancel   │  │   prompt  │        │
│  │ • multibot │  │   mode     │  │ • group     │  │           │        │
│  │ • reaction │  │ • emoji    │  │   routing   │  │           │        │
│  │ • channel  │  │            │  │             │  │           │        │
│  └─────┬──────┘  └─────┬──────┘  └──────┬──────┘  └─────┬─────┘        │
│        │               │                │               │              │
└────────┼───────────────┼────────────────┼───────────────┼──────────────┘
         │               │                │               │
         ▼               ▼                ▼               ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                    Dispatcher → dispatch_batch() → ACP Session            │
└──────────────────────────────────────────────────────────────────────────┘
```

### InboundEvent (Receiver output / Trust Gate input)

**Gateway Receiver note:** The Gateway Receiver is a **single receiver** that
connects to the openab-gateway WebSocket and **demultiplexes by platform**. Each
incoming `GatewayEvent` carries a `platform` field (e.g. `"telegram"`, `"line"`,
`"feishu"`); the Receiver uses this to populate `InboundEvent.platform`. It does
NOT spawn per-platform receivers — there is one WS connection, one event loop,
producing `InboundEvent`s tagged with the correct platform. The Trust Gate then
routes the decision to the right platform's `TrustConfig`.

```rust
/// Unified inbound event produced by all Receivers.
/// Contains the minimum fields needed for trust evaluation.
pub struct InboundEvent {
    pub platform: String,           // "discord", "telegram", "line", etc.
    pub sender_id: String,          // platform-specific sender identifier
    pub channel_id: String,         // conversation surface
    pub is_dm: bool,                // DM vs group/channel
    pub is_bot: bool,               // bot-originated message
    pub raw: RawPlatformEvent,      // opaque; Handler interprets this
}
```

### Per-platform TrustConfig

```rust
pub struct TrustConfig {
    // L2 — scope control (NOT security). Defaults OPEN.
    pub allow_all_channels: bool,           // default true
    pub allowed_channels: HashSet<String>,
    pub allow_dm: bool,                      // default true (DM surface open)

    // L3 — identity trust (THE security gate). Defaults DENY-ALL.
    pub allow_all_users: bool,               // explicit opt-in, default false
    pub allowed_users: HashSet<String>,
}

impl TrustConfig {
    /// L2: is this conversation surface in scope? (default-open)
    pub fn surface_allowed(&self, channel_id: &str, is_dm: bool) -> bool {
        if is_dm {
            return self.allow_dm;
        }
        self.allow_all_channels || self.allowed_channels.contains(channel_id)
    }

    /// L3: is this identity trusted? (default-deny)
    pub fn identity_allowed(&self, sender_id: &str) -> bool {
        if sender_id.is_empty() { return false; }  // fail-closed on empty ID
        self.allow_all_users || self.allowed_users.contains(sender_id)
    }

    /// Combined decision: L2 then L3.
    pub fn decide(&self, channel_id: &str, is_dm: bool, sender_id: &str) -> Decision {
        if !self.surface_allowed(channel_id, is_dm) {
            return Decision::DenyScope;
        }
        if !self.identity_allowed(sender_id) {
            return Decision::DenyIdentity;
        }
        Decision::Allow
    }
}

/// Decision outcome.
#[non_exhaustive]
pub enum Decision {
    Allow,
    DenyScope,       // silent drop (L2 — not a security failure)
    DenyIdentity,    // echo sender ID (L3 — request-access UX)
}
```

### PlatformTrustConfigs (registry)

```rust
pub struct PlatformTrustConfigs {
    configs: HashMap<String, TrustConfig>,  // keyed by lowercase platform name
}

impl PlatformTrustConfigs {
    /// Look up by per-event platform (case-insensitive).
    /// Unknown platform → default config (L2 open, L3 deny-all).
    pub fn decide(&self, platform: &str, channel_id: &str, is_dm: bool, sender_id: &str) -> Decision {
        let config = self.configs
            .get(&platform.to_lowercase())
            .unwrap_or(&Self::default_config());
        config.decide(channel_id, is_dm, sender_id)
    }

    fn default_config() -> TrustConfig {
        // L2 open, L3 deny-all — unknown platform = nobody in.
        TrustConfig {
            allow_all_channels: true,
            allowed_channels: HashSet::new(),
            allow_dm: true,
            allow_all_users: false,
            allowed_users: HashSet::new(),
        }
    }
}
```

Note: The default config uses a runtime-constructed `TrustConfig` (not a `static`
with `HashSet::new()` which would not compile). The actual implementation uses
`LazyLock` or returns a fresh default; see `trust.rs` on main for the real code.

### Bot message handling

Bot messages (where `InboundEvent.is_bot == true`) **bypass L3** at the Trust Gate.
Bot admission is NOT part of the identity trust model — it is platform-specific
structural logic (e.g. `trusted_bot_ids`, `allow_bot_messages`) that stays in the
Handler. The Trust Gate only evaluates human sender identity.

**Implementation note:** The `is_bot` bypass is implemented at the **Trust Gate
caller level**, not inside `TrustConfig::decide()`. This keeps `decide()` a pure
L2+L3 function with no bot-awareness:

```rust
// Trust Gate layer (pseudocode)
async fn gate_event(event: InboundEvent, configs: &PlatformTrustConfigs) -> Option<GatedEvent> {
    // Bot bypass — skip L3 entirely; bot admission is Handler's job
    if event.is_bot {
        return Some(GatedEvent { inner: event });
    }

    let decision = configs.decide(&event.platform, &event.channel_id, event.is_dm, &event.sender_id);
    match decision {
        Decision::Allow => Some(GatedEvent { inner: event }),
        Decision::DenyIdentity => { echo_sender_id(&event).await; None }
        Decision::DenyScope => None,  // silent drop
    }
}
```

This means `PlatformTrustConfigs::decide()` does NOT need an `is_bot` parameter —
the bot check happens before `decide()` is called.

### Echo reply on deny

```rust
// In the Trust Gate layer (not in any adapter)
if decision == Decision::DenyIdentity {
    let echo = format!(
        "⚠️ You are not in the trusted list.\nYour ID: {}\nPlease ask the admin to add you to [{}].allowed_users.",
        event.sender_id,
        event.platform,  // per-event platform, not adapter.platform()
    );
    send_echo(&event, &echo).await;
}
```

**Echo safeguards:**
- **Rate-limit:** max 1 echo per sender per platform per 5 minutes (prevents spam/DoS amplification)
- **Bot exclusion:** if `is_bot` → silent deny, no echo (prevents infinite reply loops between bots)
- **DM preferred:** in group/channel contexts, prefer DM reply to avoid leaking sender UID publicly; if DM unavailable, **silent drop** (do NOT fall back to in-channel echo, to avoid UID leakage in shared groups)
- **Best-effort:** echo delivery is not guaranteed (e.g. LINE reply tokens expire); this is acceptable — the echo is a UX convenience, not a security mechanism

**Platform-specific delivery caveats:**
- **LINE:** reply tokens are single-use and short-TTL (~30s). If the echo cannot use the reply token, fall back to push message API (requires separate quota/permission).
- **Other platforms:** no known delivery constraints for the echo use case.

### Sender ID format notes (for `allowed_users` configuration)

`InboundEvent.sender_id` is always a `String`. Each platform's native ID is
converted to its string representation. Operators must configure `allowed_users`
using the **exact format** the platform provides in event payloads:

| Platform | Native type | `allowed_users` format | Example | Gotcha |
|----------|-------------|----------------------|---------|--------|
| Discord | Snowflake (u64) | Numeric string | `"845835116920307722"` | — |
| Slack | String | U-prefix or W-prefix | `"U01ABCDEFGH"` | Enterprise Grid uses `W` prefix; use whichever the event payload provides |
| Telegram | Integer (i64) | Stringified integer | `"123456789"` | ⚠️ Do NOT use `@username` — only numeric ID works |
| LINE | String | U + 32 hex chars | `"U1234567890abcdef0123456789abcdef"` | — |
| Feishu | String | open_id | `"ou_xxxxxxxxxxxxxxxxxxxx"` | ⚠️ `open_id` is **per-app** — same user has different ID in different Feishu apps |
| WeCom | String | UserID | `"zhangsan"` | — |
| Google Chat | String | User resource name | `"users/123456789"` | — |
| MS Teams | String | `activity.from.id` | `"29:1abc..."` | Verify via actual event payload; may differ from AAD Object ID |

## 6. Migration

### Phased rollout (not a hard cutover)

The default flip is phased to avoid silently severing live bots on upgrade:

| Phase | Behavior | When |
|-------|----------|------|
| **Phase 0** | Types + `decide()` defined, no runtime behavior change. Additive only. | Done (on main) |
| **Phase 1** | Wire Trust Gate into ingress pipeline. **Keep current allow-all default.** Log deprecation warning when relying on implicit allow-all. | Next release |
| **Phase 2** | Require explicit `allow_all_users = true` to preserve old behavior. Deployments without it get a **startup error** (not silent denial). | Pre-GA release |
| **Phase 3** | Flip default: empty `allowed_users` + no `allow_all_users` = **deny-all**. | GA release |

### Migration path

```toml
# Before (implicit trust-all — works in Phase 0/1, warns in Phase 1, errors in Phase 2):
[discord]
bot_token = "..."

# After (explicit trust-all to keep old behavior across all phases):
[discord]
bot_token = "..."
allow_all_users = true

# Or (recommended — actually configure trust):
[discord]
bot_token = "..."
allowed_users = ["845835116920307722"]
```

### `[gateway]` vs first-class section precedence

When both a deprecated `[gateway]` section and a matching first-class section
(e.g. `[telegram]`) exist in config, the **first-class section wins**. The
`[gateway]` entry for that platform is ignored and a deprecation warning is
logged at startup. If only `[gateway]` exists for a platform, it remains
functional.

## 7. Implementation Plan

1. **Define `InboundEvent`** — unified event struct that all Receivers produce
2. **Refactor adapters into Receiver + Handler** — starting with Discord and
   Gateway (Telegram). The Receiver produces `InboundEvent`; the Handler consumes
   only events that passed the Trust Gate.
3. **Wire Trust Gate as the ingress layer** between Receiver and Handler:
   - Receives `InboundEvent` from Receiver
   - Calls `PlatformTrustConfigs::decide(event.platform, ...)`
   - Passes allowed events to Handler
   - Echoes + drops denied events
4. **Remove scattered trust checks** — replaced by the unified Trust Gate:
   - `is_denied_user()` in Discord EventHandler (`discord.rs:2892`)
   - `should_skip_event()` user/channel filter in `gateway.rs` (`:832`, `:1160`)
   - Inline user allowlist in Slack (`slack.rs:1224`)
   - Feishu L3 check in the gateway crate (`feishu.rs:425`) — must relocate to
     core, not just delete (contradicts "gateway = L1 only" model)
   - Discord reaction-dispatch gating (`discord.rs:1241`)
   - Note: `trusted_bot_ids`, `allow_bot_messages`, `allowed_role_ids` **stay in
     Handlers** — they are structural/trigger semantics, not identity trust.
5. **Add echo reply with safeguards** — rate-limit, bot exclusion, DM-preferred
6. **Structured logging** — log sender_id + platform on both deny AND allow
   (existing dispatch logs use sender name; add structured sender_id field)
7. **Update `config.toml.example`** and docs; migration guide in release notes

### What stays in Handlers (NOT moved to Trust Gate)

These are platform-specific structural concerns, not trust:
- Thread detection and routing
- @mention gating and multibot detection
- Bot-ownership and `trusted_bot_ids`
- `allowed_role_ids` (Discord role-based trigger control)
- Reaction dispatch gating (triggers, not authorization)
- Slash command routing (`/reset`, `/cancel`) — but note these now run AFTER the
  Trust Gate, so untrusted senders cannot invoke them.

## 8. Rejected Alternatives

### Per-adapter `InboundGate` trait

Each adapter implements `is_trusted_sender()`. Rejected because:
- Trust logic is identical across all platforms (`allowed_users.contains(id)`)
- Forces N identical implementations with no polymorphic benefit
- New adapter forgetting to implement = security hole
- Three-layer architecture makes this impossible to bypass by construction

### Trust check at gateway layer

Gateway adapters filter untrusted senders before forwarding. Rejected because:
- Gateway is transport (L1) — mixing L3 policy violates layer separation
- Trust config lives in core's `config.toml`, not gateway env vars
- Reply capability already wired in core via `ChatAdapter::send_message()`

### Trust gate inside Dispatcher::submit() (downstream of adapter)

Wire gate into `Dispatcher::submit()` or `AdapterRouter::handle_message()`.
Rejected because:
- Gate is downstream of the Handler — Handler still receives untrusted events
- Slash commands (`/reset`, `/cancel`) processed in the Handler would execute
  before the gate is reached
- Does not provide the architectural guarantee that untrusted events are invisible
  to platform-specific logic
- The "by construction" safety property requires the gate to be UPSTREAM of any
  platform-specific code that acts on events

### Treating L2 (channel) as a security layer

Rejected: the platform already enforces channel/group membership, so L2 is
operator scoping, not authorization. Modeling it as security would wrongly imply
DMs are protected by channel rules — they are not (a DM has no membership gate;
only L3 protects it).

### L2 default-closed

Rejected: closing surfaces by default breaks the echo/request-access onboarding
flow (an untrusted sender would be dropped before reaching L3 and never learn how
to request access).
