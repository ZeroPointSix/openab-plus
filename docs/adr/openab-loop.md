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
// ═══════════════════════════════════════════════════════════════
// Types
// ═══════════════════════════════════════════════════════════════

type LoopStatus = "CREATED" | "RUNNING" | "PAUSED" | "ESCALATED" | "DONE";

interface LoopConfig {
  id: string;                           // Unique loop instance ID (e.g. "pr-review-1045")
  maxIterations: number;                // Hard cap on cycles (default: 3)
  tokenBudget: number;                  // Max tokens across all steps (default: 50000)
  timeouts: {
    perStep: Record<string, Duration>;  // e.g. { review: "10m", fix: "15m" }
    total: Duration;                    // Hard cap on entire loop (default: "60m")
  };
  safetyPolicy: SafetyPolicy;
  workers: WorkerRegistry;              // Available workers this loop can dispatch to
}

interface Event {
  id: string;                           // Unique event ID (for dedup)
  type: string;                         // e.g. "pr.opened", "verdict.received", "push.synchronize"
  source: string;                       // e.g. "github-webhook", "discord-message", "timer"
  timestamp: string;                    // ISO 8601
  headSha?: string;                     // Current PR head (for staleness check)
  payload: Record<string, unknown>;     // Event-specific data
}

interface Task {
  dispatchId: string;                   // UUID, for idempotency
  worker: string;                       // Target worker ID
  action: string;                       // e.g. "review", "fix"
  headSha: string;                      // SHA this task is bound to
  iteration: number;                    // Current iteration count
  findings?: Finding[];                 // Findings to fix (for coder dispatch)
  metadata?: Record<string, unknown>;   // Extensible
}

interface Finding {
  fingerprint: string;                  // sha256(file + ":" + normalized_issue)[:8]
  severity: "🔴" | "🟡" | "🟢";
  category: "logic" | "performance" | "style" | "security" | "auth" | "infra" | "config";
  riskLevel: "low" | "medium" | "high" | "critical";
  file: string;
  line: number;
  issue: string;
  suggestion?: string;
}

interface ValidationResult {
  ok: boolean;
  reason?: string;                      // Why validation failed (if !ok)
  failedCheck?: string;                 // Which check failed (e.g. "token_budget")
}

interface LoopState {
  status: LoopStatus;
  innerState?: string;                  // Work-type-specific (e.g. "REVIEWING", "FIXING")
  iteration: number;
  tokenUsed: number;
  headSha: string;
  startedAt: string;
  currentStepStartedAt: string;
  retriesThisStep: number;
}

interface StepRecord {
  step: number;
  action: string;                       // "review" | "fix" | "escalate"
  worker: string;
  startedAt: string;
  completedAt: string;
  result: "success" | "failed" | "timeout" | "escalated";
  tokensConsumed: number;
  headSha: string;
  findings?: Finding[];                 // What was found/fixed in this step
}

interface Worker {
  id: string;                           // e.g. Discord UID
  name: string;                         // e.g. "超渡法師"
  capabilities: string[];               // e.g. ["review", "fix", "test"]
  dispatchChannel: string;              // How to reach them (e.g. "discord-mention")
}

interface SafetyPolicy {
  pathDenylist: { patterns: string[]; keywords: string[] };
  categoryRules: CategoryRule[];
}

interface CategoryRule {
  category: string;
  riskLevel: string;
  action: "allow" | "escalate";
}

// ═══════════════════════════════════════════════════════════════
// Loop Interface
// ═══════════════════════════════════════════════════════════════

interface Loop {
  readonly config: LoopConfig;

  // --- Lifecycle ---
  start(): void;                        // CREATED → RUNNING, dispatch first worker
  pause(): void;                        // RUNNING → PAUSED (human intervenes)
  resume(): void;                       // PAUSED | ESCALATED → RUNNING
  stop(reason: string): void;           // any → DONE, cleanup timers, emit event

  // --- Core Pipeline: consume → validate → dispatch ---
  consume(event: Event): void;          // Ingest event, dedup, reorder, state transition
  validate(task: Task): ValidationResult;  // Pre-dispatch gate (all 7 checks)
  dispatch(task: Task): void;           // Send work to worker, start step timer

  // --- Escalation ---
  escalate(reason: string): void;       // → ESCALATED, notify human with full context

  // --- Notification ---
  notify(target: string, message: string): void;  // Inform without expecting response

  // --- State & History ---
  getState(): LoopState;                // Current state snapshot
  getHistory(): StepRecord[];           // Full audit trail of all steps
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

#### Dispatch Context & Completion Report

Since each dispatch creates a **new agent session**, the Loop must pass enough context for the agent to:
1. Know which loop it belongs to
2. Work in the correct thread
3. Report back to the correct place when done

**Dispatch payload (Controller → Agent):**

```typescript
interface DispatchPayload {
  // --- Loop identity ---
  loopId: string;                       // Which loop instance this belongs to
  dispatchId: string;                   // Unique ID for this dispatch (idempotency)
  iteration: number;                    // Current iteration

  // --- Thread context (for session continuity) ---
  threadId: string;                     // Discord thread ID — agent must reply HERE
  channelId: string;                    // Discord channel ID
  replyToMessageId?: string;           // Message to reply to (maintains thread chain)

  // --- Work context ---
  task: Task;                           // The actual work to perform
  callbackFormat: "discord-reply";      // How to report back (extensible for future: webhook, etc.)
}
```

**Completion report (Agent → Controller):**

When an agent finishes its work (review done, fix pushed, or failed), it posts a structured reply **in the same thread**:

```typescript
interface CompletionReport {
  // --- Routing (how Controller finds this report) ---
  loopId: string;                       // Echo back — Controller filters on this
  dispatchId: string;                   // Echo back — Controller matches to pending dispatch
  threadId: string;                     // The thread this reply lives in
  messageId: string;                    // This message's ID (for Controller to ack)

  // --- Result ---
  status: "completed" | "failed" | "blocked";
  verdict?: "LGTM" | "CHANGES_REQUESTED";  // For reviewers
  action?: "pushed" | "escalated";          // For coders

  // --- Accounting ---
  tokensConsumed: number;               // Tokens used in this session
  headSha: string;                      // SHA the work was done against
  duration: number;                     // Seconds elapsed

  // --- Findings (reviewer) or Changes (coder) ---
  findings?: Finding[];                 // What reviewer found
  filesChanged?: string[];              // What coder modified
  newHeadSha?: string;                  // After push (coder only)

  // --- Error context (if failed/blocked) ---
  error?: {
    reason: string;
    details?: string;
  };
}
```

**Flow diagram (session boundary):**

```
┌─────────────────────┐         ┌─────────────────────────────┐
│   LOOP CONTROLLER   │         │   AGENT (new session)        │
│                     │         │                             │
│  dispatch() ────────┼────────→│  receives DispatchPayload    │
│                     │         │    - knows loopId            │
│                     │         │    - knows threadId          │
│                     │         │    - knows what to do        │
│                     │         │                             │
│                     │         │  ... does work ...           │
│                     │         │                             │
│  consume() ←────────┼─────────│  posts CompletionReport      │
│    (via Discord     │         │    - in same threadId        │
│     message event)  │         │    - echoes loopId           │
│                     │         │    - echoes dispatchId       │
└─────────────────────┘         └─────────────────────────────┘
```

**Key invariants:**

1. **Thread pinning** — All messages for one loop iteration live in the same Discord thread. Controller creates the thread; agents reply in it.
2. **Echo pattern** — Agent MUST echo `loopId` + `dispatchId` in its report. Controller uses these to correlate the response to the pending dispatch.
3. **Thread ID in state** — Controller stores `threadId` in `LoopState`. If the loop spans multiple iterations, all stay in the same thread.
4. **Agent doesn't need loop state** — Agent receives a self-contained task. It doesn't query the Controller; it just does the work and posts the result.
5. **Controller discovers completion via consume()** — The CompletionReport is just another Event. Controller's existing `consume()` pipeline handles it (dedup, validate sha, transition state).

**Updated LoopState with thread tracking:**

```typescript
interface LoopState {
  status: LoopStatus;
  innerState?: string;
  iteration: number;
  tokenUsed: number;
  headSha: string;
  startedAt: string;
  currentStepStartedAt: string;
  retriesThisStep: number;

  // --- Thread context ---
  threadId: string;                     // Discord thread for this loop
  channelId: string;                    // Parent channel
  pendingDispatchId?: string;           // Currently awaited dispatch (null if idle)
  pendingWorker?: string;               // Who we're waiting on
}
```

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

### Dispatch Content (Prompt Templates)

The **structured JSON** above is the machine-readable payload. But the actual message sent to an agent is a **rendered prompt** — natural language with embedded context. This prompt is defined per step in `config.toml`:

```toml
[[loop.steps]]
name = "review"
worker = "reviewer"
action = "review"
timeout = "10m"
prompt_template = """
請 review 這個 PR：{pr_url}
Branch: `{branch}` | HEAD: `{head_sha}` | Iteration: {iteration}/{max_iterations}

Review 完成後請回報：
- loopId: `{loop_id}`
- dispatchId: `{dispatch_id}`
- verdict: `LGTM ✅` 或 `CHANGES REQUESTED ⚠️`
- 如果 CHANGES REQUESTED，請附 findings（含 severity, category, file, issue）

請直接在這個 thread 回覆。
"""

[[loop.steps]]
name = "fix"
worker = "coder"
action = "fix"
timeout = "15m"
prompt_template = """
請修復以下 PR review findings：
PR: {pr_url} | Branch: `{branch}` | HEAD: `{head_sha}`
Iteration: {iteration}/{max_iterations}

Findings:
{findings_json}

規則：
- 優先修 🔴 (must fix)，再修 🟡 (should fix)
- 不要碰 🟢 (praise)
- 修完後 `git push`
- 回報：loopId `{loop_id}`, dispatchId `{dispatch_id}`, status, filesChanged, newHeadSha

請直接在這個 thread 回覆。
"""
```

**Template variables (Controller injects at dispatch time):**

| Variable | Source | Example |
|----------|--------|---------|
| `{pr_url}` | `LoopInstance.target` | `https://github.com/openabdev/openab/pull/42` |
| `{branch}` | `LoopInstance.target.branch` | `feat/example` |
| `{head_sha}` | `LoopInstance.headSha` | `abc1234` |
| `{iteration}` | `LoopInstance.iteration` | `1` |
| `{max_iterations}` | Template config | `3` |
| `{loop_id}` | `LoopInstance.loopId` | `pr-review-42-a3f2c1` |
| `{dispatch_id}` | Generated UUID per dispatch | `uuid-v4` |
| `{findings_json}` | Previous reviewer's findings | JSON array |
| `{thread_id}` | `LoopInstance.threadId` | Discord thread ID |

**Rendering flow:**

```
dispatch(task)
    │
    ├── Load prompt_template from step config
    ├── Inject variables from LoopInstance + task
    ├── Render final message string
    │
    ▼
Discord message: "@{worker_agent} \n{rendered_prompt}"
```

**Design rationale:**

- **Prompt in config, not code** — Users can customize review/fix instructions without changing Controller code
- **Variables make it dynamic** — Same template works across PRs, iterations, agents
- **Agent sees natural language** — Agent doesn't need to parse the Loop protocol; it just follows the prompt and posts results back
- **Structured echo required** — The prompt explicitly tells the agent to include `loopId` + `dispatchId` in its response, ensuring Controller can match it

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

Phase 1: file-based state under `~/.openab/loop/state/pr-{number}.json`

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

### PR Discovery — How Controller Finds New PRs to Loop

Before the Controller can manage a loop, it needs to **discover** which PRs should enter a loop. This is separate from monitoring PRs already in a loop.

**Discovery sources (in priority order):**

| Source | Trigger | Latency |
|--------|---------|---------|
| Discord command | Human says `loop start PR#42` | Instant |
| GitHub webhook | `pull_request.opened` or `pull_request.labeled` | Seconds |
| PR scan (polling) | Controller periodically lists open PRs | poll_interval |

**PR Scan — the polling-based discovery:**

```typescript
class PRDiscovery {
  private known_prs: Set<number>;  // PRs already in active_loops or rejected

  /**
   * Runs every poll_interval. Finds new PRs matching loop conditions.
   * Only creates loops for PRs not already tracked.
   */
  async scan(): Promise<void> {
    // Fetch open PRs (newest first, paginated)
    const prs = await github.get(
      `/repos/${repo}/pulls?state=open&sort=created&direction=desc&per_page=30`
    );

    for (const pr of prs) {
      // Skip if already tracked
      if (this.known_prs.has(pr.number)) continue;

      // Match against loop templates
      const template = this.matchTemplate(pr);
      if (!template) {
        this.known_prs.add(pr.number);  // remember rejection to avoid re-checking
        continue;
      }

      // Create new loop
      this.controller.createLoop(pr, template);
      this.known_prs.add(pr.number);
    }
  }

  /**
   * Check if a PR matches any configured loop template.
   */
  matchTemplate(pr: PullRequest): LoopTemplate | null {
    for (const tmpl of this.config.templates) {
      const match =
        this.hasRequiredLabel(pr, tmpl.trigger.labels) &&
        this.matchesBranch(pr, tmpl.trigger.base_branches) &&
        !this.isExcludedAuthor(pr, tmpl.trigger.exclude_authors) &&
        !this.isExcludedPaths(pr, tmpl.trigger.exclude_paths);

      if (match) return tmpl;
    }
    return null;
  }
}
```

**Matching conditions (all must be true):**

```toml
[[loop.templates]]
name = "pr-review"

[loop.templates.trigger]
labels = ["auto-review"]          # PR has this label
base_branches = ["main", "dev"]   # PR targets one of these
exclude_authors = ["dependabot"]  # skip these authors
exclude_paths = ["docs/**"]       # skip if PR ONLY touches these paths
min_changes = 1                   # skip empty PRs
```

**Discovery flow:**

```
Every poll_interval:
        │
        ▼
  GET /repos/{repo}/pulls?state=open
        │
        ▼
  For each PR:
        │
  already in active_loops? ── yes ──▶ skip
        │
       no
        │
        ▼
  matches a template? ── no ──▶ skip (add to known_prs)
        │
      yes
        │
        ▼
  createLoop(pr, template)
  → open Discord thread
  → state: IDLE → start() → REVIEWING
  → dispatch reviewer
```

**When webhook is available:**

If GitHub webhooks are configured, the scan is still useful as a **catch-up mechanism** (in case a webhook was missed), but primary discovery is event-driven:

```
GitHub webhook: pull_request.opened
  → payload includes labels, base, author
  → matchTemplate() → createLoop() if match

GitHub webhook: pull_request.labeled
  → check if new label triggers a template
  → createLoop() if match and no active loop exists
```

**Rate limit for discovery:**

| Scenario | API calls | Note |
|----------|-----------|------|
| Scan (per tick) | 1 | `GET /pulls?state=open` (with ETag) |
| Webhook mode | 0 | Event-driven, no polling |
| Hybrid (webhook + scan as backup) | 1 every 5 min | Reduced scan frequency |

### Event Consumption — How Controller Receives Events

Controller needs to poll **two sources**: GitHub (PR state) and Discord (agent replies). Both feed into the same `consume()` pipeline.

#### Source 1: GitHub (PR state changes)

Controller polls GitHub on each `tick()` based on current loop state:

```typescript
class GitHubPoller {
  private etag_cache: Map<string, string>;  // endpoint → last ETag

  /**
   * Called each tick for every active loop.
   * Checks only what's relevant to the current state.
   */
  async poll(loop: LoopState): Promise<LoopEvent[]> {
    const events: LoopEvent[] = [];

    switch (loop.state) {
      case "FIXING":
        // Waiting for coder push — check if head SHA changed
        const pr_data = await this.get(`/repos/${loop.repo}/pulls/${loop.pr}`);
        if (pr_data && pr_data.head.sha !== loop.head_sha) {
          events.push({
            type: "synchronize",
            pr: loop.pr,
            head_sha: pr_data.head.sha,
            payload: { old_sha: loop.head_sha, new_sha: pr_data.head.sha },
          });
        }
        break;

      case "REVIEWING":
        // Waiting for verdict — also monitor CI status
        const checks = await this.get(`/repos/${loop.repo}/commits/${loop.head_sha}/check-runs`);
        if (checks) {
          const required = this.getRequiredChecks(loop);
          const relevant = checks.check_runs.filter(c => required.includes(c.name));
          const all_done = relevant.every(c => c.status === "completed");
          if (all_done) {
            const all_pass = relevant.every(c => c.conclusion === "success");
            events.push({ type: all_pass ? "ci_passed" : "ci_failed", pr: loop.pr, head_sha: loop.head_sha });
          }
        }
        break;
    }

    return events;
  }

  /** Conditional GET with ETag — 304 = no API quota consumed. */
  private async get(url: string): Promise<any | null> {
    const headers: Record<string, string> = {};
    const etag = this.etag_cache.get(url);
    if (etag) headers["If-None-Match"] = etag;

    const resp = await fetch(url, { headers });
    if (resp.status === 304) return null;

    if (resp.headers.has("ETag")) {
      this.etag_cache.set(url, resp.headers.get("ETag")!);
    }
    return resp.json();
  }
}
```

**API calls per loop per tick:**

| Loop State | Calls | Endpoint |
|------------|-------|----------|
| FIXING | 1 | `GET /pulls/{pr}` (check head SHA) |
| REVIEWING | 1 | `GET /commits/{sha}/check-runs` |
| APPROVED | 0 | — |
| ESCALATED | 0 | — |

#### Source 2: Discord (Agent CompletionReports)

Controller monitors its `active_threads` for agent replies:

```
┌─────────────────────────────────────────────────────────┐
│  Discord Poller (runs every N seconds)                   │
│                                                         │
│  for each threadId in active_loop_threads:              │
│    GET /channels/{threadId}/messages?after={last_seen}  │
│    for each new message:                                │
│      if sender is bot AND contains loop-report marker:  │
│        parse CompletionReport                           │
│        consume(event)                                   │
│      else:                                              │
│        check for human override commands ("stop", etc.) │
│    update last_seen_message_id                          │
│                                                         │
│  Sleep poll_interval                                    │
└─────────────────────────────────────────────────────────┘
```

**Discord API endpoint:**

```
GET /channels/{thread_id}/messages?after={last_message_id}&limit=50
```

**Optimization — reduce unnecessary calls:**

| Technique | How |
|-----------|-----|
| Only poll active threads | `active_loop_threads` set — idle loops don't poll |
| Cursor-based | `?after={last_seen_id}` — only fetch new messages |
| Skip if no pending dispatch | If `pendingDispatchId == null`, skip that thread |
| Adaptive interval | Poll faster (5s) when a step was just dispatched, slow down (30s) after 2 min of silence |

**Alternative: Discord Gateway (WebSocket)**

If Controller already maintains a Discord WebSocket connection (e.g., it's a Discord bot), it receives `MESSAGE_CREATE` events in real-time — no polling needed:

```
Discord Gateway WebSocket
    → MESSAGE_CREATE event
        → threadId ∈ active_loop_threads?
            → Yes → parse & consume()
            → No  → ignore
```

| Mode | Latency | Complexity | Best for |
|------|---------|-----------|----------|
| HTTP Polling | 5-30s | Low (stateless) | Serverless, simple deploy |
| WebSocket Gateway | <1s | Higher (persistent conn) | Always-on bot process |

**Configuration:**

```toml
[loop.discord]
mode = "gateway"              # "polling" or "gateway"
poll_interval = 10            # seconds (only for polling mode)
adaptive_polling = true       # faster when step is pending
```

#### Unified Event Loop

Both sources feed into one processing loop:

```
┌────────────────────────────────────────────┐
│          CONTROLLER MAIN LOOP              │
│                                            │
│  loop {                                    │
│    // 1. Check GitHub (poll or webhook)    │
│    github_events = poll_github()           │
│                                            │
│    // 2. Check Discord threads             │
│    discord_events = poll_discord()         │
│    // (or receive via WebSocket)           │
│                                            │
│    // 3. Check timers                      │
│    timeout_events = check_timeouts()       │
│                                            │
│    // 4. Process all events                │
│    for event in github + discord + timeout │
│      loop_instance = lookup(event)         │
│      loop_instance.consume(event)          │
│                                            │
│    sleep(poll_interval)                    │
│  }                                         │
└────────────────────────────────────────────┘
```

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

### Message Routing (OAB Core Isolation)

The Loop Controller **does not modify OAB core**. OAB should not need to inspect every incoming message for loop callbacks. The routing design:

```
Discord Message arrives
       │
       ├── threadId ∈ active_loop_threads?
       │       │
       │       ├── Yes → Loop Controller handles (independent consumer)
       │       │
       │       └── No  → OAB Core handles (normal flow, zero overhead)
       │
       └── (parallel, not sequential — both can subscribe to Discord events)
```

**Implementation:** Loop Controller runs as a **separate gateway module** (or sidecar process) with its own Discord event subscription. It maintains an `active_threads: Set<threadId>` of threads it created for active loops. Messages in non-loop threads never touch loop logic.

```
┌──────────────────────────┐     ┌─────────────────────────────┐
│      OAB Core            │     │    Loop Controller           │
│                          │     │    (gateway module)          │
│  - Normal agent routing  │     │  - Subscribes only to its   │
│  - Knows nothing about   │     │    active_threads            │
│    loops                 │     │  - Filters by loopId marker  │
│  - Zero performance cost │     │  - Independent lifecycle     │
└──────────────────────────┘     └─────────────────────────────┘
           │                                │
           └──────── Discord Gateway ───────┘
                  (shared event stream)
```

**Why this works:**
- OAB core has **zero awareness** of loops — no code changes, no per-message overhead
- Loop Controller is **opt-in** — if not deployed, nothing changes
- Both consumers see the same Discord events; each filters for its own concern
- If Loop Controller crashes, OAB core is unaffected

## Loop Creation — How to Define and Trigger a Loop

A Loop can be created through three mechanisms, listed by priority:

### 1. Discord Command (ad-hoc, human-initiated)

A human mentions the controller agent with a `loop` command:

```
@超渡 loop start https://github.com/openabdev/openab/pull/42
@超渡 loop start PR#42
@超渡 loop start PR#42 --max-iterations 5 --reviewer @普渡 --coder @擺渡
```

Controller parses the command → resolves PR metadata from GitHub API → creates Loop instance → opens a dedicated Discord thread → dispatches first reviewer.

**Command grammar:**

```
loop start <pr_url | PR#number> [options]
loop stop <pr_url | PR#number> [reason]
loop status [pr_url | PR#number]     // show active loops or specific loop state

Options:
  --reviewer <agent_id>         // override default reviewer
  --coder <agent_id>            // override default coder
  --max-iterations <n>          // override default cap
  --token-budget <n>            // override default budget
```

### 2. GitHub Label (event-driven, automatic)

Adding a configured label to a PR triggers loop creation automatically:

```
PR gets label "auto-review" → GitHub webhook/poll → Controller detects → new Loop()
```

The label acts as the opt-in signal. Removing the label mid-loop triggers `loop.stop("label removed")`.

### 3. Loop Definition Files (`~/.openab/loop/*.toml`)

Following the same pattern as user cron, each `.toml` file in `~/.openab/loop/` defines **one loop template**. File exists = active. Add `enabled = false` to disable without deleting.

**Directory:** `~/.openab/loop/` (user-level) or `/etc/openab/loop/` (system-level)

**Convention:** `{name}.toml` — one file per loop definition.

```
~/.openab/loop/
├── pr-review.toml               # active
├── issue-implement.toml         # active
├── docs-review.toml             # active
└── deploy-verify.toml           # has enabled = false → inactive
```

**Example: `pr-review.toml`**

```toml
# ~/.openab/loop/pr-review.toml
name = "pr-review"
description = "Automated PR review → fix → re-review cycle"
enabled = true                          # default true; set false to disable

[trigger]
event = "pull_request.opened"
labels = ["auto-review"]
base_branches = ["main", "dev"]
exclude_authors = ["dependabot"]
exclude_paths = ["docs/**"]

[limits]
max_iterations = 3
token_budget = 50000

[timeouts]
review = "10m"
fix = "15m"
total = "60m"

[workers.reviewer]
agent_id = "1493128125402320996"        # 普渡
dispatch_via = "discord-mention"

[workers.coder]
agent_id = "1490365068863606784"        # 超渡
dispatch_via = "discord-mention"

[[steps]]
name = "review"
worker = "reviewer"
action = "review"
prompt_template = """
請 review PR：{pr_url}
HEAD: `{head_sha}` | Iteration: {iteration}/{max_iterations}
完成後回報 verdict + findings 到此 thread。
loopId: `{loop_id}` | dispatchId: `{dispatch_id}`
"""

[[steps]]
name = "fix"
worker = "coder"
action = "fix"
prompt_template = """
請修復 findings：
PR: {pr_url} | HEAD: `{head_sha}` | Iteration: {iteration}/{max_iterations}
{findings_json}
修完 push 後回報到此 thread。
loopId: `{loop_id}` | dispatchId: `{dispatch_id}`
"""

[safety]
path_denylist = [".github/workflows/**", "**/auth*", "**/*.key", ".env*"]
path_keywords = ["auth", "secret", "credential", "token"]
escalate_categories = ["security", "auth", "infra"]
escalate_risk_levels = ["high", "critical"]
```

**Example: `issue-implement.toml`**

```toml
# ~/.openab/loop/issue-implement.toml
name = "issue-implement"
description = "Issue → Coder implements → PR → Review → Merge"
enabled = true

[trigger]
event = "issues.labeled"
labels = ["auto-implement"]

[limits]
max_iterations = 3
token_budget = 80000

[timeouts]
implement = "20m"
review = "10m"
fix = "15m"
total = "90m"

[workers.coder]
agent_id = "1490365068863606784"
dispatch_via = "discord-mention"

[workers.reviewer]
agent_id = "1493128125402320996"
dispatch_via = "discord-mention"

[[steps]]
name = "implement"
worker = "coder"
action = "implement"
prompt_template = """
請實作 issue：{issue_url}
Title: {issue_title}
{issue_body}
開 PR 後回報 newPrNumber。loopId: `{loop_id}` | dispatchId: `{dispatch_id}`
"""

[[steps]]
name = "review"
worker = "reviewer"
action = "review"

[[steps]]
name = "fix"
worker = "coder"
action = "fix"
```

**Loading behavior:**

| Condition | Behavior |
|-----------|----------|
| File exists, no `enabled` field | Active (default true) |
| File exists, `enabled = true` | Active |
| File exists, `enabled = false` | Skipped — Controller ignores this template |
| File deleted | Template removed — no new loops from this trigger |
| File modified | Controller hot-reloads on next poll cycle |

**Hot-reload:** Controller watches `~/.openab/loop/*.toml` (via fs notify or periodic scan). Changes take effect within one poll interval — no restart needed.

**Precedence (unchanged):**

### Creation Priority

When multiple triggers match the same PR:

| Priority | Source | Behavior |
|----------|--------|----------|
| 1 (highest) | Discord command | Human intent always wins. Overrides any template. |
| 2 | config.toml template | First matching template applies. |
| 3 | Label trigger (no template match) | Uses system defaults. |

**Rule:** One PR = one active Loop at a time. If a Loop already exists for a PR, duplicate triggers are no-ops.

### Loop Creation Sequence

```
Trigger detected (command / label / template match)
        │
        ▼
  PR already has active Loop? ── yes ──▶ no-op (log duplicate)
        │
       no
        │
        ▼
  Resolve config (command args > template > defaults)
        │
        ▼
  Create Discord thread: "🔄 Loop: {repo}#{pr} — {pr_title}"
        │
        ▼
  Initialize LoopState (state: IDLE, iteration: 0)
        │
        ▼
  Register thread_id in active_loop_threads set
        │
        ▼
  Persist state to store (file / Redis)
        │
        ▼
  loop.start() → state: REVIEWING → dispatch(reviewer)
```

## Loop Variants

The Loop abstraction is generic. The PR Review Loop is one instantiation; other work types reuse the same Controller, interface, and safety boundaries with different state machines.

### Generic Loop Model

```typescript
interface LoopVariant {
  name: string;                       // "pr-review" | "issue-implement" | ...
  states: string[];                   // ordered states for this variant
  initial_state: string;              // first active state after start()
  transitions: TransitionRule[];      // state × event → next state
  discovery: DiscoveryConfig;         // how to find new work items
  roles: RoleBinding[];               // which agents fill which roles
}

interface TransitionRule {
  from: string;
  event: EventType;
  to: string;
  guard?: string;                     // condition that must be true
  action?: string;                    // side effect (dispatch, escalate, etc.)
}

interface RoleBinding {
  role: string;                       // "reviewer" | "coder" | "planner" | ...
  agent_id: string;                   // Discord UID
  instruction_template: string;       // what to tell this agent
}
```

### Variant 1: PR Review Loop (existing)

```
States: IDLE → REVIEWING ↔ FIXING → APPROVED → DONE
Trigger: PR opened / labeled
Roles: reviewer, coder
```

### Variant 2: Issue Implementation Loop

An issue is the starting point. Controller dispatches a Coder to implement, then enters the review loop.

**Extended state machine:**

```
  IDLE → CODING → REVIEWING ↔ FIXING → APPROVED → DONE
           │          ▲
           │          │ Coder reports PR created
           └──────────┘
```

**States:**

| State | Waiting for | Dispatch to |
|-------|-------------|-------------|
| CODING | Coder to open a PR | Coder |
| REVIEWING | Reviewer verdict | Reviewer |
| FIXING | Coder to push fix | Coder |
| APPROVED | Human to merge | — |

**Issue Loop config:**

```toml
[[loop.templates]]
name = "issue-implement"
variant = "issue"

[loop.templates.trigger]
labels = ["auto-implement"]
repos = ["openabdev/openab"]
exclude_labels = ["wontfix", "duplicate"]

[loop.templates.roles]
coder = "1490365068863606784"       # 超渡
reviewer = "1493128125402320996"    # 普渡

[loop.templates.instructions.coder_implement]
template = """
Implement issue #{issue_number}: {issue_title}

{issue_body}

Rules:
1. Create a feature branch from main.
2. Implement the solution.
3. Run tests.
4. Open a PR referencing this issue (Fixes #{issue_number}).
5. Report back with the PR URL.

Reply in thread: {thread_id}
"""
```

**Issue Loop flow:**

```
Issue #123 gets label "auto-implement"
        │
        ▼
  Controller discovers issue (scan or webhook)
  createLoop({ type: "issue", issue: 123 })
  state: CODING
  dispatch(coder, { action: "implement", issue: 123 })
        │
        ▼
  Coder works...
  opens PR #42 (body contains "Fixes #123")
  posts CompletionReport:
    {
      dispatch_id: "abc",
      status: "completed",
      action: "implement",
      pr_created: 42,
      head_sha: "def456",
      branch: "feat/issue-123"
    }
        │
        ▼
  Controller receives report:
    1. Verify PR exists: GET /repos/{repo}/pulls/42 → 200 ✓
    2. Verify PR references issue #123 ✓
    3. state: CODING → REVIEWING
    4. dispatch(reviewer, { pr: 42, head_sha: "def456" })
        │
        ▼
  (enters normal review loop: REVIEWING ↔ FIXING)
        │
        ▼
  LGTM → APPROVED → human merges → issue auto-closed → DONE
```

**Two event sources for CODING → REVIEWING transition:**

| Source | Signal | Priority |
|--------|--------|----------|
| Coder CompletionReport | Explicit: `{ pr_created: 42 }` | Primary (trusted) |
| GitHub `pull_request.opened` | Implicit: new PR references issue | Fallback (if Coder didn't report) |

**Resolution logic:**

```typescript
// Primary path: Coder reports back
on CompletionReport { pr_created } → verify PR exists → transition to REVIEWING

// Fallback path: Coder opened PR but didn't report (crash/timeout)
on tick() timeout for CODING step:
  → scan: GET /repos/{repo}/pulls?state=open&head={expected_branch_pattern}
  → if PR found AND references issue:
      → implicit completion → transition to REVIEWING
  → if no PR found:
      → escalate("Coder timeout, no PR found")
```

**Issue discovery (polling):**

```
GET /repos/{repo}/issues?state=open&labels=auto-implement&sort=created&direction=desc
→ for each issue not in active_loops:
    → createLoop({ type: "issue", issue: number })
```

### Variant Comparison

| | PR Review Loop | Issue Implementation Loop |
|---|---|---|
| Trigger | PR exists | Issue exists |
| First step | Review | Code |
| States | REVIEWING ↔ FIXING | CODING → REVIEWING ↔ FIXING |
| Coder opens PR? | No (PR already exists) | Yes |
| Controller verifies | SHA match | PR exists + references issue |
| Event sources | GitHub PR + Discord | GitHub Issue + PR + Discord |

## Loop Definition Files

Loop definitions follow the same pattern as usercron: **one file = one loop**. Place `.toml` files in `~/.openab/loop/` and they activate automatically on startup.

### File Layout

```
~/.openab/loop/
├── pr-review.toml          # active
├── issue-implement.toml    # active
├── docs-review.toml        # active
└── experiment.toml         # disabled inside file
```

### File Format

```toml
# ~/.openab/loop/pr-review.toml

name = "pr-review"
enabled = true                    # set to false to disable without deleting
variant = "pr-review"             # "pr-review" | "issue" | custom

[trigger]
labels = ["auto-review"]
repos = ["openabdev/openab"]
base_branches = ["main", "dev"]
exclude_authors = ["dependabot"]
exclude_paths = ["docs/**"]

[roles]
reviewer = "1493128125402320996"   # 普渡
coder = "1490365068863606784"      # 超渡

[limits]
max_iterations = 3
token_budget = 50000
timeout_review = "10m"
timeout_fix = "15m"
timeout_total = "60m"
retry_per_step = 1

[discovery]
mode = "polling"                   # "polling" | "webhook"
poll_interval = "60s"

[instructions.reviewer]
template = """
Review PR {pr_url} at commit `{head_sha}` (iteration {iteration}/{max_iterations}).
Focus: correctness, security, performance.
Verdict: LGTM ✅ or CHANGES REQUESTED ⚠️
Reply in thread: {thread_id}
"""

[instructions.coder]
template = """
Fix findings on branch `{branch}` at `{head_sha}`.
{findings}
Push when done. Reply in thread: {thread_id}
"""

[safety]
path_denylist = [".github/workflows/**", "**/auth*", ".env*"]
path_keywords = ["secret", "credential", "token"]

[[safety.escalation]]
category = "security"
risk_level = "*"
action = "escalate"

[[safety.escalation]]
category = "*"
risk_level = "critical"
action = "escalate"
```

### Issue Loop Example

```toml
# ~/.openab/loop/issue-implement.toml

name = "issue-implement"
enabled = true
variant = "issue"

[trigger]
labels = ["auto-implement"]
repos = ["openabdev/openab"]
exclude_labels = ["wontfix", "duplicate"]

[roles]
coder = "1490365068863606784"
reviewer = "1493128125402320996"

[limits]
max_iterations = 3
token_budget = 80000
timeout_coding = "30m"
timeout_review = "10m"
timeout_fix = "15m"
timeout_total = "90m"

[discovery]
mode = "polling"
poll_interval = "60s"

[instructions.coder_implement]
template = """
Implement issue #{issue_number}: {issue_title}
{issue_body}
Create branch, implement, open PR with "Fixes #{issue_number}".
Reply in thread: {thread_id}
"""

[instructions.coder_fix]
template = """
Fix findings on branch `{branch}` at `{head_sha}`.
{findings}
Reply in thread: {thread_id}
"""

[instructions.reviewer]
template = """
Review PR {pr_url} at `{head_sha}`.
Reply in thread: {thread_id}
"""
```

### Lifecycle

```
OAB startup / hot-reload
        │
        ▼
  Scan ~/.openab/loop/*.toml
        │
        ▼
  For each file:
    enabled = false? → skip
    enabled = true?  → register loop template → start discovery
        │
        ▼
  On file change (inotify / periodic rescan):
    file added    → register + start
    file removed  → stop active loops for this template
    enabled flipped → start / stop accordingly
```

### Comparison with usercron

| | usercron | loop |
|---|---|---|
| File pattern | `~/.openab/crons/*.cron.toml` | `~/.openab/loop/*.toml` |
| One file = | One scheduled job | One loop definition |
| Disable | `enabled = false` | `enabled = false` |
| Trigger | Schedule (cron expr) | Event (label, PR, issue) |
| Runtime state | Stateless (fire & forget) | Stateful (state machine per work item) |
| State storage | None | `~/.openab/loop/state/` |

### Runtime State Files (auto-managed)

Active loop instances write state to:

```
~/.openab/loop/state/
├── pr-review--42.json           # loop state for PR #42
├── pr-review--45.json           # loop state for PR #45
└── issue-implement--123.json    # loop state for issue #123
```

These are managed by the Controller. Users don't edit them.

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

### Loop Interface Specification

```typescript
// ─── Types ──────────────────────────────────────────────────────────────────

type State = "IDLE" | "REVIEWING" | "FIXING" | "APPROVED" | "ESCALATED" | "DONE";
type Duration = string;  // e.g. "10m", "15m", "60m"
type Timestamp = string; // ISO 8601
type Sha = string;       // git commit SHA
type Uuid = string;      // dispatch correlation ID

type EventType =
  | "pr_opened"        // new PR detected
  | "synchronize"      // new commits pushed to PR
  | "verdict"          // reviewer posted a verdict
  | "fix_complete"     // coder says done (optional, inferred from synchronize)
  | "ci_passed"        // all required checks green
  | "ci_failed"        // required check failed
  | "timeout"          // step timer expired (internal)
  | "human_override";  // human intervened

type Verdict = "LGTM" | "CHANGES_REQUESTED" | "INCOMPLETE";

type Severity = "🔴" | "🟡" | "🟢";
type Category = "logic" | "performance" | "style" | "security" | "auth" | "infra" | "config";
type RiskLevel = "low" | "medium" | "high" | "critical";

// ─── Data Structures ────────────────────────────────────────────────────────

interface LoopEvent {
  type: EventType;
  pr: number;
  head_sha: Sha;
  dispatch_id: Uuid;          // correlates request ↔ response
  in_reply_to?: Uuid;         // which dispatch this responds to
  timestamp: Timestamp;
  payload: VerdictPayload | SynchronizePayload | HumanPayload | {};
}

interface VerdictPayload {
  verdict: Verdict;
  findings: Finding[];
  tokens_consumed: number;
}

interface SynchronizePayload {
  old_sha: Sha;
  new_sha: Sha;
}

interface HumanPayload {
  command: "stop" | "resume" | "budget" | "skip";
  value?: string;             // e.g. new budget amount
}

interface Finding {
  id: number;                 // sequential within one dispatch
  severity: Severity;
  category: Category;
  risk_level: RiskLevel;
  file: string;
  line: number;
  issue: string;
  suggestion: string;
  fingerprint: string;        // sha256(file + ":" + normalized_issue)[:8]
}

interface StepRecord {
  step: "review" | "fix";
  dispatch_id: Uuid;
  head_sha: Sha;
  result: Verdict | "pushed" | "failed" | "timeout";
  tokens_consumed: number;
  timestamp: Timestamp;
}

// ─── Configuration ──────────────────────────────────────────────────────────

interface LoopConfig {
  pr: number;
  repo: string;                // "owner/repo"
  base_branch: string;         // target branch of the PR
  max_iterations: number;      // hard cap on review→fix cycles (default: 3)
  token_budget: number;        // total token limit across all steps (default: 50000)
  timeout: {
    review: Duration;          // max time for a single review step (default: "10m")
    fix: Duration;             // max time for a single fix step (default: "15m")
    total: Duration;           // max time for entire loop (default: "60m")
  };
  retry: {
    max_per_step: number;      // retries before escalation (default: 1)
  };
  safety_policy: SafetyPolicy;
  event_source: "webhook" | "polling";
  poll_interval?: Duration;    // only when event_source = "polling"
}

interface SafetyPolicy {
  path_denylist: string[];      // glob patterns — files Coder must NOT touch
  path_keywords: string[];      // substring match on file path
  escalation_rules: EscalationRule[];
}

interface EscalationRule {
  category: Category | "*";
  risk_level: RiskLevel | "*";
  action: "allow" | "escalate";
}

// ─── State ──────────────────────────────────────────────────────────────────

interface LoopState {
  state: State;
  iteration: number;
  head_sha: Sha;
  token_used: number;
  started_at: Timestamp;
  current_step_started: Timestamp;
  last_dispatch_id: Uuid;
  retries_this_step: number;
  history: StepRecord[];
  fingerprint_tracker: Map<string, number[]>;  // fingerprint → [iterations seen]
}

// ─── Loop Interface (the controller for one work item) ──────────────────────

interface Loop {
  // --- Properties (read-only from outside) ---
  readonly config: LoopConfig;
  readonly state: LoopState;

  // --- Lifecycle ---

  /** Initialize and dispatch first reviewer. IDLE → REVIEWING. */
  start(): void;

  /** Terminate the loop from any state. → DONE. */
  stop(reason: string): void;

  /** Resume from ESCALATED after human resolves the issue. */
  resume(instruction?: string): void;

  // --- Event Ingestion ---

  /**
   * Receive an external event.
   * - Deduplicates by (pr, head_sha, dispatch_id).
   * - Validates SHA freshness (stale SHA → discard).
   * - Enqueues for processing.
   * Returns: true if accepted, false if deduplicated/stale.
   */
  consume(event: LoopEvent): boolean;

  // --- Core Logic (called internally after consume) ---

  /**
   * State transition function.
   * Evaluates current state × event → determines next state + side effects.
   * This is the heart of the state machine.
   */
  transition(event: LoopEvent): void;

  /**
   * Pre-dispatch safety gate. Called before every dispatch to Coder.
   * Checks:
   *   - iteration < max_iterations
   *   - token_used + estimate < token_budget
   *   - no findings violate safety_policy (category/risk escalation)
   *   - no findings touch path_denylist
   *   - no fingerprint repeated for 2+ consecutive iterations
   * Returns: { allowed: true } or { allowed: false, reason: string }
   */
  validate(findings: Finding[]): ValidationResult;

  /**
   * Dispatch work to an agent.
   * - Generates a new dispatch_id (UUID v4).
   * - Sends structured payload to target via configured channel (Discord mention).
   * - Records dispatch in state (last_dispatch_id, current_step_started).
   * - Starts the step timer.
   */
  dispatch(target: DispatchTarget, payload: DispatchPayload): Uuid;

  /**
   * Escalate to human. Freezes the loop.
   * - Sets state → ESCALATED.
   * - Sends notification with reason + loop summary.
   * - Only resume() or stop() can move out of this state.
   */
  escalate(reason: string): void;

  /**
   * Periodic heartbeat. Called by external scheduler (cron / poll loop).
   * Checks:
   *   - Has current step exceeded its timeout?
   *   - (In polling mode) Are there new GitHub events to fetch?
   * If timeout → retry or escalate based on retry policy.
   */
  tick(): void;
}

// ─── Supporting Types ───────────────────────────────────────────────────────

interface ValidationResult {
  allowed: boolean;
  reason?: string;  // only present when allowed = false
}

interface DispatchTarget {
  agent_id: string;           // Discord UID of the target agent
  role: "reviewer" | "coder";
}

interface DispatchPayload {
  action: "review" | "fix";
  pr: number;
  repo: string;
  branch: string;
  head_sha: Sha;
  iteration: number;
  max_iterations: number;
  findings?: Finding[];       // only for action = "fix"
  thread_id: string;          // Discord thread ID — agent MUST reply here
  dispatch_id: Uuid;          // correlation ID for this dispatch
  instruction: string;        // rendered task prompt — tells agent exactly what to do
}

// ─── Dispatch Instructions ──────────────────────────────────────────────────
//
// The `instruction` field is the actual prompt the agent receives.
// It is rendered at dispatch time from a template defined in config.toml.
// Templates use {placeholder} syntax for variable interpolation.
//

interface InstructionTemplate {
  role: "reviewer" | "coder";
  template: string;           // template with {placeholders}
}

// Available placeholders (resolved at dispatch time):
//
//   {pr}              — PR number
//   {repo}            — "owner/repo"
//   {branch}          — PR head branch name
//   {head_sha}        — current commit SHA
//   {iteration}       — current iteration number
//   {max_iterations}  — configured cap
//   {findings}        — formatted findings list (for coder)
//   {thread_id}       — Discord thread to reply in
//   {dispatch_id}     — correlation ID to echo back
//   {pr_title}        — PR title
//   {pr_url}          — full PR URL
//   {diff_url}        — URL to the PR diff
```

**config.toml — Instruction Templates:**

```toml
[[loop.templates]]
name = "pr-review"
reviewer = "1493128125402320996"
coder = "1490365068863606784"
max_iterations = 3
token_budget = 50000

[loop.templates.instructions.reviewer]
template = """
Review PR {pr_url} at commit `{head_sha}` (iteration {iteration}/{max_iterations}).

Scope:
- Read the diff and assess correctness, security, performance.
- Post your verdict as: `LGTM ✅` or `CHANGES REQUESTED ⚠️`
- If CHANGES REQUESTED, list findings in structured format:
  severity | category | risk_level | file | line | issue | suggestion

Response format:
<!-- oab-loop-report:{{"dispatch_id":"{dispatch_id}","agent_id":"YOUR_ID","status":"completed"}} -->
Your review content here...

Reply in thread: {thread_id}
"""

[loop.templates.instructions.coder]
template = """
Fix the findings below on branch `{branch}` at commit `{head_sha}` (iteration {iteration}/{max_iterations}).

PR: {pr_url}

Findings to fix:
{findings}

Rules:
1. Only modify files mentioned in the findings.
2. Run tests before pushing. If tests fail, report status: "failed".
3. Do NOT touch files outside the findings scope.
4. Push to branch `{branch}` when done.

Response format:
<!-- oab-loop-report:{{"dispatch_id":"{dispatch_id}","agent_id":"YOUR_ID","status":"completed"}} -->
Summary of changes...

Reply in thread: {thread_id}
"""
```

**Rendering example:**

Template input:
```
Review PR {pr_url} at commit `{head_sha}` (iteration {iteration}/{max_iterations}).
```

Rendered output (what agent actually receives):
```
Review PR https://github.com/openabdev/openab/pull/42 at commit `abc1234` (iteration 2/3).
```

**Instruction resolution order:**

| Priority | Source | When used |
|----------|--------|-----------|
| 1 | Discord command `--instruction "..."` | Ad-hoc override |
| 2 | `config.toml` template matching the PR | Normal operation |
| 3 | Built-in defaults (hardcoded in Controller) | Fallback if no template |

The Controller MUST ship with sensible built-in defaults so loops work even without explicit templates in config.toml.

// ─── Completion Report ──────────────────────────────────────────────────────
//
// When an agent finishes a dispatched task, it MUST post a CompletionReport
// back to the SAME thread_id it received in the DispatchPayload.
// The Loop Controller listens for these reports to drive state transitions.
//

interface CompletionReport {
  // --- Routing (how Controller finds this report) ---
  dispatch_id: Uuid;          // echo back the dispatch_id from DispatchPayload
  thread_id: string;          // echo back thread_id — proves continuity
  in_reply_to: Uuid;          // same as dispatch_id (explicit link)

  // --- Identity ---
  agent_id: string;           // Discord UID of the reporting agent
  role: "reviewer" | "coder";

  // --- Result ---
  status: "completed" | "failed" | "partial";
  verdict?: Verdict;          // only for role = "reviewer"
  findings?: Finding[];       // only for role = "reviewer", when CHANGES_REQUESTED
  commits_pushed?: Sha[];     // only for role = "coder", the new commit(s)

  // --- Metadata ---
  head_sha: Sha;              // the SHA this work was performed against
  tokens_consumed: number;    // for budget tracking
  duration_ms: number;        // wall-clock time spent
  timestamp: Timestamp;

  // --- Error (when status = "failed") ---
  error?: {
    reason: string;           // human-readable explanation
    recoverable: boolean;     // can Controller retry?
  };
}
```

### Dispatch ↔ Report Flow

```
Controller                          Agent (new session)
    │                                     │
    │  dispatch(target, payload)          │
    │  payload includes:                  │
    │    - dispatch_id (UUID)             │
    │    - thread_id (Discord thread)     │
    │    - head_sha, findings, etc.       │
    │─────────────────────────────────────▶│
    │   (Discord mention in thread)       │
    │                                     │  agent opens new session
    │                                     │  agent does work...
    │                                     │
    │◀─────────────────────────────────────│
    │   CompletionReport posted to        │
    │   SAME thread_id, echoing           │
    │   dispatch_id                       │
    │                                     │
    │  Controller.consume() picks it up   │
    │  matches by dispatch_id + thread_id │
    │  → transition()                     │
    │                                     │
```

### Thread Continuity Rules

1. **Controller creates one Discord thread per Loop** (e.g., "Loop: PR #42"). All dispatches and reports happen in this thread.
2. **DispatchPayload always includes `thread_id`** — agent knows where to reply.
3. **Agent MUST post CompletionReport to the same `thread_id`** — this is how Controller receives the result even though agent's session is new.
4. **`dispatch_id` is the correlation key** — Controller matches report to pending dispatch via `(dispatch_id, thread_id)`.
5. **If agent cannot complete** — it still MUST post a report with `status: "failed"` so Controller can retry or escalate (instead of waiting for timeout).

### Why This Works with New Sessions

| Concern | Solution |
|---------|----------|
| Agent has no memory of previous iterations | `DispatchPayload` carries all context needed (findings, head_sha, iteration) |
| Agent doesn't know where to reply | `thread_id` in payload tells it exactly where |
| Controller can't find the response | Matches on `dispatch_id` echoed back in report |
| Multiple agents in same thread | Each report carries `agent_id` + `dispatch_id` — no ambiguity |
| Agent crashes without reporting | Controller's `tick()` detects timeout → retry or escalate |

### Message Routing — Callback Detection

OAB processes every incoming Discord message. To avoid adding overhead to normal message flow, callback detection uses a two-layer fast-path filter:

```
Discord message arrives
        │
        ▼
  ┌──────────────────────────────────┐
  │ Layer 1: thread_id ∈ active_loops? │  ← HashSet lookup, O(1)
  └──────────────┬───────────────────┘
                 │
          no ────┴──── yes
          │              │
          ▼              ▼
   normal flow    ┌─────────────────────────────────────┐
   (zero cost)    │ Layer 2: has structured report marker? │
                  └──────────────┬──────────────────────┘
                                 │
                          no ────┴──── yes
                          │              │
                          ▼              ▼
                   human chat in   Controller.consume()
                   loop thread     → parse → transition
                   (ignore)
```

**Layer 1 — Active Loop Thread Set**

Controller maintains: `active_loop_threads: Set<thread_id>`

- Added when `Loop.start()` creates a thread
- Removed when loop reaches DONE
- 99%+ of messages fail this check → zero additional processing

**Layer 2 — Structured Report Marker**

Agent completion reports MUST include a machine-parseable marker at the start of the message:

```
<!-- oab-loop-report:{"dispatch_id":"abc-123","agent_id":"149...","status":"completed"} -->

(human-readable content below)
LGTM ✅ — code looks good, no issues found.
```

Detection: `message.content.startsWith("<!-- oab-loop-report:")`

**Why two layers:**

| Layer | Filters out | Cost |
|-------|-------------|------|
| 1 (thread set) | All messages in non-loop threads | 1 HashSet lookup |
| 2 (marker prefix) | Human chat within loop threads | 1 string prefix check |

**Impact on normal OAB message flow: effectively zero.**

### Methods Summary

| Method | Input | Output | Mutates State? | Side Effects |
|--------|-------|--------|----------------|--------------|
| `start()` | — | void | Yes → REVIEWING | Dispatches reviewer, starts timer |
| `stop(reason)` | string | void | Yes → DONE | Logs termination |
| `resume(instruction?)` | string? | void | Yes → prev state | Re-dispatches last step |
| `consume(event)` | LoopEvent | boolean | No | Enqueues event |
| `transition(event)` | LoopEvent | void | Yes | May call validate/dispatch/escalate |
| `validate(findings)` | Finding[] | ValidationResult | No | Pure check |
| `dispatch(target, payload)` | target, payload | Uuid | Yes | Sends message, starts timer |
| `escalate(reason)` | string | void | Yes → ESCALATED | Notifies human |
| `tick()` | — | void | Maybe | Checks timeout, polls events |

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

## Loop Template Examples

### Example 1: PR Review Loop (primary use case)

```
Trigger: PR opened with label "auto-review"
Steps:   review → fix → review → ... → approved
```

(Fully defined in the Decision section above.)

### Example 2: Issue Implementation Loop

Demonstrates that Loops are generic — not limited to PR review.

**Trigger:** New issue with label `auto-implement`

**Inner states:**

```
IDLE → IMPLEMENTING → VERIFYING → REVIEWING → FIXING → ... → DONE
                          │
                          └── "Coder opened PR, Controller verifies it exists"
```

**Template:**

```toml
[[loop]]
name = "issue-implement"
description = "Issue → Coder implements → PR → Review → Fix → Merge"

[loop.trigger]
event = "issues.labeled"
conditions = { label = "auto-implement" }

[loop.limits]
max_iterations = 3
token_budget = 80000

[[loop.steps]]
name = "implement"
worker = "coder"
action = "implement"
timeout = "20m"
prompt_template = """
請實作這個 issue：{issue_url}
Title: {issue_title}
Body:
{issue_body}

完成後：
1. 開一個 PR (base: main)
2. PR body 寫上 `Closes #{issue_number}` 和 `loopId: {loop_id}`
3. 回報 CompletionReport 到這個 thread，帶上 newPrNumber 和 newHeadSha
"""

[[loop.steps]]
name = "review"
worker = "reviewer"
action = "review"
timeout = "10m"

[[loop.steps]]
name = "fix"
worker = "coder"
action = "fix"
timeout = "15m"
```

**Completion verification (AND logic):**

```
Coder "完成" = BOTH conditions true:
  ✅ CompletionReport received (agent says done, includes newPrNumber)
  ✅ GitHub API confirms PR exists (GET /repos/.../pulls/{newPrNumber} → 200)

Only then → state transition
```

**Flow diagram:**

```
Issue #100 (labeled "auto-implement")
       │
       ▼
Controller: new Loop("issue-implement", target: issue #100)
       │
       ▼ dispatch(coder, "implement")
       │
   ┌───┴────────────────────────────────────┐
   │  Coder (new session):                   │
   │    - reads issue                        │
   │    - writes code                        │
   │    - git push → opens PR #101           │
   │    - posts CompletionReport:            │
   │        { newPrNumber: 101,              │
   │          newHeadSha: "def456" }         │
   └───┬────────────────────────────────────┘
       │
       ▼ Controller consume(CompletionReport)
       │
       ├── Verify: GET /repos/.../pulls/101 → exists? ✅
       │
       ▼ State: IMPLEMENTING → REVIEWING
       │
       ▼ dispatch(reviewer, "review PR #101")
       │
       │  (now behaves like a normal PR review loop)
       │  review → fix → review → ... → LGTM
       │
       ▼
    APPROVED → notify human → await merge → DONE
```

**Fallback (agent didn't report):**

```
Timeout (20 min, no CompletionReport)
    → Controller polls GitHub: any new PR with "loopId: {loop_id}" in body?
        → Found PR #101 → reconcile, treat as completed
        → Not found → retry once, then escalate to human
```

### Key Difference from PR Review Loop

| | PR Review Loop | Issue Implementation Loop |
|--|---|---|
| Trigger | PR exists | Issue exists, PR doesn't exist yet |
| First step output | Verdict (text) | New PR (artifact) |
| Verification | SHA match | PR existence check |
| Inner states | REVIEWING ↔ FIXING | IMPLEMENTING → VERIFYING → REVIEWING ↔ FIXING |
| Longer timeout | 10-15 min/step | 20 min for implement (more work) |

This shows the Loop abstraction is **generic** — swap the template, same Controller runs a completely different workflow.

---

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
- Maintains file-based state per PR (`~/.openab/loop/state/pr-{number}.json`)
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
