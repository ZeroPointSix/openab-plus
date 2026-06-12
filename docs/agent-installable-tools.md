# Agent-Installable Tools

## How to Install Extra Tools

You don't need to read this doc yourself. Just ask your agent:

```
per docs/* from OpenAB GitHub repo, how to install <TOOL_NAME> for my OAB agent
```

Your agent will query the relevant docs under `docs/`, find the recommended approach, and guide you through the entire installation — or just do it for you. That's it. One prompt, done.

## How It Works

```
  Human                        Agent                         OpenAB Pod
  ┌─────────────────┐         ┌──────────────────┐          ┌──────────────────────────┐
  │                  │         │                  │          │  Container (read-only)   │
  │ "install glab   │────────►│ reads docs/*     │          │  ┌────────────────────┐  │
  │  for my OAB     │         │ from OpenAB repo │          │  │ curl, unzip, tini  │  │
  │  agent"         │         │                  │          │  │ bootstrap/runtime  │  │
  │                  │         │ finds install    │          │  └────────────────────┘  │
  │                  │         │ steps for glab   │          │                          │
  │                  │         │                  │          │  PVC (persistent ~/):    │
  │                  │         │ executes:        │          │  ┌────────────────────┐  │
  │                  │         │  curl ─► extract │─────────►│  │ ~/bin/             │  │
  │                  │         │  ─► ~/bin/glab   │          │  │  ├── glab    ✅ new│  │
  │                  │         │  ─► verify       │          │  │  ├── aws          │  │
  │                  │         │                  │          │  │  ├── ssh          │  │
  │  "done! glab    │◄────────│ "glab v1.46      │          │  │  ├── terraform    │  │
  │   ready to use" │         │  installed ✅"    │          │  │  └── kubectl      │  │
  │                  │         │                  │          │  │                    │  │
  └─────────────────┘         └──────────────────┘          │  │ ~/.ssh/  ~/.config/│  │
                                                            │  │ ~/.kiro/ ~/aws-cli/│  │
                                                            │  └────────────────────┘  │
                                                            └──────────────────────────┘

  ┌─────────────────────────────────────────────────────────────────────────────┐
  │  Migration: move PVC to new node / cluster / cloud                         │
  │                                                                            │
  │  Old Cluster              PVC                    New Cluster               │
  │  ┌──────────┐     ┌────────────────┐     ┌──────────────┐                 │
  │  │ Pod ──────┼────►│ ~/bin/         │────►│ New Pod      │                 │
  │  │ (delete)  │     │ ~/.ssh/        │     │ (attach PVC) │                 │
  │  └──────────┘     │ ~/.config/     │     │              │                 │
  │                    │ ~/.kiro/       │     │ Everything   │                 │
  │                    │ ~/aws-cli/     │     │ just works™  │                 │
  │                    └────────────────┘     └──────────────┘                 │
  │                                                                            │
  │  Zero reinstallation. All tools, configs, keys, and agent memory persist. │
  └─────────────────────────────────────────────────────────────────────────────┘
```

## Why This Pattern

OpenAB keeps its Docker image minimal — only bootstrap/runtime essentials and the selected agent runtime ship in the Dockerfile. Workflow tools are installed **at runtime by the agent** into the home directory (`~/bin/` or `/sandbox/bin/`). This is a deliberate design choice:

- **Lean image, infinite extensibility** — the Dockerfile never grows. Need AWS CLI today, Terraform tomorrow, glab next week? Same image, same pattern. No rebuild, no redeploy.
- **Doc-driven, AI-first** — documentation is written for agents to consume. Humans just say what they need; the agent reads the docs and executes.
- **No gatekeeping** — adding a new tool doesn't require a PR to the Dockerfile, a new Docker build, or a Helm upgrade. Any agent can install any tool at any time.
- **Full persistence on PVC** — everything installed to `~/bin/` and `~/` lives on the Persistent Volume Claim. This means:
  - **Pod restart** — tools are still there
  - **Helm upgrade** — tools are still there
  - **Migrate the PVC to a new node / new cluster** — tools, configs, credentials, SSH keys — everything moves with it. Your agent's entire environment is portable.
  - **Upgrade a tool** — just re-run the install. The old binary is overwritten in place.
- **No Dockerfile sprawl** — if we baked GitLab CLI into the image, we'd have no reason to reject AWS CLI, gcloud, azure CLI, wrangler, kubectl, terraform... The image would bloat endlessly. This pattern keeps the core small and lets each deployment customize itself.

## What Ships in the Image

| Category | Examples | Why it's built-in |
|---|---|---|
| Bootstrap | `curl`, `unzip` | Needed to download and unpack runtime-installed tools |
| Runtime | `procps`, `tini` | Healthcheck and process management |
| Base workflow | `git`, `ripgrep` | Small, common primitives used by most coding agents |
| Selected agent runtime | `openab-agent`, `kiro-cli`, `claude`, `codex`, etc. depending on image | The image must be able to start its intended agent |

Everything else is **agent-installable**. Cloud, SCM, admin, deployment, and workspace CLIs such as `gh`, `aws`, `gcloud`, `kubectl`, `terraform`, `wrangler`, and `gws` should not be pre-baked into the default agent images.

## Common Tools

The following tools are commonly installed by agents. This doc does **not** hardcode install commands — they change over time. Instead, the agent should look up the **official upstream install instructions** and adapt them to the [constraints](#constraints-for-agents) below.

| Tool | Upstream Install Docs |
|------|----------------------|
| **GitHub CLI** (`gh`) | [cli.github.com/manual/installation](https://cli.github.com/manual/installation) — use the release tarball or `.deb` extract pattern for runtime installs. |
| **OpenSSH** (`ssh`, `scp`, `ssh-keygen`) | [packages.debian.org/bookworm/openssh-client](https://packages.debian.org/bookworm/amd64/openssh-client/download) — use `.deb` extract pattern. Also see [remote-ssh-debugging.md](refarch/remote-ssh-debugging.md) for SSH key setup. |
| **AWS CLI v2** (`aws`) | [docs.aws.amazon.com/cli/latest/userguide/install-cliv2-linux.html](https://docs.aws.amazon.com/cli/latest/userguide/install-cliv2-linux.html) |
| **GitLab CLI** (`glab`) | [gitlab.com/gitlab-org/cli/-/releases](https://gitlab.com/gitlab-org/cli/-/releases) |
| **Cloudflare Wrangler** (`wrangler`) | [developers.cloudflare.com/workers/wrangler/install-and-update](https://developers.cloudflare.com/workers/wrangler/install-and-update/) |
| **Terraform** (`terraform`) | [developer.hashicorp.com/terraform/install](https://developer.hashicorp.com/terraform/install) |
| **kubectl** | [kubernetes.io/docs/tasks/tools/install-kubectl-linux](https://kubernetes.io/docs/tasks/tools/install-kubectl-linux/) |

> This is not an exhaustive list. Any tool with a prebuilt Linux binary can be installed using this pattern.

## Constraints for Agents

When installing any tool, the agent **must** follow these rules:

1. **No `sudo`** — the container has no root access and a read-only root filesystem
2. **Install to `~/bin/` or `/sandbox/bin/`** (binaries) or `~/`/`/sandbox` (larger installs like `~/aws-cli/`) — never write to `/usr/`, `/opt/`, or other system paths
3. **Detect architecture** — the pod may be ARM64 (`aarch64`) or AMD64 (`x86_64`). Always check `uname -m` and download the correct binary.
4. **Use `/tmp/` or `/sandbox/tmp` for scratch** — download and extract in scratch space, copy the final binary to `~/bin/` or `/sandbox/bin/`, then clean up
5. **Verify after install** — run `<tool> --version` or equivalent to confirm it works
6. **`export PATH="$HOME/bin:/sandbox/bin:/sandbox/.local/bin:$PATH"`** — ensure user-local install paths are in PATH before verification
7. **Look up the latest version from upstream** — do not hardcode version numbers; always fetch the latest stable release

### `.deb` Package Pattern (for tools without standalone binaries)

Some tools (like OpenSSH) are only distributed as `.deb` packages. Extract without `sudo`:

```bash
mkdir -p ~/bin /tmp/deb-extract
curl -fsSL -o /tmp/package.deb "<deb-url>"
dpkg-deb -x /tmp/package.deb /tmp/deb-extract
cp /tmp/deb-extract/usr/bin/<binary> ~/bin/
chmod +x ~/bin/<binary>
rm -rf /tmp/package.deb /tmp/deb-extract
```

## Build Test Objectives

Use these checks before declaring an OpenShell/OpenAB image ready:

1. A clean image does not contain workflow CLIs that should be runtime-installed:

   ```bash
   command -v gh && exit 1 || echo "gh missing as expected"
   command -v gcloud && exit 1 || echo "gcloud missing as expected"
   command -v aws && exit 1 || echo "aws missing as expected"
   ```

2. The sandbox runs as the non-root agent user and keeps system paths non-writable:

   ```bash
   id -un
   test -w /usr/local/bin && exit 1 || echo "/usr/local/bin not writable as expected"
   ```

3. The agent can install a standalone tool without `sudo`, `apt-get install`, `brew`, or writes to `/usr`:

   ```bash
   export HOME=/sandbox
   export PATH="/sandbox/bin:/sandbox/.local/bin:$PATH"
   mkdir -p /sandbox/bin /sandbox/tmp
   case "$(uname -m)" in
     x86_64) yq_arch=amd64 ;;
     aarch64|arm64) yq_arch=arm64 ;;
     *) echo "unsupported arch: $(uname -m)" >&2; exit 1 ;;
   esac
   curl -fsSL -o /sandbox/tmp/yq.tar.gz \
     "https://github.com/mikefarah/yq/releases/latest/download/yq_linux_${yq_arch}.tar.gz"
   tar -xzf /sandbox/tmp/yq.tar.gz -C /sandbox/tmp
   cp "/sandbox/tmp/yq_linux_${yq_arch}" /sandbox/bin/yq
   chmod +x /sandbox/bin/yq
   yq --version
   ```

4. The installed tool remains available after the sandbox/container is restarted, assuming `/sandbox` is persistent.

## Persistence & Portability

Everything under `~/` is mounted on a PVC — see the migration diagram above. Key directories that persist:

```
~/bin/           → all installed tool binaries
~/.aws/          → AWS CLI config and credentials
~/npm-global/    → npm-installed tools (wrangler, etc.)
~/.ssh/          → SSH keys and config
~/.config/       → tool configs (glab, wrangler, etc.)
~/.kiro/         → agent steering docs and memory
~/.openab/       → OpenAB runtime data (cronjobs, etc.)
```

The only time tools are lost is if the PVC itself is deleted.

## Adding a New Tool Doc

If you're contributing a doc for a new tool (e.g., `docs/gitlab.md`, `docs/cloudflare.md`):

1. **Keep it short** — provide the install commands with architecture detection and a verification step
2. **Reference this doc** — link back here for the general pattern and philosophy
3. **Test the prompt** — verify that asking your agent _"per docs/your-tool.md from OpenAB repo, install X for me"_ actually works end-to-end

## Advanced: Sidecars and Init Containers

For use cases that go beyond installing CLI tools — such as running a network tunnel, a database sidecar, or pre-installing a deterministic toolset via init containers — see [docs/sidecar.md](sidecar.md).
