# OpenShell Quick Start

Run one OpenAB Discord bot inside an [NVIDIA OpenShell](https://github.com/NVIDIA/OpenShell) sandbox.

This guide optimizes for the normal OpenAB agent workflow: keep the image lean, make the agent home writable, and let the agent install extra tools at runtime into its own home directory.

## Hard Requirements For Agents

If a user asks to install OpenAB "with OpenShell structure" or "per docs/openshell.md", this guide means **real OpenShell**, not a host-local imitation.

Agents must follow these rules:

- Do not install OpenAB directly on the host as a substitute for OpenShell.
- Do not claim that host paths such as `~/.local/bin`, `/Users/<user>`, or a normal shell environment "mimic" OpenShell.
- Do not continue past missing prerequisites by changing the architecture. If `docker` or `openshell` is missing, install or ask the user to install that prerequisite, then resume this guide.
- Do not run the long-lived bot from the host shell for this guide. The bot must run inside the OpenShell-created sandbox container.
- `/sandbox` means the sandbox filesystem created by OpenShell. It does not mean an arbitrary directory on the host.

Success requires `openshell sandbox list` to show the sandbox and `openshell sandbox connect <name>` to enter it.

## What You Will Get

By the end, you should have:

- One OpenShell sandbox named `oab`
- A writable agent home at `/sandbox`
- Runtime tool install paths under `/sandbox/bin` and `/sandbox/.local`
- One Discord bot connected to Discord
- One `openab-agent` session using ChatGPT/Codex subscription auth
- A bot that replies when mentioned in Discord

Install-friendly does **not** mean root access or `apt-get` inside the running sandbox. It means the agent can install user-local tools under `/sandbox` without changing the image.

## Prerequisites

- Docker is running on the host
- OpenShell CLI is installed
- You have a Discord bot token
- You have a ChatGPT account that can authenticate `openab-agent`

Preflight check:

```bash
command -v docker
docker info
command -v openshell
openshell sandbox list
```

If any command fails, stop here and fix that prerequisite. Do not install OpenAB directly on the host to work around the failure.

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

## 2. Build The OpenShell Sandbox Image

Build the OpenShell sandbox image from this repo:

```bash
docker build -t oab-native-sandbox -f openshell/Dockerfile .
```

Run this on the host, before creating the sandbox. Do not install OpenAB binaries into the host user's `~/.local/bin` for this guide.

This image is intentionally small. It does not add Go, Node, Python, cloud CLIs, or every tool an agent might someday need. Extra tools should be installed later by the agent into `/sandbox`, following [Agent-Installable Tools](agent-installable-tools.md).

Expected image contract:

- The image can start OpenAB and its selected agent runtime.
- The image does not preinstall workflow CLIs such as `gh`, `aws`, `gcloud`, `kubectl`, `terraform`, `wrangler`, or `gws`.
- The running sandbox stays non-root. Runtime installs go to `/sandbox/bin` or `/sandbox/.local/bin`, not `/usr`, `/opt`, or `/usr/local/bin`.

## 3. Create The Sandbox

```bash
openshell sandbox create --name oab \
  --from oab-native-sandbox:latest \
  -- bash
```

If you see `/home/sandbox/.profile: Permission denied` but `openshell sandbox list` shows `oab` as `Ready`, continue. The warning is cosmetic for this quick start.

Reconnect later with:

```bash
openshell sandbox connect oab
```

## 4. Verify Writable Runtime Install Paths

Inside the sandbox:

```bash
test "$(pwd)" = "/sandbox" || cd /sandbox
test -d /sandbox
test "$(id -un)" = "sandbox"

export HOME=/sandbox
export PATH="/sandbox/bin:/sandbox/.local/bin:$PATH"
export TMPDIR=/sandbox/tmp

test -w /sandbox
mkdir -p /sandbox/bin /sandbox/.local/bin /sandbox/tmp
printf '#!/bin/sh\necho openab-local-install-ok\n' > /sandbox/bin/openab-local-install-test
chmod +x /sandbox/bin/openab-local-install-test
openab-local-install-test

command -v openab
command -v openab-agent
command -v gh && exit 1 || echo "gh missing as expected"
test -w /usr/local/bin && exit 1 || echo "/usr/local/bin not writable as expected"
```

Expected output:

```text
openab-local-install-ok
```

This proves the agent can install standalone tools into `/sandbox/bin` at runtime. For real tools, use the same sandbox-directory install pattern from [Agent-Installable Tools](agent-installable-tools.md).

For a full build acceptance test, verify all of the following:

- `whoami` is the sandbox user, not `root`.
- `/sandbox`, `/sandbox/bin`, `/sandbox/.local/bin`, and `/sandbox/tmp` are writable.
- `/usr`, `/opt`, and `/usr/local/bin` are not runtime install targets.
- `gh`, `aws`, `gcloud`, `kubectl`, `terraform`, `wrangler`, and `gws` are absent from a clean image unless a specific downstream image intentionally adds them.
- A standalone binary can be downloaded to `/sandbox/tmp`, copied to `/sandbox/bin`, marked executable, and run from `PATH`.
- The same tool is still present after a sandbox/container restart when `/sandbox` is persistent.

## 5. Create The OpenAB Config

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

Why these defaults:

- `bot_token = "${DISCORD_BOT_TOKEN}"` keeps the secret out of the file.
- `working_dir = "/sandbox"` gives the agent a writable project directory.
- `HOME = "/sandbox"` keeps auth at `/sandbox/.openab/agent/auth.json`.
- `PATH` makes runtime-installed tools in `/sandbox/bin` and `/sandbox/.local/bin` available to the agent.
- `TMPDIR = "/sandbox/tmp"` keeps scratch work inside the writable sandbox home.
- `allow_all_channels = true` is the easiest first run. Restrict channels after the bot works.

## 6. Authenticate The Agent

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

## 7. Run OpenAB

Inside the sandbox:

```bash
export HOME=/sandbox
export PATH="/sandbox/bin:/sandbox/.local/bin:$PATH"
export TMPDIR=/sandbox/tmp
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

If `openab run` starts but Discord still shows the bot offline, keep the sandbox and launch the same config through Docker directly. This avoids a local OpenShell exec/WebSocket failure mode seen during E2E testing for long-running Discord bots.

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
    PATH=/sandbox/bin:/sandbox/.local/bin:/usr/local/bin:/usr/bin:/bin \
    TMPDIR=/sandbox/tmp \
    DISCORD_BOT_TOKEN="$DISCORD_BOT_TOKEN" \
    nohup openab run -c /sandbox/config.toml >/tmp/openab-discord.log 2>&1 &
'
```

Check logs:

```bash
docker exec -u 0 "$CONTAINER_ID" sh -lc 'tail -n 120 /tmp/openab-discord.log'
```

This still runs OpenAB in the OpenShell-created sandbox container. It only changes how the long-running process is started. Do not replace this with a plain Docker container; a plain `docker run` will not have the same OpenShell-created `/sandbox` setup.

## Installing Extra Tools At Runtime

Use the OpenAB sandbox-directory install pattern:

```bash
mkdir -p /sandbox/bin
curl -fsSL -o /sandbox/bin/<tool> "<official-linux-binary-url>"
chmod +x /sandbox/bin/<tool>
export PATH="/sandbox/bin:$PATH"
<tool> --version
```

For tools distributed as archives, download to `/sandbox/tmp`, extract there, and copy only the final executable into `/sandbox/bin`.

For tools distributed as `.deb` packages, extract without root:

```bash
mkdir -p /sandbox/bin /sandbox/tmp/deb-extract
curl -fsSL -o /sandbox/tmp/package.deb "<deb-url>"
dpkg-deb -x /sandbox/tmp/package.deb /sandbox/tmp/deb-extract
cp /sandbox/tmp/deb-extract/usr/bin/<binary> /sandbox/bin/
chmod +x /sandbox/bin/<binary>
rm -rf /sandbox/tmp/package.deb /sandbox/tmp/deb-extract
```

Rules for agents:

- Do not use `sudo`.
- Do not write to `/usr`, `/opt`, or `/usr/local/bin`.
- Install binaries to `/sandbox/bin`.
- Install larger user-local tool trees under `/sandbox`.
- Use `/sandbox/tmp` for scratch work.
- Detect architecture before downloading binaries.
- Verify every install with `<tool> --version` or equivalent.

See [Agent-Installable Tools](agent-installable-tools.md) for the full pattern.

## Troubleshooting

| Symptom | Check | Fix |
| --- | --- | --- |
| `failed to query Docker daemon version` | Docker access | Add user to `docker` group and start a new login session |
| Bot token error | `test -n "$DISCORD_BOT_TOKEN" && echo set` | Re-export the token in the shell that starts OpenAB |
| Auth file searched under `/root` | Log says `/root/.openab/...` | Run with `HOME=/sandbox` |
| Bot online but no reply | `openab-agent auth status` | Re-run `openab-agent auth codex-oauth --no-browser` |
| Network or model calls blocked | Discord connects, but model/tool calls fail | See the OpenShell preset ADR; broad policy recommendations are still testing in progress |
| Tool install says `/usr/local/bin` or `apt` is not writable | The running sandbox user is non-root | Install to `/sandbox/bin` or another `/sandbox` path |
| Tool requires system libraries not present in the image | Binary exists but fails at launch | Choose a static upstream binary, extract compatible `.deb` dependencies into `/sandbox`, or build a custom image only for that deployment |
| Agent installed OpenAB into `/Users/<user>` or host `~/.local/bin` | The guide was not followed; this is a host-local install, not OpenShell | Stop that run. Install Docker/OpenShell if missing, create the sandbox with `openshell sandbox create --from`, then install/run only inside `/sandbox` |

## E2E Test Rules

When testing this guide with an agent, keep the test honest:

- Use `openshell/Dockerfile`.
- Require real OpenShell evidence: `command -v openshell`, `openshell sandbox list`, and `openshell sandbox connect <name>` must work.
- Treat missing `docker` or missing `openshell` as a blocker, not as permission to install OpenAB on the host.
- Treat the guide author and the E2E test subject as separate roles.
- If the test subject hits a build or setup failure, let the test subject diagnose, edit, rebuild, and report the fix. Do not patch the guide or image from outside the test and then count that run as one-shot success.
- Do not rewrite OpenShell policy files to make the test pass.
- Do not silently switch to a different auth format.
- Do not copy host `~/.codex/auth.json` into `/sandbox/.openab/agent/auth.json`; `openab-agent` has its own auth file shape.
- If auth is unavailable, stop at the auth step and report that browser login is required.
- If a policy file fails to apply, report a docs/policy compatibility issue instead of generating a private fixed policy.
- Prove runtime installability with at least one `/sandbox/bin` install test.
- Fail the test if OpenAB, `openab-agent`, or workflow tools are installed into host paths such as `/Users/<user>/.local/bin`.
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

- [Agent-Installable Tools](agent-installable-tools.md) — runtime install pattern for tools under the agent home directory.
- [OpenShell OpenAB preset module ADR](adr/openshell-openab-preset-module.md) — discussion of future `safe-agent`, `web-agent`, and `dev-agent` presets, including network policy recommendations that are still testing in progress.
- [Native Agent](native-agent.md) — `openab-agent` auth and model options.
- [Secrets Management](secrets-management.md) — production secret patterns.
