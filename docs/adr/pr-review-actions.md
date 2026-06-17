# ADR: OpenAB PR Review Actions

**Status:** Proposed  
**Date:** 2026-06-17  
**Author:** 超渡法師

## Context

OpenAB's PR review workflow currently relies on manual triggers — a human @mentions 超渡 in Discord to initiate a review. This works well for ad-hoc reviews but does not scale for repositories with frequent PR activity. Maintainers want automated PR reviews that:

1. Trigger automatically when a PR is opened or updated
2. Show review status as a GitHub Check (🟡 pending → ✅/❌ complete)
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

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│  GitHub: PR opened / synchronize / ready_for_review                 │
└───────────────────────────┬─────────────────────────────────────────┘
                            │ triggers
                            ▼
┌─────────────────────────────────────────────────────────────────────┐
│  GitHub Action (.github/workflows/pr-review.yml)                    │
│                                                                     │
│  1. POST /repos/{owner}/{repo}/statuses/{sha}                       │
│     state: "pending", context: "OpenAB PR Review"                   │
│                                                                     │
│  2. Discord Webhook:                                                │
│     → POST to webhook URL with "@超渡 review <PR_URL>"              │
│                                                                     │
│  3. Job exits (fire-and-forget)                                     │
└───────────────────────────┬─────────────────────────────────────────┘
                            │ Discord message
                            ▼
┌─────────────────────────────────────────────────────────────────────┐
│  OpenAB Agent (ECS Fargate, long-lived)                             │
│                                                                     │
│  Receives @mention → opens agent session (auto-creates thread)      │
│  → Delegates to 法師團隊 (angle-based review)                        │
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

## Implementation Plan

### Phase 1: GitHub Action Workflow

Create `.github/workflows/pr-review.yml`:

```yaml
name: PR Review
on:
  pull_request:
    types: [opened, synchronize, ready_for_review]

# Debounce: cancel in-flight review when new push arrives.
# Only the latest commit gets reviewed, preventing race conditions
# where multiple reviews update the same PR concurrently.
concurrency:
  group: pr-review-${{ github.event.pull_request.number }}
  cancel-in-progress: true

permissions:
  statuses: write

jobs:
  request-review:
    if: "!github.event.pull_request.draft"
    runs-on: ubuntu-latest
    steps:
      - name: Set pending status
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          gh api repos/${{ github.repository }}/statuses/${{ github.event.pull_request.head.sha }} \
            -f state="pending" \
            -f context="OpenAB PR Review" \
            -f description="Review in progress..."

      - name: Trigger review via Discord webhook
        run: |
          set -eo pipefail

          PR_NUM=${{ github.event.pull_request.number }}
          PR_URL="https://github.com/${{ github.repository }}/pull/${PR_NUM}"
          SHA=${{ github.event.pull_request.head.sha }}

          curl -sf -X POST "${{ secrets.OAB_REVIEW_ACTION_WEBHOOK }}" \
            -H "Content-Type: application/json" \
            -d "{\"content\": \"<@${{ vars.OAB_REVIEW_ACTION_BOT_UID }}> review ${PR_URL}\n\n__commit: ${SHA}__\"}"

      - name: Mark error on Discord failure
        if: failure()
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          gh api repos/${{ github.repository }}/statuses/${{ github.event.pull_request.head.sha }} \
            -f state="error" \
            -f context="OpenAB PR Review" \
            -f description="Failed to trigger review — check workflow logs"
```

### Phase 2: Agent Callback (Status Update)

After the agent posts the final PR comment, update the commit status:

```bash
# LGTM
gh api repos/OWNER/REPO/statuses/SHA \
  -f state="success" \
  -f context="OpenAB PR Review" \
  -f description="LGTM ✅"

# Changes Requested
gh api repos/OWNER/REPO/statuses/SHA \
  -f state="failure" \
  -f context="OpenAB PR Review" \
  -f description="Changes Requested ⚠️"
```

### Phase 3: Branch Protection

Add `OpenAB PR Review` as a required status check in branch protection rules. This enforces that PRs cannot merge until the review completes successfully.

## Token & Permissions

| Secret | Purpose | Minimum Permission |
|--------|---------|-------------------|
| `GITHUB_TOKEN` (Actions) | Set initial pending status | `statuses: write` |
| `OAB_REVIEW_ACTION_WEBHOOK` | Post review request to Discord channel | Webhook URL (channel-scoped) |
| Agent's `gh` auth (PAT) | Post comment + update status | `repo` (classic) or `pull_requests: write` + `commit statuses: write` (fine-grained) |

### GitHub Actions Secrets Setup

| Secret Name | Value |
|-------------|-------|
| `OAB_REVIEW_ACTION_WEBHOOK` | Discord channel webhook URL (Settings → Integrations → Webhooks) |

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

**Mitigations:**
- Filter: skip draft PRs (`if: "!github.event.pull_request.draft"`)
- Debounce: `concurrency` group with `cancel-in-progress: true` — new push cancels in-flight review, only latest SHA gets reviewed
- Error fallback: `if: failure()` step marks status as "error" so it never stays pending on workflow failure
- Race condition: concurrency group ensures only one review runs per PR at a time; commit status is keyed to SHA so old reviews cannot overwrite newer status
- Timeout: a scheduled Action can mark stale pending statuses as "error" after N hours (agent down scenario)

## References

- [GitHub Commit Status API](https://docs.github.com/en/rest/commits/statuses)
- [Discord Webhooks](https://discord.com/developers/docs/resources/webhook#execute-webhook)
- [OpenAB PR Review Spec](../../.openab/memory/shared/pr-review-spec.md) (internal)
