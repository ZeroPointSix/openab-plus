# MiMo-Code

[MiMo-Code](https://github.com/XiaomiMiMo/MiMo-Code) supports ACP natively via the `acp` subcommand — no adapter needed.

MiMo-Code is a fork of [OpenCode](https://opencode.ai) by Xiaomi, adding persistent memory, intelligent context management, subagent orchestration, goal-driven autonomous loops, compose workflows, and dream/distill self-improvement. It includes **MiMo Auto** as a free-for-limited-time channel, so you can start with zero configuration.

```
┌──────────┐  Discord  ┌────────┐ ACP stdio ┌───────────┐   ┌───────────────────┐
│ Discord  │◄────────► │ OpenAB │◄────────► │ MiMo-Code │──►│  LLM Providers    │
│ Users    │ Gateway   │ (Rust) │ JSON-RPC  │   (ACP)   │   │                   │
└──────────┘           └────────┘           └───────────┘   │ ┌───────────────┐ │
                                                 │          │ │ MiMo Auto     │ │
                                       mimocode.json        │ │ Xiaomi MiMo   │ │
                                       sets model           │ │ OpenAI        │ │
                                                            │ │ Anthropic     │ │
                                                            │ │ AWS Bedrock   │ │
                                                            │ │ OpenRouter    │ │
                                                            │ │ Ollama (local)│ │
                                                            │ │ + more...     │ │
                                                            │ └───────────────┘ │
                                                            └───────────────────┘
```

## Docker Image

```bash
docker build -f Dockerfile.mimocode -t openab-mimocode:latest .
```

The image installs `@mimo-ai/cli` globally via npm on `node:22-bookworm-slim`.

## Helm Install

```bash
helm install openab openab/openab \
  --set agents.kiro.enabled=false \
  --set agents.mimocode.enabled=true \
  --set agents.mimocode.image=ghcr.io/openabdev/openab-mimocode:latest \
  --set-string 'agents.mimocode.discord.allowedChannels[0]=YOUR_CHANNEL_ID'
```

> The Docker image already defines `command`, `args`, and `workingDir` via
> `OPENAB_AGENT_COMMAND` and `WORKDIR` — no need to set them in Helm values.

## Authentication

MiMo-Code supports multiple auth methods:

### MiMo Auto (zero config)

MiMo Auto is built in as a free-for-limited-time channel. The first launch guides you through configuration automatically — select "MiMo Auto" for immediate use with no API keys.

### Xiaomi MiMo Platform (OAuth)

```bash
kubectl exec -it deployment/openab-mimocode -- mimo auth login
```

Select "Xiaomi MiMo Platform" and follow the OAuth flow.

### Import from Claude Code

If you already have Claude Code credentials, MiMo-Code can import them:

```bash
kubectl exec -it deployment/openab-mimocode -- mimo auth login
```

Select "Import from Claude Code".

### Custom Provider (OpenAI-compatible API)

```bash
kubectl exec -it deployment/openab-mimocode -- mimo auth login
```

Select "Custom Provider" and enter your API endpoint and key.

## Configuration

MiMo-Code is configured via `.mimocode/mimocode.json` in the working directory or `~/.config/mimocode/mimocode.json` globally. Example:

```json
{
  "model": "mimo-auto"
}
```

To set the model inside the pod:

```bash
kubectl exec deployment/openab-mimocode -- sh -c \
  'mkdir -p /home/node/.mimocode && echo "{\"model\": \"mimo-auto\"}" > /home/node/.mimocode/mimocode.json'
```

## Key Differences from OpenCode

| Feature | OpenCode | MiMo-Code |
|---------|----------|-----------|
| Persistent memory | ❌ | ✅ MEMORY.md + SQLite FTS5 |
| Subagent system | ❌ | ✅ Parallel subagents |
| Context management | Basic | ✅ Auto-checkpoint + reconstruction |
| Goal / stop condition | ❌ | ✅ Independent judge |
| Compose mode | ❌ | ✅ Specs-driven workflows |
| Dream / Distill | ❌ | ✅ Self-improvement |
| Free channel | ❌ | ✅ MiMo Auto |

## Notes

- **ACP compatibility**: MiMo-Code uses the same ACP protocol as OpenCode (`@agentclientprotocol/sdk`). All OAB ACP features (streaming, tool display, session management) work identically.
- **Tool authorization**: Like OpenCode, MiMo-Code handles tool authorization internally — all tools run without user confirmation.
- **Binary name**: The CLI binary is `mimo` (installed via `npm install -g @mimo-ai/cli`). Alternative install: `curl -fsSL https://mimo.xiaomi.com/install | bash`.
- **Session persistence**: MiMo-Code's memory system (MEMORY.md, checkpoints) persists on the PVC and carries context across sessions automatically.
