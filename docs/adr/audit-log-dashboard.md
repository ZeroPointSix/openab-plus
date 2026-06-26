# RFC: Audit Log & Usage Dashboard

- **Status:** Draft
- **Date:** 2026-06-26
- **Author:** @obrutjack, @masami-agent
- **Target:** openabdev/openab

---

## 1. Context & Problem

OpenAB is increasingly used in team and enterprise environments. Today there is **no visibility** into:

- Who used which agent, when, and for how long
- How many tokens were consumed per user/channel/agent
- What tool calls were executed (and whether they succeeded or failed)
- Whether any security-relevant actions occurred (permission grants, errors)

For enterprise adoption, audit logging and usage analytics are **table-stakes requirements** — needed for cost control, compliance, security incident investigation, and operational optimization.

Currently, the only observability is `tracing` output to stdout, which is ephemeral and unstructured for audit purposes.

---

## 2. Decision

Introduce a **pluggable audit event system** in `openab-core` with configurable sink adapters, and a separate **`openab-dashboard`** container for visualizing audit data.

### Design Principles

1. **Zero-cost when disabled** — audit system compiles in but does nothing unless `[audit]` is configured
2. **Pluggable sinks** — users choose where data goes (file, S3, OTLP, webhook)
3. **Privacy-first** — no prompt/response content stored by default; only metadata
4. **Separation of concerns** — core emits events; dashboard reads them; they share only a storage layer

---

## 3. Architecture

```
┌─────────────────────────────────────────────────────┐
│                   openab-core                        │
│                                                     │
│  Discord msg → Dispatcher → ACP Session             │
│       │              │             │                 │
│       ▼              ▼             ▼                 │
│   ┌─────────────────────────────────┐               │
│   │       AuditEventEmitter         │               │
│   │  (session_start, prompt,        │               │
│   │   tool_call, session_end,       │               │
│   │   permission, error)            │               │
│   └──────────────┬──────────────────┘               │
└──────────────────┼──────────────────────────────────┘
                   │
        ┌──────────┼──────────┬────────────┐
        ▼          ▼          ▼            ▼
   FileSink    S3Sink    OtlpSink    WebhookSink
   (JSONL)    (batch)   (gRPC/HTTP)  (HTTP POST)
        │          │
        ▼          ▼
   local disk    S3 bucket
        │          │
        └────┬─────┘
             ▼
   ┌──────────────────┐
   │ openab-dashboard │  (optional, separate container)
   │  - Web UI        │
   │  - REST API      │
   │  - reads from    │
   │    file/S3       │
   └──────────────────┘
```

---

## 4. Audit Events

### Event Schema

```rust
#[derive(Serialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub event_type: AuditEventType,
    pub agent: String,           // e.g. "kiro", "claude"
    pub platform: String,       // e.g. "discord", "slack", "teams"
    pub channel_id: String,
    pub user_id: String,         // platform user ID (hashed if configured)
    pub session_id: Option<String>,
    pub metadata: serde_json::Value,
}

pub enum AuditEventType {
    SessionStart,
    SessionEnd,
    Prompt,          // metadata: { token_count, model }
    Response,        // metadata: { token_count, duration_ms }
    ToolCall,        // metadata: { tool_name, status, duration_ms }
    PermissionGrant, // metadata: { permission, auto_approved }
    Error,           // metadata: { error_category, message }
}
```

### What is NOT stored (by default)

- Prompt text content
- Response text content
- Tool call arguments (only tool name)
- File contents accessed by agents

### Opt-in content logging

```toml
[audit]
include_content = true          # opt-in: store prompt/response text
content_encryption_key = "${AUDIT_ENCRYPTION_KEY}"  # required if include_content = true
```

---

## 5. Sink Adapters

### Trait Definition

```rust
#[async_trait]
pub trait AuditSink: Send + Sync + 'static {
    /// Emit a single audit event. Implementations may buffer internally.
    async fn emit(&self, event: &AuditEvent) -> Result<()>;

    /// Flush any buffered events. Called on graceful shutdown.
    async fn flush(&self) -> Result<()>;
}
```

### Built-in Sinks

| Sink | Config Key | Description | Buffering |
|------|-----------|-------------|-----------|
| File (JSONL) | `audit.file` | Append to local file, daily rotation | Line-buffered |
| S3 | `audit.s3` | Batch upload to S3, partitioned by date | Time-based (60s default) |
| OTLP | `audit.otlp` | OpenTelemetry log exporter | SDK-managed |
| Webhook | `audit.webhook` | HTTP POST JSON to user endpoint | Per-event or batched |

### Configuration Example

```toml
[audit]
enabled = true
sinks = ["file", "s3"]

[audit.file]
path = "/var/log/openab/audit.jsonl"
rotation = "daily"
retention_days = 30

[audit.s3]
bucket = "my-company-openab-audit"
prefix = "audit/v1/"
region = "us-west-2"
batch_interval_secs = 60
batch_max_events = 1000

[audit.otlp]
endpoint = "http://otel-collector:4317"
protocol = "grpc"  # or "http"

[audit.webhook]
url = "https://my-company.com/api/openab-audit"
headers = { "Authorization" = "Bearer ${AUDIT_WEBHOOK_TOKEN}" }
batch_size = 10
timeout_secs = 5
```

---

## 6. Dashboard (Phase 2/3)

### Deployment Model

Separate container: `ghcr.io/openabdev/openab-dashboard`

```yaml
# Helm values.yaml
dashboard:
  enabled: true
  image:
    repository: ghcr.io/openabdev/openab-dashboard
    tag: "0.1.0"
  dataSource:
    type: "s3"  # or "file"
    s3:
      bucket: "my-company-openab-audit"
      prefix: "audit/v1/"
      region: "us-west-2"
  ingress:
    enabled: true
    host: openab-dashboard.internal.company.com
```

### Dashboard Features (MVP)

- **Usage overview** — total sessions, tokens, tool calls per day/week/month
- **Per-user breakdown** — who uses the most, which agents
- **Per-agent breakdown** — which agent variant is most popular, cost comparison
- **Tool call analytics** — most used tools, failure rates
- **Error timeline** — recent errors with context
- **Export** — CSV/JSON export for further analysis

### Tech Stack (Suggested)

- Backend: Rust (axum) or lightweight Go service
- Frontend: Static SPA (React/Svelte) served by the same binary
- Storage query: Direct S3 Select / local file scan (no separate DB for MVP)

---

## 7. Privacy & Compliance

| Concern | Mitigation |
|---------|-----------|
| PII in user IDs | Option to hash user IDs before storage |
| Prompt content | Not stored by default; opt-in with encryption |
| Data residency | S3 sink respects configured region |
| Retention | Configurable per-sink retention policy |
| Access control | Dashboard behind ingress auth (OAuth2 proxy / basic auth) |
| GDPR right-to-delete | CLI tool to purge events by user ID |

---

## 8. Implementation Phases

### Phase 1: Audit Event Emitter + File Sink
- Add `AuditEvent` struct and `AuditSink` trait to `openab-core`
- Implement `FileSink` (JSONL with rotation)
- Instrument key paths: session start/end, prompt/response token counts, tool calls
- Config: `[audit]` section in `config.toml`
- **Scope:** ~500-800 lines of Rust, minimal deps (only `uuid`, which is likely already in tree)

### Phase 2: S3 + Webhook Sinks
- Implement `S3Sink` with batch buffering
- Implement `WebhookSink` for custom integrations
- Add graceful shutdown flush
- **Scope:** ~400 lines, adds `aws-sdk-s3` as optional dep

### Phase 3: OTLP Sink + Dashboard MVP
- Implement `OtlpSink` using `opentelemetry-otlp`
- Build `openab-dashboard` container (separate repo or workspace member)
- Basic Web UI with usage charts and event search
- Helm chart integration (`dashboard.enabled`)
- **Scope:** New crate/binary, ~2000-3000 lines

---

## 9. Alternatives Considered

### A. Use tracing subscribers only
- Pro: Zero new code, just add a JSON file subscriber
- Con: tracing events are debug-oriented, not audit-oriented; no structured schema; no buffering/batching for remote sinks; mixing debug noise with audit events

### B. External sidecar (Fluentd/Vector)
- Pro: Proven log shipping tools
- Con: Requires stdout parsing (fragile), adds deployment complexity, no typed audit schema, users must configure their own pipeline

### C. Embed dashboard in openab binary
- Pro: Single binary deployment
- Con: Increases attack surface, couples concerns, harder to scale independently
- **Decision:** Offer as future Phase 4 opt-in via feature flag if demand exists

---

## 10. Open Questions

1. **Should audit events include thread/conversation context?** (e.g., thread_id to correlate a multi-turn conversation)
2. **Cost attribution model** — should we estimate dollar cost per session based on model pricing? Or leave that to the dashboard layer?
3. **Real-time streaming** — should dashboard support live tail, or is batch query sufficient for MVP?
4. **Multi-tenant isolation** — if one OpenAB instance serves multiple teams, how to partition audit data?

---

## 11. References

- OpenTelemetry Logs: https://opentelemetry.io/docs/specs/otel/logs/
- AWS S3 Select: https://docs.aws.amazon.com/AmazonS3/latest/userguide/selecting-content-from-objects.html
- JSONL format: https://jsonlines.org/
- GDPR Article 17 (Right to erasure): https://gdpr-info.eu/art-17-gdpr/
