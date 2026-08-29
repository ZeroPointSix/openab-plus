# CLI native config (profile → vendor files)

OpenAB can write each vendor CLI's own config file before starting an ACP
session (Claude `~/.claude/settings.json`, Codex `~/.codex/config.toml`). That
path lives in `crates/openab-core/src/cli_config/` and is driven from the
profile pool when a profile selects a supported `agent_type`.

## Write timing

1. Resolve the session profile (model, thinking / reasoning effort, provider).
2. Take [`lock_for(agent_type)`](../crates/openab-core/src/cli_config/mod.rs) so
   the same agent type cannot interleave file writes with process start.
3. Call `apply_unlocked` (merge owned keys, atomic write, keep `.openab.bak`).
4. Spawn / attach the ACP session **while still holding that lock**.

Apply only mutates on-disk vendor config. It does not signal running processes,
re-read config every turn, or pull container images.

## Effectiveness: new sessions only

**Guarantee:** after a successful apply, **sessions created afterwards** for
that agent type read the updated files at process start.

**Not guaranteed:**

- An already-running ACP / CLI process picking up a mid-flight model or
  thinking change (no per-turn / hot reload of native config).
- Live sessions switching profile without restart / new session.
- Silent image pulls when the configured CLI binary or image tag changes.

To change model or thinking for chat traffic, start a **new session** (or
recycle the pool entry) so a fresh process starts after the next apply.

There is no `native_config_reload = per_turn | on_start` knob. OpenAB's
contract matches “only guarantee new sessions see the write” (ZER-707 /
ZER-868). Do not introduce a `per_turn` mode here.

## Isolation / concurrent profiles

Today, apply targets the shared vendor home paths for that agent type. Two
profiles that both render Claude or Codex can still overwrite each other's
files on disk; that race is tracked as **ZER-888** (PR #70). After ZER-888,
acceptance includes “a new session receives its own profile's native config
without clobbering another profile's session.”

## Related

- Code: `crates/openab-core/src/cli_config/`
- Call site: `crates/openab-core/src/acp/profile_pool.rs` (lock → apply → spawn)
- Config surface: [config-reference.md](config-reference.md#agent) (`[agent]`)
