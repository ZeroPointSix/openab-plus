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

Use a **GitHub Action → Discord Bot API → OpenAB (ECS)** architecture with GitHub Commit Status API for check status feedback.

### Why Commit Status API (not Check Runs)

Check Runs API requires a GitHub App with `checks:write` permission. Commit Status API works with a standard PAT or fine-grained token (`commit statuses: write`), which the agent already has via `gh` CLI auth. This avoids creating an additional GitHub App solely for status reporting.

### Why Discord Bot API (not webhook)

Discord webhooks cannot open threads or carry user identity. The Discord Bot API allows:
- Creating a thread per review (clean audit trail)
- @mentioning 超渡 to trigger the existing OpenAB message pipeline
- Full thread history showing multi-agent collaboration

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
│  2. Discord Bot API:                                                │
│     → Create message in channel (thread starter)                    │
│     → Create thread from message                                    │
│     → Post "@超渡 review <PR_URL>" in thread (with sha)             │
│                                                                     │
│  3. Job exits (fire-and-forget)                                     │
└───────────────────────────┬─────────────────────────────────────────┘
                            │ Discord message
                            ▼
┌─────────────────────────────────────────────────────────────────────┐
│  OpenAB Agent (ECS Fargate, long-lived)                             │
│                                                                     │
│  Receives @mention → opens agent session                            │
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

permissions:
  statuses: write

jobs:
  request-review:
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

      - name: Trigger review via Discord
        env:
          DISCORD_BOT_TOKEN: ${{ secrets.DISCORD_BOT_TOKEN }}
          CHANNEL_ID: ${{ vars.REVIEW_CHANNEL_ID }}
        run: |
          PR_NUM=${{ github.event.pull_request.number }}
          PR_URL="https://github.com/${{ github.repository }}/pull/${PR_NUM}"
          SHA=${{ github.event.pull_request.head.sha }}

          MSG_ID=$(curl -s -X POST \
            "https://discord.com/api/v10/channels/${CHANNEL_ID}/messages" \
            -H "Authorization: Bot ${DISCORD_BOT_TOKEN}" \
            -H "Content-Type: application/json" \
            -d "{\"content\": \"📋 PR #${PR_NUM} Review Request\"}" \
            | jq -r '.id')

          THREAD_ID=$(curl -s -X POST \
            "https://discord.com/api/v10/channels/${CHANNEL_ID}/messages/${MSG_ID}/threads" \
            -H "Authorization: Bot ${DISCORD_BOT_TOKEN}" \
            -H "Content-Type: application/json" \
            -d "{\"name\": \"PR #${PR_NUM} Review\"}" \
            | jq -r '.id')

          curl -s -X POST \
            "https://discord.com/api/v10/channels/${THREAD_ID}/messages" \
            -H "Authorization: Bot ${DISCORD_BOT_TOKEN}" \
            -H "Content-Type: application/json" \
            -d "{\"content\": \"<@1490365068863606784> review ${PR_URL}\n\n__commit: ${SHA}__\"}"
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
| `DISCORD_BOT_TOKEN` | Create thread + send message | Bot in the review channel |
| Agent's `gh` auth (PAT) | Post comment + update status | `repo` (classic) or `pull_requests: write` + `commit statuses: write` (fine-grained) |

## Consequences

**Positive:**
- Fully automated — no manual @mention needed for PR reviews
- PR Checks tab shows live review status (🟡 → ✅/❌)
- Can enforce review via branch protection rules
- Discord thread preserves full review audit trail
- No architectural changes to OpenAB — agent receives messages normally
- Fire-and-forget Action — no runner time wasted waiting for review

**Negative:**
- Requires a Discord Bot Token stored in GitHub Secrets
- Every PR push triggers a review (may want to filter by label or draft status)
- If OpenAB agent is down, status stays "pending" indefinitely (need timeout/alerting)

**Mitigations:**
- Filter: skip draft PRs, skip PRs with `skip-review` label
- Timeout: a scheduled Action can mark stale pending statuses as "error" after N hours
- Rate limiting: debounce rapid pushes (only review latest commit after a cooldown)

## References

- [GitHub Commit Status API](https://docs.github.com/en/rest/commits/statuses)
- [Discord Bot API — Create Message](https://discord.com/developers/docs/resources/message#create-message)
- [Discord Bot API — Start Thread from Message](https://discord.com/developers/docs/resources/channel#start-thread-from-message)
- [OpenAB PR Review Spec](../../.openab/memory/shared/pr-review-spec.md) (internal)
