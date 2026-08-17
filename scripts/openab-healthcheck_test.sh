#!/bin/sh
# Unit tests for scripts/openab-healthcheck.sh decision matrix.
# Does not start openab; uses OPENAB_HEALTHCHECK_DRY_RUN=1.

set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
SCRIPT="$ROOT/scripts/openab-healthcheck.sh"
fail=0

run_case() {
  name=$1
  expected=$2
  shift 2
  # shellcheck disable=SC2086
  actual=$(env -i PATH="$PATH" OPENAB_HEALTHCHECK_DRY_RUN=1 "$@" sh "$SCRIPT")
  if [ "$actual" = "$expected" ]; then
    printf 'ok - %s\n' "$name"
  else
    printf 'not ok - %s\n  expected: %s\n  actual:   %s\n' "$name" "$expected" "$actual"
    fail=1
  fi
}

run_case "no admin env uses process probe" \
  "mode=process process=openab"

run_case "OPENAB_ADMIN_ENABLED=false uses process probe" \
  "mode=process process=openab" \
  OPENAB_ADMIN_ENABLED=false

run_case "OPENAB_ADMIN_ENABLED=0 uses process probe" \
  "mode=process process=openab" \
  OPENAB_ADMIN_ENABLED=0

run_case "OPENAB_ADMIN_ENABLED=true uses default http port" \
  "mode=http port=8080 url=http://127.0.0.1:8080/health" \
  OPENAB_ADMIN_ENABLED=true

run_case "OPENAB_ADMIN_ENABLED=1 is case-insensitive true" \
  "mode=http port=8080 url=http://127.0.0.1:8080/health" \
  OPENAB_ADMIN_ENABLED=TRUE

run_case "OPENAB_ADMIN_TOKEN enables http probe" \
  "mode=http port=8080 url=http://127.0.0.1:8080/health" \
  OPENAB_ADMIN_TOKEN=secret

run_case "GATEWAY_ADMIN_TOKEN enables http probe" \
  "mode=http port=8080 url=http://127.0.0.1:8080/health" \
  GATEWAY_ADMIN_TOKEN=secret

run_case "custom GATEWAY_LISTEN port is used" \
  "mode=http port=8081 url=http://127.0.0.1:8081/health" \
  OPENAB_ADMIN_ENABLED=true GATEWAY_LISTEN=0.0.0.0:8081

run_case "false flag with empty tokens stays process mode" \
  "mode=process process=openab" \
  OPENAB_ADMIN_ENABLED=false OPENAB_ADMIN_TOKEN= GATEWAY_ADMIN_TOKEN=

run_case "false flag with non-empty token still enables http (matches Rust)" \
  "mode=http port=8080 url=http://127.0.0.1:8080/health" \
  OPENAB_ADMIN_ENABLED=false OPENAB_ADMIN_TOKEN=secret

if [ "$fail" -ne 0 ]; then
  printf 'healthcheck unit tests failed\n' >&2
  exit 1
fi

printf 'all healthcheck unit tests passed\n'
