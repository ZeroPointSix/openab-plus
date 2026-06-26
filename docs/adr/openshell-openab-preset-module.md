# ADR: OpenShell OpenAB Day 1 Preset

- **Status:** Discussion
- **Date:** 2026-06-10
- **Updated:** 2026-06-25
- **Author:** OpenAB POC contributors
- **Related:** [OpenShell quick start](../openshell.md), [Custom Gateway](./custom-gateway.md)

## Context

OpenAB can run through Kubernetes-style infrastructure or through NVIDIA OpenShell sandboxes. Kubernetes is operationally familiar but heavy. OpenShell is attractive for local and enterprise setup because it starts with an explicit sandbox boundary:

- non-root sandbox user
- constrained writable paths
- provider-managed credentials
- default-deny network egress
- network policy tied to destination host/port/protocol and binary path

That boundary is valuable, but it also makes a broad "any agent can do anything" quick start unrealistic. Coding agents have different authentication flows, endpoint sets, child binaries, web access needs, and runtime install behavior.

The docs should therefore avoid promising that OpenShell is a universal all-agent replacement for Kubernetes on Day 1.

## Decision

Keep `docs/openshell.md` intentionally narrow:

```text
OpenShell sandbox -> OpenAB Discord bot -> Kiro CLI default agent
```

The quick start should prove only that:

1. OpenShell can create the sandbox.
2. OpenShell can inject the Discord bot token through a provider.
3. `openab` can connect to Discord REST and Discord Gateway WebSocket through policy.
4. `kiro-cli` can authenticate and reach its documented service endpoints through policy.
5. A user can mention the Discord bot and get a Kiro-backed reply.

All other agent backends and broad web/tool access remain Day 2 work.

The default Day 1 policy should live as a checked-in sample artifact, not as a long inline YAML block in the first-page guide. The quick start applies:

```text
openshell/samples/kiro-discord-day1-policy.yaml
```

Agent/platform combinations should grow under `openshell/samples/` after they have a real E2E pass. For example, Kiro-specific combinations belong in `openshell/samples/kiro.md`; Codex, Claude, Copilot, Gemini, and other coding CLIs should get their own files when maintainers or users contribute tested policies.

## Why The Quick Start Needs Policy

OpenShell is default-deny for outbound traffic. A working OpenAB policy must define:

- the destination host and port
- protocol shape, such as REST or WebSocket
- enforcement mode
- the binary allowed to connect

For Day 1 this means:

| Binary | Required purpose |
| --- | --- |
| `/usr/local/bin/openab` | Discord REST API and Discord Gateway WebSocket |
| `/usr/local/bin/kiro-cli*` | Kiro authentication, runtime, management, telemetry, and related AWS service endpoints |

Use `enforcement: enforce` for the documented happy path. `enforcement: audit` is useful for investigation, but it should not be documented as a substitute for a known-good allowlist.

## Image Shape

Do not modify the existing coding CLI images just to make OpenShell work.

The supported pattern is an OpenShell-specific wrapper image:

```text
existing OpenAB/Kiro image
  -> OpenShell wrapper
  -> sandbox user
  -> HOME=/sandbox
  -> writable /sandbox runtime paths
```

This keeps the default Kiro image behavior intact while adapting it to OpenShell's runtime expectations. It also avoids repeating this work across `Dockerfile.codex`, `Dockerfile.claude`, `Dockerfile.gemini`, and other agent-specific images.

For this Day 1 preset, the wrapper is `openshell/Dockerfile.kiro`. It should not require changes to the existing default Kiro image or the existing native sandbox image.

## Credential Route

The quick start should not ask users to paste raw long-lived tokens into `config.toml`. The preferred route is:

```text
host secret source or local .env
  -> host shell environment
  -> openshell provider create --credential DISCORD_BOT_TOKEN
  -> openshell sandbox create --provider openab-discord
  -> /sandbox/config.toml references ${DISCORD_BOT_TOKEN}
```

This follows the OpenShell provider model: `openshell provider create --credential API_KEY` can read the value from the host shell environment, store it in the provider, and attach it to a sandbox. The committed OpenAB config should keep only the environment reference.

## E2E Contract

A passing E2E run must prove the real OpenShell path:

- Use real OpenShell. Do not replace the test with native host install, plain `docker run`, Kubernetes, or a host-local OpenAB process.
- Create a real OpenShell sandbox named `oab`.
- Launch `openab run` through `openshell sandbox connect`, `openshell sandbox exec`, or an OpenShell-generated SSH config.
- Do not use raw `docker exec` as the success path. It can be useful for debugging, but it does not prove the same sandbox namespace, proxy, and policy behavior.
- Do not print secrets. Read `DISCORD_BOT_TOKEN` from the host environment or local secret file, then keep command output redacted.
- Stop for human input when Kiro device-flow login requires browser approval.
- Success requires a Discord bot connection and a human mention/reply test in Discord.
- On macOS with Docker Desktop, a successful run must prove that Discord Gateway WebSocket works through the OpenShell-managed route. A Linux-only pass does not prove the macOS path.
- A negative web-access check should ask the agent for a live external fact, such as current weather. The Day 1 preset should not add arbitrary weather/search/browser/package endpoints to make that pass. If the agent can answer only through already-allowed Kiro service traffic, record that separately; it does not prove broad sandbox web access.

## Known Day 2 Boundary

The following are intentionally out of scope for the first-page quick start:

- Codex, Claude, Gemini, AGY, Cursor, Copilot, OpenCode, and other agent-specific OpenShell profiles.
- Arbitrary web browsing.
- GitHub, npm, PyPI, crates.io, Go proxy, cloud SDKs, Docker registries, and third-party APIs.
- Runtime `sudo`, `apt-get install`, or writes to `/usr`, `/opt`, or `/usr/local/bin`.
- Broad wildcard egress policies.
- Automated policy approval flows where the agent asks an external controller to open new endpoints.

Those use cases can be added later as explicit profiles or policy artifacts after they are tested.

The documented Day 2 workflow is log-driven:

1. Watch `openshell logs oab --tail`.
2. Trigger the blocked workflow.
3. Extract the denied host, port, protocol shape, and binary path.
4. Add the narrowest policy entry with `openshell policy update`.
5. Repeat until the relevant denials stop.

This keeps the sandbox boundary useful while still giving users a clear way to expand egress for their own workflow.

## Host Support Boundary

The OpenAB quick start should point users to the OpenShell support matrix before they start. Day 1 should use host/runtime combinations listed by OpenShell, currently Linux Debian/Ubuntu on `amd64` or `arm64`, or macOS Apple Silicon with Docker Desktop. Other combinations are not first-run targets for this guide.

OpenAB can add notes for combinations we have tested and rejected, but those notes should not turn the Day 1 guide into a platform-debugging document.

## Future Direction

OpenAB may eventually need a small OpenShell preset vocabulary:

| Preset | Purpose | Network posture |
| --- | --- | --- |
| `openab-day1-kiro` | First successful Discord + Kiro setup | Narrow, documented allowlist |
| `openab-dev-agent` | Developer-heavy agent use | Broader web and package ecosystem access |
| `openab-safe-agent` | Enterprise/security-sensitive deployments | Strict per-agent/provider endpoint policy |

The immediate PR should not solve all three. It should make Day 1 simple, honest, and testable.

## References

- [OpenShell policy schema](https://docs.nvidia.com/openshell/reference/policy-schema)
- [OpenShell first network policy tutorial](https://docs.nvidia.com/openshell/get-started/tutorials/first-network-policy)
- [OpenShell support matrix](https://docs.nvidia.com/openshell/reference/support-matrix)
- [OpenShell providers](https://docs.nvidia.com/openshell/sandboxes/manage-providers)
- [Kiro firewall and proxy documentation](https://kiro.dev/docs/cli/privacy-and-security/firewalls/)
- [Kiro remote authentication](https://kiro.dev/docs/cli/authentication/)
