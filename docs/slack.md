# Slack Bot Setup Guide

Step-by-step guide to create and configure a Slack bot for openab.

## 1. Create a Slack App

1. Go to https://api.slack.com/apps
2. Click **Create New App** → **From scratch**
3. Enter an app name (e.g. "OpenAB") and select your workspace
4. Click **Create App**

## 2. Enable Socket Mode

Socket Mode uses a persistent WebSocket connection — no public URL or ingress needed.

1. In the left sidebar, click **Socket Mode**
2. Toggle **Enable Socket Mode** to ON
3. You'll be prompted to generate an **App-Level Token**:
   - Token name: `openab-socket` (or any name)
   - Scope: `connections:write`
   - Click **Generate**
4. Copy the token (`xapp-...`) — this is your `SLACK_APP_TOKEN`

## 3. Subscribe to Events

1. In the left sidebar, click **Event Subscriptions**
2. Toggle **Enable Events** to ON
3. Under **Subscribe to bot events**, add:
   - `app_mention` — triggers when someone @mentions the bot
   - `message.channels` — receives messages in public channels (for thread follow-ups)
   - `message.groups` — receives messages in private channels (for thread follow-ups)
4. Click **Save Changes**

## 4. Add Bot Token Scopes

1. In the left sidebar, click **OAuth & Permissions**
2. Under **Bot Token Scopes**, add:

| Scope | Purpose |
|-------|---------|
| `app_mentions:read` | Receive @mention events |
| `chat:write` | Send and edit messages |
| `channels:history` | Read public channel messages (for thread context) |
| `groups:history` | Read private channel messages (for thread context) |
| `channels:read` | List public channels |
| `groups:read` | List private channels |
| `reactions:write` | Add/remove emoji reactions |
| `files:read` | Download file attachments (images, audio) |
| `users:read` | Resolve user display names |
| `assistant:write` | Native streaming + assistant status line (required when `assistant_mode = true`) |

## 5. Install to Workspace

1. In the left sidebar, click **Install App**
2. Click **Install to Workspace** (or **Reinstall** if you've changed scopes)
3. Authorize the requested permissions
4. Copy the **Bot User OAuth Token** (`xoxb-...`) — this is your `SLACK_BOT_TOKEN`

## 6. Configure openab

> 📖 Full config options with defaults: [docs/config-reference.md](config-reference.md#slack)

Add the `[slack]` section to your `config.toml`:

```toml
[slack]
bot_token = "${SLACK_BOT_TOKEN}"
app_token = "${SLACK_APP_TOKEN}"
allowed_channels = []                # empty = allow all channels
# allowed_users = ["U0123456789"]    # empty = allow all users
```

Set the environment variables:

```bash
export SLACK_BOT_TOKEN="xoxb-..."
export SLACK_APP_TOKEN="xapp-..."
```

### Assistant Mode (default: enabled)

`assistant_mode` is **enabled by default**. It uses Slack's native AI-app status API (`assistant.threads.setStatus`) for a short high-level status line. Slack reply content always uses **public-reply mode**: OpenAB buffers agent output, hides internal reasoning and execution context, and posts only the final user-facing answer. Raw ACP text deltas are never streamed into Slack.

**Requirements:** Your Slack app must be an [AI app](https://api.slack.com/docs/apps/ai) with the `assistant:write` scope. If your app already has `chat:write`, you only need to add `assistant:write` and reinstall.

**Non-AI app compatibility:** If your Slack app is **not** an AI app (or lacks `assistant:write`), set:

```toml
[slack]
assistant_mode = false
```

This keeps the same public-reply message flow and uses the concise thread status message plus emoji reactions (👀 / ✅ / ❌). Tool names, tool inputs, prompts, and reasoning are not used as status text.
## 7. Invite the Bot

In each Slack channel where you want to use the bot:

```
/invite @OpenAB
```

## 8. Test

In a channel where the bot is invited:

```
@OpenAB explain this code
```

The bot will reply in a thread. After that, just type in the thread — no @mention needed for follow-ups.

## Public Slack replies and OpenAB session links

Each Slack turn keeps user-visible feedback in the original message thread:

- OpenAB immediately posts a short status such as `OpenAB · 正在处理…`.
- The status can change to `正在执行…` or `正在整理结果…`, but it never contains prompts, reasoning, tool names, tool inputs, tool output, or system instructions.
- OpenAB does not stream raw agent text into Slack. It buffers the turn, removes private tagged blocks, drops inter-tool narration, and posts the final answer as a separate thread reply.
- The final footer shows the OpenAB run link, the reported model, and the active agent. Missing runtime metadata is shown as `未报告` instead of being guessed.

To append an `Open in OpenAB` session link to the progress message and
the final reply, configure the public base URL of the OpenAB Plus web console
(any of the following environment variables, checked in this order):

```bash
export OPENAB_SESSION_PUBLIC_BASE_URL="https://openab.example.com"   # recommended
# OPENAB_PUBLIC_BASE_URL / GATEWAY_PUBLIC_URL / PUBLIC_BASE_URL also work
```

The link points to `<base>/#/sessions/<session-id>` (the OpenAB Plus admin
console session route). Without one of these variables no link is appended;
restart the process after setting it. Configuring this public base URL is a
deployment step — see ZER-403.

## Slash commands are not supported on Slack

openab supports `/models`, `/agents`, and `/cancel` on **Discord**, but **not on Slack**. If you previously configured these commands in your Slack app's **Slash Commands** page, you can safely delete them — the Slack adapter ignores both `slash_commands` and `interactive` envelope types.

The root cause is a combination of three Slack-specific platform constraints, none of which is fixable from openab's side:

1. **Slack blocks third-party slash commands inside threads.** Invoking `/models` from a thread's reply composer returns the Slack error `"/models is not supported in threads. Sorry!"`. This is enforced by the Slack client itself, not by any app setting — enabling Interactivity, Socket Mode, or reinstalling the app does not bypass it. Slack's built-in commands (`/remind`, `/shrug`, etc.) get special treatment that custom apps cannot.

2. **Channel-level slash command payloads have no thread context.** If the user types `/models` in the channel's main composer instead of a thread, Slack delivers the command but the payload carries no `thread_ts`. Since openab keys each ACP session by thread (`slack:<thread_ts>` or `slack:<trigger_ts>`), the command cannot be routed to the right session. Sessions are never keyed by `channel_id` alone, so there's no workaround on the adapter side.

3. **Most ACP agents don't expose a model-switch surface.** Even when routing succeeded, `/models` reads the session's `configOptions` from the ACP `initialize` response. Only `kiro-cli` emits these in the expected format (via its `models`/`modes` fallback). `claude-code`, `codex`, `gemini`, `cursor-agent`, and `opencode` do not, so the menu would be empty for those backends — the user would see `"⚠️ No model options available"` with no recourse.

On Discord, none of these apply: slash commands work in thread-channels, the channel ID *is* the thread key, and users typically stay within a single agent per deployment anyway.

### If you need to switch models or agents with a Slack deployment

- **Change the agent**: edit `[agent]` in `config.toml` (or the Helm chart values) and restart the pod / process
- **Change the Claude model** (for `claude-code`): set `ANTHROPIC_DEFAULT_MODEL` (or equivalent env var depending on your claude-code-acp version) and restart — model selection happens at process start, not at runtime
- **Cancel an in-flight turn**: there is no built-in way on Slack currently.

## Finding Channel and User IDs

- **Channel ID**: Right-click the channel name → **View channel details** → ID at the bottom (starts with `C` for public, `G` for private)
- **User ID**: Click a user's profile → **...** menu → **Copy member ID** (starts with `U`)

## Troubleshooting

### Bot doesn't respond to @mentions

1. Verify Socket Mode is enabled in your app settings
2. Check that `app_mention` is subscribed under **bot events** (not user events)
3. Ensure the app is reinstalled after adding new event subscriptions
4. Check the bot is invited to the channel (`/invite @YourSlackAppName`)
5. Run with `RUST_LOG=openab=debug cargo run` to see incoming events

### Bot doesn't respond to thread follow-ups

1. Verify `message.channels` (and `message.groups` for private channels) are subscribed under **bot events**
2. Reinstall the app after adding these events

### "not_authed" or "invalid_auth" errors

1. Verify your `SLACK_BOT_TOKEN` starts with `xoxb-`
2. Verify your `SLACK_APP_TOKEN` starts with `xapp-`
3. Check the tokens haven't been revoked in your app settings

### Reactions not showing

1. Verify `reactions:write` scope is added
2. Reinstall the app after adding the scope
3. If `assistant_mode = true` (the default), emoji reactions are intentionally suppressed — status is shown via the assistant status line instead. Set `assistant_mode = false` if you want emoji reactions back.

### Assistant status line not showing ("正在处理…" / "正在执行…")

1. Verify your Slack app is configured as an [AI app](https://api.slack.com/docs/apps/ai)
2. Verify `assistant:write` scope is added under **Bot Token Scopes**
3. Reinstall the app after adding the scope
4. If your app cannot be an AI app, set `assistant_mode = false` to use emoji reactions instead

### Replies are not streaming live

This is intentional. Slack public-reply mode never exposes raw agent deltas because they can contain internal narration or private execution context. The thread shows a concise status while the turn runs, then receives one final user-facing answer with the OpenAB link, Model, and Agent fields.



