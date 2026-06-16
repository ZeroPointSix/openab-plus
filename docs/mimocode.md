# MiMoCode (mimo)

MiMoCode is a fork of OpenCode. It supports ACP over stdio and can be used as an OpenAB agent backend.

## Setup

| Field | Value |
|-------|-------|
| Image | `openab-mimocode` (or any image with `mimo` installed) |
| Command | `mimo` |
| Args | `["acp"]` |
| Working dir | `/home/node` |

## Authentication

MiMoCode offers a free tier (`MiMo Auto`) that requires no API key — just a one-time device auth:

```bash
mimo auth login --provider mimo --method "MiMo Auto (free)"
```

This is **fully non-interactive** and can be used in Dockerfiles, pre-boot hooks, or CI scripts. It sets `mimo/mimo-auto` as the default model (1M context, free).

The token expires in ~1 hour but auto-refreshes on next ACP session start. For persistent deployments, run this in a `[hooks.pre_boot]` script to ensure fresh auth on every container start.

## ⚠️ Important: SQLite DB Locking

MiMoCode uses a SQLite database (`~/.local/share/mimocode/mimocode.db`) for state. **Only one process can access it at a time.**

**Do NOT** run manual `mimo` commands (e.g. `mimo auth login`, `mimo debug config`, `mimo models`) while `mimo acp` is actively handling a request. This will corrupt or lock the database, causing all subsequent ACP requests to fail with empty responses or "Connection Lost".

### Safe workflow:
1. Start the bot (openab spawns `mimo acp` on first message)
2. Auth **before** the first message, or while the session is idle
3. If the DB gets corrupted:
   ```bash
   # As root:
   rm -f ~/.local/share/mimocode/mimocode.db*
   chown -R node:node ~/.local/share/mimocode/
   # As node:
   mimo auth login
   ```

## AWS/Bedrock Auto-Detection

When running on AWS (ECS, EC2), MiMoCode auto-detects AWS credentials and registers `amazon-bedrock` as a provider. If Bedrock models are not enabled in your account or the task role lacks `bedrock:InvokeModel`, this causes silent empty responses.

### Solutions:

1. **Fresh DB** — If the only auth you run is `mimo auth login` → MiMo Auto (free), then mimo-auto becomes the only provider and the default. No Bedrock conflict.

2. **Block AWS detection** (if needed) — Set in `[agent].env`:
   ```toml
   env = { AWS_CONTAINER_CREDENTIALS_RELATIVE_URI = "" }
   ```
   ⚠️ This breaks `aws` CLI access from within the container.

3. **Config file** (schema TBD) — MiMoCode reads `~/.config/mimocode/config.json` but the schema differs from OpenCode. The `bedrockDiscovery` key from OpenCode docs is not recognized by MiMoCode.

## Config (gist)

```toml
[agent]
command = "mimo"
args = ["acp"]
env = { GHPOOL_URL = "http://ghpool.openab.local:8080", PATH = "/home/node/bin:/usr/local/bin:/usr/bin:/bin" }
```

## Known Limitations

- `mimo acp` does not accept `--model` flag (unlike the TUI)
- Default model is set during `mimo auth login` and stored in the DB
- No `config set` CLI command — model selection is via auth flow only
- The `-m/--model` flag only works for TUI/run modes, not ACP
