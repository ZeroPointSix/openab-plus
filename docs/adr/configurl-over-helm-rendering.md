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
2. **Helm chart retains responsibility only for runtime posture** — this is where Helm continues to deliver real value that raw config cannot:
   - **Non-root execution** — enforce `runAsUser`/`runAsGroup` so the container never runs as root, reducing blast radius of container escapes.
   - **Read-only root filesystem** — `readOnlyRootFilesystem: true` with `drop: ALL` capabilities ensures the container cannot be tampered with at runtime; only the HOME PVC is writable.
   - **HOME PVC persistence** — dedicated PersistentVolumeClaim mounted at the agent's `$HOME`, providing durable workspace (git repos, session state, caches) that survives pod restarts.
   - Image version pinning and pull policy
   - ServiceAccount assignment (for IRSA)
   - Recreate strategy (RWO PVC constraint)
3. **Introduce `configFile` as a zero-logic alternative** — users place a raw `config.toml` alongside their Helm values. Helm copies it verbatim into a ConfigMap via `{{ .Files.Get }}` — no template rendering, no conditionals, no enum validation. This gives the "full config visibility" benefit without requiring S3/IRSA setup.
4. **Deprecate legacy ConfigMap rendering** — existing template logic remains for backward compatibility but is **no longer maintained**. No bug fixes, no new config features, no chart PRs for this path. Users on legacy rendering are encouraged to migrate to `configUrl` or `configFile`.
5. **Config lives externally or inline** — users maintain `config.toml` either in S3 (`s3://`), HTTPS, or as a local file next to their Helm values. Changes take effect on pod restart (configUrl) or on `helm upgrade` (configFile).
6. **Secrets live in AWS Secrets Manager** — referenced via `aws-sm://` in config.toml. No Kubernetes Secret objects required.

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

## 4. Configuration Modes

| Mode | Source | When to use | Helm interaction |
|------|--------|-------------|-----------------|
| **`configUrl`** | S3, HTTPS, R2 | Production — full GitOps, IAM audit | `helm install` once; config changes need only pod restart |
| **`configFile`** | Local `config.toml` next to values.yaml | Dev / simple deployments — no external deps | `helm upgrade` picks up file changes |
| **Legacy rendering** ⚠️ | `values.yaml` → template → ConfigMap | **Deprecated — not maintained** | Every config change = chart PR |

### configUrl mode (recommended for production)

```yaml
agents:
  kiro:
    configUrl: "s3://my-bucket/openab/kiro/config.toml"
    serviceAccountName: "openab"
```

Pod starts with `openab run -c s3://...` — config fetched at boot.

### configFile mode (recommended for dev / simple deployments)

```yaml
agents:
  kiro:
    configFile: "configs/kiro/config.toml"  # relative to chart root
    serviceAccountName: "openab"
```

Helm template logic is trivial — just a file copy, zero conditionals:

```yaml
{{- if .Values.agents.kiro.configFile }}
apiVersion: v1
kind: ConfigMap
metadata:
  name: {{ .Release.Name }}-kiro-config
data:
  config.toml: |
{{ .Files.Get .Values.agents.kiro.configFile | indent 4 }}
{{- end }}
```

The user maintains a **real `config.toml`** — what they write is exactly what the agent reads. No values-to-template translation layer.

### Legacy rendering (⚠️ deprecated — unmaintained)

Existing `values.yaml` → `templates/configmap.yaml` rendering continues to work for backward compatibility but is **no longer maintained**. It will not receive bug fixes, new config features, or support for new platforms.

> **Community notice:** We recommend all users migrate to `configUrl` (production) or `configFile` (dev/simple). The legacy `values.yaml` ConfigMap rendering path will not be updated going forward. Both new paths give you full visibility into your actual config.toml — no more guessing what Helm templates produce.

## 5. Minimal Helm Values (configUrl mode)

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
    # ── This is what Helm enforces (you don't touch config for this) ──
    securityContext:
      runAsUser: 1000
      runAsGroup: 1000
      runAsNonRoot: true
      readOnlyRootFilesystem: true
      allowPrivilegeEscalation: false
      capabilities:
        drop: ["ALL"]
```

**Why these three matter:**

| Helm-managed concern | What it prevents |
|---------------------|-----------------|
| `runAsUser: 1000` (non-root) | Container escape → host root access |
| `readOnlyRootFilesystem` | Runtime binary tampering, malware persistence outside HOME |
| HOME PVC (`persistence.enabled`) | Agent state loss on restart; provides durable workspace isolated from the immutable image |

## 6. Boot Behavior

OpenAB uses **fail-closed** boot semantics when `configUrl` is set:

- If the config source (S3/HTTPS) is unreachable at startup, the process exits with a non-zero code.
- Kubernetes will restart the pod per the Deployment's restart policy, providing automatic retry.
- There is no local cache or fallback — this is intentional to guarantee config freshness and avoid split-brain states.

This design choice is acceptable because:
- S3 provides 99.99% availability SLA.
- HTTPS endpoints (CDN-backed) have comparable availability.
- Pod restart loops are visible via standard Kubernetes monitoring (CrashLoopBackOff alerts).

## 7. Config Change Workflow

The recommended workflow is **edit-and-restart**:

1. Update `config.toml` in S3 (or HTTPS source).
2. Restart the pod: `kubectl rollout restart deployment/<agent>` or equivalent.
3. Pod fetches fresh config on boot.

**Hot-reload is explicitly out of scope for v1.** A future ADR may propose watch/poll mode, but the current design prioritizes simplicity and predictability.

## 8. Migration Path

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

## 9. Pre-deploy Validation

Since Helm template-time checks (e.g. Discord ID precision, enum validation) no longer apply in `configUrl` mode, validation shifts to:

1. **Fail-closed boot** — OAB validates config on startup and exits with clear error messages if invalid.
2. **`openab config validate`** (planned) — CLI command to validate a config.toml before uploading, suitable for CI pipelines.
3. **S3 versioning** — enables instant rollback to last-known-good config if a bad config is deployed.

## 10. Consequences

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

## 11. References

- `docs/config-reference.md` — s3:// config source documentation
- `docs/secrets-management.md` — aws-sm:// provider
- `crates/openab-core/src/config.rs` — s3:// URI parser
- `operator/examples/fleet.yaml` — fleet-scale s3:// configFrom usage
