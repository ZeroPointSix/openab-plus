# Devin CLI — Agent Backend Guide

How to run OpenAB with [Devin CLI](https://docs.devin.ai/cli) as the agent backend.

## Prerequisites

- A [Devin](https://devin.ai/) subscription (Enterprise or individual plan) from Cognition AI
- Devin CLI with native ACP support (`devin acp`)

## Architecture

```
┌──────────────┐  Gateway WS   ┌──────────────┐  ACP stdio    ┌──────────────┐
│   Discord    │◄─────────────►│ openab       │──────────────►│ devin acp    │
│   User       │               │   (Rust)     │◄── JSON-RPC ──│ (Devin CLI)  │
└──────────────┘               └──────────────┘               └──────────────┘
```

OpenAB spawns `devin acp` as a child process and communicates via stdio JSON-RPC. No intermediate adapter needed — Devin CLI natively implements the Agent Client Protocol.

## Configuration

```toml
[agent]
command = "devin"
args = ["acp"]
working_dir = "/home/agent"
env = { DEVIN_MODEL = "glm-5.2", DEVIN_PERMISSION_MODE = "dangerous" }
```

> **Note:** Setting `DEVIN_PERMISSION_MODE = "dangerous"` is recommended for
> headless/container deployments. Without it, Devin CLI may prompt for permission
> confirmations on certain operations, causing the agent to get stuck in
> non-interactive environments.

## Docker

Build with the unified Dockerfile:

```bash
docker build --target devin -f Dockerfile.unified -t openab-devin .
```

Or via docker buildx bake:

```bash
docker buildx bake devin
```

The Dockerfile installs a pinned version of Devin CLI from `static.devin.ai` with SHA256 checksum verification. The version is controlled by the `DEVIN_VERSION` build arg.

## Authentication

Devin CLI requires authentication via a Devin account. In a headless container:

```bash
# 1. Exec into the running pod/container
kubectl exec -it deployment/openab-devin -- bash

# 2. Authenticate via manual token flow (headless-friendly)
devin auth login --force-manual-token-flow

# 3. Follow the instructions to paste your token

# 4. Restart the pod (credentials persist via PVC)
kubectl rollout restart deployment/openab-devin
```

Credentials are stored under `~/.local/share/devin/` and persist across pod restarts via PVC.

## Helm Install

```bash
helm install openab openab/openab \
  --set agents.kiro.enabled=false \
  --set agents.devin.discord.botToken="$DISCORD_BOT_TOKEN" \
  --set-string 'agents.devin.discord.allowedChannels[0]=YOUR_CHANNEL_ID' \
  --set agents.devin.command=devin \
  --set 'agents.devin.args={acp}' \
  --set agents.devin.persistence.enabled=true \
  --set agents.devin.workingDir=/home/agent \
  --set image.tag=beta
```

### Image Tag

Use `--set image.tag=<version>` to set the image version globally.
The chart auto-appends `-<agent>` to produce the final tag (see [image-tags.md](image-tags.md) for full details).

| Tag | Resolves to | Description |
|-----|-------------|-------------|
| `beta` | `beta-devin` | Floating beta channel (latest pre-release) |
| `stable` | `stable-devin` | Floating stable channel |

## Features

Devin CLI provides:

- **Native ACP**: `devin acp` speaks JSON-RPC over stdio directly
- **AGENTS.md support**: Reads `AGENTS.md` at project root automatically
- **MCP servers**: Full MCP support (stdio + HTTP transports)
- **Subagents**: Can spawn foreground/background subagents for parallel work
- **Session persistence**: Conversation history saved and resumable
- **Models**: SWE-1.6 series with adaptive routing

## Model Selection

Devin CLI uses its own model routing by default (SWE-1.6 series). To specify a model at startup:

```toml
[agent]
command = "devin"
args = ["acp", "--model", "opus"]
working_dir = "/home/agent"
```

Available models can be checked via the interactive CLI with `/model`. In ACP mode, the `--model` flag selects the model for the session.

## MCP Usage

Devin CLI supports MCP servers configured via `.devin/config.json` or `devin mcp add`:

```bash
# Add an MCP server (persists in ~/.config/devin/)
kubectl exec -it deployment/openab-devin -- devin mcp add github \
  -- npx -y @modelcontextprotocol/server-github

# List configured servers
kubectl exec -it deployment/openab-devin -- devin mcp list
```

MCP configuration can also be placed in the project's `.devin/config.json`:

```json
{
  "mcpServers": {
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": { "GITHUB_TOKEN": "ghp_xxx" }
    }
  }
}
```

Devin CLI supports both stdio and HTTP (Streamable HTTP + SSE fallback) transports.

## AGENTS.md Compatibility

Devin CLI reads `AGENTS.md` from the project root — the same file OpenAB already uses. It also reads `CLAUDE.md` (for Claude Code compatibility) and rules from `.cursor/rules/` and `.windsurf/rules/`.

## Recommended Permission Config (Headless)

In headless/container deployments, Devin CLI's "cog" permission model may still prompt
for tool approval even with `DEVIN_PERMISSION_MODE=dangerous` set as an env var or
`--permission-mode dangerous` CLI flag. The **only reliable fix** is to place a
`config.json` in the user config directory with explicit permission overrides:

**File:** `~/.config/devin/config.json`

```json
{
  "version": 1,
  "permission_mode": "bypass",
  "permissions": {
    "allow": [
      "Read(**)",
      "Write(**)",
      "Fetch(**)",
      "Exec(**)",
      "exec",
      "read",
      "edit",
      "grep",
      "glob",
      "webfetch",
      "web_search",
      "mcp__*"
    ],
    "deny": [],
    "ask": []
  },
  "agent": {
    "model": "glm-5-2"
  }
}
```

**Why both scope-based and tool-based entries?**

- **Scope-based** (`Read(**)`, `Write(**)`, `Exec(**)`, `Fetch(**)`): match by path/URL/command pattern
- **Tool-based** (`exec`, `read`, `edit`, etc.): match by tool name directly
- `mcp__*`: wildcard for all MCP tools (documented by Devin)

There is no documented "allow all" wildcard — you must list each tool explicitly.

**What doesn't work alone:**

| Method | Result |
|--------|--------|
| `DEVIN_PERMISSION_MODE=dangerous` env var | Ignored in ACP mode |
| `--permission-mode dangerous` CLI flag | Overridden by team settings / cog |
| `permission_mode: "bypass"` in config.json alone | Cog still evaluates per-tool |
| Removing `org_id` from config.json | Team settings still fetched via auth |

**What works:**

`permission_mode: "bypass"` **combined with** explicit `permissions.allow` entries in
`~/.config/devin/config.json`. This tells the cog to auto-approve matching tools.

### OAB Gist Config

When using OAB's gist-based config, set the `[agent]` section to pass the CLI flag
as belt-and-suspenders:

```toml
[agent]
command = "devin"
args = ["--permission-mode", "dangerous", "acp"]
env = { DEVIN_MODEL = "glm-5.2", GHPOOL_URL = "http://ghpool.openab.local:8080", PATH = "/home/agent/bin:/usr/local/bin:/usr/bin:/bin" }
```

The config.json must be pre-seeded via the home tarball (`ddu-home.tar.gz`) or written
by a pre-boot hook.

## Known Limitations

- Requires a paid Devin subscription (Cognition AI); no free tier for CLI access
- `devin auth login` requires interactive terminal for browser flow; use `--force-manual-token-flow` in headless environments
- Enterprise features (team settings, controls) require Devin Enterprise plan
- Config file must be mounted at `/etc/openab/config.toml` at runtime (not baked into image)
