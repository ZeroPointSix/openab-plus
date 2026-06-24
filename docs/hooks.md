# Lifecycle Hooks

OpenAB supports lifecycle hooks that run custom scripts at specific points during the container lifecycle. Hooks are configured in `config.toml` under the `[hooks]` table.

## Lifecycle Order

```
pre_seed → pre_boot → (agent running) → pre_shutdown
```

| Phase | Purpose | Config |
|-------|---------|--------|
| `pre_seed` | Download & extract S3 zip archives to seed the environment | `[pre_seed]` |
| `pre_boot` | Run custom setup scripts before agent pool creation | `[hooks.pre_boot]` |
| `pre_shutdown` | Run custom cleanup scripts after pool shutdown | `[hooks.pre_shutdown]` |

## Pre-Seed Phase

The `pre_seed` phase runs **before** any hook. It downloads zip archives from S3 and extracts them into the agent's home directory (or a custom target). This eliminates the need for users to install AWS CLI and write download scripts in `pre_boot`.

### Configuration

```toml
[pre_seed]
sources = [
  "s3://my-bucket/base-env.zip",
  "s3://my-bucket/shared-memory.zip",
  "s3://my-bucket/agent-overrides.zip",
]
# target = "/home/agent"    # default: $HOME
# timeout_seconds = 300     # per-source timeout (default: 300)
# on_failure = "abort"      # "abort" or "warn" (default: "abort")
```

### Layer Concept

Sources are extracted sequentially (first → last). Files from later archives overwrite earlier ones — like layers in a container image:

```
Layer 3 (last)   ─── highest priority, overwrites all below
Layer 2          ─── overwrites layer 1
Layer 1 (first)  ─── base layer
─────────────────
     $HOME
```

### Constraints

- Maximum **5** sources
- Only `s3://` URIs supported
- Only `.zip` format supported
- Uses the standard AWS credential chain (IRSA, ECS task role, env vars)

### IAM Policy

```json
{
  "Effect": "Allow",
  "Action": ["s3:GetObject"],
  "Resource": [
    "arn:aws:s3:::my-bucket/base-env.zip",
    "arn:aws:s3:::my-bucket/shared-memory.zip",
    "arn:aws:s3:::my-bucket/agent-overrides.zip"
  ]
}
```

---

## Available Hooks

| Hook | Timing | Use Case |
|------|--------|----------|
| `pre_boot` | Before agent pool creation | Bootstrap files, sync from S3, install CLIs |
| `pre_shutdown` | After pool shutdown, before exit | Backup state, sync to S3 |

## Configuration

Each hook supports exactly **one** script source:

### Option A: File path

```toml
[hooks.pre_boot]
script = "/etc/openab/pre-boot.sh"
timeout_seconds = 60
on_failure = "abort"
```

### Option B: Inline script

```toml
[hooks.pre_boot]
inline = '''
#!/bin/sh
set -e
aws s3 sync "$BOOTSTRAP_URI" "$HOME/"
'''
timeout_seconds = 120
on_failure = "abort"
```

### Option C: Remote URL (with SHA-256 verification)

```toml
[hooks.pre_boot]
url = "https://raw.githubusercontent.com/acme/config/main/pre-boot.sh"
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
timeout_seconds = 60
on_failure = "abort"
```

## Fields

| Field | Default | Description |
|-------|---------|-------------|
| `script` | — | Absolute path to an executable script |
| `inline` | — | Script content (written to temp file and executed) |
| `url` | — | Remote script URL (max 1 MiB) |
| `sha256` | — | Required with `url` — hex-encoded SHA-256 of the script |
| `timeout_seconds` | `60` | Max wall-clock time before the script is killed |
| `on_failure` | `"abort"` | `"abort"` exits openab; `"warn"` logs and continues |

## Validation Rules

- Exactly one of `script`, `inline`, or `url` must be set
- `url` requires `sha256`
- `script` must be an absolute path

Validation runs at startup — config errors are caught before any side effects.

## Environment

Scripts run with a sanitized environment:

**Always passed:**
- `HOME`, `PATH`, `USER` (unix) / `USERPROFILE`, `USERNAME`, `SystemRoot`, `SystemDrive` (windows)

**Cloud credentials (auto-detected and passed through):**
- `AWS_*`, `AMAZON_*`, `ECS_CONTAINER_METADATA_URI*`
- `GOOGLE_*`, `GCLOUD_*`, `CLOUDSDK_*`
- `AZURE_*`

**Bootstrap variables (passed if set):**
- `BOOTSTRAP_URI`, `BOOTSTRAP_BASE_URI`, `BOOTSTRAP_PERSONAL_URI`
- `STATE_BUCKET`, `TASK_FAMILY`

**OpenAB identity (passed if set):**
- `OPENAB_AGENT_NAME` — the agent's configured name
- `OPENAB_BACKEND_AGENT` — the backend agent type (e.g. `claude`, `codex`)

> **Note:** `DISCORD_BOT_TOKEN` and other openab secrets are NOT exposed to hook scripts.

## Security

- Temp files are created atomically with `0700` permissions (unix)
- Remote scripts require SHA-256 verification — openab refuses to execute on mismatch
- Scripts run as the container's UID (not root, unless the container runs as root)
- Remote script size is capped at 1 MiB

## Examples

### Sync config from S3 on startup

```toml
[hooks.pre_boot]
timeout_seconds = 120
on_failure = "abort"
inline = '''
#!/bin/sh
set -e
if [ ! -f "$HOME/AGENTS.md" ]; then
  aws s3 sync "$BOOTSTRAP_BASE_URI" "$HOME/"
fi
'''
```

### Backup state on shutdown (ECS Fargate)

```toml
[hooks.pre_shutdown]
timeout_seconds = 30
on_failure = "warn"
inline = '''
#!/bin/sh
aws s3 sync "$HOME/" "s3://$STATE_BUCKET/$TASK_FAMILY/" \
  --exclude "aws-cli/*" --exclude "bin/*" --quiet
'''
```

### Conditional OAuth token refresh

```toml
[hooks.pre_boot]
script = "/etc/openab/pre-boot.sh"
timeout_seconds = 60
on_failure = "warn"
```

```bash
#!/bin/sh
# /etc/openab/pre-boot.sh
set -e
if [ -f "$HOME/.kiro/auth.json" ]; then
  EXPIRES=$(jq -r '.expires' "$HOME/.kiro/auth.json")
  NOW=$(date +%s)
  if [ "$NOW" -gt "$EXPIRES" ]; then
    kiro-cli auth refresh || true
  fi
fi
```

## Platform Comparison

| Option | Best for | Requires redeploy? | Network at boot? |
|--------|----------|--------------------|-------------------|
| `script` | k8s (ConfigMap mount), EFS, image bake | Only if image-baked | No |
| `inline` | ECS, Docker Compose, bare metal | Config change only | No |
| `url` + `sha256` | Central script repo, multi-cluster | No (update sha256 to roll) | Yes |
