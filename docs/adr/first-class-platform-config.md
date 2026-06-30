# ADR: First-Class Per-Platform Configuration & Trust-None Default

- **Status:** Proposed
- **Date:** 2026-06-30
- **Author:** @chaodu-agent
- **Reviewers:** @pahud
- **Tracking issues:** #1262

---

## 1. Context & Decision

Promote all gateway-connected platforms (Telegram, LINE, Feishu, WeCom, Google Chat, MS Teams) to **first-class citizens** in `config.toml`, each with their own top-level section — identical in structure to the existing `[discord]` and `[slack]` sections.

Additionally, flip the default trust model from **allow-all** to **trust-none**: when `allowed_users` is empty and `allow_all_users` is not explicitly set to `true`, deny all incoming messages.

## 2. Motivation

### Problem 1: Gateway platforms are second-class

Currently, all gateway-connected platforms share a single `[gateway]` config section:

```toml
# ❌ Current: one catch-all for ALL gateway platforms
[gateway]
url = "ws://openab-gateway:8080/ws"
platform = "telegram"              # only identifies which gateway to connect to
allowed_users = ["123456789"]      # shared list for ALL platforms behind this gateway
```

This is fundamentally broken:
- **ID format mixing** — Telegram UIDs (`123456789`) and LINE User IDs (`U1234abc...`) in the same list
- **No per-platform trust** — trusting a Telegram user implicitly trusts that same string on LINE
- **Asymmetry** — Discord and Slack get rich per-platform config; everything else is a second-class `[gateway]` blob
- **Multi-gateway deployments** — running Telegram + LINE requires multiple `[gateway]` sections with unclear semantics

### Problem 2: Trust-all default is insecure

All adapters auto-detect: empty `allowed_users` → `allow_all_users = true`. This means a fresh deployment with no user configuration trusts **everyone** by default.

## 3. Decision

### 3.1 Per-platform top-level config sections

Every platform gets its own section with platform-specific settings + unified trust fields:

```toml
[discord]
bot_token = "${DISCORD_BOT_TOKEN}"
allowed_users = ["845835116920307722"]
# allow_all_users = true                  # opt-in to trust-all

[slack]
bot_token = "${SLACK_BOT_TOKEN}"
app_token = "${SLACK_APP_TOKEN}"
allowed_users = ["U01ABCDEFGH"]

[telegram]
bot_token = "${TELEGRAM_BOT_TOKEN}"
secret_token = "${TELEGRAM_SECRET_TOKEN}"
allowed_users = ["123456789"]

[line]
channel_secret = "${LINE_CHANNEL_SECRET}"
channel_access_token = "${LINE_CHANNEL_ACCESS_TOKEN}"
allowed_users = ["U1234567890abcdef0123456789abcdef"]

[feishu]
app_id = "${FEISHU_APP_ID}"
app_secret = "${FEISHU_APP_SECRET}"
allowed_users = ["ou_xxxxxxxxxxxxxxxxxxxx"]
allowed_groups = ["oc_xxxxx"]

[wecom]
corp_id = "${WECOM_CORP_ID}"
agent_id = "${WECOM_AGENT_ID}"
allowed_users = ["zhangsan"]

[googlechat]
service_account = "${GOOGLE_CHAT_SA_JSON}"
allowed_users = ["users/123456789"]

[teams]
app_id = "${TEAMS_APP_ID}"
allowed_tenants = ["tenant-uuid"]
allowed_users = ["29:1abc..."]
```

### 3.2 Trust-none default

```
Current:  empty allowed_users → allow_all_users = true  (TRUST ALL)
Proposed: empty allowed_users → allow_all_users = false (TRUST NONE)
```

When a message arrives from an untrusted sender, the system:
1. Logs the event (sender ID, platform, timestamp)
2. Replies with an echo message showing the sender their own ID
3. Does NOT dispatch to any agent

### 3.3 Trust check at router level (single gate)

Trust enforcement happens in **one place only**: `AdapterRouter::handle_message()`. The gateway remains a pure transport layer.

```
Gateway (transport):  webhook → verify authenticity → normalize → forward
Core (policy):        AdapterRouter → TrustConfig::is_allowed(platform, sender_id) → echo or dispatch
```

---

## 4. Architecture

```
┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│   Telegram   │  │     LINE     │  │    Feishu    │  │  WeCom / GC  │
│   Webhook    │  │   Webhook    │  │  WebSocket   │  │   Webhook    │
└──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘
       │                 │                 │                 │
       ▼                 ▼                 ▼                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│                  openab-gateway (transport only)                      │
│                                                                     │
│  ✅ Verify webhook signature / secret token / IP                    │
│  ✅ Normalize → GatewayEvent                                        │
│  ✅ Forward ALL events                                               │
│  ❌ No trust check, no user filtering                               │
└────────────────────────────┬────────────────────────────────────────┘
                             │ WebSocket
                             ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         openab-core                                   │
│                                                                     │
│  ┌───────────┐ ┌───────────┐ ┌─────────────────────────────┐       │
│  │  Discord  │ │   Slack   │ │  GatewayAdapter             │       │
│  │  Handler  │ │  Handler  │ │  (TG/LINE/Feishu/WeCom/GC)  │       │
│  └─────┬─────┘ └─────┬─────┘ └──────────────┬──────────────┘       │
│        │              │                      │                      │
│        └──────────────┼──────────────────────┘                      │
│                       ▼                                             │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ 🔒 AdapterRouter::handle_message()                            │  │
│  │                                                               │  │
│  │   trust = platform_trust_configs.get(adapter.platform())      │  │
│  │   if !trust.is_allowed(sender_id):                            │  │
│  │       log + echo sender ID + RETURN                           │  │
│  │   else:                                                       │  │
│  │       dispatch to ACP ✅                                       │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                       │                                             │
│                       ▼                                             │
│              ┌─────────────────┐                                    │
│              │  ACP Session    │                                    │
│              └─────────────────┘                                    │
└─────────────────────────────────────────────────────────────────────┘
```

### Per-platform TrustConfig

```rust
pub struct TrustConfig {
    pub allow_all_users: bool,       // explicit opt-in, defaults to false
    pub allowed_users: HashSet<String>,
}

impl TrustConfig {
    pub fn is_allowed(&self, sender_id: &str) -> bool {
        self.allow_all_users || self.allowed_users.contains(sender_id)
    }
}

/// Router holds one TrustConfig per platform
pub struct PlatformTrustConfigs {
    configs: HashMap<String, TrustConfig>,  // keyed by platform name
}

impl PlatformTrustConfigs {
    pub fn get(&self, platform: &str) -> &TrustConfig {
        self.configs.get(platform).unwrap_or(&DEFAULT_DENY)
    }
}

static DEFAULT_DENY: TrustConfig = TrustConfig {
    allow_all_users: false,
    allowed_users: HashSet::new(),  // empty = deny all
};
```

### Echo reply on deny

```rust
// In AdapterRouter::handle_message()
let echo = format!(
    "⚠️ You are not in the trusted list.\nYour ID: {}\nPlease ask the admin to add you to [{}].allowed_users.",
    msg.sender_id,
    adapter.platform()
);
let _ = adapter.send_message(&msg.channel, &echo).await;
```

---

## 5. Migration

### Breaking change

Existing deployments with no `allowed_users` configured will stop accepting messages after this change.

### Migration path

Add `allow_all_users = true` to maintain old behavior:

```toml
# Before (implicit trust-all):
[discord]
bot_token = "..."

# After (explicit trust-all):
[discord]
bot_token = "..."
allow_all_users = true
```

### `[gateway]` deprecation

The `[gateway]` section remains functional for backward compatibility but is deprecated. Users should migrate to per-platform sections:

```toml
# ❌ Deprecated
[gateway]
platform = "telegram"
allowed_users = ["123"]

# ✅ Migrate to
[telegram]
allowed_users = ["123"]
```

---

## 6. Sender ID Formats

| Platform | Config section | ID format | Example |
|----------|---------------|-----------|---------|
| Discord | `[discord]` | Snowflake UID | `845835116920307722` |
| Slack | `[slack]` | Workspace User ID | `U01ABCDEFGH` |
| Telegram | `[telegram]` | Numeric UID | `123456789` |
| LINE | `[line]` | User ID string | `U1234567890abcdef0123456789abcdef` |
| Feishu | `[feishu]` | Open ID | `ou_xxxxxxxxxxxxxxxxxxxx` |
| WeCom | `[wecom]` | UserID | `zhangsan` |
| Google Chat | `[googlechat]` | User resource name | `users/123456789` |
| MS Teams | `[teams]` | AAD Object ID | `29:1abc...` |

---

## 7. Implementation Plan

1. **Define `TrustConfig` struct** and `PlatformTrustConfigs` in `openab-core`
2. **Add per-platform config parsing** — each `[platform]` section reads `allowed_users` and `allow_all_users`
3. **Wire trust gate into `AdapterRouter::handle_message()`** — single check point
4. **Remove scattered trust checks** from:
   - `is_denied_user()` in Discord EventHandler
   - `should_skip_event()` user filter in `gateway.rs`
   - `allowed_users` check in Feishu gateway adapter
5. **Add echo reply** on deny using `ChatAdapter::send_message()`
6. **Deprecation warning** for `[gateway].allowed_users` — log warning if old config detected
7. **Update `config.toml.example`** and docs
8. **Migration guide** in release notes

---

## 8. Rejected Alternatives

### Per-adapter `InboundGate` trait

Each adapter implements `is_trusted_sender()`. Rejected because:
- Trust logic is identical across all platforms (`allowed_users.contains(id)`)
- Forces N identical implementations with no polymorphic benefit
- New adapter forgetting to implement = security hole
- Router-level gate is impossible to bypass by construction

### Trust check at gateway layer

Gateway adapters filter untrusted senders before forwarding. Rejected because:
- Gateway is transport — mixing business logic violates separation of concerns
- Trust config lives in core's `config.toml`, not gateway env vars
- Would split config into two places (env vars + toml)
- Reply capability is already wired in core via `ChatAdapter::send_message()`

### Keep `[gateway]` with per-platform sub-sections

```toml
[gateway.telegram]
allowed_users = [...]
[gateway.line]
allowed_users = [...]
```

Rejected because it still treats gateway platforms as subordinate. A `[telegram]` section is more intuitive and symmetric with `[discord]` / `[slack]`.
