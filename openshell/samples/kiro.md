# OpenShell Kiro Samples

This file collects OpenShell policy notes for Kiro-backed OpenAB setups.

## Discord Day 1

Use [`kiro-discord-day1-policy.yaml`](kiro-discord-day1-policy.yaml) with the main [OpenShell quick start](../../docs/openshell.md).

This is the default Day 1 combination:

```text
OpenShell sandbox -> OpenAB Discord bot -> Kiro CLI
```

The policy permits:

- `/usr/local/bin/openab` to reach Discord REST and Gateway WebSocket endpoints.
- `/usr/local/bin/kiro-cli*` to reach Kiro auth, runtime, management, telemetry, download, and related AWS service endpoints.

## Other Chat Platforms

Telegram, LINE, Slack, Teams, WeCom, and other adapters should be added here only after an end-to-end OpenShell test proves:

- the bot process can connect to the platform API and any required WebSocket or webhook endpoint
- Kiro can authenticate and reply through OpenAB
- blocked egress logs are resolved by the documented policy

Until a platform section exists here, treat it as a Day 2 policy task rather than a guaranteed Day 1 path.
