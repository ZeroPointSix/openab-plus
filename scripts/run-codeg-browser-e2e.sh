#!/usr/bin/env bash
set -euo pipefail

readonly REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly IMAGE="${OPENAB_E2E_IMAGE:-openab-unified-codex:e2e}"
readonly PORT="${OPENAB_E2E_PORT:-18080}"
readonly BASE_URL="http://127.0.0.1:${PORT}"
readonly ARTIFACT_DIR="${OPENAB_E2E_ARTIFACT_DIR:-$REPOSITORY_ROOT/artifacts/codeg-e2e}"
readonly STATE_DIR="$ARTIFACT_DIR/state"
readonly CONTAINER_NAME="openab-codeg-e2e-$$"
readonly TOKEN="${OPENAB_E2E_TOKEN:-zer957-$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')}"
readonly RAW_LOG="$ARTIFACT_DIR/openab.raw.log"

mkdir -p "$STATE_DIR"
chmod 0777 "$ARTIFACT_DIR" "$STATE_DIR"
printf '\n' > "$STATE_DIR/config.toml"

cleanup() {
  if docker inspect "$CONTAINER_NAME" >/dev/null 2>&1; then
    docker logs "$CONTAINER_NAME" > "$RAW_LOG" 2>&1 || true
    sed "s/${TOKEN}/[REDACTED]/g" "$RAW_LOG" > "$ARTIFACT_DIR/openab.log"
    rm -f "$RAW_LOG"
    docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

docker run --detach --name "$CONTAINER_NAME" \
  --publish "127.0.0.1:${PORT}:8080" \
  --env "GATEWAY_LISTEN=0.0.0.0:8080" \
  --env "OPENAB_ADMIN_ENABLED=true" \
  --env "OPENAB_ADMIN_TOKEN=$TOKEN" \
  --env "OPENAB_ACP_ENABLED=true" \
  --env "OPENAB_ACP_AUTH_KEY=$TOKEN" \
  --env "HOME=/tmp/openab-e2e-home" \
  --env "RUST_LOG=info" \
  --volume "$STATE_DIR/config.toml:/etc/openab/config.toml:ro" \
  --volume "$STATE_DIR:/tmp/openab-e2e-home" \
  --volume "$ARTIFACT_DIR:/artifacts" \
  --volume "$REPOSITORY_ROOT/scripts/codeg-e2e-agent.mjs:/opt/openab/codeg-e2e-agent.mjs:ro" \
  "$IMAGE" >/dev/null

for _ in $(seq 1 120); do
  if curl --fail --silent "$BASE_URL/health" >/dev/null; then
    break
  fi
  if ! docker inspect --format '{{.State.Running}}' "$CONTAINER_NAME" 2>/dev/null | grep -qx true; then
    echo "OpenAB E2E container exited before becoming healthy" >&2
    exit 1
  fi
  sleep 0.5
done
curl --fail --silent "$BASE_URL/health" >/dev/null

OPENAB_BASE_URL="$BASE_URL" \
OPENAB_E2E_TOKEN="$TOKEN" \
OPENAB_E2E_SCREENSHOT="$ARTIFACT_DIR/codeg-workbench.png" \
OPENAB_E2E_EVIDENCE="$ARTIFACT_DIR/evidence.json" \
CODEG_REVISION="f3d82482e78467aa2636d181ae7154c433cb6f94" \
node "$REPOSITORY_ROOT/scripts/codeg-browser-e2e.cjs"

grep -Fq '"method":"session/prompt","text":"exercise unified Codeg"' "$ARTIFACT_DIR/agent.log"
grep -Fq '"method":"session/cancel"' "$ARTIFACT_DIR/agent.log"

printf 'Browser E2E evidence: %s\n' "$ARTIFACT_DIR/evidence.json"
