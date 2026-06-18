# ADR: iMessage Integration via macOS Gateway

- **Status:** Proposed
- **Date:** 2026-06-18
- **Author:** @chaodu-agent
- **Reviewers:** @pahud
- **Tracking issues:** (none yet)

---

## 1. Context & Decision

Enable OAB agents to receive and respond to iMessage conversations, allowing users to interact with their agent team through Apple's native messaging platform. This extends OAB's multi-platform adapter architecture (see [ADR: Multi-Platform Adapters](./multi-platform-adapters.md)) to a platform that lacks an official API.

**Decision:** Provide two integration paths — a self-hosted macOS gateway (using SQLite DB polling + AppleScript, same pattern as `agy-acp`) and a cloud-hosted option via Photon Spectrum SDK. Both implement the existing `ChatAdapter` trait and connect to OAB core via WebSocket through the Custom Gateway.

---

## 2. Motivation

- Users want to interact with OAB agents from iMessage — the default messaging app on iOS/macOS with 1B+ active users
- iMessage offers a more personal, low-friction interaction surface compared to Discord/Slack
- Apple provides **no official iMessage API** — all third-party integrations rely on macOS Messages.app as a bridge
- The existing `agy-acp` component already proves the "poll SQLite DB" pattern works reliably in OAB
- Two deployment paths accommodate different user needs: full self-sovereignty vs. operational simplicity

### Why iMessage Over Existing Channels

iMessage fills a gap that LINE, Telegram, and Slack cannot:

| Advantage | Detail |
|-----------|--------|
| **North America default** | iPhone-to-iPhone messaging uses iMessage automatically — no app install required. Dominant in US/Canada market. |
| **Zero-friction reach** | Only requires a phone number. No "add friend" / "find bot username" step. Ideal for cold outreach: customer support, appointment reminders, order notifications. |
| **High trust signal** | Conversations appear alongside friends/family in the native Messages.app. Users perceive it as personal communication, not "yet another bot." |
| **Apple ecosystem integration** | Siri dictation, Apple Watch, CarPlay, Focus Mode — notifications are not filtered as "app push." |
| **No new app required** | Enterprise scenario: clients/employees already have iPhones; no need to mandate LINE/Telegram/Slack installation. |

**When NOT to use iMessage:**

| Scenario | Better choice |
|----------|--------------|
| Asia market (Taiwan/Japan/Thailand) | LINE |
| Developer/tech communities | Discord / Telegram |
| Cross-platform users (Android + iPhone) | Telegram / WhatsApp |
| Rich UI (buttons, carousels) | LINE / Slack |
| Group bot interactions | Discord / Slack |
| Message editing / threading | Discord / Slack |

**Summary:** iMessage's core value is **North American market + zero-install barrier + high trust perception**. For Asian markets or technical communities, LINE/Telegram remain more practical.

---

## 3. Architecture

### 3.1 Self-Hosted (Mac mini Gateway)

```
                                    ┌─────────────────────────────────────┐
                                    │  K8s / ECS (containerized)          │
                                    │                                     │
┌────────────────┐                  │  ┌───────────────────────────────┐  │
│  iPhone User   │                  │  │  OAB Core                     │  │
│  (iMessage)    │                  │  │  ├── AdapterRouter             │  │
└───────┬────────┘                  │  │  ├── SessionPool               │  │
        │                           │  │  └── ACP agents (法師們)        │  │
        ▼                           │  └──────────────┬────────────────┘  │
═══════════════════                 │                  │ WebSocket          │
║ Apple iMessage  ║                 │  ┌──────────────▼────────────────┐  │
║    Network      ║                 │  │  Custom Gateway               │  │
═══════════════════                 │  │  (webhook receiver)           │  │
        │                           │  └──────────────▲────────────────┘  │
        ▼                           │                  │                   │
┌───────────────────────────┐       └──────────────────┼───────────────────┘
│  Mac mini (bare metal)    │                          │
│                           │       HTTPS POST         │
│  ┌─────────────────────┐  │  ────────────────────────┘
│  │  Messages.app        │  │
│  │  ~/Library/Messages/ │  │
│  │  chat.db (SQLite)    │  │
│  └──────────┬───────────┘  │
│             │ poll every    │
│             │ 100-500ms     │
│  ┌──────────▼───────────┐  │
│  │  imessage-bridge      │  │
│  │                       │  │
│  │  - Poll chat.db       │  │
│  │    (WHERE rowId > N)  │  │
│  │  - AppleScript send   │  │
│  │  - HTTP POST to       │  │
│  │    Custom Gateway     │  │
│  └───────────────────────┘  │
└─────────────────────────────┘
```

### 3.2 Cloud-Hosted (Photon Spectrum)

```
┌────────────────┐        ═══════════════        ┌──────────────────┐
│  iPhone User   │───────║ Apple iMessage ║──────│  Photon Cloud     │
│  (iMessage)    │        ═══════════════        │  (managed Mac群)  │
└────────────────┘                               └────────┬─────────┘
                                                          │ gRPC stream
                                                          ▼
┌─────────────────────────────────────────────────────────────────────┐
│  OAB Deployment (any OS, containerized)                             │
│                                                                     │
│  ┌──────────────────┐     ┌───────────────┐     ┌───────────────┐  │
│  │ Spectrum Sidecar  │────►│ Custom Gateway│────►│ OAB Core      │  │
│  │ (Node.js/Bun)    │     │               │     │ + Agents      │  │
│  │                   │     └───────────────┘     └───────────────┘  │
│  │ - gRPC to Photon  │                                              │
│  │ - HTTP to Gateway │                                              │
│  └──────────────────┘                                               │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 4. How iMessage Bridging Works

Apple does not provide an iMessage API. The bridge relies on two macOS-native mechanisms:

### 4.1 Receiving Messages (Inbound)

Messages.app writes all received messages to a local SQLite database at `~/Library/Messages/chat.db`. The bridge polls this DB for new rows:

```sql
SELECT rowid, text, handle_id, date, is_from_me, cache_roomnames
FROM message
WHERE rowid > ?last_seen_rowid
ORDER BY rowid ASC
```

This is the **same pattern as `agy-acp`**, which polls `conversations/*.db` for new `step_payload` rows. Both use:
- SQLite read-only connection with WAL mode
- Polling interval (100-500ms)
- Monotonically increasing row ID as cursor

### 4.2 Sending Messages (Outbound)

Messages are sent by invoking AppleScript via `osascript`:

```applescript
tell application "Messages"
    set targetService to 1st account whose service type = iMessage
    set targetBuddy to participant "+1234567890" of targetService
    send "Hello from OAB" to targetBuddy
end tell
```

### 4.3 Comparison with agy-acp

| Aspect | agy-acp | imessage-bridge |
|--------|---------|-----------------|
| Monitored program | `agy` (Gemini CLI) | Messages.app |
| Data source | `conversations/*.db` | `~/Library/Messages/chat.db` |
| Poll mechanism | `WHERE idx > last` every 100ms | `WHERE rowid > last` every 100-500ms |
| Data format | protobuf `step_payload` field 20.1 | `attributedBody` blob / plain `text` column |
| Send mechanism | spawn `agy -p "prompt"` | spawn `osascript` (AppleScript) |
| Output | JSON-RPC streaming notifications | HTTP POST to Custom Gateway |

---

## 5. Message Flow

### Inbound (User → Agent)

```
1. User sends iMessage from iPhone
2. Apple iMessage network delivers to Mac mini's Messages.app
3. Messages.app writes row to chat.db
4. imessage-bridge detects new row (poll)
5. Bridge formats as OpenAB inbound event:
   { "platform": "imessage", "sender": "+1234567890",
     "text": "...", "channel_id": "iMessage;-;+1234567890" }
6. Bridge POSTs to Custom Gateway webhook endpoint
7. Custom Gateway routes to OAB core via WebSocket
8. AdapterRouter dispatches to SessionPool → agent
```

### Outbound (Agent → User)

```
1. Agent produces response via ACP session
2. AdapterRouter calls iMessageAdapter.send_message()
3. Adapter sends HTTP request to Mac mini bridge
4. Bridge invokes AppleScript to send via Messages.app
5. Messages.app → Apple network → User's iPhone
```

---

## 6. iMessage Adapter (ChatAdapter impl)

```rust
pub struct IMessageAdapter {
    gateway_url: String,  // Custom Gateway WebSocket URL
    bridge_url: String,   // Mac mini bridge HTTP endpoint (for outbound)
}

#[async_trait]
impl ChatAdapter for IMessageAdapter {
    fn platform(&self) -> &'static str { "imessage" }
    fn message_limit(&self) -> usize { 10000 }  // iMessage has no practical limit

    async fn send_message(&self, channel: &ChannelRef, content: &str) -> Result<MessageRef>;
    async fn edit_message(&self, msg: &MessageRef, content: &str) -> Result<()>;
    // edit_message → not supported by iMessage, returns Ok(()) no-op
    async fn create_thread(&self, ..) -> Result<ChannelRef>;
    // iMessage doesn't have threads — replies go to same conversation
    async fn add_reaction(&self, msg: &MessageRef, emoji: &str) -> Result<()>;
    // Maps to tapback if supported, otherwise no-op
    async fn remove_reaction(&self, ..) -> Result<()>;
}
```

**Platform limitations:**
- No message editing (iMessage supports "edit" natively on iOS 16+ but AppleScript cannot trigger it)
- No threading (conversations are flat)
- Reactions map to tapbacks (❤️, 👍, 👎, 😂, ‼️, ❓) — only 6 options
- No typing indicators via AppleScript (Spectrum Cloud supports this)
- No structured @mention field (see §6.1 below)

### 6.1 @Mention Detection (Group Chat)

iMessage supports @mentions (iOS 14+, displayed as bold blue text), but `chat.db` does **not** expose them as a structured column. The mention data is embedded inside the `attributedBody` blob — a serialized `NSAttributedString` (NSKeyedArchiver / typedstream format).

**To extract mentions, the bridge must:**

1. Read `message.attributedBody` (binary blob)
2. Decode NSKeyedArchiver binary plist
3. Locate ranges where `__kIMMessagePartAttributeName` = 1 (indicates a mention)
4. Extract the mentioned handle ID from `__kIMMentionConfirmedMention`

**Comparison with other platforms:**

| Platform | Mention detection | Complexity |
|----------|-------------------|-----------|
| Discord | `message.mentions` array | Trivial — structured field |
| LINE | `mentionees` in webhook payload | Trivial — structured field |
| Slack | `<@BOT_ID>` in text + `app_mention` event type | Easy — text pattern |
| iMessage | Parse binary `attributedBody` blob | Hard — undocumented binary format |

**Implications for group chat:**
- **1:1 conversations (Phase 1):** No mention detection needed — all messages are directed at the bot
- **Group chat (Phase 5):** Bridge must parse `attributedBody` to know when the bot is mentioned, or fall back to keyword-prefix trigger (e.g. `/ask ...`)
- The `attributedBody` format is undocumented and may change across macOS versions — Rust `plist` crate can decode the binary plist, but the internal schema requires reverse-engineering
- **Outbound:** AppleScript `send` does not support sending @mentions — bot replies are plain text only

---

## 7. Config Design

```toml
[imessage]
mode = "self-hosted"  # "self-hosted" | "spectrum"

# Self-hosted mode
[imessage.bridge]
url = "https://mac-mini.local:8443"   # Bridge HTTP endpoint
api_key = "${IMESSAGE_BRIDGE_API_KEY}" # Shared secret for auth
poll_interval_ms = 200

# Spectrum mode (alternative)
[imessage.spectrum]
project_id = "${PHOTON_PROJECT_ID}"
project_secret = "${PHOTON_PROJECT_SECRET}"
```

---

## 8. Security Considerations

### 8.1 Mac mini (Self-Hosted) Risks

| Risk | Mitigation |
|------|-----------|
| No container isolation on macOS | Dedicated macOS user account with minimal privileges |
| Mac mini compromise → iMessage access | Bridge runs as dumb forwarder; all agent logic stays in K8s |
| Network exposure | Bridge only accepts connections from known OAB gateway IP; mTLS recommended |
| Apple account credential exposure | Use a dedicated Apple ID, not personal |
| AppleScript injection | Sanitize all outbound text; no user input in script template |

### 8.2 Photon Spectrum Risks

| Risk | Mitigation |
|------|-----------|
| Third-party dependency | Photon manages Apple infra; you trust their SLA |
| Shared phone numbers (free tier) | Upgrade to dedicated line ($250/mo) for consistent identity |
| Apple ToS enforcement | Photon assumes this risk; self-hosted is fallback |
| gRPC stream reliability | SDK handles auto-reconnect; SMS/RCS fallback |

### 8.3 General

- Messages contain PII — bridge and OAB must encrypt at rest and in transit
- Rate limiting on outbound to avoid Apple throttling/blocking
- Bridge API key rotation via Kubernetes secrets

---

## 9. Deployment Options

| Deployment | Hardware | Isolation | Cost | Complexity |
|-----------|----------|-----------|------|------------|
| Self-hosted (all-in-one) | Mac mini | None (bare metal) | ~$600 one-time | Low |
| Self-hosted (split) | Mac mini + K8s cluster | Bridge on Mac, OAB in pods | ~$600 + cluster | Medium |
| Photon Spectrum (free) | None | Full container | $0/mo (10 users) | Low |
| Photon Spectrum (dedicated) | None | Full container | $250/mo/line | Low |

**Recommended for most users:** Split deployment — Mac mini runs only the bridge (~200 lines of code), OAB core stays containerized with full pod security.

---

## 10. Implementation Phases

| Phase | Scope | Dependencies |
|-------|-------|-------------|
| **Phase 1** | `imessage-bridge` binary (Rust): poll chat.db, POST to gateway, receive outbound via HTTP | Custom Gateway (#TBD) |
| **Phase 2** | `IMessageAdapter` in OAB core implementing `ChatAdapter` trait | Multi-Platform Adapters (done) |
| **Phase 3** | Spectrum sidecar adapter (Node.js/Bun wrapper) as alternative to self-hosted bridge | Photon account |
| **Phase 4** | Helm chart additions: bridge sidecar, Spectrum sidecar, config templates | Phase 1 or 3 |
| **Phase 5** | Rich features: tapback reactions, group chat support, @mention parsing, attachment handling | Phase 2 |

---

## 11. Apple Compliance & Risks

Apple does not provide an official iMessage API. All known approaches rely on:

1. **macOS Messages.app + SQLite** — reading `chat.db` (requires Full Disk Access)
2. **AppleScript automation** — sending via `osascript` (uses public macOS APIs)
3. **No protocol reverse-engineering** — no private framework usage

**Current landscape (as of 2026-06):**
- BlueBubbles, AirMessage have operated for years without Apple enforcement
- Photon Spectrum launched April 2026, commercially offering managed iMessage lines
- Apple has not issued cease-and-desist to any known project
- Risk: Apple could restrict `chat.db` access or AppleScript Messages automation in a future macOS update

**Mitigation:** The adapter architecture is modular — if Apple blocks the self-hosted path, Spectrum Cloud remains as fallback (they absorb the compliance risk). If both paths are blocked, the adapter can be disabled without affecting other OAB platforms.

---

## 12. Open Questions

| # | Question | Options | Notes |
|---|----------|---------|-------|
| 1 | Bridge language | Rust (consistent with OAB) vs TypeScript (reuse imessage-kit) | Rust preferred for single-binary deployment |
| 2 | Poll interval default | 100ms vs 200ms vs 500ms | Tradeoff: latency vs CPU. agy-acp uses 100ms |
| 3 | Group chat support in Phase 1? | Yes / defer to Phase 5 | Recommend defer — no structured @mention field means bot can't reliably detect when addressed. 1:1 is the sweet spot. |
| 4 | Should bridge run as launchd service? | Yes (auto-restart) / manual | launchd is macOS best practice for daemons |
| 5 | Photon free tier shared numbers acceptable? | Yes for POC / require dedicated | Shared numbers may confuse recipients |

---

_This ADR was drafted based on research into Photon Spectrum (photon-hq/spectrum-ts), imessage-kit, and the existing agy-acp polling pattern in OAB._
