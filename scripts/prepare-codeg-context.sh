#!/usr/bin/env bash
set -euo pipefail

readonly DEFAULT_CODEG_REVISION="f3d82482e78467aa2636d181ae7154c433cb6f94"
readonly REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

CODEG_REPOSITORY="${CODEG_REPOSITORY:-https://github.com/ZeroPointSix/codeg.git}"
CODEG_REVISION="${CODEG_REVISION:-$DEFAULT_CODEG_REVISION}"
CODEG_DIR="${CODEG_DIR:-$REPOSITORY_ROOT/codeg}"

if [[ ! "$CODEG_REVISION" =~ ^[0-9a-f]{40}$ ]]; then
  echo "CODEG_REVISION must be a full 40-character commit SHA" >&2
  exit 2
fi

if [[ -e "$CODEG_DIR" && ! -d "$CODEG_DIR/.git" ]]; then
  echo "Refusing to replace non-git path: $CODEG_DIR" >&2
  exit 2
fi

if [[ ! -d "$CODEG_DIR/.git" ]]; then
  git clone --filter=blob:none --no-checkout "$CODEG_REPOSITORY" "$CODEG_DIR"
else
  dirty="$({ git -C "$CODEG_DIR" status --porcelain --untracked-files=all || true; } | grep -v '^?? \.openab-revision$' || true)"
  if [[ -n "$dirty" ]]; then
    echo "Refusing to update a dirty Codeg checkout: $CODEG_DIR" >&2
    exit 2
  fi
fi

git -C "$CODEG_DIR" fetch --depth=1 origin "$CODEG_REVISION"
git -C "$CODEG_DIR" checkout --detach --force "$CODEG_REVISION"
git -C "$CODEG_DIR" clean -ffd

actual_revision="$(git -C "$CODEG_DIR" rev-parse HEAD)"
if [[ "$actual_revision" != "$CODEG_REVISION" ]]; then
  echo "Codeg revision mismatch: expected $CODEG_REVISION, got $actual_revision" >&2
  exit 1
fi

for required in package.json pnpm-lock.yaml next.config.ts LICENSE; do
  test -f "$CODEG_DIR/$required" || {
    echo "Codeg checkout is missing $required" >&2
    exit 1
  }
done

printf '%s\n' "$actual_revision" > "$CODEG_DIR/.openab-revision"
printf 'Prepared Codeg revision %s\n' "$actual_revision"
