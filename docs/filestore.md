# Filestore — S3/R2-Compatible Object Store for Large Attachments

## Problem

When a user attaches a text file larger than 512 KB, OAB cannot inline it into
the prompt (it would bloat the context window). Previously, these files were
**silently dropped** — the agent never knew the file existed.

PR #1346 proposed returning the platform's raw URL as a hint, but this has
fundamental limitations:

| Platform | Issue |
|----------|-------|
| Discord | CDN URLs expire in ~24 hours |
| Slack | `url_private_download` requires a Bearer token the agent does not have |
| Any | Agent may lack web-fetch capability or network access to platform CDNs |

## Solution

The `[filestore]` feature solves all three issues:

```
User attaches large text file (> 512 KB)
  → OAB downloads using its platform token (Slack Bearer / Discord CDN)
  → Uploads to user-configured S3/R2 bucket
  → Generates a presigned GET URL (configurable TTL)
  → Returns a ContentBlock::Text hint with the presigned URL
  → Agent fetches via bare HTTP GET (no auth needed)
```

Key insight: **OAB already has the platform credentials to download the file** —
it just wasn't using them for files above the inline limit.

## Configuration

Add a `[filestore]` section to your `config.toml`:

```toml
[filestore]
bucket = "my-oab-files"
region = "us-west-2"
prefix = "incoming/"       # object key prefix (default)
presigned_ttl = 3600       # URL expiry in seconds (default: 1 hour)
```

### With Cloudflare R2

```toml
[filestore]
bucket = "my-oab-files"
region = "auto"
endpoint = "https://<ACCOUNT_ID>.r2.cloudflarestorage.com"
presigned_ttl = 3600
access_key_id = "${R2_ACCESS_KEY_ID}"
secret_access_key = "${R2_SECRET_ACCESS_KEY}"
```

### With AWS S3

```toml
[filestore]
bucket = "my-oab-files"
region = "us-west-2"
presigned_ttl = 3600
# Credentials resolved via AWS provider chain (IRSA, env vars, instance role)
```

### With MinIO (self-hosted)

```toml
[filestore]
bucket = "oab-uploads"
region = "us-east-1"
endpoint = "http://minio.internal:9000"
access_key_id = "${MINIO_ACCESS_KEY}"
secret_access_key = "${MINIO_SECRET_KEY}"
presigned_ttl = 7200
```

## Fields

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `bucket` | ✅ | — | S3 bucket name |
| `region` | ✅ | — | AWS region (`"auto"` for R2) |
| `endpoint` | ❌ | AWS default | Custom S3-compatible endpoint URL |
| `prefix` | ❌ | `"incoming/"` | Object key prefix |
| `presigned_ttl` | ❌ | `3600` | Presigned URL lifetime in seconds |
| `access_key_id` | ❌ | provider chain | Explicit access key |
| `secret_access_key` | ❌ | provider chain | Explicit secret key |

## Behavior

| File size | Filestore configured | Result |
|-----------|---------------------|--------|
| ≤ 512 KB | any | Inlined into prompt (unchanged) |
| > 512 KB, ≤ 50 MB | ✅ yes | Uploaded → presigned URL returned |
| > 50 MB | ✅ yes | Dropped (defense-in-depth cap) |
| > 512 KB | ❌ no | Silently dropped (legacy behavior) |

## What the Agent Sees

When a file is uploaded to the filestore, the agent receives a text block like:

```
[File: test-results.txt]
This file (1024 KB) exceeds the 512 KB inline limit. It has been uploaded to
temporary storage. Fetch the contents using the URL below:
https://my-bucket.s3.us-west-2.amazonaws.com/incoming/abc123_test-results.txt?X-Amz-Algorithm=...
Note: this URL expires in 60 minutes.
```

The agent can then use any HTTP tool (`web-fetch`, `curl`, etc.) to download
the file — no authentication headers required.

## Security

### Credentials

- **Platform tokens (Slack/Discord)** stay server-side — never exposed to agent
- **S3 credentials** stay server-side — only used for upload + presigning
- **Presigned URLs** are time-limited and scoped to a single object

### Object Keys

Object keys are server-generated: `{prefix}{uuid}_{filename}`. The UUID
prevents collision and enumeration. The filename is appended for human
readability in S3 console but is not security-critical.

### Size Limits

- Per-file cap: 50 MB (configurable in code, defense-in-depth)
- File count cap: 5 text files per message (unchanged)
- Aggregate inline cap: 1 MB for inlined files (filestore uploads bypass this)

### Recommended: S3 Lifecycle Rules

OAB does not delete uploaded objects. Configure a lifecycle rule on your bucket
to auto-expire objects after a reasonable period:

```json
{
  "Rules": [{
    "ID": "expire-filestore-uploads",
    "Filter": { "Prefix": "incoming/" },
    "Status": "Enabled",
    "Expiration": { "Days": 1 }
  }]
}
```

For Cloudflare R2, set an equivalent object lifecycle rule in the dashboard.

### Minimum IAM Policy

```json
{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Action": ["s3:PutObject", "s3:GetObject"],
    "Resource": "arn:aws:s3:::my-oab-files/incoming/*"
  }]
}
```

## Error Handling

| Failure | Behavior |
|---------|----------|
| S3 upload fails | File is dropped (warn log), agent not notified |
| Download from platform fails | File is dropped (warn log) |
| File exceeds 50 MB | File is dropped (warn log) |
| Presigned URL generation fails | File is dropped (error log) |
| Filestore not configured | Legacy behavior (>512KB files silently dropped) |

In all failure cases, the file is not inlined and the agent receives no hint.
This is a deliberate "fail-closed" approach — a broken filestore should not
degrade the core prompt pipeline.

## Build Requirement

The filestore feature requires the `filestore` Cargo feature flag:

```bash
cargo build --features filestore
```

When built without it, the `[filestore]` config section is ignored and all
behavior is unchanged from before.

## Cost Considerations

| Backend | Storage | PUT | GET (via presigned) | Egress |
|---------|---------|-----|---------------------|--------|
| AWS S3 | $0.023/GB/mo | $0.005/1K | $0.0004/1K | $0.09/GB |
| Cloudflare R2 | $0.015/GB/mo | $0.0045/1K | Free (Class B) | **Free** |
| MinIO (self-hosted) | Disk cost | — | — | — |

For typical usage (a few large files per day, auto-expired after 24h):
- **R2**: essentially free (zero egress + negligible storage)
- **S3**: < $0.01/month for most teams

## Comparison with Alternatives

| Approach | Pros | Cons |
|----------|------|------|
| **Filestore (this)** | Works for all agents, no platform auth leakage, configurable TTL | Requires S3/R2 bucket setup |
| Raw URL hint (PR #1346) | Zero infra needed | Slack broken, Discord expires, agent needs web-fetch |
| Local filesystem | No external deps | Only works in colocate mode, no remote agents |
| OAB HTTP proxy | No bucket needed | Complex, single-instance only, needs port management |

## Future Directions

- **Structured `ContentBlock::File`** in ACP for richer metadata (mime, size, TTL)
- **Metrics** — upload success rate, latency, file size distribution
- **Chunked download** — for extremely large files, stream to S3 in parts
- **Multi-modal** — extend filestore to images/audio when inline is too large
