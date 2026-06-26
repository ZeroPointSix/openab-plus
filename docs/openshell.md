# OpenShell Quick Start

Run one OpenAB Discord bot inside an [NVIDIA OpenShell](https://github.com/NVIDIA/OpenShell) sandbox.

This guide is intentionally narrow. The Day 1 path is:

```text
OpenShell sandbox -> OpenAB Discord bot -> Kiro CLI default agent
```

Other coding CLIs can work, but they need their own authentication, network, and tool-install policies. Do not treat this page as a generic Codex/Claude/Gemini/AGY OpenShell guide.

## Architecture

```text
┌─────────────────────────────────────────────────────────────────────┐
│ Host: Linux server / Zeabur / macOS with Docker Desktop             │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ OpenShell Gateway                                             │  │
│  │ - creates and manages sandbox lifecycle                       │  │
│  │ - injects provider credentials                                │  │
│  │ - enforces network policy by binary + host + port + protocol  │  │
│  └──────────────────────────┬────────────────────────────────────┘  │
│                             │ creates sandbox "oab"                 │
│                             ▼                                        │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ OpenShell Sandbox                                             │  │
│  │                                                               │  │
│  │  /sandbox/                                                    │  │
│  │   ├── config.toml              OpenAB config                  │  │
│  │   ├── .local/share/kiro-cli/   Kiro auth/session state        │  │
│  │   ├── .kiro/                   Kiro settings/skills           │  │
│  │   ├── bin/                     optional user tools            │  │
│  │   └── tmp/                     scratch files                  │  │
│  │                                                               │  │
│  │  openab run ──stdio JSON-RPC──► kiro-cli acp --trust-all-tools│  │
│  │       │                              │                        │  │
│  │       │ Discord API/Gateway          │ Kiro service endpoints │  │
│  └───────┼──────────────────────────────┼────────────────────────┘  │
│          │                              │                           │
│  ┌───────▼──────────────────────────────▼───────────────────────┐   │
│  │ Day 1 Network Policy                                          │   │
│  │ - /usr/local/bin/openab    -> Discord REST + WebSocket        │   │
│  │ - /usr/local/bin/kiro-cli* -> Kiro auth/runtime endpoints     │   │
│  │ - everything else is denied unless explicitly added           │   │
│  └───────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

In OpenShell, `/sandbox` is the writable runtime root for this guide. Keep OpenAB config, Kiro auth state, scratch files, and optional user-installed tools under `/sandbox`.

## TL;DR

From the OpenAB repo root:

```bash
read -rsp "Discord bot token: " DISCORD_BOT_TOKEN
echo
test -n "$DISCORD_BOT_TOKEN" || { echo "DISCORD_BOT_TOKEN is required"; exit 1; }
export DISCORD_BOT_TOKEN

openshell sandbox delete oab >/dev/null 2>&1 || true
openshell provider delete openab-discord >/dev/null 2>&1 || true

openshell provider create --name openab-discord --type generic --credential DISCORD_BOT_TOKEN

# Until ghcr.io/openabdev/openab-kiro-sandbox is published, build the wrapper locally.
docker build -t openab-kiro-sandbox -f openshell/Dockerfile.kiro .

openshell sandbox create --name oab \
  --provider openab-discord \
  --from openab-kiro-sandbox:latest \
  -- bash

openshell policy set oab --policy openshell/samples/kiro-discord-day1-policy.yaml --wait
openshell sandbox connect oab
```

Then inside the sandbox:

```bash
cd /sandbox
test -n "${DISCORD_BOT_TOKEN:-}" || { echo "DISCORD_BOT_TOKEN was not injected"; exit 1; }

cat > /sandbox/config.toml <<'EOF'
[discord]
bot_token = "${DISCORD_BOT_TOKEN}"
allow_all_channels = true
allow_all_users = true
allow_dm = false
message_processing_mode = "per-thread"

[agent]
command = "kiro-cli"
args = ["acp", "--trust-all-tools"]
working_dir = "/sandbox"

[agent.env]
HOME = "/sandbox"
PATH = "/sandbox/bin:/sandbox/.local/bin:/usr/local/bin:/usr/bin:/bin"
TMPDIR = "/sandbox/tmp"

[pool]
max_sessions = 1
session_ttl_hours = 1

[reactions]
enabled = true
remove_after_reply = false
EOF

kiro-cli login --use-device-flow
kiro-cli whoami
openab run -c /sandbox/config.toml
```

Mention the bot in Discord. The Day 1 test passes only when the bot replies while `openab run` is running inside the OpenShell sandbox.

## Supported Day 1 Environments

Use one of these host paths for this guide:

| Host path | Day 1 status | Notes |
| --- | --- | --- |
| Linux Debian/Ubuntu `amd64` or `arm64` + Docker Engine | Recommended Day 1 path | Best first path for a remote VM, Zeabur, EC2, home server, or Raspberry Pi. |
| macOS Apple Silicon + Docker Desktop | OpenShell-supported, OpenAB E2E-gated | Docker Desktop provides the Linux runtime substrate. Treat this as valid for OpenAB only after a real Discord reply from inside the OpenShell sandbox. |

OpenShell's current [support matrix](https://docs.nvidia.com/openshell/reference/support-matrix) lists these supported host platforms and Docker Desktop or Docker Engine as the Docker-backed gateway prerequisite. If a host/runtime combination is not listed there, do not use it as your first Day 1 path.

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
openshell --version
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
openshell --version
openshell sandbox list
```

If Docker Desktop needs macOS approval or first-run setup, finish that in the GUI before continuing.

## Why Policy Is Required

OpenShell sandboxes are default-deny for outbound network traffic. A working policy must name:

- the destination host and port
- the protocol shape, such as REST or WebSocket
- the exact binary allowed to connect

For OpenAB Day 1, this means `openab` needs Discord REST and Discord Gateway WebSocket access, while `kiro-cli` needs the Kiro service endpoints listed by Kiro's firewall documentation.

The sample policy uses `enforcement: enforce`. Do not use `enforcement: audit` as a shortcut for this quick start: audit is useful for investigation, but it is not a replacement for a known-good Day 1 allowlist.

## Day 2: Add Egress Deliberately

After the bot can reply in Discord, keep OpenShell policy narrow and add only the egress your workflow actually needs.

Watch the OpenShell logs in another terminal:

```bash
openshell logs oab --tail
```

Or inspect the last few minutes after a failed request:

```bash
openshell logs oab --since 5m
```

When an outbound request is blocked, identify three fields from the log:

- blocked host and port
- binary path
- protocol shape, such as REST or WebSocket

Then add the minimum endpoint for that binary:

```bash
openshell policy update oab \
  --add-endpoint api.example.com:443:read-only:rest:enforce \
  --binary /usr/local/bin/kiro-cli
```

Repeat until the logs stop showing relevant denials for the workflow you are testing. Do not add broad package registries, cloud APIs, search APIs, or arbitrary web access to the Day 1 policy unless that access is part of a tested sample.

For Kiro with other chat platforms, start with [OpenShell Kiro samples](../openshell/samples/kiro.md). Missing combinations are expected to be contributed after a real E2E pass.

## E2E Rules

For a test run to count:

- Launch `openab run` through OpenShell: `openshell sandbox connect`, `openshell sandbox exec`, or an OpenShell-generated SSH config.
- Do not use raw `docker exec` as proof. It can enter the OpenShell-created container without proving the same sandbox namespace, proxy, and policy path.
- Do not edit the coding CLI images to make this work. The OpenShell Kiro sandbox is a wrapper around the existing default Kiro image.
- Do not broaden this guide to arbitrary web, GitHub, npm, PyPI, cloud SDKs, or other agents. Those are Day 2 policy work.
- On macOS with Docker Desktop, the proof is the actual Discord bot connection and reply. If the Discord WebSocket cannot connect through the OpenShell-managed route, the guide is not yet valid for that host path.

For deeper policy, access, usability, and E2E notes, read [ADR: OpenShell OpenAB Preset Module](adr/openshell-openab-preset-module.md).

## OpenShell Or Kubernetes?

| Choose | When |
| --- | --- |
| OpenShell | You want a single-machine sandbox that can get the default OpenAB + Discord + Kiro path running quickly, with explicit egress policy. |
| Kubernetes | You already operate Kubernetes, need mature production rollout controls, or expect many agents, services, secrets, and network policies. |

## Troubleshooting

| Symptom | Meaning | Next step |
| --- | --- | --- |
| `docker info` fails | Docker is not ready | Start Docker Desktop or Docker Engine |
| `openshell sandbox list` fails | OpenShell gateway/driver is not ready | Fix OpenShell before continuing |
| `DISCORD_BOT_TOKEN was not injected` | Provider was not attached or token env was empty | Recreate the provider and sandbox |
| Discord connects on Linux but not macOS | macOS/Docker route may require proxy-aware WebSocket behavior | Treat as an OpenShell/macOS E2E blocker, not a successful run |
| Kiro login cannot reach auth/runtime endpoints | Kiro endpoint policy is incomplete | Check `openshell logs oab --since 10m` for denied host and binary |
| Agent tries to install into `/usr` | Runtime sandbox is non-root | Install only under `/sandbox` |

## Cleanup

```bash
openshell sandbox delete oab
openshell provider delete openab-discord
```

## Related Docs

- [Discord setup](discord.md)
- [Kiro CLI](kiro.md)
- [Agent-installable tools](agent-installable-tools.md)
- [OpenShell Kiro samples](../openshell/samples/kiro.md)
- [OpenShell policy ADR](adr/openshell-openab-preset-module.md)
- [OpenShell support matrix](https://docs.nvidia.com/openshell/reference/support-matrix)
- [OpenShell policy schema](https://docs.nvidia.com/openshell/reference/policy-schema)
- [OpenShell first network policy tutorial](https://docs.nvidia.com/openshell/get-started/tutorials/first-network-policy)
- [Kiro firewall documentation](https://kiro.dev/docs/cli/privacy-and-security/firewalls/)
