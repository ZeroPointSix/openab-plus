# ADR: OpenShell-Compatible Gateway WebSocket Authentication

- **Status:** Proposed
- **Date:** 2026-06-06
- **Author:** OpenAB POC contributors
- **Related:** [ADR: Custom Gateway](./custom-gateway.md), [OpenShell](../openshell.md), [Google Chat](../google-chat.md)

---

## 1. Context

OpenAB's Custom Gateway lets OAB connect outbound over WebSocket while the
gateway owns inbound webhooks and platform credentials. This works well for
Google Chat, LINE, Telegram, and other webhook platforms.

When OAB runs inside an NVIDIA OpenShell sandbox, gateway authentication has an
extra constraint: long-lived secrets should not be placed in the sandbox as raw
environment variables or raw config values.

OpenShell provider credentials appear in the sandbox as resolver placeholders,
for example:

```text
provider-OPENSHELL-RESOLVE-ENV-GATEWAY_WS_TOKEN
openshell:resolve:env:...
```

OpenShell resolves those placeholders at the network boundary only when the
policy proxy can inspect the request. This allows the agent process to use a
credential without ever seeing the raw value.

## 2. Problem

The original gateway auth path appended the token to the WebSocket URL:

```text
ws://gateway/ws?token=<token>
```

That is simple for ordinary Docker/Kubernetes deployments, but it is a poor fit
for OpenShell provider credentials:

- The sandbox config would otherwise need a raw gateway token.
- WebSocket handshake query/header credential rewrite was unreliable in the
  OpenShell bridge path used by this POC.
- A Rust WebSocket client may not honor the injected HTTP proxy environment, so
  local OpenShell/Docker setups sometimes need a small local bridge to reach the
  host gateway.

During the POC, these routes were tested and rejected as the primary OpenShell
path:

```text
[gateway].token = "${GATEWAY_WS_TOKEN}"
[gateway].token = "openshell:resolve:env:GATEWAY_WS_TOKEN"
[gateway].token = "provider-OPENSHELL-RESOLVE-ENV-GATEWAY_WS_TOKEN"
ws://.../ws?token=<placeholder>
Authorization: Bearer <placeholder>
```

OpenShell can rewrite placeholders in normal HTTP requests. For example, a
`curl` request with `Authorization: Bearer <placeholder>` to the gateway's HTTP
endpoint proves header rewrite works when the request is inspected as REST.
However, WebSocket auth in the upgrade handshake is not the right compatibility
point for the OpenShell setup.

OpenShell's documented WebSocket credential rewrite applies to client-to-server
WebSocket text frames after the `101 Switching Protocols` upgrade.

## 3. Decision

Add a first-text-frame gateway authentication message.

OAB connects to the gateway WebSocket without putting the gateway token in the
URL query string. Immediately after the WebSocket upgrade succeeds, OAB sends:

```json
{
  "schema": "openab.gateway.auth.v1",
  "token": "provider-OPENSHELL-RESOLVE-ENV-GATEWAY_WS_TOKEN"
}
```

When running under OpenShell with:

```text
websocket_credential_rewrite: true
```

the policy proxy rewrites the provider placeholder inside that client-to-server
text frame before forwarding it to the gateway.

The gateway behavior is:

1. If `GATEWAY_WS_TOKEN` is not set, accept WebSocket clients with a warning.
2. If a legacy query token or `Authorization: Bearer` token is present, validate
   it before upgrade handling continues.
3. Otherwise, accept the upgrade and require the first client text frame to be
   `openab.gateway.auth.v1` with the expected token.
4. If the first frame is missing, malformed, or invalid, close the socket and
   do not forward events.

This keeps the old query-token path compatible while enabling a hardened
OpenShell path.

## 4. Reference Architecture

```text
                         Google Workspace
                    +-----------------------+
                    |      Google Chat      |
                    |  signed HTTPS webhook |
                    +-----------+-----------+
                                |
                                v
+-----------------------------------------------------------------------+
| Host / trusted runtime                                                |
|                                                                       |
|  +-----------------------------+        +---------------------------+ |
|  | openab-gateway              |        | Host secret/token broker   | |
|  |                             |        |                           | |
|  | - owns webhook endpoint     |        | - stores SA JSON          | |
|  | - verifies Google Chat JWT  |        | - mints short-lived       | |
|  | - owns Google Chat SA JSON  |        |   Google access tokens    | |
|  | - owns raw GATEWAY_WS_TOKEN |        | - updates OpenShell       | |
|  +--------------+--------------+        |   provider credentials    | |
|                 ^                       +-------------+-------------+ |
|                 |                                     |               |
|                 | validates raw token                  | stores raw    |
|                 | after rewrite                        | provider data |
|                 |                                     v               |
|  +--------------+--------------------------------------+-------------+ |
|  |                  OpenShell provider store                         | |
|  |  GATEWAY_WS_TOKEN, GOOGLE_ACCESS_TOKEN                            | |
|  +--------------+--------------------------------------+-------------+ |
+-----------------+--------------------------------------+---------------+
                  |                                      |
                  | WebSocket 101 + text-frame rewrite   |
                  | REST header rewrite for Google APIs  |
                  v                                      v
+-----------------------------------------------------------------------+
| OpenShell sandbox                                                     |
|                                                                       |
|  +-----------------------------+       +----------------------------+ |
|  | openab                      |       | curl / Google API tools     | |
|  |                             |       |                            | |
|  | - config has placeholders   |       | - Authorization header     | |
|  | - opens ws://gateway/ws     |       |   contains placeholder     | |
|  | - first text frame sends    |       | - OpenShell rewrites       | |
|  |   openab.gateway.auth.v1    |       |   token at egress          | |
|  +--------------+--------------+       +----------------------------+ |
|                 |                                                     |
|                 | stdio ACP                                           |
|                 v                                                     |
|  +-----------------------------+                                      |
|  | kiro-cli                    |                                      |
|  |                             |                                      |
|  | - agent-owned auth state    |                                      |
|  |   remains local credential  |                                      |
|  |   material                  |                                      |
|  +-----------------------------+                                      |
+-----------------------------------------------------------------------+
```

Credential flow:

```text
Host/OpenShell provider stores raw GATEWAY_WS_TOKEN
  -> sandbox config stores provider placeholder only
  -> OAB sends placeholder in first WebSocket text frame
  -> OpenShell rewrites placeholder at network boundary
  -> gateway validates raw token
```

Google API credential flow:

```text
service-account JSON stays on host/gateway side
  -> host broker mints short-lived GOOGLE_ACCESS_TOKEN
  -> OpenShell provider stores raw short-lived token
  -> sandbox command sends provider placeholder in HTTP Authorization header
  -> OpenShell rewrites placeholder at egress
  -> Google API receives raw short-lived token
```

## 5. Change From Earlier `openshell.md` Guidance

The earlier OpenShell quick-start pattern focused on running OAB inside an
OpenShell sandbox with provider placeholders for simple outbound APIs such as
Discord. It did not cover webhook platforms where a Custom Gateway receives
inbound traffic, owns platform credentials, and authenticates OAB over
WebSocket.

The new recommendation keeps that provider-placeholder principle but changes
where secrets live and how WebSocket auth is performed.

| Area | Earlier `openshell.md` pattern | New Google Chat/Kiro gateway pattern |
|---|---|---|
| Platform surface | Direct bot API from sandbox, for example Discord gateway/API | Google Chat HTTPS webhook terminates at `openab-gateway` outside OpenShell |
| Platform credentials | Provider placeholder in sandbox config for the bot token | Google Chat service-account JSON stays in the gateway/host layer |
| Gateway auth | Not applicable, or token could be appended to `ws://.../ws?token=...` in non-OpenShell deployments | OAB sends `openab.gateway.auth.v1` as first WebSocket text frame so OpenShell can rewrite the provider placeholder after upgrade |
| `.env` / local secret practice | Easy to accidentally fall back to raw env/config values during local setup | Sandbox config should contain placeholders only; raw values live in OpenShell provider, gateway env, or host token broker |
| Google Drive/Sheets/Docs access | Not covered | Host keeps service-account JSON, mints short-lived Google token, passes only a provider placeholder through sandbox egress |
| Agent-owned auth state | Not addressed | Explicitly documented as a remaining limitation: Kiro's own login cache is still local credential material |

This is not a claim that OpenShell can make all agent secrets disappear. It is a
more precise split:

```text
Injected operator/platform secrets
  -> use provider placeholders, gateway ownership, or host token broker

Agent CLI's own login cache
  -> remains agent-owned credential material unless the CLI supports brokering
```

## 6. Consequences

Positive:

- The sandbox does not need the raw gateway token.
- OpenShell can enforce which binary may send the credential and to which
  endpoint.
- Gateway authentication remains enabled for OpenShell deployments.
- Existing query-token deployments continue to work.

Tradeoffs:

- The gateway briefly accepts the WebSocket upgrade before authentication
  completes. It must not send events until the first-frame auth succeeds.
- A malformed or unauthenticated client consumes a socket briefly until the
  auth timeout expires or the gateway closes it.
- Operators need a WebSocket policy with credential rewrite enabled.

## 7. Operational Notes

Minimal OpenShell endpoint shape:

```bash
openshell policy update oab \
  --add-endpoint gateway-host:8080:read-write:websocket:enforce:websocket-credential-rewrite \
  --binary /usr/local/bin/openab \
  --wait

openshell policy update oab \
  --add-allow 'gateway-host:8080:GET:/ws' \
  --wait
```

For local Docker Desktop setups where direct TCP to the host gateway is not
available to the Rust WebSocket client, a small bridge may be used inside the
sandbox. Even then, policy should classify the upstream gateway endpoint rather
than treating the OpenShell HTTP proxy as the application endpoint.

## 8. Security Boundary

Do not pass long-lived platform secrets or service-account JSON files into the
agent sandbox. Keep those in the host gateway or in a host-side token broker.

For Google Drive/Sheets/Docs access, prefer:

```text
service-account JSON on host
  -> host mints short-lived OAuth access token
  -> OpenShell provider stores GOOGLE_ACCESS_TOKEN
  -> sandbox uses provider placeholder in Authorization header
  -> OpenShell rewrites at egress
```

This avoids exposing the long-lived service-account private key to the agent.

## 9. Limitation: Agent-Owned Credential Stores

OpenShell provider placeholders protect credentials that the operator injects
through OpenShell, such as gateway tokens or short-lived Google API access
tokens. They do not automatically protect credentials that an agent CLI creates
and persists for itself.

For example, Kiro CLI maintains its own local authentication state. In the POC,
Kiro could still boot after `.aws/sso/cache/*.json` was removed, but failed when
`.local/share/kiro-cli/data.sqlite3` was removed. This means the decisive Kiro
auth state is not only the AWS SSO cache files; it also lives in Kiro's own
SQLite state under `.local/share/kiro-cli/`.

Observed credential-bearing paths include:

```text
/sandbox/.local/share/kiro-cli/data.sqlite3
/sandbox/.aws/sso/cache/*.json
```

Those files are agent-owned credential stores, not OpenShell provider
credentials. Replacing their JSON fields with provider placeholder strings is
not a natural fix unless the agent CLI itself supports placeholder resolution or
external credential brokering. Otherwise, the CLI expects its native local token
format and needs readable auth state to start.

Operational consequence:

- Provider placeholders are appropriate for injected secrets controlled by
  OpenAB, the gateway, or a host-side broker.
- They are not sufficient to isolate a third-party agent's own login cache.
- Treat agent auth state such as Kiro's `data.sqlite3` as high-risk credential
  material.
- Use a dedicated low-privilege agent identity per bot, avoid shared sandboxes,
  and prefer an agent-supported auth proxy/token broker when available.
