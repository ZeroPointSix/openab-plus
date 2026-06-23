# Gemini CLI

Gemini CLI supports ACP natively via the `--acp` flag — no adapter needed.

## Docker Image

```bash
docker build -f Dockerfile.gemini -t openab-gemini:latest .
```

The image installs `@google/gemini-cli` globally via npm.

## Helm Install

```bash
helm install openab openab/openab \
  --set agents.kiro.enabled=false \
  --set agents.gemini.discord.enabled=true \
  --set agents.gemini.discord.botToken="$DISCORD_BOT_TOKEN" \
  --set-string 'agents.gemini.discord.allowedChannels[0]=YOUR_CHANNEL_ID' \
  --set agents.gemini.command=gemini \
  --set agents.gemini.args='{--acp}' \
  --set agents.gemini.workingDir=/home/node \
  --set agents.gemini.image.tag=beta-gemini
```

> Set `agents.kiro.enabled=false` to disable the default Kiro agent.
> 
> (Optional) `agents.gemini.args='{--acp}'` could be modified as `{--model,gemini-3-pro-preview,--acp}` if specific model is required. Otherwise, the default value will be 'Auto (Gemini 3)'.

### Image Tag

Use `--set agents.gemini.image.tag=<version>-gemini` to pin the image version.
The tag format is `<version>-<agent>` (see [image-tags.md](image-tags.md) for full details).

| Example | Description |
|---------|-------------|
| `beta-gemini` | Floating beta channel (latest pre-release) |
| `0.9.0-beta.2-gemini` | Pinned to exact version |
| `stable-gemini` | Floating stable channel |

> ⚠️ There is no `latest` tag — you must include the `-gemini` agent suffix.

## Manual config.toml

```toml
[agent]
# command and args default from OPENAB_AGENT_COMMAND="gemini --acp"
# Only override if you need non-default behavior
env = { GEMINI_API_KEY = "${GEMINI_API_KEY}" }
```

## Authentication

Gemini supports Google OAuth or an API key:

- **API key**: Set `GEMINI_API_KEY` environment variable
- **OAuth**: Run Google OAuth flow inside the pod
