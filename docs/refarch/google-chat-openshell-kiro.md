# Google Chat + OpenShell + Kiro Reference Architecture

Run OpenAB with Kiro inside an OpenShell sandbox while exposing Google Chat
through the Custom Gateway outside the sandbox.

This pattern is useful when the agent is powerful enough that long-lived
credentials should not be placed in the same filesystem or environment that the
agent can inspect.

## Architecture

```text
Google Chat
  -> HTTPS webhook
  -> openab-gateway outside OpenShell
  -> WebSocket
  -> OpenShell policy proxy
  -> openab inside OpenShell sandbox
  -> kiro-cli acp --trust-all-tools
```

Local Docker Desktop deployments may need a sandbox-local bridge if the Rust
WebSocket client cannot directly reach the host gateway:

```text
openab
  -> ws://127.0.0.1:18080/ws
  -> sandbox bridge
  -> OpenShell proxy
  -> host openab-gateway:8080/ws
```

The bridge is an operational workaround for local host routing. The security
boundary remains OpenShell policy plus provider credential rewrite.

## Credential Model

Do not put raw gateway tokens, Google service-account JSON, or Google OAuth
tokens in sandbox config.

Use OpenShell providers:

```bash
openshell provider create --name openab-googlechat --type generic \
  --credential GATEWAY_WS_TOKEN \
  --credential GOOGLE_ACCESS_TOKEN
```

In sandbox config, use provider placeholders:

```toml
[gateway]
url = "ws://127.0.0.1:18080/ws"
platform = "googlechat"
token = "provider-OPENSHELL-RESOLVE-ENV-GATEWAY_WS_TOKEN"
allow_all_channels = true
allow_all_users = true

[agent]
command = "kiro-cli"
args = ["acp", "--trust-all-tools"]
working_dir = "/sandbox"
```

`GATEWAY_WS_TOKEN` is authenticated using the first WebSocket text frame. See
[ADR: OpenShell-Compatible Gateway WebSocket Authentication](../adr/openshell-websocket-auth.md).

## Gateway

Run the Custom Gateway outside OpenShell. The gateway should own Google Chat
webhook verification and Google Chat reply credentials.

Example:

```bash
docker run -d --name openab-gateway \
  --env-file /path/to/gateway.env \
  -v /path/to/google-chat-service-account.json:/secrets/google-chat-service-account.json:ro \
  -p 8080:8080 \
  ghcr.io/openabdev/openab-gateway:latest
```

Gateway env:

```text
GOOGLE_CHAT_ENABLED=true
GOOGLE_CHAT_AUDIENCE=https://your-domain.example/webhook/googlechat
GOOGLE_CHAT_WEBHOOK_PATH=/webhook/googlechat
GOOGLE_CHAT_SA_KEY_FILE=/secrets/google-chat-service-account.json
GATEWAY_WS_TOKEN=<long random token>
```

## OpenShell Policy

Apply policy updates serially and wait for each revision to load.

Gateway WebSocket endpoint:

```bash
openshell policy update oab \
  --add-endpoint gateway-host:8080:read-write:websocket:enforce:websocket-credential-rewrite \
  --binary /usr/local/bin/openab \
  --wait

openshell policy update oab \
  --add-allow 'gateway-host:8080:GET:/ws' \
  --wait
```

Kiro endpoints vary by version, but a typical policy includes:

```bash
openshell policy update oab \
  --add-endpoint oidc.us-east-1.amazonaws.com:443:read-write:rest:enforce \
  --binary /usr/local/bin/kiro-cli \
  --wait

openshell policy update oab \
  --add-endpoint cognito-identity.us-east-1.amazonaws.com:443:read-write:rest:enforce \
  --binary /usr/local/bin/kiro-cli \
  --wait

openshell policy update oab \
  --add-endpoint q.us-east-1.amazonaws.com:443:read-write:rest:enforce \
  --binary /usr/local/bin/kiro-cli \
  --wait

openshell policy update oab \
  --add-endpoint client-telemetry.us-east-1.amazonaws.com:443:read-write:rest:enforce \
  --binary /usr/local/bin/kiro-cli \
  --wait
```

## Google Drive, Sheets, And Docs Access

Keep the service-account JSON on the host. Do not mount it into the sandbox.

For a simple POC, mint a short-lived access token on the host and store it in an
OpenShell provider:

```bash
CLOUDSDK_CONFIG=/tmp/openab-gcloud-sa \
  gcloud auth activate-service-account \
  --key-file=/path/to/google-chat-service-account.json

GOOGLE_ACCESS_TOKEN="$(
  CLOUDSDK_CONFIG=/tmp/openab-gcloud-sa \
    gcloud auth print-access-token \
    --scopes=https://www.googleapis.com/auth/drive.readonly,https://www.googleapis.com/auth/spreadsheets.readonly,https://www.googleapis.com/auth/documents.readonly
)"
export GOOGLE_ACCESS_TOKEN

openshell provider update openab-googlechat --credential GOOGLE_ACCESS_TOKEN
```

Allow read-only Google API hosts for the tool binary that will fetch content:

```bash
openshell policy update oab \
  --add-endpoint www.googleapis.com:443:read-write:rest:enforce \
  --binary /usr/bin/curl \
  --wait

openshell policy update oab \
  --add-endpoint sheets.googleapis.com:443:read-write:rest:enforce \
  --binary /usr/bin/curl \
  --wait

openshell policy update oab \
  --add-endpoint docs.googleapis.com:443:read-write:rest:enforce \
  --binary /usr/bin/curl \
  --wait
```

Sandbox smoke tests:

```bash
curl -H "Authorization: Bearer provider-OPENSHELL-RESOLVE-ENV-GOOGLE_ACCESS_TOKEN" \
  "https://www.googleapis.com/drive/v3/about?fields=user(displayName,emailAddress)"

curl -H "Authorization: Bearer provider-OPENSHELL-RESOLVE-ENV-GOOGLE_ACCESS_TOKEN" \
  "https://sheets.googleapis.com/v4/spreadsheets/SPREADSHEET_ID?fields=spreadsheetId,properties.title"

curl -L -H "Authorization: Bearer provider-OPENSHELL-RESOLVE-ENV-GOOGLE_ACCESS_TOKEN" \
  "https://www.googleapis.com/drive/v3/files/DOC_ID/export?mimeType=text/plain"
```

The service account must have access to the target Drive files or folders. A
Drive `404` or Sheets `403` usually means the file has not been shared with the
service account or a group that includes it.

## Known Failure Modes

| Symptom | Likely cause | Fix |
|---|---|---|
| Google Chat says "not responding" | Gateway unreachable or webhook URL stale | Check tunnel, gateway logs, and Chat app URL |
| `Connection Lost` | Gateway received event but OAB/Kiro did not complete the round trip | Check WebSocket connection and Kiro auth |
| `Internal Error (code: -32603)` | Agent process failed, often due to blocked Kiro endpoint | Add required Kiro endpoint policy |
| WebSocket `401` | Gateway token mismatch or placeholder not rewritten | Use first-frame auth and sync provider token |
| WebSocket `403` | WebSocket endpoint or `GET /ws` not allowed | Add websocket endpoint and `GET /ws` allow rule |
| Drive `404` / Sheets `403` | Service account lacks file access | Share the file/folder with the service account |

## Production Notes

- Use a stable HTTPS domain for Google Chat webhooks.
- Run the gateway in an always-on environment; local tunnels are only for POC.
- Keep platform credentials in the gateway or a host-side broker.
- Keep agent sandbox credentials as OpenShell provider placeholders.
- Prefer short-lived Google access tokens over mounting service-account JSON
  into the sandbox.
- Treat Kiro's own auth state as a separate credential class. In the POC,
  `.local/share/kiro-cli/data.sqlite3` was required for Kiro to boot even when
  `.aws/sso/cache` was absent. OpenShell provider placeholders do not naturally
  protect this agent-owned SQLite login state unless Kiro itself supports an
  external credential broker or placeholder resolution.
- Use a dedicated low-privilege Kiro identity per bot and do not share that
  sandbox/state across users.
