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

### Loop Lifecycle

A Loop is a generic, reusable abstraction. Regardless of what work it coordinates (PR review, deployment, testing), every Loop instance shares the same lifecycle, methods, and state transitions.

#### Lifecycle Diagram

```
                         ┌─────────────────────────────────────────────┐
                         │              LOOP LIFECYCLE                  │
                         └─────────────────────────────────────────────┘

  create()                    start()                                    stop()
     │                          │                                          │
     ▼                          ▼                                          ▼
 ┌────────┐  start()  ┌─────────────────┐  stop()/budget/safety  ┌────────────┐
 │ CREATED │─────────→│    RUNNING       │─────────────────────→│    DONE     │
 └────────┘           │                  │                       └────────────┘
                      │  ┌────────────┐  │                              ▲
                      │  │  consume() │  │  escalate()           ┌──────┴─────┐
                      │  │     ↓      │  │──────────────────────→│ ESCALATED  │
                      │  │ validate() │  │                       │ (paused)   │
                      │  │     ↓      │  │  resume()             └──────┬─────┘
                      │  │ dispatch() │  │←─────────────────────────────┘
                      │  └────────────┘  │
                      │                  │  pause()       resume()
                      │                  │───→ PAUSED ────→│
                      └─────────────────┘                  │
                               ▲                           │
                               └───────────────────────────┘
```

#### Internal Processing Loop (per event)

```
    Event arrives (webhook, message, timer)
         │
         ▼
    ┌──────────┐     duplicate?
    │ consume()│────────────────→ discard (no-op)
    └────┬─────┘     stale sha?
         │
         ▼
    ┌────────────┐    budget exceeded?
    │ validate() │    safety violation? ───→ escalate()
    └─────┬──────┘    max iterations?
          │
          ▼ (all checks pass)
    ┌────────────┐
    │ dispatch() │───→ worker (reviewer / coder / any agent)
    └────────────┘
```

#### Loop Interface (Generic)

```typescript
interface Loop {
  // --- Lifecycle ---
  start(): void;                        // CREATED → RUNNING
  pause(): void;                        // RUNNING → PAUSED (human intervenes)
  resume(): void;                       // PAUSED | ESCALATED → RUNNING
  stop(reason: string): void;           // any → DONE

  // --- Core methods (the consume → validate → dispatch pipeline) ---
  consume(event: Event): void;          // Ingest event, dedup, trigger state transition
  validate(task: Task): ValidationResult;  // Pre-dispatch gate (budget, safety, staleness)
  dispatch(worker: Worker, task: Task): void;  // Send work to a processor

  // --- Escalation ---
  escalate(reason: string): void;       // → ESCALATED, notify human

  // --- State ---
  getState(): LoopState;                // Current state snapshot
  getHistory(): StepRecord[];           // Full audit trail
}
```

#### Loop States

```
┌──────────────────────────────────────────────────────────────────────┐
│                                                                      │
│   CREATED ──→ RUNNING ──→ DONE                                       │
│                 │  ▲                                                  │
│                 │  │ resume()                                         │
│                 ▼  │                                                  │
│               PAUSED                                                 │
│                                                                      │
│              RUNNING ──→ ESCALATED ──→ DONE (if human says stop)      │
│                              │                                       │
│                              └──→ RUNNING (if human says resume)      │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

Within `RUNNING`, the Loop maintains an **inner state machine** specific to the work type. For PR review loops:

```
RUNNING.inner:  REVIEWING ←──→ FIXING ──→ APPROVED
```

#### Method Responsibilities

| Method | Responsibility | Failure mode |
|--------|---------------|--------------|
| `consume(event)` | Dedup, order, update `head_sha`, trigger state transition | Stale/duplicate → discard silently |
| `validate(task)` | Check ALL pre-conditions before dispatch | Any check fails → return `{ ok: false, reason }` |
| `dispatch(worker, task)` | Send structured payload to worker, start step timer | Worker unreachable → retry once, then escalate |
| `escalate(reason)` | Pause loop, notify human with context | — |
| `stop(reason)` | Record final state, clean up timers, emit completion event | — |

#### Validate Checks (in order)

1. **Dedup** — Has this `(dispatch_id, head_sha)` been processed before?
2. **Staleness** — Is `head_sha` still the PR head? (API check)
3. **Iteration budget** — `iteration < max_iterations`?
4. **Token budget** — `token_used + estimated_next > token_budget`?
5. **Timeout** — Has the total loop time exceeded hard cap?
6. **Safety policy** — Category/risk + path denylist check on task content
7. **Fingerprint repeat** — Same finding unresolved for 2+ iterations?

All must pass. First failure short-circuits and returns the reason.

---

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
5. **Replication delay tolerance** — GitHub webhooks may arrive before the API reflects new state. If a `synchronize` event arrives but the API still returns the old SHA, the controller retries with exponential backoff (1s, 2s, 4s, max 3 retries) before discarding.

**Completion criteria for a loop step:**

A step is complete only when **both** conditions are met for the **same** `head_sha`:
1. Reviewer verdict received (LGTM or CHANGES_REQUESTED)
2. All required CI checks pass

```
# Step 1: Get required status check contexts from branch protection
GET /repos/{owner}/{repo}/branches/{base}/protection/required_status_checks
→ returns { contexts: ["ci/test", "ci/lint", ...] }

# Step 2: Get check runs for head_sha, filter by required contexts
GET /repos/{owner}/{repo}/commits/{head_sha}/check-runs
→ filter by names matching required contexts
→ all must have conclusion: "success"
```

**Required checks — source of truth (in priority order):**

1. **Explicit config** (most reliable): list check names in `config.toml`
   ```toml
   [loop.ci]
   required_checks = ["build", "test", "clippy", "fmt"]
   ```
2. **Branch protection API** (auto-discover): `GET /repos/{owner}/{repo}/branches/{base}/protection/required_status_checks`
3. **Fallback** (if neither available): wait until all check-runs on `head_sha` reach a terminal state, and none have `conclusion: "failure"`

The controller tries each source in order. If `required_checks` is set in config, it is authoritative and the API is not consulted.

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
- `token_used >= token_budget` (default: 50000 tokens)
- Safety policy violation (see machine-enforceable rules below)
- Same finding unresolved for 2 consecutive iterations (matched by finding fingerprint)
- Tests fail after fix
- Required CI checks fail after push
- Human explicitly intervenes ("stop" / "I'll handle it")

Human override has the highest priority at all times.

#### Token Budget Enforcement

| Rule | Detail |
|------|--------|
| Who reports | Each agent reports `tokens_consumed` in its verdict/completion message |
| When checked | Controller checks budget **before** each dispatch (not after) |
| Accumulation | `token_used += agent_response.tokens_consumed` after each step |
| Hard stop | If `token_used + estimated_next_step > token_budget` → escalate |
| Estimation | `estimated_next_step` = max of previous step costs in this loop (conservative) |
| Override | Human can raise budget mid-loop via "budget 100000" command |

### Finding Fingerprint (Cross-Iteration Dedup)

To detect "same finding unresolved across iterations," each finding carries a stable fingerprint:

```
fingerprint = sha256(file + ":" + normalized_issue_text)[:8]
```

Line numbers are explicitly excluded — code edits shift lines, which would cause the same defect to produce a different fingerprint across iterations.

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

> **Principle:** Path rules are **defense-in-depth**, not the sole safety classifier. The primary classifier is the structured `category` + `risk_level` in findings. Path denylist catches cases where the Coder attempts to modify sensitive files not explicitly flagged by the Reviewer.

```toml
[safety.path_denylist]
patterns = [
  ".github/workflows/**",
  "infra/**",
  "helm/**",
  "terraform/**",
  "**/auth/**",
  "**/auth*",         # catches src/auth.rs, lib/auth.ts, etc.
  "**/secrets/**",
  "**/credential*",
  "**/*.pem",
  "**/*.key",
  ".env*",
]

# Keyword matching on file path (catches files path globs miss)
keywords = ["auth", "secret", "credential", "token", "password", "infra"]
```

**Matching logic:** A file is denied if it matches **any** glob pattern OR its path contains **any** keyword. This dual approach (glob + keyword) ensures safety without needing to enumerate every possible file layout.

**Principle: path denylist is defense-in-depth, not the sole classifier.** Patterns can never exhaustively cover all sensitive files. The primary safety gate is the structured `category` + `risk_level` in findings. Path denylist is a secondary catch-all that triggers even when a reviewer fails to tag a finding correctly.

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
                if event.head_sha != loop_state.head_sha:
                    backoff_retry(event, max=3)  # API replication delay
                    return
                cancel_timer()
                loop_state.token_used += event.tokens_consumed
                if event.verdict == "LGTM":
                    loop_state.state = "APPROVED"
                    notify_human("PR ready to merge")
                elif event.verdict == "CHANGES_REQUESTED":
                    # --- Pre-dispatch safety checks (all must pass) ---
                    if loop_state.iteration >= max_iterations:
                        escalate("Max iterations reached")
                    elif loop_state.token_used + estimate_next_step(loop_state) > loop_state.token_budget:
                        escalate("Token budget would be exceeded")
                    elif not validate_safety_policy(event.findings, loop_state):
                        # Checks: category/risk_level, path denylist, fingerprint repeat
                        escalate("Safety policy violation — requires human")
                    else:
                        loop_state.state = "FIXING"
                        loop_state.token_used += event.tokens_consumed
                        dispatch_coder(event.findings, head_sha=loop_state.head_sha)
                        start_timer(timeout=15min)
                elif event.verdict == "INCOMPLETE":
                    retry_or_escalate(loop_state)

            if event == TIMEOUT:
                retry_or_escalate(loop_state)

        case "FIXING":
            if event == SYNCHRONIZE:
                cancel_timer()
                loop_state.iteration += 1
                loop_state.head_sha = event.head_sha
                loop_state.token_used += event.tokens_consumed
                if loop_state.token_used >= loop_state.token_budget:
                    escalate("Token budget exhausted")
                    return
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
| Phase 1 (MVP) | **Per-PR single-writer**: each PR gets its own sequential event queue (actor pattern). Different PRs process events concurrently; within a single PR, events are serialized. No cross-PR blocking. | Simple, no locking needed per PR; scales to multiple active loops |
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

## Loop Lifecycle

A Loop is a **bounded, stateful coordination unit** managing one work item (e.g., one PR) through repeated agent cycles until completion or escalation.

### Lifecycle Diagram

```
                         ┌─────────────────────────────────────────┐
                         │              LOOP LIFECYCLE              │
                         └─────────────────────────────────────────┘

  new Loop(config)
        │
        ▼
   ┌─────────┐   start()    ┌────────────┐  consume(verdict)   ┌─────────┐
   │  IDLE   │─────────────▶│ REVIEWING  │────────────────────▶│ FIXING  │
   └─────────┘              └────────────┘                     └─────────┘
                                  ▲    │                            │
                                  │    │ verdict=LGTM              │ synchronize
                                  │    ▼                           │ (new push)
                                  │  ┌──────────┐                  │
                                  │  │ APPROVED │                  │
                                  │  └──────────┘                  │
                                  │                                │
                                  └────────────────────────────────┘
                                        iteration++

   Any state ──── timeout / budget / safety violation ────▶ ┌───────────┐
                                                           │ ESCALATED │
                                                           └───────────┘
                                                                 │
                                                    resume()     │  stop()
                                                    (human ok)   │
                                                        │        ▼
                                                        │   ┌────────┐
                                                        └──▶│  DONE  │
                                                            └────────┘
```

### Loop Instance — Properties

```typescript
interface LoopConfig {
  pr: number;
  repo: string;
  max_iterations: number;       // hard cap (default: 3)
  token_budget: number;         // max tokens across all steps (default: 50000)
  timeout: {
    review: Duration;           // per review step (default: 10m)
    fix: Duration;              // per fix step (default: 15m)
    total: Duration;            // entire loop (default: 60m)
  };
  safety_policy: SafetyPolicy;  // path denylist + category escalation rules
}

interface LoopState {
  state: "IDLE" | "REVIEWING" | "FIXING" | "APPROVED" | "ESCALATED" | "DONE";
  iteration: number;
  head_sha: string;
  token_used: number;
  started_at: Timestamp;
  current_step_started: Timestamp;
  last_dispatch_id: string;
  history: StepRecord[];
}
```

### Loop Instance — Methods

| Method | Responsibility | Mutates State? |
|--------|---------------|----------------|
| `start()` | Initialize state, dispatch first reviewer | Yes → REVIEWING |
| `consume(event)` | Receive event, deduplicate, enqueue | No (queues only) |
| `transition(event)` | Evaluate state × event → decide next state | Yes |
| `validate(findings)` | Pre-dispatch safety check (budget, policy, iteration cap) | No (read-only) |
| `dispatch(target, payload)` | Send work to a worker agent (via Discord mention) | Yes (records dispatch_id) |
| `escalate(reason)` | Hard stop, notify human, freeze loop | Yes → ESCALATED |
| `tick()` | Periodic check: timeout expired? poll for new events? | Maybe (triggers transition if timeout) |
| `stop(reason)` | Terminate loop (human or merged) | Yes → DONE |
| `resume()` | Recover from ESCALATED after human intervention | Yes → previous state |

### Method Flow Diagram

```
              External Events
              (webhook, Discord, timer)
                      │
                      ▼
               ┌──────────────┐
               │  consume()   │  ← deduplicate, normalize, enqueue
               └──────┬───────┘
                      │
                      ▼
               ┌──────────────┐
               │ transition() │  ← state × event → new state
               └──────┬───────┘
                      │
              ┌───────┴───────┐
              │               │
         need dispatch?    terminal?
              │               │
              ▼               ▼
       ┌──────────────┐  ┌──────────┐
       │  validate()  │  │  stop()  │
       └──────┬───────┘  └──────────┘
              │
         pass? ──── no ──▶ escalate()
              │
             yes
              │
              ▼
       ┌──────────────┐
       │  dispatch()  │  ← mention worker, record dispatch_id
       └──────┬───────┘
              │
              ▼
       ┌──────────────┐
       │ start timer  │  ← tick() will check this
       └──────────────┘
```

### Typical Lifecycle Example

```
1. l = new Loop({ pr: 42, max_iterations: 3, token_budget: 50000 })
   → state: IDLE

2. l.start()
   → dispatch(reviewer, { pr: 42, head_sha: "aaa" })
   → state: REVIEWING, timer: 10m

3. l.consume({ type: "verdict", verdict: "CHANGES_REQUESTED", findings: [...] })
   → transition: REVIEWING × CHANGES_REQUESTED → validate(findings) → pass
   → dispatch(coder, { findings, head_sha: "aaa" })
   → state: FIXING, iteration: 1, timer: 15m

4. l.consume({ type: "synchronize", head_sha: "bbb" })
   → transition: FIXING × SYNCHRONIZE → iteration++
   → dispatch(reviewer, { pr: 42, head_sha: "bbb" })
   → state: REVIEWING, iteration: 2, timer: 10m

5. l.consume({ type: "verdict", verdict: "LGTM" })
   → transition: REVIEWING × LGTM → notify human
   → state: APPROVED

6. l.stop("merged")
   → state: DONE
```

### Invariants (must hold at all times)

1. **Bounded** — `iteration` never exceeds `max_iterations`; `token_used` never exceeds `token_budget`.
2. **Single state** — A Loop is in exactly one state at any moment.
3. **SHA-bound** — Every dispatch and verdict is tied to a specific `head_sha`. Stale SHA = discard.
4. **Idempotent** — Same `(pr, head_sha, dispatch_id)` processed twice = no-op.
5. **Human supreme** — `stop()` is callable from any state and always wins.

## Open vs Closed Looping

| | Open Loop | Closed Loop |
|---|---|---|
| Nature | Exploratory, wide solution space | Bounded, predefined path |
| Pros | Can discover unexpected solutions | Cheap, controllable, improves each run |
| Cons | Burns tokens, risk of low-quality output | Requires upfront path design |
| Use for | Architecture exploration, new features | Routine work, repeatable flows |

This ADR implements a **closed loop**. Open loops may be explored in future ADRs once token budgets and model capabilities mature.

## Implementation Phases

### Phase 0 (Considered, Not Implemented) — Agent-Based Coordination

In this model, agents self-dispatch via Discord mentions without a dedicated controller. Reviewer directly mentions Coder on `CHANGES REQUESTED`, and Coder self-validates safety before acting.

**Why we skip this:**
- Agents are stateless — they can hallucinate, forget context, or skip steps mid-loop
- No centralized audit trail or timeout enforcement
- Safety policy enforcement distributed across agents is unreliable and hard to verify
- Debugging requires reading multiple agent logs with no single source of truth

This phase exists as documentation of the rejected alternative. We start directly at Phase 1.

### Phase 1 (MVP) — Controller in Core, Polling Mode

Phase 1 ships the Loop Controller as part of the **OAB core process**. It runs alongside the existing agent runtime, using polling to detect events.

**Phase 1 scope:**
- Controller embedded in the core agent process
- Polls GitHub API for PR events (new commits, CI status)
- Listens to Discord messages for verdicts
- Maintains file-based state per PR (`~/.openab/loops/state/pr-{number}.json`)
- Enforces timeouts via timestamp comparison on each poll cycle
- Enforces safety policy (path denylist, category escalation) before dispatching to Coder
- Per-PR event queue (actor pattern) — events for different PRs process concurrently; within one PR, events are serialized

**Phase 1 flow:**
1. Controller detects new PR (via polling or human trigger)
2. Controller dispatches Reviewer via Discord mention
3. Controller receives verdict from Discord, validates SHA binding
4. If `CHANGES REQUESTED` → Controller validates safety policy → dispatches Coder
5. Controller detects push (via polling), validates new head SHA
6. Controller dispatches Reviewer for re-review
7. Loop repeats until LGTM, max_iterations, or escalation

**Why in-core + polling:**
- No additional infrastructure needed — runs where agents already run
- Polling requires only repo read permission (no admin webhook config)
- Single process = single log = easy to debug
- Controller holds the single source of truth (state file)
- All safety enforcement is centralized and auditable

### Phase 2 — Controller in Gateway, Webhook Mode

Move the Loop Controller out of core into a **dedicated Gateway service** that receives GitHub webhooks in real-time.

- Real-time event delivery (seconds vs polling interval)
- Gateway handles webhook validation, event normalization, and routing
- Supports multiple concurrent loops at scale
- State migrates to Redis/DynamoDB for distributed deployment
- Add structured eval gates at each step beyond "does it build"

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
