# ADR: Shift from Helm ConfigMap Rendering to External Config URL

- **Status:** Proposed
- **Date:** 2026-07-01
- **Author:** @pahud

---

## 1. Problem Statement

OpenAB has historically relied on Helm chart templates to render `config.toml` into a Kubernetes ConfigMap. This approach served early users well — it provided a single `values.yaml` surface for all configuration, bundled security defaults, and automated Secret creation.

However, as OpenAB matured, two key features eliminated the core reasons for ConfigMap rendering:

1. **`configUrl` support** — OpenAB can now fetch its config directly from an external URL (`https://`, `s3://`) at boot time via `openab run -c <url>`.
2. **`aws-sm://` secrets resolution** — Credentials are resolved in-app from AWS Secrets Manager (or exec providers), removing the need for Kubernetes Secrets entirely.

This leaves the Helm chart's ConfigMap rendering as a **maintenance burden with diminishing value**:

- Every new config feature (e.g. `pre_seed`, `ambient`, `trust`, `hooks`) requires a synchronized PR to update `values.yaml`, `templates/configmap.yaml`, and chart tests.
- Users must reason backwards from `values.yaml` through Helm templates to understand their actual config — they never see the final `config.toml` directly.
- The rendering logic contains ~300+ lines of conditionals, enum validations, and platform-specific assembly.

Meanwhile, declarative tooling (`ecsctl`, `oabctl`, Operator CRDs) operates on the final config state directly, making the Helm rendering layer an outlier in the architecture.

## 2. Decision

1. **Recommend `configUrl` as the primary configuration path** for all new deployments.
2. **Helm chart retains responsibility only for runtime posture:**
   - Pod security context (`runAsNonRoot`, `readOnlyRootFilesystem`, `drop: ALL`)
   - PVC lifecycle and persistence
   - Image version pinning and pull policy
   - ServiceAccount assignment (for IRSA)
   - Recreate strategy (RWO PVC constraint)
3. **Freeze ConfigMap rendering** — existing template logic remains for backward compatibility but is no longer the recommended path. New config features do NOT require chart PRs.
4. **Config lives externally** — users maintain `config.toml` in S3 (`s3://`), Cloudflare R2, GitHub Gist, or any HTTPS endpoint. Changes take effect on pod restart with zero Helm interaction.
5. **Secrets live in AWS Secrets Manager** — referenced via `aws-sm://` in config.toml. No Kubernetes Secret objects required.

## 3. Target Architecture

```
┌─────────────────────────────────────────────────────┐
│ Helm / kubectl / ecsctl / Operator                  │
│ (runtime posture only: image, security, PVC, SA)    │
└──────────────────────┬──────────────────────────────┘
                       │ deploys pod with:
                       │   args: ["openab", "run", "-c", "s3://..."]
                       ▼
┌─────────────────────────────────────────────────────┐
│ OpenAB process                                       │
│  1. Fetch config.toml from s3://bucket/key           │
│  2. Run pre_boot hooks                               │
│  3. Resolve aws-sm:// secrets                        │
│  4. Start agent sessions                             │
└─────────────────────────────────────────────────────┘
```

## 4. Minimal Helm Values (configUrl mode)

```yaml
image:
  repository: ghcr.io/openabdev/openab
  tag: "0.9.0-beta.6"

agents:
  kiro:
    configUrl: "s3://my-bucket/openab/kiro/config.toml"
    serviceAccountName: "openab"  # IRSA for S3 + Secrets Manager
    persistence:
      enabled: true
      size: 1Gi
```

## 5. Boot Behavior

OpenAB uses **fail-closed** boot semantics when `configUrl` is set:

- If the config source (S3/HTTPS) is unreachable at startup, the process exits with a non-zero code.
- Kubernetes will restart the pod per the Deployment's restart policy, providing automatic retry.
- There is no local cache or fallback — this is intentional to guarantee config freshness and avoid split-brain states.

This design choice is acceptable because:
- S3 provides 99.99% availability SLA.
- HTTPS endpoints (CDN-backed) have comparable availability.
- Pod restart loops are visible via standard Kubernetes monitoring (CrashLoopBackOff alerts).

## 6. Config Change Workflow

The recommended workflow is **edit-and-restart**:

1. Update `config.toml` in S3 (or HTTPS source).
2. Restart the pod: `kubectl rollout restart deployment/<agent>` or equivalent.
3. Pod fetches fresh config on boot.

**Hot-reload is explicitly out of scope for v1.** A future ADR may propose watch/poll mode, but the current design prioritizes simplicity and predictability.

## 7. Migration Path

For existing users on full Helm ConfigMap rendering:

### Step 1: Export current config

```bash
# Extract rendered config from the running ConfigMap
kubectl get configmap <agent>-config -o jsonpath='{.data.config\.toml}' > config.toml
```

### Step 2: Upload to S3

```bash
# Recommended bucket structure
aws s3 cp config.toml s3://my-openab-configs/agents/<name>/config.toml

# Enable versioning for rollback capability
aws s3api put-bucket-versioning \
  --bucket my-openab-configs \
  --versioning-configuration Status=Enabled
```

### Step 3: Configure IRSA

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": ["s3:GetObject"],
      "Resource": "arn:aws:s3:::my-openab-configs/agents/*"
    },
    {
      "Effect": "Allow",
      "Action": ["secretsmanager:GetSecretValue"],
      "Resource": "arn:aws:secretsmanager:*:*:secret:openab/*"
    }
  ]
}
```

### Step 4: Switch Helm values

```yaml
# Before (ConfigMap rendering)
agents:
  kiro:
    discord:
      token: "aws-sm://openab/kiro/discord-token"
    # ... 50+ lines of config in values.yaml

# After (configUrl mode)
agents:
  kiro:
    configUrl: "s3://my-openab-configs/agents/kiro/config.toml"
    serviceAccountName: "openab"
```

### Step 5: Deploy and verify

```bash
helm upgrade openab charts/openab -f values.yaml
kubectl logs -f deployment/kiro | head -20
# Look for: "Config loaded from s3://..."
```

## 8. Pre-deploy Validation

Since Helm template-time checks (e.g. Discord ID precision, enum validation) no longer apply in `configUrl` mode, validation shifts to:

1. **Fail-closed boot** — OAB validates config on startup and exits with clear error messages if invalid.
2. **`openab config validate`** (planned) — CLI command to validate a config.toml before uploading, suitable for CI pipelines.
3. **S3 versioning** — enables instant rollback to last-known-good config if a bad config is deployed.

## 9. Consequences

### Positive

- **Zero chart maintenance for new config features** — schema changes never propagate to Helm.
- **Users see the full config** — no mental model of values → template → ConfigMap required.
- **Edit-and-restart workflow** — change config in S3/gist, restart pod, done.
- **Aligned with declarative tooling** — ecsctl, oabctl, and Operator all operate on final config state.
- **Reduced issue surface** — eliminates "my Helm values don't render correctly" class of bugs.
- **S3 availability** — `s3://` path gives 99.99% SLA, private access via IAM, versioning, and CloudTrail audit.

### Negative

- **Boot-time dependency on S3/network** — if the config source is unreachable, OAB cannot start. Mitigated by S3's extreme availability and pod restart policy.
- **Backward compatibility** — existing users on full Helm rendering need the migration path above.
- **No pre-deploy validation** — Helm template-time checks no longer catch errors before deploy. Mitigated by fail-closed boot and planned `openab config validate` CLI command.

### Neutral

- Helm is not deprecated — it remains the recommended way to enforce runtime security posture on Kubernetes. Its scope simply narrows.
- Multi-agent deployments still benefit from Helm's `agents.<name>` loop for generating multiple Deployments/PVCs from a single release.

## 10. References

- `docs/config-reference.md` — s3:// config source documentation
- `docs/secrets-management.md` — aws-sm:// provider
- `crates/openab-core/src/config.rs` — s3:// URI parser
- `operator/examples/fleet.yaml` — fleet-scale s3:// configFrom usage
