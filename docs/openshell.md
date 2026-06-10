# OpenShell Dev Quick Start

Run one OpenAB Discord bot inside an [NVIDIA OpenShell](https://github.com/NVIDIA/OpenShell) dev sandbox.

This guide optimizes for the common local developer workflow: run the bot, edit files under `/sandbox`, and install common tools without needing root.

## What You Will Get

By the end, you should have:

- One OpenShell sandbox named `oab`
- One install-friendly dev environment with writable `/sandbox`
- Common developer tools available: `git`, `gh`, `curl`, `jq`, `python3`, `pip`, `node`, `npm`, and `go`
- One Discord bot connected to Discord
- One `openab-agent` session using ChatGPT/Codex subscription auth
- A bot that replies when mentioned in Discord

Install-friendly means user-local installs under `/sandbox`, not unrestricted root access. The dev sandbox supports Go, npm, Python virtualenv/pip, and standalone binaries without writing to `/usr/local/bin`.

## Prerequisites

- Docker is running on the host
- OpenShell CLI is installed
- You have a Discord bot token
- You have a ChatGPT account that can authenticate `openab-agent`

Install OpenShell:

```bash
curl -LsSf https://raw.githubusercontent.com/NVIDIA/OpenShell/main/install.sh | sh
```

If OpenShell cannot talk to Docker, add your user to the `docker` group and start a new login session:

```bash
sudo usermod -aG docker "$USER"
# Log out and back in, or run: loginctl terminate-user "$USER"
```

## 1. Set Your Discord Token On The Host

Keep the token in your shell environment. Do not paste it into `config.toml`.

```bash
export DISCORD_BOT_TOKEN="your-discord-bot-token"
```

## 2. Create The Dev Sandbox

Build the dev sandbox image from this repo:

```bash
docker build -t oab-native-dev-sandbox -f openshell/Dockerfile.dev .

openshell sandbox create --name oab \
  --from oab-native-dev-sandbox:latest \
  -- bash
```

After `ghcr.io/openabdev/openab-native-dev-sandbox:latest` is published, you can skip the local build and create the sandbox directly:

```bash
openshell sandbox create --name oab \
  --from ghcr.io/openabdev/openab-native-dev-sandbox:latest \
  -- bash
```

If you see `/home/sandbox/.profile: Permission denied` but `openshell sandbox list` shows `oab` as `Ready`, continue. The warning is cosmetic for this quick start.

Reconnect later with:

```bash
openshell sandbox connect oab
```

## 3. Verify The Dev Environment

Inside the sandbox:

```bash
export HOME=/sandbox
export PATH="/sandbox/bin:/sandbox/.local/bin:/sandbox/go/bin:$PATH"
export GOPATH=/sandbox/go
export GOCACHE=/sandbox/.cache/go-build
export npm_config_prefix=/sandbox/.local
export npm_config_cache=/sandbox/.cache/npm
export PIP_CACHE_DIR=/sandbox/.cache/pip

test -w /sandbox
openab --version
command -v openab-agent
git --version
gh --version
go version
node --version
npm --version
python3 --version
```

## 4. Create The OpenAB Config

Inside the sandbox, export the same token for this shell session:

```bash
export DISCORD_BOT_TOKEN="your-discord-bot-token"
```

Then create `/sandbox/config.toml`:

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
PATH = "/sandbox/bin:/sandbox/.local/bin:/sandbox/go/bin:/usr/local/bin:/usr/bin:/bin"
GOPATH = "/sandbox/go"
GOCACHE = "/sandbox/.cache/go-build"
npm_config_prefix = "/sandbox/.local"
npm_config_cache = "/sandbox/.cache/npm"
PIP_CACHE_DIR = "/sandbox/.cache/pip"
OPENAB_AGENT_OPENAI_MODEL = "gpt-5.4-mini"

[pool]
max_sessions = 1
session_ttl_hours = 1

[reactions]
enabled = true
remove_after_reply = false
EOF
```

Why these defaults:

- `bot_token = "${DISCORD_BOT_TOKEN}"` keeps the secret out of the file.
- `working_dir = "/sandbox"` gives the agent a writable project directory.
- `HOME = "/sandbox"` keeps auth at `/sandbox/.openab/agent/auth.json`.
- `PATH`, `GOPATH`, npm, and pip cache settings make user-local installs land under `/sandbox`.
- `allow_all_channels = true` is the easiest first run. Restrict channels after the bot works.

## 5. Authenticate The Agent

Inside the sandbox:

```bash
HOME=/sandbox openab-agent auth codex-oauth --no-browser
```

Open the printed URL in your browser. After approval, the browser will try to redirect to `localhost:1455`. Copy the full callback URL from the browser address bar and paste it back into the terminal.

Verify auth:

```bash
HOME=/sandbox openab-agent auth status
```

Expected result: the status command reports a valid Codex/OpenAI auth file under `/sandbox/.openab/agent/auth.json`.

## 6. Run OpenAB

Inside the sandbox:

```bash
export HOME=/sandbox
: "${DISCORD_BOT_TOKEN:?set DISCORD_BOT_TOKEN first}"
openab run -c /sandbox/config.toml
```

Expected log lines:

```text
config loaded agent_cmd=openab-agent
discord bot running
discord bot connected user=<your-bot-name>
registered global slash commands
```

Mention the bot in Discord. It should reply in the channel or thread.

## If The Bot Shows Offline

If `openab run` starts but Discord still shows the bot offline, keep the sandbox and launch the same config through Docker directly. This avoids a known local OpenShell exec/WebSocket failure mode for long-running Discord bots.

From the host:

```bash
: "${DISCORD_BOT_TOKEN:?export DISCORD_BOT_TOKEN in this host shell first}"

CONTAINER_ID="$(docker ps \
  --filter 'label=openshell.ai/sandbox-name=oab' \
  --format '{{.ID}}' \
  | head -n 1)"
: "${CONTAINER_ID:?could not find OpenShell container for sandbox oab}"

SANDBOX_UID="$(docker exec "$CONTAINER_ID" id -u sandbox)"
SANDBOX_GID="$(docker exec "$CONTAINER_ID" id -g sandbox)"

docker exec -u 0 \
  -e DISCORD_BOT_TOKEN="$DISCORD_BOT_TOKEN" \
  "$CONTAINER_ID" sh -lc '
  cd /sandbox &&
  setpriv --reuid='"$SANDBOX_UID"' --regid='"$SANDBOX_GID"' --clear-groups \
    env HOME=/sandbox USER=sandbox LOGNAME=sandbox \
    DISCORD_BOT_TOKEN="$DISCORD_BOT_TOKEN" \
    nohup openab run -c /sandbox/config.toml >/tmp/openab-discord.log 2>&1 &
'
```

Check logs:

```bash
docker exec -u 0 "$CONTAINER_ID" sh -lc 'tail -n 120 /tmp/openab-discord.log'
```

This still runs OpenAB in the OpenShell-created sandbox container. It only changes how the long-running process is started. Do not replace this with a plain Docker container; a plain `docker run` will not have the same OpenShell-created `/sandbox` setup.

## Installing Extra Tools

The dev sandbox supports user-local installs under `/sandbox`.

Standalone binary:

```bash
mkdir -p /sandbox/bin
cp ./tool /sandbox/bin/tool
chmod +x /sandbox/bin/tool
export PATH="/sandbox/bin:$PATH"
```

Go:

```bash
export GOPATH=/sandbox/go
export GOCACHE=/sandbox/.cache/go-build
export PATH="/sandbox/go/bin:$PATH"
go install github.com/googleworkspace/cli/cmd/gws@latest
gws --help
```

npm:

```bash
export npm_config_prefix=/sandbox/.local
export npm_config_cache=/sandbox/.cache/npm
export PATH="/sandbox/.local/bin:$PATH"
npm install -g <package>
```

Python:

```bash
python3 -m venv /sandbox/.venv
. /sandbox/.venv/bin/activate
pip install <package>
```

This sandbox does not promise `sudo apt-get install ...` or writes to `/usr/local/bin` after creation. If you need additional system packages, bake them into a custom dev image:

```dockerfile
FROM oab-native-dev-sandbox:latest
USER root
RUN apt-get update && apt-get install -y --no-install-recommends <package> \
  && rm -rf /var/lib/apt/lists/*
USER sandbox
```

Build your custom image, then create the OpenShell sandbox from that image instead of `oab-native-dev-sandbox:latest`.

## Troubleshooting

| Symptom | Check | Fix |
| --- | --- | --- |
| `failed to query Docker daemon version` | Docker access | Add user to `docker` group and start a new login session |
| Dev image not found | `docker pull ghcr.io/openabdev/openab-native-dev-sandbox:latest` fails | Use the local `docker build -f openshell/Dockerfile.dev .` path |
| Bot token error | `test -n "$DISCORD_BOT_TOKEN" && echo set` | Re-export the token in the shell that starts OpenAB |
| Auth file searched under `/root` | Log says `/root/.openab/...` | Run with `HOME=/sandbox` |
| Bot online but no reply | `openab-agent auth status` | Re-run `openab-agent auth codex-oauth --no-browser` |
| Network or model calls blocked | Discord connects, but model/tool calls fail | See the OpenShell preset ADR; broad policy recommendations are still testing in progress |
| Tool install says `/usr/local/bin` or `apt` is not writable | The running sandbox user is non-root | Use the user-local install paths above, or bake system packages into the dev image |
| Runtime package install hangs | Package manager is building from source | Prefer tools preinstalled in the dev image; avoid `brew install` during a bot turn |

## E2E Test Rules

When testing this guide with an agent, keep the test honest:

- Use the dev sandbox image or `openshell/Dockerfile.dev`.
- Treat the guide author and the E2E test subject as separate roles.
- If the test subject hits a build or setup failure, let the test subject diagnose, edit, rebuild, and report the fix. Do not patch the guide or image from outside the test and then count that run as one-shot success.
- Do not rewrite OpenShell policy files to make the test pass.
- Do not silently switch to a different auth format.
- Do not copy host `~/.codex/auth.json` into `/sandbox/.openab/agent/auth.json`; `openab-agent` has its own auth file shape.
- If auth is unavailable, stop at the auth step and report that browser login is required.
- If a policy file fails to apply, report a docs/policy compatibility issue instead of generating a private fixed policy.
- Always delete disposable test sandboxes and scratch files after the run.

## Optional: Restrict To One Discord Channel

After the bot works, change `[discord]`:

```toml
[discord]
bot_token = "${DISCORD_BOT_TOKEN}"
allow_all_channels = false
allowed_channels = ["your-discord-channel-id"]
allow_all_users = true
allow_dm = false
message_processing_mode = "per-thread"
```

Restart `openab run` after changing the config.

## Network Policy

OpenShell is policy-oriented. Some environments start with default-deny egress, which can block Discord, ChatGPT, GitHub, npm, PyPI, or other tools. The quick start above intentionally does not require users to understand policy files.

The OpenAB broad policy proposal is still testing in progress. Treat current policy files and commands as ADR-level recommendations, not a guaranteed quick-start contract.

For policy design, recommended tiers, and current known gaps, see [OpenShell OpenAB preset module ADR](adr/openshell-openab-preset-module.md).

## Cleanup

```bash
openshell sandbox delete oab
```

## Advanced Reading

- [OpenShell OpenAB preset module ADR](adr/openshell-openab-preset-module.md) — discussion of future `safe-agent`, `web-agent`, and `dev-agent` presets, including network policy recommendations that are still testing in progress.
- [Native Agent](native-agent.md) — `openab-agent` auth and model options.
- [Secrets Management](secrets-management.md) — production secret patterns.
