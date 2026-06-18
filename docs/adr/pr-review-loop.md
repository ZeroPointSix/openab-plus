# ADR: OpenAB PR Review Loop

**Status:** Proposed  
**Date:** 2026-06-17  
**Author:** chaodu-agent

## Context

OpenAB's PR review workflow currently relies on manual triggers — a human @mentions the review bot in Discord to initiate a review. This works well for ad-hoc reviews but does not scale for repositories with frequent PR activity. Maintainers want automated PR reviews that:

1. Trigger automatically when a PR is opened or updated
2. Show review status as a commit status (🟡 pending → ✅/❌ complete)
3. Preserve the full review process in a Discord thread for auditability
4. Post a single aggregated comment on the PR (hiding previous comments)
5. Work with the existing OpenAB agent running on ECS Fargate (long-lived)

The agent should not need to be ephemeral — it stays running and receives review requests like any other Discord message.

## Decision

Use a **GitHub Action → Discord Webhook → OpenAB (ECS)** architecture with GitHub Commit Status API for check status feedback.

### Why Commit Status API (not Check Runs)

Check Runs API requires a GitHub App with `checks:write` permission. Commit Status API works with a standard PAT or fine-grained token (`commit statuses: write`), which the agent already has via `gh` CLI auth. This avoids creating an additional GitHub App solely for status reporting.

### Why Discord Webhook

- Simplest setup — only one secret (webhook URL), no Bot Token management
- Webhook messages posted to a channel will trigger OpenAB's existing message pipeline via @mention
- OpenAB auto-creates a thread for the conversation (existing behavior)

### Configuration Prerequisites

Discord webhook messages are flagged `author.bot == true` at the API level. OpenAB's Discord adapter defaults to `allow_bot_messages: "off"`, which silently drops bot messages. For this automation to work, the deployment **must** configure one of:

1. Set `allow_bot_messages: "mentions"` — allows bot messages that @mention the agent
2. Add the webhook's author ID to `trusted_bot_ids`

These settings are configured in the OpenAB ECS task definition's environment variables or the agent's runtime configuration file.

**Example (ECS environment variable):**
```
DISCORD_ALLOW_BOT_MESSAGES=mentions
```

**Example (config.toml):**
```toml
[discord]
allow_bot_messages = "mentions"
```

Without this, the webhook @mention will be ignored and reviews will never trigger.

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│  GitHub: PR opened / synchronize / ready_for_review / labeled        │
└───────────────────────────┬─────────────────────────────────────────┘
                            │ triggers
                            ▼
┌─────────────────────────────────────────────────────────────────────┐
│  GitHub Action (.github/workflows/pr-bot-review.yml)                │
│                                                                     │
│  1. POST /repos/{owner}/{repo}/statuses/{sha}                       │
│     state: "pending", context: "OpenAB PR Review"                   │
│                                                                     │
│  2. Discord Webhook:                                                │
│     → POST to webhook URL with "@bot review <PR_URL>"              │
│                                                                     │
│  3. Job exits (fire-and-forget)                                     │
└───────────────────────────┬─────────────────────────────────────────┘
                            │ Discord message
                            ▼
┌─────────────────────────────────────────────────────────────────────┐
│  OpenAB Agent (ECS Fargate, long-lived)                             │
│                                                                     │
│  Receives @mention → opens agent session (auto-creates thread)      │
│  → Delegates to reviewer team (angle-based review)                  │
│  → Collects findings in Discord thread                              │
│  → Aggregates into single review comment                            │
└───────────────────────────┬─────────────────────────────────────────┘
                            │ review complete
                            ▼
┌─────────────────────────────────────────────────────────────────────┐
│  Post Results to GitHub                                             │
│                                                                     │
│  1. Minimize all previous chaodu-agent comments (GraphQL)           │
│  2. Post aggregated review comment (gh pr comment)                  │
│  3. Update commit status:                                           │
│     → "success" if LGTM ✅                                          │
│     → "failure" if CHANGES REQUESTED ⚠️                             │
└─────────────────────────────────────────────────────────────────────┘
```

## Review Loop (Auto-Fix Cycle)

The architecture supports a closed-loop review cycle:

```
                    ┌─────────────────────────────────────────┐
                    │                                         │
                    ▼                                         │
┌──────────┐    ┌─────────────────┐     ┌──────────────┐      │
│  PR push │───▶│  GitHub Action   │───▶│  OpenAB      │      │
│          │    │  (set pending)   │    │  Review      │      │
└──────────┘    └─────────────────┘     └──────┬───────┘      │
                                              │               │
                                    ┌─────────┴─────────┐     │
                                    │                   │     │
                                    ▼                   ▼     │
                             ┌────────────┐    ┌────────────┐ │
                             │  LGTM ✅   │    │ CHANGES    │ │
                             │            │    │ REQUESTED  │ │
                             └─────┬──────┘    └─────┬──────┘ │
                                   │                 │        │
                                   ▼                 ▼        │
                             ┌────────────┐    ┌────────────┐ │
                             │  status:   │    │ Auto-fix   │ │
                             │  success   │    │ commit +   │─┘
                             │  (done)    │    │ push       │
                             └────────────┘    └────────────┘
                                              (re-triggers Action)
```

By default, the agent **only reviews and reports findings** — it does not push fixes automatically. The auto-fix loop is only activated when a human (maintainer) explicitly requests it (e.g. "fix and push" or option 4️⃣ in the post-review menu).

When explicitly requested:

1. Agent fixes the code directly on the PR branch
2. Commits and pushes the fix
3. The `synchronize` event re-triggers the workflow, starting a new review cycle
4. Repeat until LGTM or max iterations reached

### Safeguards

- **Max iterations** — agent enforces a soft cap (3 cycles per auto-fix request) to prevent runaway fixes within a single session. The workflow's circuit breaker (30 cycles) is a hard cap across the entire PR lifetime, catching edge cases where the agent cap is bypassed (e.g. multiple maintainer requests).
- **Human-only issues** — if findings require design decisions or are ambiguous, the agent requests human input instead of auto-fixing
- **Commit attribution** — auto-fix commits are authored by `chaodu-agent` with a clear prefix (e.g. `fix(review):`) so the loop is auditable

### When Auto-Fix Is Skipped

- Any 🔴 Critical finding (correctness, security) — requires human judgment
- Ambiguous 🟡 findings where multiple valid solutions exist
- Maintainer explicitly opts out of auto-fix for the PR

## Dedup & Performance

Rapid pushes can cause multiple review requests for the same PR. The system deduplicates at two layers:

### Layer 1: GitHub Actions (Concurrency Group)

```yaml
concurrency:
  group: pr-review-${{ github.event.pull_request.number }}
  cancel-in-progress: true
```

A new push cancels any in-flight Action run for the same PR. If the webhook has not yet been sent, only the latest SHA triggers a review.

### Layer 2: Agent-Side SHA Validation

If the webhook was already delivered before cancellation, the agent receives a stale request. To handle this:

1. Agent extracts `__commit: <SHA>__` from the trigger message
2. Agent queries current PR HEAD: `gh pr view <N> --json headRefOid --jq .headRefOid`
3. If request SHA ≠ HEAD → skip review, respond "Superseded by newer commit"
4. If request SHA = HEAD → proceed with normal review

This prevents wasting API tokens and reviewer compute on commits that are no longer relevant.

### Cost Impact

Without dedup, N rapid pushes could trigger N full reviews (~5 LLM calls each for angle-based delegation). With both layers active, at most 1 review runs per push burst.

## Implementation Plan

### Phase 1: GitHub Action Workflow

> **Canonical source:** [`.github/workflows/pr-bot-review.yml`](../../.github/workflows/pr-bot-review.yml)
>
> Refer to the workflow file for the current implementation. Key design points:

- **Trigger events:** `opened`, `synchronize`, `ready_for_review`, `labeled`
- **Concurrency group:** per-PR number with `cancel-in-progress: true`
- **Guard condition:** skips drafts, untrusted authors, irrelevant labels, and PRs with `review-limit-reached` label
- **Steps:** circuit breaker check → set pending status → trigger Discord webhook → error fallback on failure

### Phase 2: Agent Callback (Status Update)

After the agent posts the final PR comment, update the commit status (using the comment's `html_url` from the API response as `target_url` so "Details" links directly to the review):

```bash
# LGTM
gh api repos/OWNER/REPO/statuses/SHA \
  -f state="success" \
  -f context="OpenAB PR Review" \
  -f description="LGTM ✅" \
  -f target_url="<comment_html_url>"

# Changes Requested
gh api repos/OWNER/REPO/statuses/SHA \
  -f state="failure" \
  -f context="OpenAB PR Review" \
  -f description="Changes Requested ⚠️" \
  -f target_url="<comment_html_url>"
```

### Phase 3: Branch Protection

Add `OpenAB PR Review` as a required status check in branch protection rules. This enforces that PRs cannot merge until the review completes successfully.

**Setup:** Repository Settings → Branches → Branch protection rules → Edit `main` → Require status checks to pass before merging → Add `OpenAB PR Review` to the required checks list.

## Token & Permissions

| Secret | Purpose | Minimum Permission |
|--------|---------|-------------------|
| `GITHUB_TOKEN` (Actions) | Set pending status + circuit breaker label | `statuses: write` + `issues: write` |
| `OAB_REVIEW_ACTION_WEBHOOK` | Post review request to Discord channel | Webhook URL (channel-scoped) |
| Agent's `gh` auth (PAT) | Post comment + update status + push auto-fix | `repo` (classic) or `contents: write` + `pull_requests: write` + `commit statuses: write` (fine-grained) |

### GitHub Actions Secrets Setup

| Secret Name | Value |
|-------------|-------|
| `OAB_REVIEW_ACTION_WEBHOOK` | Discord channel webhook URL (Settings → Integrations → Webhooks) |
| `OAB_REVIEW_ACTION_BOT_UID` | Discord user ID of the bot to @mention (e.g. the review agent's UID) |

`GITHUB_TOKEN` is automatically provided by Actions — no manual setup needed.

## Consequences

**Positive:**
- Fully automated — no manual @mention needed for PR reviews
- PR Checks tab shows live review status (🟡 → ✅/❌)
- Can enforce review via branch protection rules
- Discord thread preserves full review audit trail (OpenAB auto-creates threads)
- No architectural changes to OpenAB — agent receives messages normally
- Fire-and-forget Action — no runner time wasted waiting for review
- Minimal secrets — only one webhook URL needed in GitHub Secrets

**Negative:**
- Every PR push triggers a review (may want to filter by label or draft status)
- If OpenAB agent is down, status stays "pending" indefinitely (need timeout/alerting)
- Webhook messages lack user identity — OpenAB must allow webhook-originated messages
- Fork PRs: `OAB_REVIEW_ACTION_WEBHOOK` and `OAB_REVIEW_ACTION_BOT_UID` secrets are not available to workflows triggered by fork PRs (GitHub security policy). The webhook step will fail, and since fork PRs receive a read-only `GITHUB_TOKEN`, the error fallback **cannot** write commit statuses either — the workflow will fail silently with no status update. Fork PRs can still be reviewed manually via Discord @mention. Note: the `safe-to-review` label does **not** grant secrets access to fork PRs — it only bypasses the `author_association` gate for same-repo PRs.

**Mitigations:**
- Filter: skip draft PRs and untrusted authors — only `OWNER`, `MEMBER`, `COLLABORATOR`, and `CONTRIBUTOR` (returning contributor with merged PR) trigger automatic review. First-time contributors and unknown authors are skipped; maintainers can manually @mention the agent to review those PRs.
- Debounce: `concurrency` group with `cancel-in-progress: true` — new push cancels in-flight review, only latest SHA gets reviewed
- Error fallback: a scoped `if: failure()` step marks status as "error" when the Discord webhook step specifically fails (not triggered by circuit breaker), so status never stays pending on webhook failure
- Race condition: concurrency group prevents duplicate GitHub Action runs per PR; commit status is keyed to SHA so old reviews cannot overwrite newer status. **Note:** the concurrency group only operates at the GitHub Actions layer — if a webhook was already delivered before cancellation, the agent may receive a stale request. The agent **must** implement SHA validation (Layer 2 above) to skip stale requests and avoid wasted compute or comment race conditions.
- Timeout: a scheduled Action can mark stale pending statuses as "error" after N hours (agent down scenario)

## Safeguards

### Trusted Contributor Filter

The workflow uses GitHub's `author_association` field to gate automatic reviews. Only PRs from trusted authors trigger the review pipeline:

| `author_association` | Auto-review? | Meaning |
|---------------------|--------------|---------|
| `OWNER` | ✅ | Repository owner |
| `MEMBER` | ✅ | Organization member |
| `COLLABORATOR` | ✅ | Explicitly granted write access |
| `CONTRIBUTOR` | ✅ | Has previously merged a PR |
| `FIRST_TIME_CONTRIBUTOR` | ❌ | First PR to this repo |
| `NONE` | ❌ | No prior relationship |

**Why:** Prevents token waste and prompt-injection risk from untrusted PR diffs being fed into agent context. Maintainers can still manually @mention the agent to review skipped PRs after visual inspection.

### Label Override: `safe-to-review`

Maintainers can add the `safe-to-review` label to any PR to bypass the `author_association` check. This triggers the workflow via the `labeled` event, allowing untrusted contributors' PRs to be reviewed automatically after a maintainer has visually confirmed the PR is safe.

**Important:** This label only enables automatic review for **same-repo PRs**. Fork PRs lack access to repository secrets regardless of labels — adding `safe-to-review` to a fork PR will not trigger automation. Fork PRs must be reviewed manually via Discord @mention.

**Note:** The `labeled` event is filtered — only `safe-to-review` and `auto-fix` labels trigger the workflow. Other labels (e.g. `documentation`, `bug`) are ignored to avoid unnecessary review runs.

### Auto-Fix Mode: `auto-fix`

When the `auto-fix` label is present, the webhook payload includes `__mode: auto-fix__`. The agent enters an iterative loop:

1. Review PR → identify actionable findings (🔴/🟡)
2. Fix all findings → push commit
3. Re-review until LGTM or max iterations reached (agent-side cap, recommended: 3)

When the auto-fix loop completes (LGTM or cap reached), the agent removes the `auto-fix` label to prevent subsequent pushes from re-entering the fix loop.

**Constraints:**
- Only effective on same-repo branches (agent needs push access)
- Fork PRs: automation does not trigger (no secrets available); review manually via Discord @mention
- Agent must implement iteration cap to prevent infinite push→review loops

### Circuit Breaker (workflow hard cap: 30)

The workflow enforces a hard cap of 30 review cycles per PR (across all triggers over the PR's lifetime). This is distinct from the agent-side soft cap of 3 cycles per auto-fix session — the workflow cap catches edge cases where multiple auto-fix requests accumulate. On each run, it counts how many `pending` statuses with context `"OpenAB PR Review"` exist across all commits in the PR. If the count reaches 30:

1. Adds `review-limit-reached` label to the PR
2. Sets commit status to `error` with description "Circuit breaker: exceeded 30 review cycles"
3. Fails the workflow step

The `review-limit-reached` label is checked in the job `if` condition — once applied, no further review runs will trigger. A maintainer can remove the label to reset the circuit breaker if needed.

## References

- [GitHub Commit Status API](https://docs.github.com/en/rest/commits/statuses)
- [Discord Webhooks](https://discord.com/developers/docs/resources/webhook#execute-webhook)
- OpenAB PR Review Spec — internal agent document (not in this repository)
