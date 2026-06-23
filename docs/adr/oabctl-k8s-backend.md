# ADR: oabctl Kubernetes Backend (implement the stubbed `Runtime::Kubernetes`)

- **Status:** Proposed
- **Date:** 2026-06-23
- **Author:** @pahud.hsieh
- **Related:** [ECS Control Plane](./ecs-control-plane.md), [Multi-Platform Adapters](./multi-platform-adapters.md), [Unified Binary](./unified-binary.md)

---

## 0. Correction note (v2, not v1)

An earlier draft of this ADR was written against a **stale `feat/unified-binary-workspace`
copy** of `operator/src/manifest.rs` (a flat `oab.dev/v1` spec with top-level
`capacityProvider` / `networking`). **`main` has already moved past that.** This ADR has
been corrected to the actual `main` schema:

- `apiVersion: **oab.dev/v2**`
- A `Runtime` **enum** — `Ecs(EcsRuntime)` | `Kubernetes(KubernetesRuntime)` —
  serde-tagged by `type`, so the two runtimes are **mutually exclusive by construction**.
- `KubernetesRuntime` **already exists** (`nodeSelector`, `serviceAccount`, `tolerations`).
- `secrets` is already `HashMap<String,String>`; agent config is a **`configFrom`
  reference** (e.g. `s3://…/config.toml`), not an inline typed struct.
- `apply.rs:55-57` and `create.rs:56-57` already **stub** the K8s path with
  `"Kubernetes runtime not yet implemented"`.

**Consequence:** the schema refactor proposed by the old draft is **already done**.
The real, narrow scope of this ADR is: **implement the stubbed `Runtime::Kubernetes`
branch** (apply / create / delete / get) using `kube-rs`, and extend
`KubernetesRuntime` with the few fields a Deployment needs.

---

## 1. Context & Motivation

OpenAB is deployed to Kubernetes today via **Helm charts** (`charts/openab` plus the
`openab-line` / `openab-telegram` / `openab-feishu` sub-charts). Separately,
[`oabctl`](../../operator) provisions agents on **Amazon ECS Fargate** from an
`oab.dev/v2` `OABService` manifest with an S3-backed control plane.

We want **one tool and one spec** to deploy to both runtimes, and for `oabctl` to
eventually **replace the Helm chart** as the recommended K8s path. The good news
(see §0): the schema already supports both runtimes — the K8s code path is just not
implemented yet.

### Why replace Helm?

Helm's value is not "edit one `values.yaml` and `helm install`" — that is the UX
surface. The real value is three layers underneath: (1) **templating with logic**,
(2) **release lifecycle** (`install`/`upgrade`/`rollback`/`uninstall`/`history`),
(3) **distribution & ecosystem** (versioned charts, ArgoCD/Flux, `helm diff`).
`oabctl` can match all three and improve on (1): Helm templating is stringly-typed Go
templates, whereas `oabctl` renders from **typed Rust structs with real validation**.
The cost is reproducing Helm's **rendering surface** (the long pole) and its
**lifecycle verbs**.

---

## 2. Current State (verified against `main`)

| Piece | Status on `main` |
|-------|------------------|
| Manifest schema | `oab.dev/v2`, `OABService` + `OABFleet`, `Runtime` enum `Ecs`/`Kubernetes` |
| `KubernetesRuntime` | **Exists** (`nodeSelector`, `serviceAccount`, `tolerations`) — minimal |
| `oabctl` ECS path | Implemented: `apply`/`get`/`delete`, S3 config sync, ECS task def + service |
| `oabctl` K8s path | **Stubbed** — `apply.rs:55-57` / `create.rs:56-57` bail `"not yet implemented"` |
| Config delivery | `spec.configFrom` (e.g. `s3://…/config.toml`); `apply` syncs local config → S3 |
| Secrets | `spec.secrets: HashMap<String,String>` (name → `valueFrom` reference) |
| Helm charts | Mature K8s path. Uses **native Secret + `secretEnv`/`existingSecret`** (no ExternalSecret template), ConfigMap excludes secret values via `inherit_env` + `secretKeyRef` |
| K8s operator (CRD) | Not started (ECS ADR Phase 3) |

> Correction vs older drafts: Helm currently uses **native K8s Secrets**, *not*
> ExternalSecrets. Any "Helm already uses ExternalSecrets" statement is wrong and must
> not drive the K8s secret design.

---

## 3. Decision

Implement `oabctl`'s **`Runtime::Kubernetes` backend** consuming the **same
`oab.dev/v2` `OABService` manifest** as ECS, via **client-side render & apply**
(`kube-rs` server-side apply), with **no in-cluster component** in the first
milestone. An in-cluster CRD + operator remains a later, optional milestone
(ECS ADR Phase 3).

Two design choices, informed by review:

- **Keep the `Runtime` enum** (do *not* flatten to `platform.ecs`/`platform.k8s`
  struct overlays). The enum already enforces mutual exclusivity and lets `oabctl`
  **infer the target from the manifest** — no `--target` flag needed, and the ECS
  regression risk largely disappears.
- **Keep `configFrom` and `secrets: HashMap`** as-is. Config stays a path reference
  (clean boundary — `oabctl` never needs to compile the agent's config schema), and
  the SSM-vs-SecretsManager source is **inferred from the ARN prefix**.

---

## 4. Architecture

```
                  ┌───────────────────────────┐
                  │  oab.dev/v2 OABService     │  one spec
                  │  spec.runtime = Ecs | K8s  │  (enum → target inferred)
                  └─────────────┬──────────────┘
                                │ load + validate + resolve configFrom
                        ┌───────┴────────┐
                Runtime::Ecs        Runtime::Kubernetes
                        ▼                ▼
              ┌──────────────┐   ┌──────────────────┐
              │ ECS path     │   │  K8s path (NEW)  │
              │ (implemented)│   │  (kube-rs)       │
              └──────┬───────┘   └─────────┬────────┘
                     ▼                     ▼
        S3 config sync + ECS       Deployment + ConfigMap/initContainer +
        TaskDef + Service          Secret/ExternalSecret + PVC + SA (+ Service)
```

### 4.1 Provisioner trait

Today `apply.rs` calls the ECS SDK directly and matches on `spec.runtime`. Introduce a
`Provisioner` trait so the two runtimes are interchangeable, sharing manifest
loading, validation, and `configFrom` resolution.

```rust
#[async_trait]
trait Provisioner {
    async fn apply(&self, m: &OABServiceManifest) -> Result<()>;   // generation handled internally
    async fn delete(&self, ns: &str, name: &str) -> Result<()>;
    async fn get(&self, ns: &str, name: Option<&str>) -> Result<Vec<Status>>;
    // Grow deliberately as needs land (avoid a too-thin trait that breaks later):
    async fn status(&self, ns: &str, name: &str) -> Result<Health>;
    async fn logs(&self, ns: &str, name: &str, opts: LogOpts) -> Result<()>;
}
```

> **Generation is backend-specific** (S3 manifest for ECS, Deployment annotation for
> K8s). Each `Provisioner::apply` therefore **looks up and bumps generation
> internally**, rather than receiving it from the shared layer.

> **Shared-crate boundary:** keep the manifest types + renderers **inside the
> `operator` crate** for now. Do **not** spin up a separate `openab-manifest` crate
> unless the in-cluster controller (Phase K4) needs to compile independently — this
> avoids dependency pollution into the core agent workspace.

---

## 5. `KubernetesRuntime` Extension

`KubernetesRuntime` exists but is minimal. Extend it with the fields a Deployment +
PVC + Secret actually need. Additive only — no change to ECS or to `Spec`.

```rust
pub struct KubernetesRuntime {
    // existing
    #[serde(default)] pub node_selector: HashMap<String, String>,
    #[serde(default)] pub service_account: Option<String>,
    #[serde(default)] pub tolerations: Vec<serde_yaml::Value>,
    // additions
    #[serde(default)] pub storage_class: Option<String>,
    #[serde(default)] pub storage_size: Option<String>,        // PVC request, e.g. "1Gi"
    #[serde(default)] pub image_pull_secrets: Vec<String>,
    #[serde(default = "default_secret_backend")] pub secret_backend: String, // "external" | "native"
    #[serde(default)] pub service: Option<ServiceSpec>,        // optional Service/Ingress
}
```

`replicas` is intentionally **not** exposed: OAB agents hold a single stateful gateway
connection (Discord/Telegram/Slack websocket), so the Deployment generator
**hardcodes `replicas: 1`**. (`Spec` has no `replicas` field today — keep it that way.)

`spec.resources` is `{ cpu: String, memory: String }`. ECS validates against the
Fargate CPU table; **K8s maps them to `resources.requests/limits`** (`cpu: "512"` →
`500m`, `memory: "1024"` → `1Gi`) and defers format validation to the K8s API.

---

## 6. Kubernetes Backend Implementation

Add `kube` + `k8s-openapi` to [`operator/Cargo.toml`](../../operator/Cargo.toml).
Replace the `apply.rs:55-57` stub with a `K8sProvisioner` that builds typed objects
and does **server-side apply**:

```rust
let dep:  Deployment             = render_deployment(m);  // image, resources, replicas=1, env, mounts, nodeSelector, tolerations, SA
let cfg /* ConfigMap | initContainer */ = render_config(m);  // see config delivery below
let pvc:  PersistentVolumeClaim  = render_pvc(m);         // storageClass + storageSize
let sa:   ServiceAccount         = render_sa(m);          // IRSA / Pod Identity annotation
let sec /* Secret | ExternalSecret */ = render_secrets(m);

let pp = PatchParams::apply("oabctl").force();
for obj in objects { api.patch(&obj.name(), &pp, &Patch::Apply(&obj)).await?; }
```

### Config delivery

`spec.configFrom` is already an S3 path. Two viable K8s mappings:

1. **initContainer `s3 cp`** (mirrors ECS startup) → writes `config.toml` onto the PVC.
   Requires the pod SA to have AWS read perms (IRSA / Pod Identity). Most consistent
   with ECS.
2. **ConfigMap** — `oabctl` fetches `configFrom`, then writes a ConfigMap mounted at
   `/home/agent/config.toml`. No in-pod AWS creds needed for config.

Recommend (1) for parity with ECS and to avoid `oabctl` reading config content; make
it selectable later if needed. **Either way, `config.toml` must never contain secret
values** (see §7).

### Translation table (core → backend)

| Core (`oab.dev/v2`) | ECS backend | K8s backend |
|---------------------|-------------|-------------|
| `resources.cpu: "512"` | TaskDef `cpu=512` (Fargate table) | `resources.requests/limits.cpu: 500m` |
| `resources.memory: "1024"` | TaskDef `memory=1024` | `…memory: 1Gi` |
| `configFrom: s3://…` | startup sync → `config.toml` | initContainer `s3 cp` (or ConfigMap) |
| `secrets{name: valueFrom}` | ECS native `secrets` field | ExternalSecret (ESO) **or** native Secret |
| `bootstrapFrom` | startup `s3 cp` | initContainer `s3 cp` → PVC |
| `runtime: Ecs{…}` | used | n/a (enum) |
| `runtime: Kubernetes{…}` | n/a (enum) | used |

---

## 7. Secret Handling (security-critical)

`spec.secrets` is `name → valueFrom` (an ARN/SSM path). Source is **inferred from the
ARN prefix** (`arn:aws:ssm:…` vs `arn:aws:secretsmanager:…`). `KubernetesRuntime.secretBackend`
selects how those references become pod env:

- **`external` (default, recommended)** — emit an **`ExternalSecret`** (External
  Secrets Operator) that syncs from SSM / Secrets Manager into a K8s Secret consumed
  via `secretKeyRef`. `oabctl` never sees the secret value — preserves the ECS ADR
  principle that the provisioner handles **references, not values**.
- **`native`** — `oabctl` reads the value and writes a K8s `Secret` directly.
  **Discouraged in production**: it turns `oabctl` into a plaintext secret relay,
  binding AWS read creds to the operator's machine. If kept, gate it behind an
  explicit opt-in flag and document the rotation/audit implications.

### Hard requirements (from security review)

1. **`config.toml` / ConfigMap must never carry secret values.** Match Helm's existing
   protection: secrets reach the pod only via env `secretKeyRef`; the rendered config
   references env, never literals. The renderer must **reject** a config that inlines a
   secret key (parity with the chart's guard).
2. **No `--set` on secret fields.** When `--set` lands, maintain a **denylist** for
   secret-value paths, and store any companion/last-applied manifest **redacted** —
   otherwise a plaintext secret could be persisted into an annotation/ConfigMap (etcd
   readable).
3. **K8s Secret Injection Contract** (parity with ECS ADR §6) — document: the pod SA's
   IRSA/Pod-Identity role, the SSM/SM **resource ARN scoping** per agent, an ESO
   `SecretStore`/`ClusterSecretStore` example, and an **apply-time preflight** that
   verifies the ESO CRDs/operator exist (else `ExternalSecret` fails silently and the
   pod starts without env).
4. **`bootstrapFrom` + config initContainer need AWS creds** (IRSA/Pod Identity) — list
   the IAM requirement explicitly.
5. **Minimum `oabctl` kube RBAC** — server-side apply needs `patch`/`get` on
   `deployments`, `configmaps`, `secrets`, `serviceaccounts`, `persistentvolumeclaims`
   (+ `externalsecrets` when `external`). Document the Role.
6. **Secret rotation** — define how rotation triggers a rollout: ESO `refreshInterval`
   + a checksum annotation on the Deployment to force restart (parity with ECS ADR §6's
   `autoRestart`/circuit-breaker).

---

## 8. Lifecycle, Target Selection, State

### Verbs map to Helm

| Helm | oabctl | Status |
|------|--------|--------|
| `helm install` / `helm upgrade` | `oabctl apply -f` (declarative create-or-update) | ECS done; add K8s |
| `helm uninstall` | `oabctl delete` | ECS done; add K8s |
| `helm template` | `oabctl template` (render-only) | new — needed for GitOps/CI |
| `helm rollback` | `oabctl rollback` | new |
| `helm history` | `oabctl history` | new |
| `helm … --set` | `oabctl apply --set` (patch-then-reapply; secret-field denylist) | new |

> Underspecified vs Helm and tracked as follow-ups: `--values` file merging,
> sub-chart/`dependency` composition (line/telegram/feishu), and `helm test`. These are
> real operational features and must be addressed in the parity phase, not hand-waved.

### Target selection

**Inferred from `spec.runtime`** — no `--target` flag. `Runtime::Ecs` → ECS path,
`Runtime::Kubernetes` → K8s path. The enum guarantees exactly one.

### Generation / state

- **ECS:** generation in the S3 manifest (existing).
- **K8s:** generation + manifest **hash** in Deployment **annotations**
  (`oab.dev/generation`, `oab.dev/manifest-hash`); for `history`/`rollback` keep a
  **redacted** last-applied manifest (never store secret values). K8s mode needs no S3
  control plane.

---

## 9. Example (same spec, runtime selects target)

```yaml
apiVersion: oab.dev/v2
kind: OABService
metadata: { name: chaodu, namespace: prod }
spec:
  image: ghcr.io/openabdev/openab:latest
  resources: { cpu: "512", memory: "1024" }
  configFrom: s3://oab-control-plane/config/prod/chaodu/config.toml
  secrets:
    KIRO_API_KEY: arn:aws:secretsmanager:us-east-1:123:secret:kiro
  runtime:
    type: kubernetes
    serviceAccount: oab-agent
    storageClass: gp3
    storageSize: 1Gi
    secretBackend: external
```

Swap `runtime` to `type: ecs` with `networking`/`capacityProvider` to target ECS — same
everything else.

---

## 10. Phase Plan

### Phase K1 — Implement `Runtime::Kubernetes` (Helm replacement, core)
- Add `kube`/`k8s-openapi`; introduce `Provisioner` trait; move ECS code behind
  `EcsProvisioner`.
- Extend `KubernetesRuntime` (§5). Replace the `apply.rs`/`create.rs` stubs.
- Generate Deployment + (initContainer config) + PVC + Secret/ExternalSecret + SA.
- `oabctl apply/get/delete` for K8s; add `oabctl template`. Validate on a real cluster
  with one Kiro agent. Document the kube RBAC (§7.5) and Secret Injection Contract (§7.3).

### Phase K2 — Parity with the Helm chart
- **Gating quality bar:** golden-file tests diffing `oabctl template` vs
  `helm template` for representative cases — including the secret-not-in-ConfigMap
  guard (§7.1).
- Cover gateway, adapter sub-charts (line/telegram/feishu), ingress/Service,
  imagePullSecrets, persistence, message-processing modes, `--values` merging.

### Phase K3 — Migration & Helm deprecation
- `oabctl migrate --from-helm <release>` → emit `oab.dev/v2` manifest.
- `oabctl rollback` / `history`. Run both in parallel one release; deprecate charts once
  parity tests are green.

### Phase K4 — CRD + in-cluster operator (optional, ECS ADR Phase 3)
- `OABService` CRD + reconciler for GitOps/self-healing; reuses K1 rendering.

---

## 11. CI (close the gate gap)

The operator currently lacks automated K8s gating. Add from K1:

- `operator/**` PRs → `cargo test` + `cargo clippy` + an `oabctl template` smoke test.
- K2 → an `oabctl template` vs `helm template` **diff job** in CI (not local/manual).
- Optional policy test: fail if any rendered `ConfigMap` `data` contains
  `api_key`/`token`-like values (enforces §7.1).
- Note: a **docs-only ADR PR does not trigger operator CI** — so kube-rs / SSA / RBAC
  correctness is only exercised once K1 code lands. Plan the CI alongside K1, not after.

---

## 12. Risks

| # | Risk | Mitigation |
|---|------|------------|
| R1 | ECS/K8s render drift | Shared renderers in the `operator` crate; both paths call the same config resolution |
| R2 | Helm feature-parity surface is large (~22 KB values, 14 tests) | Golden-file `oabctl template` vs `helm template` is a **gating** K2 criterion |
| R3 | `native` secret backend = plaintext relay | Default `external`; gate `native` behind explicit opt-in; denylist `--set` on secret fields; redacted companion manifest |
| R4 | ESO assumed but absent → silent failure | Apply-time **preflight** for ESO CRDs/operator; clear error |
| R5 | Losing Helm ecosystem (rollback/history/`--values`/sub-charts/`helm test`) | `oabctl template` + `rollback`/`history`; track `--values`/sub-chart/`test` parity explicitly in K2 |
| R6 | Too-thin `Provisioner` trait → later API breakage | Include `status`/`logs` from the start; grow deliberately |

---

## 13. Alternatives Considered

| Alternative | Why not |
|-------------|---------|
| Flatten `runtime` enum → `platform.ecs`/`platform.k8s` overlays | The enum already gives mutual exclusivity + target inference; flattening adds risk for no gain |
| Inline typed `AgentConfig` in the manifest | Couples `oabctl` to the agent config schema; `configFrom` reference is a cleaner boundary (already in `main`) |
| `secrets: Vec<SecretRef>` with explicit `source` | `HashMap<name, valueFrom>` + ARN-prefix inference is simpler and already in `main` |
| Keep Helm for K8s, `oabctl` for ECS | Two tools/specs; defeats the one-spec goal |
| Shell out to `helm`/`kubectl` | Reintroduces Go-template fragility; loses typed validation |
| CRD + operator first | Larger lift; not needed for parity; blocks Helm deprecation |

---

## 14. Open Questions

1. **ESO hard dependency?** Require ESO for `external`, or allow `oabctl` to write native
   Secrets when ESO is absent (with the §7 safeguards)?
2. **Config delivery** — initContainer `s3 cp` (ECS parity, needs pod AWS creds) vs
   `oabctl`-fetched ConfigMap (no pod creds). Pick a default.
3. **Gateway** — port the chart's gateway resources into `oabctl`, or keep gateway on
   Helm until K2?
4. **CRD timing** — ship K4 at all, or is client-render sufficient for the roadmap?

---

## 15. Recommendation

Scope is now **narrow and concrete**: implement the already-stubbed
`Runtime::Kubernetes` branch. First PR = **Phase K1 spike**: add `kube`/`k8s-openapi`,
introduce the `Provisioner` trait, extend `KubernetesRuntime`, and replace the
`apply.rs` stub to deploy the minimal Deployment + (config initContainer) + PVC +
Secret for one Kiro agent against a real cluster — with the secret-injection contract
(§7) and operator CI (§11) landed alongside the code, not deferred.
