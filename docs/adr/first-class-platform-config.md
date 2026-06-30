# ADR: First-Class Per-Platform Configuration

- **Status:** Proposed
- **Date:** 2026-06-30
- **Author:** @chaodu-agent
- **Reviewers:** @pahud
- **Tracking issues:** #1262
- **Related:** [Identity Trust-None Default & Trust Pyramid](identity-trust-none.md) — builds on the per-platform sections defined here to hold each platform's `allowed_users`.

---

## 1. Context & Decision

Promote all gateway-connected platforms (Telegram, LINE, Feishu, WeCom, Google Chat, MS Teams) to **first-class citizens** in `config.toml`, each with their own top-level section — identical in structure to the existing `[discord]` and `[slack]` sections — and deprecate the single `[gateway]` catch-all.

## 2. Motivation: gateway platforms are second-class

Currently, all gateway-connected platforms share a single `[gateway]` config section:

```toml
# ❌ Current: one catch-all for ALL gateway platforms
[gateway]
url = "ws://openab-gateway:8080/ws"
platform = "telegram"
allowed_users = ["123456789"]  # shared list — what platform is this ID for?
```

This is fundamentally broken:
- **ID format mixing** — Telegram UIDs (`123456789`) and LINE User IDs (`U1234abc...`) in the same list
- **No per-platform scoping** — a value configured for a Telegram user implicitly applies to that same string on LINE
- **Asymmetry** — Discord and Slack get rich per-platform config; everything else is second-class
- **Multi-gateway deployments** — running Telegram + LINE simultaneously has unclear semantics

## 3. Decision

### 3.1 Per-platform top-level config sections

Every platform gets its own section with platform-specific settings + unified fields (`allowed_users`, `allow_all_users`, `allowed_channels`, …). Per-platform trust semantics are specified in the [Identity Trust-None ADR](identity-trust-none.md).

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
token = "${WECOM_TOKEN}"
allowed_users = ["zhangsan"]

[googlechat]
service_account = "${GOOGLE_CHAT_SA_JSON}"
allowed_users = ["users/123456789"]

[teams]
app_id = "${TEAMS_APP_ID}"
app_secret = "${TEAMS_APP_SECRET}"
allowed_tenants = ["tenant-uuid"]
allowed_users = ["29:1abc..."]
```

### 3.2 `[gateway]` deprecation

The `[gateway]` section remains functional for backward compatibility but is deprecated:

```toml
# ❌ Deprecated
[gateway]
platform = "telegram"
allowed_users = ["123"]

# ✅ Migrate to
[telegram]
allowed_users = ["123"]
```

## 4. Sender ID Formats

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

## 5. Migration

```toml
# Before — gateway catch-all:
[gateway]
platform = "telegram"
allowed_users = ["123456789"]

# After — first-class section:
[telegram]
bot_token = "${TELEGRAM_BOT_TOKEN}"
secret_token = "${TELEGRAM_SECRET_TOKEN}"
allowed_users = ["123456789"]
```

The `[gateway]` section continues to work (with a deprecation warning) for one
release cycle to give deployments time to migrate.

## 6. Implementation Plan

1. **Add per-platform config structs** — `[telegram]`, `[line]`, `[feishu]`, `[wecom]`, `[googlechat]`, `[teams]` parsed as top-level sections
2. **Map gateway events to the owning platform's config** by `platform` name
3. **Deprecation warning** when `[gateway]` is present
4. **Update `config.toml.example`** and per-platform docs
5. **Migration guide** in release notes

## 7. Rejected Alternatives

### Keep `[gateway]` with per-platform sub-sections

```toml
[gateway.telegram]
allowed_users = [...]
```

Rejected because it still treats gateway platforms as subordinate. A `[telegram]`
section is more intuitive and symmetric with `[discord]` / `[slack]`.
