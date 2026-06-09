# ADR: Review Loop Delegation Pattern

- **Status:** Accepted
- **Date:** 2026-06-09
- **Author:** @pahud.hsieh

---

## 1. Context & Motivation

The Review Loop (Event → Controller → Coder → Controller → Reviewer → loop) needs to support multi-reviewer scenarios without increasing Controller complexity.

Naïve approaches (Controller fans out to N reviewers, manages barrier/timeout, aggregates findings) leak multi-party coordination into the Controller — making it harder to reason about the core loop, debug synchronization issues, and evolve the reviewer topology independently.

---

## 2. Design: Delegation Pattern

The Controller always dispatches to **one** Lead Reviewer. The Lead Reviewer internally decides whether to fan-out to additional reviewers.

```
┌────────────┐
│ Controller │
└─────┬──────┘
      │  dispatch(review_request)
      ▼
┌──────────────────────────────────────────────┐
│          Lead Reviewer (超渡)                 │
│                                              │
│  ┌─── internal decision ───────────────┐     │
│  │ Need more eyes?                     │     │
│  │   YES → fan-out to 法師:            │     │
│  │     ├─ 普渡 (正確性)                │     │
│  │     ├─ 擺渡 (架構)                  │     │
│  │     ├─ 口渡 (安全/CI)               │     │
│  │     └─ 覺渡 (文件/UX)              │     │
│  │   NO → review alone                 │     │
│  └─────────────────────────────────────┘     │
│                                              │
│  Aggregate findings → deduplicate → verdict  │
└─────┬────────────────────────────────────────┘
      │  ReviewResult { verdict, findings[] }
      ▼
┌────────────┐
│ Controller │  → LGTM? done : loop back to Coder
└────────────┘
```

### Controller Contract (unchanged)

```
dispatch(lead_reviewer, review_request) → ReviewResult {
  verdict: LGTM | CHANGES_REQUESTED,
  findings: Vec<Finding>
}
```

The Controller neither knows nor cares how many sub-reviewers were involved. Single reviewer and multi-reviewer produce the same `ReviewResult` interface.

---

## 3. Key Properties

| Property | Guarantee |
|----------|-----------|
| Controller simplicity | Always talks to 1 entity; no barrier, no timeout logic |
| Encapsulation | Fan-out/barrier/dedup complexity lives inside Lead Reviewer |
| Topology independence | Lead Reviewer can change who it consults without Controller changes |
| Backward compatible | Single-reviewer loop works unchanged (Lead Reviewer just doesn't fan-out) |
| Verdict rule | Any 🔴/🟡 from any sub-reviewer → CHANGES_REQUESTED |

---

## 4. Lead Reviewer Responsibilities

1. **Decide** whether to fan-out (based on PR scope, file types, risk level)
2. **Dispatch** review angles to sub-reviewers (if fan-out)
3. **Wait** for all sub-reviewers (with timeout fallback)
4. **Deduplicate** findings (by fingerprint — same file + same issue = 1 finding)
5. **Aggregate** into a single `ReviewResult` and return to Controller

---

## 5. When to Fan-out

The Lead Reviewer uses heuristics to decide:

| Signal | Action |
|--------|--------|
| PR touches auth/IAM | Always include security reviewer |
| PR touches docs only | Solo review sufficient |
| PR > 300 lines changed | Fan-out to 2+ angles |
| PR touches multiple subsystems | Fan-out with relevant angles |
| Trivial fix (typo, version bump) | Solo review |

---

## 6. Relationship to Existing Architecture

This pattern formalizes what our 法師 team already does in practice:

- **超渡法師** = Lead Reviewer (coordinator)
- **法師團隊** = Sub-reviewers (普渡, 擺渡, 覺渡, 口渡, X渡)
- **Discord thread** = internal coordination channel (invisible to Controller)
- **GitHub comment** = the single `ReviewResult` returned to Controller

---

## 7. Symmetric Application: Lead Coder

The same pattern applies to the Coder role:

```
Controller
  │  dispatch(code_request)
  ▼
┌──────────────────────────────────────────┐
│         Lead Coder                        │
│                                          │
│  ┌─── split work ─────────────────┐      │
│  │  Sub-coder A → backend changes │      │
│  │  Sub-coder B → frontend changes│      │
│  │  Sub-coder C → tests           │      │
│  └────────────────────────────────┘      │
│                                          │
│  Merge → resolve conflicts → push        │
└─────┬────────────────────────────────────┘
      │  CodeResult { files_changed, summary }
      ▼
Controller
```

Controller's full loop is therefore:

```
Controller ←→ Lead Coder    (1:1, internal fan-out invisible)
Controller ←→ Lead Reviewer  (1:1, internal fan-out invisible)
```

---

## 8. Thread Model

**Two levels of isolation:**
- **Inter-role: isolated** — Coder thread and Reviewer thread are separate. They cannot see each other.
- **Intra-role: shared** — Lead and members within the same role share one thread and can freely communicate.

```
┌─────────────────────────────────────────────────────┐
│ Controller                                           │
│                                                     │
│  Only sees:                                         │
│    ← Lead Coder's CodeResult                        │
│    ← Lead Reviewer's ReviewResult                   │
└──────┬──────────────────────────────┬───────────────┘
       │                              │
       ▼                              ▼
┌─── Thread A (Coder) ─────┐  ┌─── Thread B (Reviewer) ────┐
│ Lead Coder                │  │ Lead Reviewer               │
│   ↕ Sub-coder 1          │  │   ↕ Reviewer A (正確性)     │
│   ↕ Sub-coder 2          │  │   ↕ Reviewer B (架構)       │
│   ↕ Sub-coder 3          │  │   ↕ Reviewer C (安全)       │
│                           │  │                             │
│ (members see each other,  │  │ (members see each other,    │
│  can discuss freely)      │  │  can discuss freely)        │
└───────────────────────────┘  └─────────────────────────────┘
```

### Rules

| Rule | Description |
|------|-------------|
| Lead speaks for the group | Controller only listens to Lead's final report |
| Members are invisible to Controller | Their messages stay within the thread |
| Cross-thread isolation | Coder thread cannot see Reviewer thread, and vice versa |
| Intra-thread freedom | Lead and members discuss, debate, iterate freely |
| Structured handoff | Controller passes data between threads as structured objects (not paraphrased text) |

### Why This Model

| Concern | Benefit |
|---------|---------|
| Internal collaboration | Members discuss with Lead → better quality output |
| Controller simplicity | Always 1:1 with Lead — no multi-party protocol |
| Context isolation | Coders don't get polluted by review chatter |
| Token efficiency | Each thread carries only role-relevant history |
| Accountability | Lead owns the final output — clear responsibility |

### Controller as Mediator

Controller transfers structured data between threads:
- Reviewer finding → `Finding { severity, description, location }` → passed verbatim to Coder thread
- Coder result → `CodeResult { files_changed, commit_sha }` → passed to Reviewer thread as new diff

No paraphrasing. Structured data transfer only.

---

## 9. Role Thread Configuration

Each role must define: (1) who is the Lead, (2) which thread they operate in.

```toml
[roles]
coder = "1496097857940361326"      # Lead Coder Discord UID
reviewer = "1490365068863606784"   # Lead Reviewer Discord UID

[roles.threads]
coder = ""       # empty = auto-create on first dispatch
reviewer = ""    # empty = auto-create on first dispatch
```

### Controller's View

| What Controller knows | What Controller doesn't know |
|----------------------|------------------------------|
| Lead Coder ID | Who the sub-coders are |
| Lead Reviewer ID | Who the sub-reviewers are |
| Coder thread_id | What's discussed inside |
| Reviewer thread_id | What's discussed inside |

### Thread Lifecycle

1. Loop starts → Controller creates 2 threads (one per role)
2. Controller dispatches to Lead in the correct thread
3. Lead internally recruits members into their thread (optional)
4. Lead posts `CompletionReport` → Controller reads it
5. Loop ends → threads persist as audit trail

---

## 10. Phase Plan

| Phase | Thread Model | Config |
|-------|-------------|--------|
| **Phase 1 (current)** | Single shared thread per loop — Controller, Coder, and Reviewer all operate in the same Discord thread | `thread_id` (singular) in `LoopInstance` |
| **Phase 2** | Dual isolated threads — Coder thread + Reviewer thread, Controller mediates via structured data | `[roles.threads]` config with per-role thread_id |

Phase 1 validates the core loop mechanics (state machine, dispatch, safety policy). Phase 2 adds thread isolation when the single-thread model is proven stable.

---

## 11. Decision

1. **Delegation Pattern** — Controller dispatches to one Lead per role; internal fan-out is encapsulated.
2. **Symmetric** — applies to both Lead Reviewer and Lead Coder.
3. **Session isolation** — Coder and Reviewer operate in separate sessions; Controller mediates all cross-role communication via structured data.
