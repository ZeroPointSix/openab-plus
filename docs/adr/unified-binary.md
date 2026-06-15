# ADR: Unified Single-Binary Architecture

- **Status:** Proposed
- **Date:** 2026-06-15
- **Author:** @pahud
- **Supersedes:** Deployment model from [ADR: Custom Gateway](./custom-gateway.md)

---

## 1. Context & Problem

Today, supporting webhook-based platforms (Telegram, LINE, Feishu, Google Chat, WeCom, Teams) requires running **two processes** — `openab` core and `openab-gateway` — wired together via WebSocket, often in the same pod with a shared volume for colocate-mode media passing.

This creates operational friction:

- **Two containers** in a single pod (or two separate services)
- **Shared volume** required for media colocate mode
- **WebSocket wiring** between core and gateway (auth token, reconnect logic)
- **Version matrix** — gateway releases independently, version mismatches cause subtle bugs
- **Double serialization** — every message is serialized to JSON, sent over WS, then deserialized

For most users who just want "Discord + Telegram in one bot", the two-process model is unnecessary complexity.

---

## 2. Decision

Restructure the project as a **Cargo workspace** with the final binary shipping **all adapters compiled in**, activated at runtime via config. The standalone gateway remains available for advanced deployments.

### Workspace Layout

```
openab/
├── Cargo.toml              (workspace root)
├── crates/
│   ├── openab-core/        (ChatAdapter trait, ACP, Dispatcher, SessionPool,
│   │                        Discord adapter, Slack adapter)
│   └── openab-gateway/     (platform adapters: Telegram, LINE, Feishu,
│                             Google Chat, WeCom, Teams — impl ChatAdapter)
├── src/                    (final binary — thin main.rs wiring both crates)
└── gateway/                (standalone gateway binary — kept for backward compat)
```

### Feature Flags (on the final binary crate)

```toml
[features]
default = ["discord", "slack", "telegram", "line", "feishu", "googlechat", "wecom", "teams"]

discord   = ["openab-core/discord"]
slack     = ["openab-core/slack"]
telegram  = ["dep:openab-gateway", "openab-gateway/telegram"]
line      = ["dep:openab-gateway", "openab-gateway/line"]
feishu    = ["dep:openab-gateway", "openab-gateway/feishu"]
googlechat = ["dep:openab-gateway", "openab-gateway/googlechat"]
wecom     = ["dep:openab-gateway", "openab-gateway/wecom"]
teams     = ["dep:openab-gateway", "openab-gateway/teams"]
```

### Runtime Activation

Adapters start **only if their config section is present and has required fields** (e.g., `bot_token`). Compiled-in but unconfigured adapters have zero runtime overhead.

```toml
# Only Discord and Telegram start — others dormant
[discord]
bot_token = "${DISCORD_BOT_TOKEN}"
allowed_channels = ["123456789"]

[telegram]
bot_token = "${TELEGRAM_BOT_TOKEN}"
```

---

## 3. Architecture — Before & After

### Before (two-process model)

```
┌─────────────────────────────┐     ┌───────────────────────────────────┐
│  openab core                │     │  openab-gateway (sidecar)         │
│                             │     │                                   │
│  Discord ──┐                │     │  Telegram ──┐                     │
│  Slack ────┤► Dispatcher    │◄─WS─┤  LINE ──────┤► axum → GatewayEvent│
│            │                │     │  Feishu ────┘                     │
│  GatewayAdapter (WS client) │     │                                   │
└─────────────────────────────┘     └───────────────────────────────────┘
        shared volume for media colocate
```

### After (single binary)

```
┌────────────────────────────────────────────────────────────────┐
│  openab (single binary)                                        │
│                                                                │
│  Discord ────┐                                                 │
│  Slack ──────┤                                                 │
│  Telegram ───┤► Dispatcher → SessionPool → ACP (child process) │
│  LINE ───────┤                                                 │
│  Feishu ─────┘                                                 │
│                                                                │
│  axum HTTP (:9090) — only starts if webhook adapters active    │
└────────────────────────────────────────────────────────────────┘
```

---

## 4. Message Flow Change

```
BEFORE:
  Platform → HTTP → gateway/telegram.rs → serialize GatewayEvent
    → WebSocket → core/gateway.rs → deserialize → Dispatcher.submit()

AFTER:
  Platform → HTTP → src/telegram.rs → Dispatcher.submit() (direct call)
```

Reply path is similarly direct — the adapter calls the platform API in its `ChatAdapter` impl without WS round-trip.

---

## 5. Published Artifacts

| Image | Contents | Use case |
|-------|----------|----------|
| `openab:latest` | All adapters compiled in | Default — one image for everyone |
| `openab:slim` | Discord + Slack only | Minimal deploy (no axum/crypto deps) |
| `openab-gateway:latest` | Standalone gateway (unchanged) | Advanced: geo-distributed gateway, legacy compat |

Custom builds via feature flags:
```bash
cargo build --no-default-features --features telegram  # Telegram-only ~10MB
```

---

## 6. Migration Path

| Phase | Description |
|-------|-------------|
| **Phase 1** | Restructure into workspace. Move adapter code behind feature flags. Ship `openab:latest` with all adapters. Keep standalone gateway as-is. |
| **Phase 2** | Users migrate from two-container to single-container. Helm chart defaults to single binary; gateway sidecar becomes opt-in. |
| **Phase 3** | Deprecate standalone gateway after 2 minor releases. Remove `gateway.url` config field (or keep as hidden legacy). |

### Backward Compatibility

- Existing `[gateway]` config section + standalone gateway continues to work throughout all phases
- The `GatewayAdapter` (WebSocket client in core) remains available for users who need remote gateway
- No breaking change to config schema — new `[telegram]`, `[line]` sections are additive

---

## 7. Trade-offs

### Advantages

- **One container, one config, one release** — dramatically simpler deployment
- **Lower latency** — no WS serialization hop
- **One log stream** — easier debugging
- **No shared volume** — media passed in-process
- **Smaller attack surface** — no exposed WS port between containers

### Disadvantages

- **Larger default binary** — ~25MB vs ~12MB (Discord-only). Mitigated by `slim` image.
- **Coupled release cadence** — platform adapter fix requires full release. Mitigated by workspace allowing independent crate versioning.
- **More deps in tree** — `axum`, `jsonwebtoken`, `prost`, `quick-xml`, `aes/cbc` pulled in even if unused at runtime. Mitigated by feature flags for custom builds.
- **Build time** — full build is longer. Mitigated by workspace incremental compilation (only changed crate recompiles).

---

## 8. Core Changes Required

| Area | Change | Scope |
|------|--------|-------|
| `main.rs` | Start axum server + register adapter routes | ~50 lines |
| `config.rs` | Add `TelegramConfig`, `LineConfig`, etc. | ~100 lines (additive) |
| `Cargo.toml` | Workspace restructure + feature flags | Medium |
| Adapter code | Move from `gateway/src/adapters/` → `crates/openab-gateway/src/` | Mechanical move |
| Per-adapter glue | Replace WS broadcast with `Dispatcher.submit()` | ~10 lines each |
| Existing modules | **Zero changes** — ACP, pool, dispatcher, discord, slack untouched | None |

---

## 9. Rejected Alternatives

### A. Compile-time only (no fat binary)

Users must `docker build` themselves with desired features. Poor UX — rejected.

### B. Merge all code into one crate

Couples platform-specific complexity (Feishu AES-CBC, WeCom XML) with clean core abstractions. Rejected in favor of workspace separation.

### C. Keep current architecture, improve docs

The operational complexity is inherent to the two-process model. Better docs don't eliminate the shared volume requirement or WS failure modes. Rejected.
