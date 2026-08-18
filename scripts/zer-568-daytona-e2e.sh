#!/usr/bin/env bash
# ZER-568 live E2E: verify Admin profile overrides against real ACP agents in Daytona.
#
# Prerequisites:
#   export DAYTONA_API_KEY=...
#   export OPENAB_ADMIN_TOKEN=...
#   Optional: OPENAI_API_KEY / ANTHROPIC_API_KEY for real agent auth inside sandbox
#
# This script creates a Daytona sandbox, installs openab-plus prerequisites,
# starts the unified openab binary with admin API, and exercises session
# creation with model/thinking overrides for codex + claude profiles.
set -euo pipefail

if [[ -z "${DAYTONA_API_KEY:-}" ]]; then
  echo "DAYTONA_API_KEY is required. Get one from https://app.daytona.io/dashboard/keys" >&2
  exit 1
fi

DAYTONA_BIN="${DAYTONA_BIN:-daytona}"
SANDBOX_NAME="${SANDBOX_NAME:-zer-568-e2e-$(date +%s)}"
OPENAB_REPO="${OPENAB_REPO:-https://github.com/ZeroPointSix/openab-plus.git}"
OPENAB_BRANCH="${OPENAB_BRANCH:-cursor/zer-568-review-fixes-c04a}"

echo "==> Creating sandbox ${SANDBOX_NAME}"
$DAYTONA_BIN create --name "$SANDBOX_NAME" --snapshot daytona-medium || \
  $DAYTONA_BIN create --name "$SANDBOX_NAME"

run_remote() {
  $DAYTONA_BIN exec "$SANDBOX_NAME" -- bash -lc "$1"
}

echo "==> Installing build deps"
run_remote 'sudo apt-get update -qq && sudo apt-get install -y -qq build-essential pkg-config libssl-dev python3 curl git'

echo "==> Installing Rust"
run_remote 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y'
run_remote 'source "$HOME/.cargo/env" && rustc --version'

echo "==> Cloning openab-plus (${OPENAB_BRANCH})"
run_remote "rm -rf ~/openab-plus && git clone --branch ${OPENAB_BRANCH} --depth 1 ${OPENAB_REPO} ~/openab-plus"

echo "==> Building openab (unified)"
run_remote 'source "$HOME/.cargo/env" && cd ~/openab-plus && cargo build --release --bin openab --features unified'

echo "==> Preparing workspace + profiles"
run_remote 'mkdir -p ~/workspace/AGENTS.md && printf "# E2E workspace\n" > ~/workspace/AGENTS.md'

if [[ -n "${OPENAI_API_KEY:-}" ]]; then
  echo "==> Configuring Codex API key auth (required for codex-acp session/new)"
  run_remote "export PATH=\$HOME/.npm-global/bin:\$PATH && printf '%s' '${OPENAI_API_KEY}' | codex login --with-api-key"
fi

ADMIN_TOKEN="${OPENAB_ADMIN_TOKEN:-zer-568-daytona-test-token}"
run_remote "cat > ~/openab-plus/config/agent-profiles.toml <<'TOML'
default_profile = \"codex-live\"

[[profiles]]
id = \"codex-live\"
name = \"Codex Live\"
agent_type = \"codex\"
enabled = true
command = \"codex-acp\"
default_model = \"gpt-4\"
reasoning_effort = \"low\"
workdir_strategy = \"profile_default\"
working_dir = \"/home/daytona/workspace\"
recovery_strategy = \"none\"

[[profiles]]
id = \"claude-live\"
name = \"Claude Live\"
agent_type = \"claude\"
enabled = true
command = \"claude-agent-acp\"
default_model = \"claude-sonnet-4\"
reasoning_effort = \"medium\"
workdir_strategy = \"profile_default\"
working_dir = \"/home/daytona/workspace\"
recovery_strategy = \"none\"
TOML"

echo "==> Starting openab in background"
run_remote "source \"\$HOME/.cargo/env\" && cd ~/openab-plus && \
  GATEWAY_ADMIN_TOKEN='${ADMIN_TOKEN}' \
  OPENAB_WORKSPACE_ROOT='/home/daytona/workspace' \
  OPENAB_AGENT_PROFILES_PATH=~/openab-plus/config/agent-profiles.toml \
  nohup ./target/release/openab > /tmp/openab.log 2>&1 & sleep 8"

BASE_URL="http://127.0.0.1:8080"
echo "==> Creating codex session with overrides"
run_remote "curl -sf -X POST ${BASE_URL}/api/v1/sessions \
  -H 'Authorization: Bearer ${ADMIN_TOKEN}' \
  -H 'Content-Type: application/json' \
  -d '{\"profile_id\":\"codex-live\",\"overrides\":{\"model\":\"gpt-5\",\"reasoning_effort\":\"high\"}}' | tee /tmp/session-codex.json"

echo "==> Creating claude session with overrides"
run_remote "curl -sf -X POST ${BASE_URL}/api/v1/sessions \
  -H 'Authorization: Bearer ${ADMIN_TOKEN}' \
  -H 'Content-Type: application/json' \
  -d '{\"profile_id\":\"claude-live\",\"overrides\":{\"model\":\"claude-opus-4\",\"reasoning_effort\":\"high\"}}' | tee /tmp/session-claude.json"

echo "==> Asserting codex override landed via ACP"
run_remote "python3 - <<'PY'
import json, sys
data = json.load(open('/tmp/session-codex.json'))
assert data.get('profile_id') == 'codex-live', data
assert data.get('reasoning_effort') == 'high', data
assert data.get('model'), f'expected model in snapshot: {data}'
errors = data.get('profile_config_errors') or []
assert not errors, errors
print('codex ok:', data.get('model'), data.get('reasoning_effort'))
PY"

echo "==> Asserting claude override landed before ACP session start"
run_remote "python3 - <<'PY'
import json
data = json.load(open('/tmp/session-claude.json'))
assert data.get('profile_id') == 'claude-live', data
errors = data.get('profile_config_errors') or []
assert not errors, errors
assert data.get('model') == 'claude-opus-4', data
assert data.get('reasoning_effort') == 'high', data
assert data.get('metadata_source') == 'configured', data
print('claude ok:', data.get('model'), data.get('reasoning_effort'))
PY"

echo "==> Fetching workspace files"
run_remote "curl -sf -H 'Authorization: Bearer ${ADMIN_TOKEN}' ${BASE_URL}/api/v1/workspace/files | head -c 2000"

echo "==> Done. Inspect logs:"
echo "    daytona exec ${SANDBOX_NAME} -- tail -100 /tmp/openab.log"
echo "    daytona exec ${SANDBOX_NAME} -- cat /tmp/session-codex.json"
echo "    daytona exec ${SANDBOX_NAME} -- cat /tmp/session-claude.json"
