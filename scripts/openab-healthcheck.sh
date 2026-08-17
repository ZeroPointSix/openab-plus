#!/bin/sh
# Docker HEALTHCHECK for OpenAB containers that may run Slack-only or with Admin HTTP.
# Mirrors admin_http_enabled_from_values() in src/main.rs:
# - Admin HTTP is on when OPENAB_ADMIN_ENABLED is 1/true (case-insensitive),
#   or when OPENAB_ADMIN_TOKEN / GATEWAY_ADMIN_TOKEN is non-empty.
# - OPENAB_ADMIN_ENABLED=false does NOT force Admin on, even if the string is non-empty.
# - When Admin is on, probe http://127.0.0.1:<GATEWAY_LISTEN port>/health.
# - When Admin is off, only require the openab process to be running.

set -eu

admin_http_enabled() {
  enabled=0
  explicit=$(printf '%s' "${OPENAB_ADMIN_ENABLED:-}" | tr '[:upper:]' '[:lower:]')
  case "$explicit" in
    1|true) enabled=1 ;;
  esac
  if [ -n "${OPENAB_ADMIN_TOKEN:-}" ]; then
    enabled=1
  fi
  if [ -n "${GATEWAY_ADMIN_TOKEN:-}" ]; then
    enabled=1
  fi
  printf '%s' "$enabled"
}

admin_listen_port() {
  listen="${GATEWAY_LISTEN:-0.0.0.0:8080}"
  port=${listen##*:}
  if [ -z "$port" ] || [ "$port" = "$listen" ]; then
    port=8080
  fi
  printf '%s' "$port"
}

admin=$(admin_http_enabled)
port=$(admin_listen_port)

if [ "${OPENAB_HEALTHCHECK_DRY_RUN:-}" = "1" ]; then
  if [ "$admin" -eq 1 ]; then
    printf 'mode=http port=%s url=http://127.0.0.1:%s/health\n' "$port" "$port"
  else
    printf 'mode=process process=openab\n'
  fi
  exit 0
fi

if [ "$admin" -eq 1 ]; then
  curl -fsS "http://127.0.0.1:${port}/health" >/dev/null
else
  pgrep -x openab >/dev/null
fi
