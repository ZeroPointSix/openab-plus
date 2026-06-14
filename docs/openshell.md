# OpenShell Quick Start

Run one OpenAB Discord bot inside a real [NVIDIA OpenShell](https://github.com/NVIDIA/OpenShell) sandbox.

This guide is intentionally Day 1 only:

- install the host prerequisites
- build the OpenAB sandbox image
- create an OpenShell sandbox
- authenticate `openab-agent`
- start OpenAB and talk to the agent through Discord

Day 2 work such as installing extra tools, visiting arbitrary websites, using GitHub, npm, PyPI, cloud CLIs, or third-party APIs requires OpenShell network policy work. OpenShell does not currently provide a simple documented "open all outbound network" switch. Choose Kubernetes or another container platform if you need a mature broad-network production path today.

## TL;DR

From the OpenAB repo root:

```bash
docker build -t oab-native-sandbox -f openshell/Dockerfile .
openshell sandbox create --name oab --from oab-native-sandbox:latest -- bash
openshell sandbox connect oab
```

Then inside the sandbox:

```bash
export DISCORD_BOT_TOKEN="your-discord-bot-token"
cd /sandbox
openab-agent auth codex-oauth --no-browser
openab run -c /sandbox/config.toml
```

The full commands below include the config file and the Day 1 network policy.

## What This Path Is For

Use OpenShell when you want a simple sandboxed Day 1 OpenAB demo on one machine.

Use Kubernetes, Zeabur, or another container platform when you need broad agent internet access, many agents, persistent services, or production operations.

OpenShell is convenient, but network egress is policy-driven:

- Day 1: Discord plus `openab-agent` model/auth endpoints.
- Day 2: extra tools and arbitrary web/cloud access need explicit policy additions.

`enforcement: audit` helps with L7 method/path discovery after an endpoint is already matched, but it is not global allow-all. If host + port + binary do not match a policy entry, the request is still blocked.

## Choose Your Host Path

OpenAB runs inside a Linux OpenShell sandbox in both paths. The difference is how your host provides Docker and OpenShell.

### Path 1: Linux Host

Use this path for a Linux VM or server such as Zeabur, EC2, a home server, or a Raspberry Pi.

Install Docker with your platform package manager, start it, and verify the current user can use it:

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

### Path 2: macOS Host

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

## Requirements

Before continuing, you should have:

- `docker info` working.
- `openshell sandbox list` working.
- A Discord bot token.
- A ChatGPT account that can authenticate `openab-agent`.

Check:

```bash
docker info
openshell sandbox list
```

If either command fails, finish the Linux or macOS host path above before continuing.

## 1. Build The Sandbox Image

From the OpenAB repo root:

```bash
docker build -t oab-native-sandbox -f openshell/Dockerfile .
```

This image contains `openab` and `openab-agent`. It intentionally does not try to preinstall every possible coding-agent tool.

## 2. Create The Sandbox

```bash
openshell sandbox create --name oab \
  --from oab-native-sandbox:latest \
  -- bash
```

Connect:

```bash
openshell sandbox connect oab
```

Inside the sandbox, verify the basic shape:

```bash
cd /sandbox
whoami
command -v openab
command -v openab-agent
test -w /sandbox && echo "/sandbox writable"
```

Expected user is `sandbox`.

## 3. Apply The Day 1 Network Policy

OpenShell network policy is scoped by host, port, and binary. The Day 1 policy only opens the endpoints needed for Discord and `openab-agent` auth/model traffic.

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

This is not a general developer policy. It does not allow GitHub, npm, PyPI, cloud SDKs, or arbitrary web browsing.

To inspect network logs:

```bash
openshell logs oab --source sandbox --since 10m -n 300
```

Durable logs also exist inside the sandbox under `/var/log/openshell.*.log`.

## 4. Create The OpenAB Config

Inside the sandbox:

```bash
export DISCORD_BOT_TOKEN="your-discord-bot-token"
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
```

## 5. Authenticate The Agent

Inside the sandbox:

```bash
HOME=/sandbox openab-agent auth codex-oauth --no-browser
```

Open the printed URL in your browser. If the browser redirects to a localhost URL, copy the full callback URL from the browser address bar and paste it back into the sandbox terminal.

Verify:

```bash
HOME=/sandbox openab-agent auth status
```

## 6. Run OpenAB

Inside the sandbox:

```bash
export HOME=/sandbox
export PATH="/sandbox/bin:/sandbox/.local/bin:/usr/local/bin:/usr/bin:/bin"
export TMPDIR=/sandbox/tmp
: "${DISCORD_BOT_TOKEN:?set DISCORD_BOT_TOKEN first}"

openab run -c /sandbox/config.toml
```

Expected result:

- OpenAB loads the config.
- The Discord bot connects.
- Mention the bot in Discord.
- The bot replies through `openab-agent`.

## Day 2 Boundary

If the agent later needs to install tools or reach other services, that is Day 2 OpenShell policy work.

Examples that may require more policy:

- `git clone` from GitHub
- `gh auth login`
- `npm install`
- `pip install`
- `cargo install`
- cloud CLIs such as `aws`, `gcloud`, `kubectl`, `terraform`
- arbitrary web search or third-party APIs

For those use cases, add explicit OpenShell network policies for the required hosts and binaries, or use Kubernetes/Zeabur when broad network access is the priority.

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
