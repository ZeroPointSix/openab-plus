#!/usr/bin/env python3
"""Fail if a Cargo dummy-cache Dockerfile omits the codeg-facade workspace member.

Root Cargo.toml lists crates/codeg-facade. Any image that COPY-s Cargo.toml
and then runs cargo build must also copy that crate's manifest and dummy
lib.rs, or Cargo exits 101 before compiling.
"""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SKIP = {
    "Dockerfile.package",
    "Dockerfile.final",
    "Dockerfile.ci",
}

REQUIRED = [
    "COPY crates/codeg-facade/Cargo.toml crates/codeg-facade/Cargo.toml",
    "echo '' > crates/codeg-facade/src/lib.rs",
    "rm -rf src crates/codeg-facade/src crates/openab-core/src crates/openab-gateway/src",
    "crates/codeg-facade/src/lib.rs",
]


def main() -> int:
    missing: list[str] = []
    checked = 0
    for path in sorted(ROOT.glob("Dockerfile*")):
        if path.name in SKIP:
            continue
        text = path.read_text()
        if "COPY Cargo.toml" not in text or "cargo build" not in text:
            continue
        checked += 1
        for needle in REQUIRED:
            if needle not in text:
                missing.append(f"{path.name}: missing `{needle}`")
    if checked < 1:
        print("no Cargo dummy-cache Dockerfiles found", flush=True)
        return 1
    if missing:
        print("Docker dummy workspace is missing codeg-facade:", flush=True)
        for line in missing:
            print(f"  {line}", flush=True)
        return 1
    print(f"ok: {checked} Dockerfiles copy codeg-facade dummy sources")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
