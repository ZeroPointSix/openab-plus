# ADR: OpenAB Loop — Closed-Loop Agent Automation

- **Status:** Proposed
- **Date:** 2026-06-09
- **Author:** Pahud Hsieh / OpenAB Team

## Context

For the past two years, multi-agent systems have been driven one prompt at a time — a human issues a task, an agent executes, the human reviews, and manually triggers the next step. This linear model creates bottlenecks at every handoff point.

The industry is moving toward **agent looping**: a pattern where agents autonomously cycle through discovery → planning → execution → verification until a goal is met, without requiring human intervention at each step.

OpenAB already has the primitives for a loop:
- Agent identities and role boundaries
- Discord-based inter-agent communication
- GitHub webhook integration
- A structured PR review spec with severity levels

What we lack is the **automatic iteration mechanism** — the wiring that lets agents hand off to each other and repeat cycles without human triggering.

## Decision

We will implement a **closed-loop PR review flow** as the first OpenAB Loop, using the existing communication infrastructure (Discord mentions + GitHub webhooks) without modifying the OAB core.

### Loop Design

```
PR opened → webhook → Reviewer Agent → verdict
                                          │
                          ┌───────────────┴───────────────┐
                          │                               │
                     LGTM ✅                    CHANGES REQUESTED ⚠️
                          │                               │
                     notify human                  dispatch to Coder
                     (await merge)                 (Discord mention)
                                                          │
                                                   Coder fixes → push
                                                          │
                                                   GitHub webhook
                                                   (synchronize)
                                                          │
                                                   Reviewer re-review
                                                          │
                                                   (loop back to verdict)
```

### Trigger Chain

| Step | Trigger | Actor | Mechanism |
|------|---------|-------|-----------|
| 1 | PR opened / synchronize | Loop Controller | GitHub webhook (existing) |
| 2 | Review complete, verdict = CHANGES_REQUESTED | Reviewer Agent | Discord mention to Coder (new) |
| 3 | Fix complete | Coder Agent | `git push` (existing) |
| 4 | New commits on PR | GitHub | webhook `synchronize` event (existing) |

Only **Step 2 is new**. All other steps use existing infrastructure.

### Dispatch Format

When Reviewer posts `CHANGES REQUESTED`, it sends a structured Discord message mentioning the Coder agent:

```json
{
  "action": "fix",
  "pr": 42,
  "repo": "openabdev/openab",
  "branch": "feat/example",
  "iteration": 1,
  "max_iterations": 3,
  "findings": [
    {
      "id": 1,
      "severity": "🔴",
      "file": "src/auth.rs",
      "line": 23,
      "issue": "missing input validation",
      "suggestion": "add bounds check"
    }
  ]
}
```

### Control Plane Contract

Every dispatch and verdict message **must** include these fields to ensure idempotency, replay protection, and SHA-bound correctness:

```json
{
  "dispatch_id": "uuid-v4",
  "pr": 42,
  "head_sha": "abc1234",
  "iteration": 1,
  "in_reply_to": "<dispatch_id of the message being responded to>",
  "timestamp": "2026-06-09T03:35:00Z"
}
```

**Invariants:**

1. **SHA binding** — A verdict is only valid for the `head_sha` it was produced against. If the PR head has advanced, the verdict is stale and must be discarded.
2. **Idempotent handling** — The controller deduplicates on `(pr, head_sha, dispatch_id)`. Receiving the same dispatch twice is a no-op.
3. **Replay protection** — Events with a `head_sha` older than the current PR head are dropped. The controller always checks `GET /repos/{owner}/{repo}/pulls/{pr}` to confirm head before acting.
4. **Ordering** — Events are processed in `timestamp` order per PR. Out-of-order arrivals are reordered in the state machine before transition.

**Completion criteria for a loop step:**

A step is complete only when **both** conditions are met for the **same** `head_sha`:
1. Reviewer verdict received (LGTM or CHANGES_REQUESTED)
2. All required CI checks pass (GitHub Checks API, not legacy commit status)

```
GET /repos/{owner}/{repo}/commits/{head_sha}/check-runs
→ filter: required checks only
→ all must have conclusion: "success"
```

### Coder Agent Behavior on Dispatch

1. Verify sender is in the allowed dispatcher list
2. Validate `dispatch_id` and `head_sha` match current PR state
3. Check `iteration < max_iterations`, otherwise escalate to human
4. Check findings against safety policy (see below), escalate if restricted
5. Checkout branch at `head_sha`, apply fixes for 🔴 (must fix) then 🟡 (should fix)
6. Run tests — if fail, escalate to human
7. Push — GitHub webhook automatically triggers re-review

### Safety Boundaries (Hard Stop Conditions)

- `iteration >= max_iterations` (default: 3)
- Safety policy violation (see machine-enforceable rules below)
- Same finding unresolved for 2 consecutive iterations (matched by finding fingerprint)
- Tests fail after fix
- Required CI checks fail after push
- Human explicitly intervenes ("stop" / "I'll handle it")

Human override has the highest priority at all times.

### Finding Fingerprint (Cross-Iteration Dedup)

To detect "same finding unresolved across iterations," each finding carries a stable fingerprint:

```
fingerprint = sha256(file + ":" + line_range + ":" + normalized_issue_text)[:8]
```

The controller tracks fingerprints across iterations in `history`. If a fingerprint appears in iteration N and N+1 without resolution, the hard stop triggers. Per-dispatch sequential `id` fields are NOT stable across iterations — use fingerprint instead.

### Machine-Enforceable Safety Policy

Safety boundaries must be **structural and automatable**, not reliant on natural language interpretation.

**Finding classification (required in dispatch):**

```json
{
  "id": 1,
  "severity": "🔴",
  "category": "logic | performance | style | security | auth | infra | config",
  "risk_level": "low | medium | high | critical",
  "file": "src/auth.rs",
  "line": 23,
  "issue": "missing input validation",
  "suggestion": "add bounds check"
}
```

**Path denylist — Coder agent must NOT modify these without human approval:**

```toml
[safety.path_denylist]
patterns = [
  ".github/workflows/**",
  "infra/**",
  "helm/**",
  "terraform/**",
  "**/auth/**",
  "**/secrets/**",
  "**/*.pem",
  "**/*.key",
  ".env*",
]
```

**Category escalation rules:**

| category | risk_level | Action |
|----------|-----------|--------|
| security, auth, infra | any | Always escalate to human |
| any | critical | Always escalate to human |
| logic, performance, style, config | low, medium | Coder may fix autonomously |
| logic, performance, style, config | high | Escalate to human |

**Enforcement point:** The controller validates these rules **before** dispatching to Coder. If any finding triggers escalation, the entire dispatch is blocked.

## Loop Controller Design

The Loop Controller is the central coordinator. It does no actual work (no reviewing, no coding) — it only monitors state, enforces timeouts, and dispatches agents.

### Architecture

```
┌──────────────────────────────────────────────────────────┐
│                    LOOP CONTROLLER                         │
│                                                          │
│  Input:  GitHub webhooks, Discord messages                │
│  Output: Dispatch commands, Escalation alerts            │
│  State:  Persisted in state store (file / Redis / DB)    │
│                                                          │
│  ┌────────────┐  ┌────────────┐  ┌────────────────────┐ │
│  │ Event      │  │ State      │  │ Timer / Watchdog   │ │
│  │ Listener   │→ │ Machine    │→ │                    │ │
│  └────────────┘  └────────────┘  └────────────────────┘ │
└──────────────────────────────────────────────────────────┘
         │                                    │
         ▼                                    ▼
┌─────────────────┐                 ┌──────────────────┐
│ Agents          │                 │ Human (escalation)│
│ (Reviewer/Coder)│                 │                  │
└─────────────────┘                 └──────────────────┘
```

### State Machine (one instance per PR)

```
┌─────────────────────────────────────────────────────┐
│  Loop State for PR #N                               │
│                                                     │
│  state: IDLE | REVIEWING | FIXING | APPROVED | ESCALATED | DONE │
│  iteration: <current>                               │
│  max_iterations: 3                                  │
│  started_at: <timestamp>                            │
│  current_step_started: <timestamp>                  │
│  timeout_per_step: 10m (review) / 15m (fix)         │
│  token_used: <accumulated>                          │
│  token_budget: 50000                                │
│  retries: 0 | 1                                     │
│  history: [{step, result, timestamp}, ...]          │
└─────────────────────────────────────────────────────┘
```

### Event Routing

| Event Source | Event | Controller Action |
|-------------|-------|-------------------|
| GitHub webhook | `pull_request.opened` | Create loop state, dispatch reviewer |
| GitHub webhook | `pull_request.synchronize` | Update state, dispatch reviewer (re-review) |
| Discord message | Reviewer posted verdict | Parse verdict, update state, decide next step |
| Discord message | Coder says pushed | Update state, await webhook confirmation |
| Discord message | Human says "stop" | Terminate loop |
| Internal timer | Step timeout reached | Retry or escalate |

### Decision Logic

```python
def on_event(event, loop_state):
    match loop_state.state:

        case "IDLE":
            if event == PR_OPENED:
                loop_state.state = "REVIEWING"
                dispatch_reviewer(loop_state.pr)
                start_timer(timeout=10min)

        case "REVIEWING":
            if event == VERDICT_RECEIVED:
                cancel_timer()
                if event.verdict == "LGTM":
                    loop_state.state = "APPROVED"
                    notify_human("PR ready to merge")
                elif event.verdict == "CHANGES_REQUESTED":
                    if loop_state.iteration >= max_iterations:
                        escalate("Max iterations reached")
                    elif has_security_findings(event.findings):
                        escalate("Security findings need human")
                    else:
                        loop_state.state = "FIXING"
                        dispatch_coder(findings)
                        start_timer(timeout=15min)
                elif event.verdict == "INCOMPLETE":
                    retry_or_escalate(loop_state)

            if event == TIMEOUT:
                retry_or_escalate(loop_state)

        case "FIXING":
            if event == SYNCHRONIZE:
                # Canonical trigger: GitHub webhook `pull_request.synchronize`
                # Discord "I pushed" message is informational only, not a state trigger
                cancel_timer()
                loop_state.iteration += 1
                loop_state.head_sha = event.head_sha
                loop_state.state = "REVIEWING"
                dispatch_reviewer(loop_state.pr, head_sha=event.head_sha)
                start_timer(timeout=10min)

            if event == FIX_FAILED:
                escalate("Coder failed to fix")

            if event == TIMEOUT:
                retry_or_escalate(loop_state)

        case "APPROVED":
            if event == HUMAN_SAYS_MERGE:
                merge_pr()
                loop_state.state = "DONE"

        case "ESCALATED":
            # Loop paused — only human commands accepted
            if event == HUMAN_OVERRIDE:
                handle_override(event)
```

### Retry and Timeout Policy

```
Per-step timeout:
  - Review: 10 min
  - Fix: 15 min
  - Entire loop: 60 min (hard cap)

Retry policy:
  - Max 1 retry per step
  - If step fails after retry → escalate to human
  - Retry resets the step timer
```

**Timer implementation (stateless-safe):**

Timers must survive process restarts, container recycling, and serverless cold starts. In-memory timers (`tokio timer`, `setTimeout`) are **not acceptable** as the sole mechanism.

| Phase | Implementation | Survivability |
|-------|---------------|---------------|
| Phase 1 (MVP) | `current_step_started` timestamp in state file + cron/polling loop checks `now - current_step_started > timeout` every 2 min | Survives restart — timer state is on disk |
| Phase 2+ | External delayed queue (SQS delay, Redis ZRANGEBYSCORE, EventBridge scheduled rule) | Survives anything — timer is externalized |

The controller's polling loop (which already runs for GitHub event detection) doubles as the timeout checker — no additional infrastructure needed for Phase 1.

### Completion Check

Reviewer output must contain one of:
- `LGTM ✅`
- `CHANGES REQUESTED ⚠️`
- `INCOMPLETE ⏸️ — reason: <reason>`

If none appears within the timeout window, the controller treats it as an incomplete step and applies the retry policy.

### State Store

Phase 1: file-based state under `~/.openab/loops/state/pr-{number}.json`

```json
{
  "pr": 42,
  "repo": "openabdev/openab",
  "state": "REVIEWING",
  "head_sha": "abc1234",
  "iteration": 2,
  "max_iterations": 3,
  "started_at": "2026-06-09T03:30:00Z",
  "current_step_started": "2026-06-09T03:35:00Z",
  "token_used": 18000,
  "token_budget": 50000,
  "retries": 0,
  "last_dispatch_id": "uuid-v4",
  "history": [
    {"step": "review", "result": "CHANGES_REQUESTED", "head_sha": "abc1234", "dispatch_id": "...", "ts": "..."},
    {"step": "fix", "result": "pushed", "commit": "def5678", "dispatch_id": "...", "ts": "..."}
  ]
}
```

**Concurrency protection (race condition mitigation):**

Multiple events (webhook `synchronize` + Discord verdict) can arrive simultaneously for the same PR. Without protection, concurrent reads/writes to the state file cause data loss or corrupt transitions.

| Phase | Mechanism | Tradeoff |
|-------|-----------|----------|
| Phase 1 (MVP) | **Single-writer guarantee**: all state transitions run in a single-threaded event loop (one process, one PR at a time). Incoming events are queued in-memory and processed sequentially. | Simple, no locking needed; limits throughput to one event per PR at a time |
| Phase 1 (alt) | **File lock** (`flock` / `fcntl`): acquire exclusive lock on `pr-{number}.json` before read-modify-write | Works for multi-process; adds OS-level dependency |
| Phase 2+ | Atomic compare-and-swap in Redis/DynamoDB (optimistic locking with version field) | Production-grade; handles distributed deployments |

**Invariant:** No state transition may proceed without holding exclusive write access to the PR's state. Violations must be detected and retried.

Later phases can migrate to Redis or DynamoDB.

### Event Source: Webhook vs Polling

Not all users can configure GitHub webhooks (no admin access, private repo policies, firewall restrictions). The controller must support both push and pull models.

**Push mode (webhook):** GitHub sends events to the controller in real-time.

**Pull mode (polling):** Controller periodically queries GitHub API for changes.

```
┌─────────────────────────────────────┐
│         EVENT SOURCE                 │
│                                     │
│  ┌─────────┐    ┌─────────────┐    │
│  │ Webhook │ OR │ Poller      │    │
│  │ (HTTP)  │    │ (cron/timer)│    │
│  └────┬────┘    └──────┬──────┘    │
│       │                │           │
│       └───────┬────────┘           │
│               ▼                    │
│      Normalized Event              │
│      {type, pr, payload}           │
└───────────────┬─────────────────────┘
                │
                ▼
         State Machine
         (same logic regardless of source)
```

Polling endpoints:

| Detection target | GitHub API |
|-----------------|------------|
| New commits pushed | `GET /repos/{owner}/{repo}/pulls/{pr}/commits` |
| New comment (verdict) | `GET /repos/{owner}/{repo}/issues/{pr}/comments` |
| CI status | `GET /repos/{owner}/{repo}/commits/{sha}/check-runs` |

**Rate limit optimization:** All polling requests MUST use conditional requests (`If-None-Match` / `ETag`). A `304 Not Modified` response does not count against the rate limit. The poller stores the last `ETag` per endpoint per PR and sends it on subsequent requests.

Comparison:

| | Webhook | Polling |
|--|---------|---------|
| Latency | Seconds | Depends on interval (30-120s) |
| GitHub API quota | None | 2-3 calls per active loop per minute |
| Setup requirement | Admin webhook config | Only repo read permission |
| Best for | Teams with infra control | Everyone |

Configuration:

```toml
[loop]
event_source = "polling"   # "webhook" or "polling"
poll_interval = 60         # seconds, only used when polling

# Only PRs matching these conditions enter the loop
[loop.conditions]
labels = ["auto-review"]        # PR must have this label to activate
base_branch = ["main", "dev"]   # Only PRs targeting these branches
exclude_authors = ["bot"]       # Skip PRs from these authors
exclude_paths = ["docs/**"]     # Skip PRs that only touch these paths
```

Users opt in by adding the `auto-review` label to a PR. No label = traditional manual flow.

**Polling is the universal default; webhook is an opt-in acceleration for teams that can configure it.**

### Deployment Options

| Option | Description | Tradeoff |
|--------|-------------|----------|
| Standalone process | Persistent server (Express/Axum) listening to webhooks + Discord | Most reliable, needs hosting |
| Embedded in webhook handler | Add state machine as middleware in existing handler | No new infra, couples concerns |
| Serverless | Lambda + EventBridge for timers | Scales to zero, cold start latency |

### Decoupling Principle

The controller communicates with agents **only through Discord mentions**. Agents do not know the controller exists — they only respond to mentions. This keeps agents stateless and the controller replaceable.

```
Controller → Discord mention → Agent works → Discord/GitHub event → Controller
```

## Open vs Closed Looping

| | Open Loop | Closed Loop |
|---|---|---|
| Nature | Exploratory, wide solution space | Bounded, predefined path |
| Pros | Can discover unexpected solutions | Cheap, controllable, improves each run |
| Cons | Burns tokens, risk of low-quality output | Requires upfront path design |
| Use for | Architecture exploration, new features | Routine work, repeatable flows |

This ADR implements a **closed loop**. Open loops may be explored in future ADRs once token budgets and model capabilities mature.

## Implementation Phases

### Phase 1 (MVP) — Loop Controller as Single Process

Phase 1 ships with a **lightweight Loop Controller** from day one. Relying on agents to self-dispatch is unreliable — agents can hallucinate, forget context, or skip steps. A dedicated controller provides deterministic event routing and safety enforcement.

**Phase 1 controller scope:**
- Single long-running process (can be a simple Node/Python/Rust daemon)
- Polls GitHub API for PR events (no webhook infra required)
- Listens to Discord messages for verdicts
- Maintains file-based state per PR (`~/.openab/loops/state/pr-{number}.json`)
- Enforces timeouts via timestamp comparison on each poll cycle
- Enforces safety policy (path denylist, category escalation) before dispatching to Coder
- Single-threaded event loop — one event per PR at a time (no race conditions)

**Phase 1 flow:**
1. Controller detects new PR (via polling or human trigger)
2. Controller dispatches Reviewer via Discord mention
3. Controller receives verdict from Discord, validates SHA binding
4. If `CHANGES REQUESTED` → Controller validates safety policy → dispatches Coder
5. Controller detects push (via polling), validates new head SHA
6. Controller dispatches Reviewer for re-review
7. Loop repeats until LGTM, max_iterations, or escalation

**Why controller-first:**
- Agents are stateless and unreliable for coordination — they may drop context mid-loop
- Controller holds the single source of truth (state file)
- All safety enforcement is centralized and auditable
- Easy to debug: one process, one log, deterministic behavior

### Phase 2 — Eval Gates & Webhook Support

- Add structured quality checks at each step beyond "does it build"
- Add webhook support as opt-in acceleration (faster than polling)
- Migrate state to Redis for multi-instance deployment

### Phase 3 — Loop History & Feedback

Each completed loop feeds context to future runs. Historical findings inform reviewer focus areas.

### Phase 4 — Configurable Loops

Configurable loop parameters in `config.toml` (max_iterations, auto_merge, severity filters, escalation rules).

## Consequences

### Positive
- Reduces human bottleneck in PR review cycles
- Leverages existing infrastructure (Discord, webhooks) — no core changes needed
- Human retains full visibility and override capability via Discord
- Each iteration improves agent behavior through accumulated feedback

### Negative
- Risk of infinite loops if safety boundaries fail — mitigated by hard iteration cap
- Token cost increases with iteration count — bounded by max_iterations
- Agents may produce low-quality fixes under pressure — mitigated by eval gates (Phase 2)

### Neutral
- Does not require changes to OAB core
- Does not require user-facing config changes in Phase 1
- Compatible with future open-loop exploration

## References

- Agent looping concept (industry pattern, 2025-2026)
- OpenAB communication protocol (`shared/communication-protocol.md`)
- OpenAB PR review spec (`shared/pr-review-spec.md`)
