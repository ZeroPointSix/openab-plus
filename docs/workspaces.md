# Workspaces

## Overview

A single OAB bot instance can serve multiple projects. Workspaces let users switch project context at session start using the `[[ws:...]]` [control directive](control-directives.md).

When a workspace is set, the agent:
- Uses the workspace path as its working directory
- Loads steering rules from `AGENTS.md` and `.kiro/steering/`
- Activates skills from `.kiro/skills/`
- Has correct git context (branch, remote, history)

## Configuration

Define workspace aliases in `config.toml`:

```toml
[workspace.aliases]
openab = "~/projects/openab"
infra  = "~/projects/infra-cdk"
web    = "~/projects/frontend"
```

Paths starting with `~` expand to the bot's home directory (`$HOME`).

## Usage

Reference aliases with `@` prefix in the first message:

```
@Bot [[ws:@openab]] help me debug the smoke tests
```

Or use raw paths:

```
@Bot [[ws:~/projects/myapp]] investigate the build failure
```

## Security Boundary

All workspace paths are validated before use:

1. **Must be absolute** — relative paths (e.g., `../secrets`) are rejected
2. **`~` expands to bot home** — not the requesting user's home
3. **Canonicalized** — symlinks, `..`, `.` are resolved
4. **Must be within bot home subtree** — paths outside are rejected
5. **Must be a directory** — file paths are rejected
6. **Must exist** — non-existent paths are rejected with a clear error showing the expanded path

## Session Behavior

- Workspace is set **once** at session creation and is immutable
- The workspace persists across session suspend/resume and eviction rebuilds
- To change workspace, start a new session
- If workspace resolution fails, no session is created (clean failure)

## Error Messages

| Scenario | Error |
|----------|-------|
| Unknown alias | `Unknown workspace alias @foo. Available: openab, infra, web` |
| Relative path | `Workspace path must be absolute (start with ~ or /): relative/path` |
| Outside home | `Workspace path is outside allowed directory: /etc/passwd` |
| Not a directory | `Workspace path is not a directory: /home/bot/Cargo.toml` |
| Does not exist | `Workspace path does not exist: ~/nope (expanded to /home/bot/nope)` |

## User workspace vs derived worktree

OpenAB keeps two path classes separate (ZER-889 / ZER-865):

| Kind | Source | Validator | Must exist? | Allowed outside bot home? |
|------|--------|-----------|-------------|---------------------------|
| **User workspace** | `[[ws:...]]` / aliases | `directives::resolve_workspace` | Yes (directory) | No — must stay under bot home after canonicalize |
| **Derived worktree** | daemon per-thread dir under `[worktree].dir` / `OPENAB_WORK_DIR` | `path_bounds::ensure_derived_dir` (+ ZER-865 session allocation) | No at check time; created by daemon | Yes — root is typically `/var/lib/openab/worktrees`, not under bot home |

Do **not** pass derived worktree paths through `resolve_workspace`. That API rejects non-existent paths and requires the bot-home subtree, which would block legitimate worktree creation.

For non-git bases, use `path_bounds::ensure_plain_folder` (D4): `create_dir_all` only — never require a git repository.

## See Also

- [Control Directives](control-directives.md) — full directive syntax and rules
- [Config Reference](config-reference.md#workspace) — `[workspace.aliases]` configuration
- Linear [ZER-889](https://linear.app/zerodotsix/issue/ZER-889) — path boundary split
- Linear [ZER-865](https://linear.app/zerodotsix/issue/ZER-865) — per-session git worktree / plain folder allocation
