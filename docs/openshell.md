# OpenShell Quick Start

Run one OpenAB Discord bot inside an [NVIDIA OpenShell](https://github.com/NVIDIA/OpenShell) sandbox.

OpenShell is the easy local sandbox path for Day 1: get a bot online, authenticate `openab-agent`, and talk to it through Discord. If you need broad agent internet access, many long-running agents, or production operations, Kubernetes/Zeabur may still be the better path today.

## Architecture

```text
┌─────────────────────────────────────────────────────────────────────┐
│ Host: Linux server / Zeabur / Raspberry Pi / macOS + Docker Desktop │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ OpenShell Gateway                                             │  │
│  │ - creates and manages sandbox lifecycle                       │  │
│  │ - enforces egress network policy                              │  │
│  │ - policy matches binary + host + port                         │  │
│  └──────────────────────────┬────────────────────────────────────┘  │
│                             │ creates sandbox "oab"                 │
│                             ▼                                        │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ OpenShell Sandbox                                             │  │
│  │                                                               │  │
│  │  /sandbox/                                                    │  │
│  │   ├── config.toml              OpenAB config                  │  │
│  │   ├── .openab/agent/auth.json  openab-agent auth state        │  │
│  │   ├── bin/                     agent-installed tools          │  │
│  │   ├── .local/bin/              user-local tools               │  │
│  │   └── tmp/                     temp files                     │  │
│  │                                                               │  │
│  │  openab run ──stdio JSON-RPC──► openab-agent                  │  │
│  │       │                              │                        │  │
│  │       │ Discord Gateway/API          │ OpenAI/Auth endpoints  │  │
│  └───────┼──────────────────────────────┼────────────────────────┘  │
│          │                              │                           │
│  ┌───────▼──────────────────────────────▼───────────────────────┐   │
│  │ Network Policy                                                │   │
│  │ - /usr/local/bin/openab       -> Discord endpoints            │   │
│  │ - /usr/local/bin/openab-agent -> auth/model endpoints         │   │
│  │ - everything else is denied unless explicitly added           │   │
│  └───────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

In OpenShell, `/sandbox` is the agent's writable root. OpenAB config, `openab-agent` auth state, temporary files, and agent-installed tools should live under `/sandbox`.

## TL;DR

From the OpenAB repo root:

```bash
export DISCORD_BOT_TOKEN="your-discord-bot-token"
openshell provider create --name openab-discord --type generic --credential DISCORD_BOT_TOKEN

openshell sandbox create --name oab \
  --provider openab-discord \
  --from ghcr.io/openabdev/openab-native-sandbox:latest \
  -- bash

openshell sandbox connect oab
```

Before authenticating or starting OpenAB, apply the [Day 1 network policy](#apply-the-day-1-network-policy) from the host.

Then inside the sandbox:

```bash
cd /sandbox
cat > config.toml <<'EOF'
[discord]
bot_token = "${DISCORD_BOT_TOKEN}"
allow_all_channels = true
allow_all_users = true
allow_dm = false
message_processing_mode = "per-thread"

[agent]
command = "openab-agent"
working_dir = "/sandbox"

[agent.env]
HOME = "/sandbox"
PATH = "/sandbox/bin:/sandbox/.local/bin:/usr/local/bin:/usr/bin:/bin"
TMPDIR = "/sandbox/tmp"
OPENAB_AGENT_OPENAI_MODEL = "gpt-5.4-mini"

[pool]
max_sessions = 1
session_ttl_hours = 1
EOF

HOME=/sandbox openab-agent auth codex-oauth --no-browser
openab run -c /sandbox/config.toml
```

Open the printed auth URL in your browser. If it redirects to a localhost callback URL, copy the full callback URL from the browser address bar and paste it back into the sandbox terminal.

## Choose Your Host Path

OpenAB runs inside a Linux OpenShell sandbox in both paths.

### Linux Host

Use this path for a Linux VM or server such as Zeabur, EC2, a home server, or a Raspberry Pi.

Install Docker with your platform package manager, start it, and verify:

```bash
docker info
```

Install OpenShell with the official installer:

```bash
curl -LsSf https://raw.githubusercontent.com/NVIDIA/OpenShell/main/install.sh | sh
```

Verify:

```bash
openshell sandbox list
```

### macOS Host

Use this path for a MacBook or Mac mini. Docker Desktop provides the Linux runtime substrate; OpenShell still creates a Linux sandbox.

Install and start Docker Desktop, then verify:

```bash
docker info
```

Install OpenShell with the official installer:

```bash
curl -LsSf https://raw.githubusercontent.com/NVIDIA/OpenShell/main/install.sh | sh
```

Verify:

```bash
openshell sandbox list
```

If Docker Desktop needs macOS approval or first-run setup, finish that in the GUI before continuing.

## Create The Sandbox

Keep your Discord token outside `config.toml`. Load it into your host shell from a secret manager or a local `.env` file that is not committed, then let OpenShell store and inject it as a provider credential:

```bash
export DISCORD_BOT_TOKEN="your-discord-bot-token"

openshell provider create \
  --name openab-discord \
  --type generic \
  --credential DISCORD_BOT_TOKEN
```

This follows the OpenShell provider model: the CLI can read credential values from local environment variables and attach them to sandboxes as provider credentials. The OpenAB config should reference `${DISCORD_BOT_TOKEN}` instead of containing the raw token.

Use the prebuilt OpenAB sandbox image:

```bash
openshell sandbox create --name oab \
  --provider openab-discord \
  --from ghcr.io/openabdev/openab-native-sandbox:latest \
  -- bash
```

If you need to test local source changes instead, build the image yourself:

```bash
docker build -t oab-native-sandbox -f openshell/Dockerfile .
openshell sandbox create --name oab --provider openab-discord --from oab-native-sandbox:latest -- bash
```

Connect:

```bash
openshell sandbox connect oab
```

Inside the sandbox, verify the shape:

```bash
cd /sandbox
whoami
command -v openab
command -v openab-agent
test -w /sandbox && echo "/sandbox writable"
```

Expected user is `sandbox`.

## Apply The Day 1 Network Policy

OpenShell network policy is scoped by host, port, and binary. The Day 1 policy opens only the endpoints needed for Discord and `openab-agent` auth/model traffic.

Run these commands on the host:

```bash
for endpoint in \
  'discord.com:443:full' \
  'gateway.discord.gg:443:full' \
  'cdn.discordapp.com:443:full' \
  'media.discordapp.net:443:full'
do
  openshell policy update oab \
    --add-endpoint "$endpoint" \
    --binary /usr/local/bin/openab \
    --wait
done

for endpoint in \
  'chatgpt.com:443:full:rest:audit' \
  '*.chatgpt.com:443:full:rest:audit' \
  'auth.openai.com:443:full:rest:audit' \
  '*.openai.com:443:full:rest:audit' \
  '*.oaistatic.com:443:full:rest:audit' \
  '*.oaiusercontent.com:443:full:rest:audit'
do
  openshell policy update oab \
    --add-endpoint "$endpoint" \
    --binary /usr/local/bin/openab-agent \
    --wait
done
```

## Configure And Run OpenAB

Inside the sandbox:

```bash
cd /sandbox
mkdir -p /sandbox/bin /sandbox/.local/bin /sandbox/tmp

cat > /sandbox/config.toml <<'EOF'
[discord]
bot_token = "${DISCORD_BOT_TOKEN}"
allow_all_channels = true
allow_all_users = true
allow_dm = false
message_processing_mode = "per-thread"

[agent]
command = "openab-agent"
working_dir = "/sandbox"

[agent.env]
HOME = "/sandbox"
PATH = "/sandbox/bin:/sandbox/.local/bin:/usr/local/bin:/usr/bin:/bin"
TMPDIR = "/sandbox/tmp"
OPENAB_AGENT_OPENAI_MODEL = "gpt-5.4-mini"

[pool]
max_sessions = 1
session_ttl_hours = 1

[reactions]
enabled = true
remove_after_reply = false
EOF

HOME=/sandbox openab-agent auth codex-oauth --no-browser
HOME=/sandbox openab-agent auth status

openab run -c /sandbox/config.toml
```

Expected result:

- OpenAB loads the config.
- The Discord bot connects.
- Mention the bot in Discord.
- The bot replies through `openab-agent`.

## Day 2 Boundary

This quick start is only the Day 1 path. If the agent later needs GitHub, npm, PyPI, cloud SDKs, arbitrary web search, third-party APIs, or extra tool installs, you will need more OpenShell policy work.

OpenShell does not currently provide a simple documented "open all outbound network" switch. `enforcement: audit` helps inspect matched endpoints, but it is not a global allow-all mode. If host + port + binary do not match a policy entry, the request is still blocked.

For deeper policy, access, usability, and E2E notes, read [ADR: OpenShell OpenAB Preset Module](adr/openshell-openab-preset-module.md).

## Troubleshooting

| Symptom | Meaning | Next step |
| --- | --- | --- |
| `docker info` fails | Docker is not ready | Start Docker Desktop or Docker Engine |
| `openshell sandbox list` fails | OpenShell gateway/driver is not ready | Fix OpenShell before continuing |
| Sandbox cannot reach Discord | Day 1 policy is missing or incomplete | Run `openshell logs oab --source sandbox --since 10m` |
| Auth URL opens but callback fails | Browser/device auth needs manual callback paste | Copy the full redirected localhost URL back into the terminal |
| Model/tool call is blocked | Network policy blocked it | Treat as Day 2 policy work |
| Agent tries to install into `/usr` | Runtime sandbox is non-root | Install only under `/sandbox` |

## Cleanup

```bash
openshell sandbox delete oab
```

## Related Docs

- [Discord setup](discord.md)
- [Native Agent](native-agent.md)
- [Agent-installable tools](agent-installable-tools.md)
- [OpenShell policy ADR](adr/openshell-openab-preset-module.md)
