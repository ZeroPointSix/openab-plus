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

openshell provider create --name openab-discord --type generic --credential DISCORD_BOT_TOKEN

# Until ghcr.io/openabdev/openab-kiro-sandbox is published, build the wrapper locally.
docker build -t openab-kiro-sandbox -f openshell/Dockerfile.kiro .

openshell sandbox create --name oab \
  --provider openab-discord \
  --from openab-kiro-sandbox:latest \
  -- bash

cat > openab-kiro-day1-policy.yaml <<'EOF'
version: 1
filesystem_policy:
  include_workdir: true
  read_only:
    - /usr
    - /lib
    - /lib64
    - /bin
    - /sbin
    - /etc
    - /app
    - /var/log
    - /proc
    - /dev/urandom
  read_write:
    - /sandbox
    - /tmp
    - /dev/null
landlock:
  compatibility: best_effort
process:
  run_as_user: sandbox
  run_as_group: sandbox
network_policies:
  discord:
    name: openab-discord
    endpoints:
      - host: discord.com
        port: 443
        protocol: rest
        enforcement: enforce
        access: full
      - host: gateway.discord.gg
        port: 443
        protocol: websocket
        enforcement: enforce
        access: full
      - host: cdn.discordapp.com
        port: 443
        protocol: rest
        enforcement: enforce
        access: read-only
      - host: media.discordapp.net
        port: 443
        protocol: rest
        enforcement: enforce
        access: read-only
    binaries:
      - { path: /usr/local/bin/openab }
  kiro:
    name: kiro-cli
    endpoints:
      - { host: app.kiro.dev, port: 443, protocol: rest, enforcement: enforce, access: full }
      - { host: assets.app.kiro.dev, port: 443, protocol: rest, enforcement: enforce, access: read-only }
      - { host: cli.kiro.dev, port: 443, protocol: rest, enforcement: enforce, access: full }
      - { host: prod.us-east-1.auth.desktop.kiro.dev, port: 443, protocol: rest, enforcement: enforce, access: full }
      - { host: prod.us-east-1.telemetry.desktop.kiro.dev, port: 443, protocol: rest, enforcement: enforce, access: full }
      - { host: prod.download.desktop.kiro.dev, port: 443, protocol: rest, enforcement: enforce, access: read-only }
      - { host: prod.download.cli.kiro.dev, port: 443, protocol: rest, enforcement: enforce, access: read-only }
      - { host: desktop-release.q.us-east-1.amazonaws.com, port: 443, protocol: rest, enforcement: enforce, access: read-only }
      - { host: q.us-east-1.amazonaws.com, port: 443, protocol: rest, enforcement: enforce, access: full }
      - { host: q.eu-central-1.amazonaws.com, port: 443, protocol: rest, enforcement: enforce, access: full }
      - { host: runtime.us-east-1.kiro.dev, port: 443, protocol: rest, enforcement: enforce, access: full }
      - { host: runtime.eu-central-1.kiro.dev, port: 443, protocol: rest, enforcement: enforce, access: full }
      - { host: management.us-east-1.kiro.dev, port: 443, protocol: rest, enforcement: enforce, access: full }
      - { host: management.eu-central-1.kiro.dev, port: 443, protocol: rest, enforcement: enforce, access: full }
      - { host: telemetry.us-east-1.kiro.dev, port: 443, protocol: rest, enforcement: enforce, access: full }
      - { host: telemetry.eu-central-1.kiro.dev, port: 443, protocol: rest, enforcement: enforce, access: full }
      - { host: cognito-identity.us-east-1.amazonaws.com, port: 443, protocol: rest, enforcement: enforce, access: full }
      - { host: oidc.us-east-1.amazonaws.com, port: 443, protocol: rest, enforcement: enforce, access: full }
      - { host: client-telemetry.us-east-1.amazonaws.com, port: 443, protocol: rest, enforcement: enforce, access: full }
    binaries:
      - { path: /usr/local/bin/kiro-cli* }
EOF

openshell policy set oab --policy openab-kiro-day1-policy.yaml --wait
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

The policy above uses `enforcement: enforce`. Do not use `enforcement: audit` as a shortcut for this quick start: audit is useful for investigation, but it is not a replacement for a known-good Day 1 allowlist.

## E2E Rules

For a test run to count:

- Launch `openab run` through OpenShell: `openshell sandbox connect`, `openshell sandbox exec`, or an OpenShell-generated SSH config.
- Do not use raw `docker exec` as proof. It can enter the OpenShell-created container without proving the same sandbox namespace, proxy, and policy path.
- Do not edit the coding CLI images to make this work. The OpenShell Kiro sandbox is a wrapper around the existing default Kiro image.
- Do not broaden this guide to arbitrary web, GitHub, npm, PyPI, cloud SDKs, or other agents. Those are Day 2 policy work.
- On macOS with Docker Desktop, the proof is the actual Discord bot connection and reply. If the Discord WebSocket cannot connect through the OpenShell-managed route, the guide is not yet valid for that host path.

For deeper policy, access, usability, and E2E notes, read [ADR: OpenShell OpenAB Preset Module](adr/openshell-openab-preset-module.md).

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
rm -f openab-kiro-day1-policy.yaml
```

## Related Docs

- [Discord setup](discord.md)
- [Kiro CLI](kiro.md)
- [Agent-installable tools](agent-installable-tools.md)
- [OpenShell policy ADR](adr/openshell-openab-preset-module.md)
- [OpenShell policy schema](https://docs.nvidia.com/openshell/reference/policy-schema)
- [OpenShell first network policy tutorial](https://docs.nvidia.com/openshell/get-started/tutorials/first-network-policy)
- [Kiro firewall documentation](https://kiro.dev/docs/cli/privacy-and-security/firewalls/)
