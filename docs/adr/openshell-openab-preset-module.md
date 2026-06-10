# ADR: OpenShell OpenAB Preset Module

- **Status:** Discussion
- **Date:** 2026-06-10
- **Author:** OpenAB POC contributors
- **Related:** [OpenShell](../openshell.md), [ADR: OpenShell-Compatible Gateway WebSocket Authentication](./openshell-websocket-auth.md), [ADR: Custom Gateway](./custom-gateway.md)

---

## 1. Context

OpenAB supports deployment through Kubernetes-style infrastructure and through
OpenShell sandboxes. Kubernetes is operationally familiar to many platform
teams, but it is heavy for average clients. OpenShell gives OpenAB a stronger
agent sandbox boundary, but the current setup experience exposes too many
low-level details: sandbox image shape, provider credentials, network policy,
WebSocket policy, tool installation, writable paths, and gateway placement.

During the Google Chat + Kiro + OpenShell POC, the team validated the broad
shape but found the OpenShell route difficult to operate reliably without a
large amount of explicit configuration. The main blockers were:

- WebSocket credential rewriting and gateway authentication.
- Local proxy or bridge routing from the sandbox to the gateway.
- Kiro and model endpoint policy discovery.
- Runtime tool installation limitations.
- Provider credential values appearing as resolver placeholders rather than raw
  environment values.
- Runtime-vs-development expectations: the native quick-start image can run a
  bot but does not support `sudo`, `apt-get`, or system-wide installs.
- Policy file compatibility: the proposed broad OAB policy must be tested
  against the currently documented OpenShell CLI before it is treated as a
  getting-started requirement.

This ADR proposes discussing an OpenAB-owned OpenShell preset module that makes
the common case simple while preserving OpenShell's security value.

## 2. Question

Is there a very simple, singular configuration that enables normal OpenAB agent
functionality, including web access and common tools, without forcing average
clients to understand every OpenShell policy primitive?

Today, based on the POC, the answer is effectively **no**. OpenShell does not
currently feel like it has a single "just make it work like Kubernetes" config
for OpenAB agents.

The closest product direction would be an opinionated preset, for example:

```text
openshell sandbox create --preset openab-full-agent
```

or:

```yaml
preset: openab-full-agent
network:
  mode: broad_web
tools:
  mode: image_bundled
filesystem:
  writable:
    - /sandbox
    - /tmp
    - /sandbox/.local
secrets:
  provider: openab
gateway:
  mode: host_or_managed
```

This should be treated as a product/API sketch, not a current OpenShell feature.

## 2.1 Current Testing Status

Status as of 2026-06-11: **testing in progress**.

The current `docs/openshell.md` quick start is intended to prove the local
developer path:

```text
create sandbox
verify install-friendly /sandbox paths
write config
authenticate openab-agent
run openab
connect Discord bot
reply to a mention
```

It should not be read as proof that the sandbox has unrestricted root access,
nor that the broad network policy is stable across OpenShell versions.

Recent local E2E testing found:

- The bot can come online and reply when launched with `HOME=/sandbox` and a
  valid `openab-agent` auth file.
- The original runtime image is not suitable for system package installation.
  Attempts to install tools into `/usr/local/bin` or use `apt` fail under the
  non-root sandbox user.
- Most local users need an install-friendly dev sandbox. The recommended
  direction is user-local installs under `/sandbox`, with common tools
  preinstalled in the image.
- Small standalone binaries can be staged under `/sandbox/bin`; Go, npm, and
  Python installs should write under `/sandbox/go`, `/sandbox/.local`, and
  `/sandbox/.venv` or other `/sandbox` paths.
- Host Codex auth (`~/.codex/auth.json`) is not interchangeable with
  `openab-agent` auth (`/sandbox/.openab/agent/auth.json`).
- The proposed `oab-open.yaml` broad policy needs compatibility validation
  against the OpenShell CLI that users install. Until that is validated, policy
  commands belong in this ADR or policy-specific docs, not in the required
  quick-start path.

Recommended docs stance while testing continues:

- Keep `docs/openshell.md` focused on the install-friendly local dev quick
  start.
- Link policy recommendations from the quick start, but do not require policy
  edits for the first successful run unless the tested CLI/image combination
  requires them.
- If a policy file fails to apply during E2E, treat that as a policy/docs issue.
  Do not rewrite the policy in a private scratch file and call the guide
  successful.

## 3. Core Difference

| Topic | Kubernetes Default Mental Model | OpenShell Default Mental Model |
|---|---|---|
| Agent rights by default | A pod/container can often run with broad outbound network, mounted env/secrets, writable container FS depending on image/user | Sandbox user, limited writable paths, network endpoints must be allowed, credentials resolved through provider model |
| Filesystem | Image FS plus writable container layer; can install tools if root/package manager is available | `/usr`, `/etc`, `/lib`, package-manager state are effectively not runtime-editable; writable paths are mainly `/sandbox` and `/tmp` |
| Network/web | Usually broad outbound unless NetworkPolicy/firewall restricts it | Network is policy-driven; endpoints often need allowlisting |
| Tool availability | Whatever is in image; can sometimes apt/pip/npm install at runtime | Better to bake tools into image; runtime installs are friction-heavy |
| Secrets | Kubernetes Secrets/env/volumes are straightforward and raw values enter container | OpenShell provider values may appear as resolver handles, not raw env values; app must cooperate with resolution/rewrites |
| WebSocket/gateway | Usually direct networking between services/pods or host ingress | WebSocket policy/credential rewrite/proxy behavior can be tricky |
| Setup style | YAML-heavy but familiar: Deployment, Secret, Service, Ingress | Sandbox image + provider + policy + uploads + endpoint discovery |
| Failure mode | Misconfigured infra, image, ingress, RBAC | Policy blocks, credential resolution mismatch, non-writable paths, endpoint allowlist gaps |
| Average client UX | Complex, but many people know the pattern | Safer but less obvious; needs presets/templates to be approachable |
| "Enable everything" path | Run privileged/broad-network container if the operator accepts risk | Not really first-class; broad policy is possible in theory but cuts against OpenShell's design |

## 4. What Can Be Enabled

| Capability | Kubernetes | OpenShell |
|---|---|---|
| Full web browsing / HTTP calls | Usually yes by default | Possible, but needs broad or wildcard-like network policy |
| Google Chat webhook | Via ingress/service/gateway | Gateway likely still best outside sandbox or as managed component |
| Google API credentials | Mount secret/env directly | Prefer host gateway or provider-injected short-lived tokens |
| CLI tools like `gws`, `gh`, `aws`, `python`, `node` | Bake into image or install runtime | Strongly prefer bake into image |
| File editing | Whatever the container user can write | Mostly `/sandbox` unless policy/image is designed otherwise |
| Package installs | Easy if root + network + package manager | Not good in runtime sandbox; use image build or dev sandbox |
| Long-running bot | Deployment/restart policy | Possible, but more host/OpenShell lifecycle dependent |
| Strict per-agent security | Possible but requires Kubernetes hardening | Native strength of OpenShell |

## 5. Setup Shape

Kubernetes setup is heavier upfront but conceptually linear:

```text
build image
create secrets
deploy pod
expose service/ingress
set env
run
```

OpenShell setup, as seen in the POC, is more segmented:

```text
build OpenShell-compatible image
create provider credentials
create sandbox
upload config/state
set network policies
handle credential rewrite/resolution
handle gateway bridge/proxy
run OpenAB
discover blocked endpoints
iterate
```

The OpenShell path is harder for average clients unless OpenAB or OpenShell
ships a blessed preset.

## 6. Proposed Presets

For OpenAB productization, average clients should not need to understand
OpenShell policy primitives. OpenAB should expose a small number of presets.

| Preset | Purpose | Default Rights |
|---|---|---|
| `safe-agent` | Enterprise/security-sensitive deployments | Narrow endpoints, no runtime installs, secrets via provider |
| `web-agent` | Most normal OpenAB bots | Broad HTTPS/WebSocket outbound, common tools baked in, writable `/sandbox` |
| `dev-agent` | Debug/setup only | Broader network, package install paths under `/sandbox`, short-lived, not production webhook |

The most useful default is probably `web-agent`: broad web access, common CLI
tools, browser/search/GitHub/Google Workspace tools preinstalled, writable
`/sandbox`, and a managed gateway path. This gets close to Kubernetes
convenience without pretending the sandbox has no security boundary.

### 6.1 Policy Recommendations

These recommendations are **testing in progress** and should not yet be treated
as a stable public contract.

| Tier | Use case | Network posture | Install posture | Docs posture |
|---|---|---|---|---|
| `dev-agent` | First successful local OpenAB bot and normal developer use | Broad enough for model providers, Discord, GitHub, npm/PyPI, Google APIs as needed | Common tools preinstalled; Go/npm/Python/user binaries write under `/sandbox` | `docs/openshell.md` quick start |
| `web-agent` | Normal deployed OpenAB assistant with web/API tools | Broad HTTPS/WebSocket egress for selected providers and tools | Tools preinstalled in image; limited user-local `/sandbox/bin` for one-off binaries | Policy-specific docs after validation |
| `runtime-agent` | Smaller production-like bot | Only what the image and current OpenShell defaults require to connect Discord and model APIs | No runtime installs; writable `/sandbox` only | Advanced/runtime note |
| `safe-agent` | Enterprise/security-sensitive deployment | Narrow endpoint allowlist per agent/provider | No runtime installs | Production hardening guide |

Policy implementation guidance:

- Validate policy files against the documented OpenShell CLI before linking them
  from stable docs.
- Prefer an OpenAB-owned policy artifact only after CI or release testing proves
  it applies cleanly with the supported OpenShell version.
- Keep broad policy files out of the first-run happy path until they are proven
  one-shot.
- Treat policy edits made during E2E as findings, not as test harness fixes.
- Separate network policy from install policy. A sandbox can have broad egress
  and still be intentionally non-root/non-installable.

## 7. Preset Responsibilities

An `openab-full-agent` or `web-agent` preset should own the following decisions:

- Build or select an OpenAB-compatible sandbox image.
- Set `HOME=/sandbox`.
- Make `/sandbox`, `/sandbox/.local`, `/sandbox/.cache`, and `/tmp` writable.
- Keep system paths read-only.
- Run as the non-root `sandbox` user.
- Include common tools in the image rather than requiring runtime installs.
- Enable broad outbound HTTPS for normal web/model/API access.
- Enable WebSocket egress for OpenAB gateway connectivity.
- Provide a clear gateway mode: host gateway, managed gateway, or direct
  platform connection.
- Provide a provider/secret convention that avoids raw long-lived secrets in
  sandbox config.
- Preserve an escape hatch for stricter endpoint allowlists.

## 8. Product Positioning

The product question is not "can OpenShell safely expose everything?" It is:

```text
Can OpenShell offer a one-command default that is permissive enough for normal
OpenAB agents, while still safer than a random Kubernetes pod?
```

That would be strong positioning. Kubernetes can run the same workload, but the
operator must add the security boundary deliberately. OpenShell starts with the
security boundary, but needs an accessible preset that makes the common OpenAB
agent path feel obvious.

## 9. Decision To Discuss

OpenAB should consider an OpenShell preset module with three layers:

1. A blessed runtime image family with common agent tools already installed.
2. A small preset policy vocabulary such as `safe-agent`, `web-agent`, and
   `dev-agent`.
3. A gateway and credential pattern that handles webhook platforms without
   forcing clients to debug WebSocket credential rewrite behavior.

The immediate proposal is not to remove OpenShell's explicit policy model. The
proposal is to hide the routine policy decisions behind product-level presets
and leave the low-level controls for operators who need them.
