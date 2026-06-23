# ADR: oabctl Kubernetes Backend (one spec, two runtimes)

- **Status:** Proposed
- **Date:** 2026-06-23
- **Author:** @pahud.hsieh
- **Related:** [ECS Control Plane](./ecs-control-plane.md), [Multi-Platform Adapters](./multi-platform-adapters.md), [Unified Binary](./unified-binary.md)

---

## 1. Context & Motivation

OpenAB is deployed to Kubernetes today via **Helm charts** (`charts/openab` plus the
`openab-line` / `openab-telegram` / `openab-feishu` sub-charts). Separately,
[`oabctl`](../../operator) provisions agents on **Amazon ECS Fargate** using an
`oab.dev/v1` `OABService` manifest and an S3-backed control plane.

We want a **single tool and a single spec** to deploy OpenAB to **both** ECS and
Kubernetes, and ultimately for `oabctl` to **replace the Helm chart** as the
recommended K8s deployment path.

The [ECS Control Plane ADR](./ecs-control-plane.md) already anticipated this: §4
("Multi-Runtime Support") defines a platform-agnostic core spec with optional
`platform.ecs` / `platform.k8s` overlays, and lists a "K8s operator" as Phase 3.
This ADR makes that concrete — it specifies **how** `oabctl` gains a Kubernetes
backend using the same spec, and how it reaches feature parity with (and replaces)
Helm.

### Why replace Helm?

Helm's value is not "edit one `values.yaml` and `helm install`" — that is just the
UX surface. The real value is three layers underneath:

1. **Templating with logic** — one small input expands into many K8s objects.
2. **Release lifecycle** — `install` / `upgrade` / `rollback` / `uninstall` / `history`
   over a named, versioned release.
3. **Distribution & ecosystem** — versioned chart artifacts, ArgoCD/Flux, `helm diff`.

`oabctl` can match all three, and improve on #1: Helm templating is stringly-typed Go
templates, whereas `oabctl` renders from **typed Rust structs with real validation** —
better error messages and schema enforcement. The cost is reproducing Helm's
**rendering surface** (the long pole) and its **lifecycle verbs** (mostly already
present, see §7).

---

## 2. Current State & The Core Blocker

| Piece | Status |
|-------|--------|
| Helm charts | Mature K8s path. ~22 KB `values.yaml`, 14 chart tests, gateway, PVC, ExternalSecrets, ServiceAccount, adapter sub-charts |
| `oabctl` | ECS-only: `apply` / `get` / `delete` → S3 manifest store + ECS service reconcile |
| `oab.dev/v1` schema | Platform overlays designed in ECS ADR §4 — **not yet implemented** in code |
| K8s operator | Not started (ECS ADR Phase 3) |

**Blocker:** despite the ADR's intent, the spec in
[`operator/src/manifest.rs`](../../operator/src/manifest.rs) is **ECS-coupled**:

- `Spec` has ECS-isms as **required top-level** fields: `capacityProvider`, and
  `networking.subnets` / `networking.securityGroups`.
- `validate()` **hard-rejects** any manifest lacking subnets / security groups or a
  valid Fargate capacity provider.

So a K8s-only user is currently forced to supply meaningless ECS networking. Making
the spec genuinely platform-agnostic is therefore **step one**.

---

## 3. Decision

Add a Kubernetes backend to `oabctl` that consumes the **same `oab.dev/v1`
`OABService` manifest** as ECS, selected at apply time. Approach:

- **Client-side render & apply** (like Helm and `kubectl apply`) as the first
  milestone — no in-cluster component required.
- A typed, **platform-agnostic core spec** with `platform.ecs` / `platform.k8s`
  overlays (ECS ADR §4), enforced by **target-aware validation**.
- A **`Provisioner` trait** abstraction so ECS and K8s are interchangeable behind one
  CLI, sharing manifest loading, validation, generation tracking, and
  `config.toml` rendering.
- An optional **in-cluster CRD + operator** as a later milestone for GitOps /
  self-healing (ECS ADR Phase 3), reusing the same rendering layer.

We explicitly choose client-render-first over CRD-first because it is the honest 1:1
Helm replacement, requires zero cluster install, reuses existing rendering code, and
unblocks Helm deprecation fastest.

---

## 4. Architecture

```
                  ┌──────────────────────────┐
                  │   oab.dev/v1 OABService   │  one spec, platform-agnostic core
                  │   + platform.{ecs,k8s}    │
                  └─────────────┬─────────────┘
                                │  load + validate(target) + render config.toml
                                │  (shared layer)
                        ┌───────┴────────┐
              --target ecs            --target k8s
                        ▼                ▼
              ┌──────────────┐   ┌──────────────────┐
              │EcsProvisioner│   │  K8sProvisioner  │
              │ (aws-sdk)    │   │  (kube-rs)       │
              └──────┬───────┘   └─────────┬────────┘
                     ▼                     ▼
            S3 artifact + ECS      Deployment + ConfigMap +
            TaskDef + Service      Secret/ExternalSecret +
                                   PVC + ServiceAccount (+ Service/Ingress)
```

### 4.1 Provisioner trait

```rust
#[async_trait]
trait Provisioner {
    async fn apply(&self, m: &OABServiceManifest, generation: u64) -> Result<()>;
    async fn delete(&self, ns: &str, name: &str) -> Result<()>;
    async fn get(&self, ns: &str, name: Option<&str>) -> Result<Vec<Status>>;
}

struct EcsProvisioner { ecs: aws_sdk_ecs::Client, s3: aws_sdk_s3::Client }
struct K8sProvisioner { client: kube::Client }
```

Shared, backend-independent layer: manifest parsing, `validate()`, generation
bump, and `render_config_toml()`. This is what prevents the two backends from
drifting (see Risk R1).

---

## 5. Schema Refactor (Step 1)

Pull ECS-specifics out of the core `Spec` into `platform.ecs`; mirror with
`platform.k8s`. Core stays cross-platform.

```rust
pub struct Spec {
    // --- core (cross-platform) ---
    pub cpu: i32,
    pub memory: i32,
    pub image: String,                      // was task_definition.image
    #[serde(default)] pub replicas: u32,    // validated == 1
    #[serde(default)] pub bootstrap_from: Option<String>,
    pub config: AgentConfig,
    #[serde(default)] pub secrets: Vec<SecretRef>,
    // --- platform overlays (both optional) ---
    #[serde(default)] pub platform: Platform,
}

#[derive(Default)]
pub struct Platform {
    #[serde(default)] pub ecs: Option<EcsPlatform>,
    #[serde(default)] pub k8s: Option<K8sPlatform>,
}

pub struct EcsPlatform {
    #[serde(default = "default_capacity_provider")] pub capacity_provider: String,
    pub networking: Networking,             // subnets, securityGroups, assignPublicIp
    #[serde(default)] pub execution_role: Option<String>,
    #[serde(default)] pub task_role: Option<String>,
}

pub struct K8sPlatform {
    #[serde(default)] pub service_account: Option<String>,
    #[serde(default)] pub storage_class: Option<String>,
    #[serde(default)] pub node_selector: std::collections::HashMap<String, String>,
    #[serde(default)] pub image_pull_secrets: Vec<String>,
    #[serde(default = "default_secret_backend")] pub secret_backend: String, // "external" | "native"
    #[serde(default)] pub service: Option<ServiceSpec>,   // optional Service/Ingress
}
```

### Target-aware validation

`validate()` takes the resolved target and enforces only that platform's invariants:

- **ECS**: `platform.ecs.networking.subnets` and `securityGroups` non-empty;
  `capacityProvider ∈ {FARGATE, FARGATE_SPOT}`.
- **K8s**: `platform.k8s` keys are well-formed; `secretBackend ∈ {external, native}`.
- **Core (both)**: `apiVersion == oab.dev/v1`, `kind == OABService`, `name` /
  `namespace` present, `replicas == 1`.

Each backend **strict-validates its own** `platform.*` key and **ignores** the other
(ECS ADR §4 rules).

### Backward compatibility

Existing ECS manifests use top-level `capacityProvider` / `networking`. To avoid
breaking them, the loader supports a one-release **migration shim**: if legacy
top-level ECS fields are present and `platform.ecs` is absent, fold them into
`platform.ecs` and emit a deprecation warning. Drop the shim in the next minor.

---

## 6. Kubernetes Backend (Step 3)

Add `kube` + `k8s-openapi` to [`operator/Cargo.toml`](../../operator/Cargo.toml).
`K8sProvisioner::apply` builds typed objects from the **same** manifest and performs
**server-side apply** (the K8s-native analogue of ECS register-task-def +
update-service):

```rust
let dep:  Deployment             = render_deployment(m);  // image, cpu/mem→resources, replicas, env, mounts
let cm:   ConfigMap              = render_configmap(m);   // render_config_toml() → config.toml
let pvc:  PersistentVolumeClaim  = render_pvc(m);         // storageClass from platform.k8s
let sa:   ServiceAccount         = render_sa(m);          // IRSA / Pod Identity annotation if set
let sec /* Secret | ExternalSecret */ = render_secrets(m);

let pp = PatchParams::apply("oabctl").force();
for obj in [dep, cm, pvc, sa, sec] {
    api.patch(&obj.name(), &pp, &Patch::Apply(&obj)).await?;
}
```

All objects carry owner labels (`app.kubernetes.io/managed-by: oabctl`,
`oab.dev/namespace`, `oab.dev/name`) so `get` / `delete` work via label selectors.

### Translation table (core → backend)

| Core spec | ECS backend | K8s backend |
|-----------|-------------|-------------|
| `cpu: 512` | TaskDef `cpu=512` | `resources.requests/limits.cpu: 500m` |
| `memory: 1024` | TaskDef `memory=1024` | `resources.requests/limits.memory: 1Gi` |
| `config` | render → S3 artifact + startup wrapper | render → **ConfigMap** mounted at `/home/agent/config.toml` |
| `secrets[].source: ssm` | ECS native `secrets` field | **ExternalSecret** → K8s Secret (or native Secret) |
| `secrets[].source: secretsmanager` | ECS native `secrets` field | ExternalSecret (ESO) / native Secret |
| `bootstrapFrom` | startup wrapper `s3 cp` | **initContainer** `s3 cp` → PVC |
| `replicas: 1` | `desiredCount=1` | `replicas: 1` |
| `platform.ecs.*` | used | ignored |
| `platform.k8s.*` | ignored | used |

**Config delivery differs by design:** ECS has no ConfigMap equivalent, so it renders
to an immutable S3 artifact and downloads at startup; K8s mounts a ConfigMap
directly. Both use the **same** `render_config_toml()`. This asymmetry is expected.

### Secret backend

`platform.k8s.secretBackend` selects:

- `external` (default) — emit an `ExternalSecret` (External Secrets Operator) that
  syncs from SSM / Secrets Manager. Requires ESO installed in-cluster.
- `native` — `oabctl` reads the source value and writes a K8s `Secret` directly
  (requires `oabctl` to have AWS read perms; simpler clusters, no ESO dependency).

---

## 7. Lifecycle & Target Selection

### CLI verbs map cleanly to Helm

| Helm | oabctl | Status |
|------|--------|--------|
| `helm install` | `oabctl apply -f` | exists (ECS); add K8s |
| `helm upgrade` | `oabctl apply -f` (same command — declarative create-or-update) | exists (ECS); add K8s |
| `helm uninstall` | `oabctl delete` | exists (ECS); add K8s |
| `helm template` | `oabctl template -f` (render-only, no apply) | new — needed for GitOps/CI dry-run |
| `helm rollback` | `oabctl rollback <name> --to-generation N` | new — generation data already recorded |
| `helm history` | `oabctl history <name>` | new — list generations |
| `helm ... --set k=v` | `oabctl apply --set k=v` (patch-then-reapply on the stored manifest) | new — see below |

`install` and `upgrade` are intentionally the **same** declarative command (like
`kubectl apply`): create if absent, diff-and-roll if present.

`--set` is implemented as a **read-modify-write on the stored manifest** (pull
canonical manifest → apply patch → bump generation → reconcile), so the source of
truth stays declarative and `oabctl get -o yaml` always reflects reality. (Caveat:
in future CRD mode, `--set` must patch the CR object, not an S3 manifest — define the
semantics per backend before shipping.)

### Target selection (priority order)

1. `--target ecs|k8s` flag (explicit, wins) — ship first.
2. `~/.oabctl/config` default (`target = "k8s"`).
3. Inference: `platform.k8s` present and no ECS/AWS context → K8s.

### Generation / state per backend

- **ECS**: generation in the S3 manifest (existing model).
- **K8s**: generation + manifest hash in Deployment **annotations**
  (`oab.dev/generation`, `oab.dev/manifest-hash`); optional companion ConfigMap holds
  the last-applied manifest for `history` / `rollback`. **K8s mode needs no S3
  control plane** — that is an ECS implementation detail.

---

## 8. One Manifest, Two Targets (example)

```yaml
apiVersion: oab.dev/v1
kind: OABService
metadata:
  name: chaodu
  namespace: prod
spec:
  cpu: 512
  memory: 1024
  image: ghcr.io/openabdev/openab:latest
  replicas: 1
  config:
    backend: { type: kiro }
    channels: [{ type: discord }]
  secrets:
    - name: KIRO_API_KEY
      source: secretsmanager
      arn: arn:aws:secretsmanager:us-east-1:123:secret:kiro
  platform:
    ecs:
      capacityProvider: FARGATE_SPOT
      networking: { subnets: [subnet-a], securityGroups: [sg-1] }
    k8s:
      serviceAccount: oab-agent
      storageClass: gp3
      secretBackend: external
```

```bash
oabctl apply -f chaodu.yaml --target k8s    # → Deployment + ConfigMap + PVC + Secret
oabctl apply -f chaodu.yaml --target ecs    # → S3 artifact + ECS service
```

---

## 9. Phase Plan

### Phase K0 — Foundation
- Extract manifest types + `render_config_toml()` into a shared module/crate
  (`openab-manifest`) used by both backends.
- Refactor schema to `platform.{ecs,k8s}` + target-aware `validate()` + legacy shim.
- **No behavior change for existing ECS users.**

### Phase K1 — K8s render & apply (Helm replacement, core)
- Add `kube` / `k8s-openapi`; implement `Provisioner` trait + `K8sProvisioner`.
- Generate Deployment + ConfigMap + PVC + Secret/ExternalSecret + ServiceAccount.
- `oabctl apply/get/delete --target k8s`; add `oabctl template`.
- Validate against a real cluster with a single Kiro agent.

### Phase K2 — Parity with the Helm chart
- Golden-file tests: diff `oabctl template` vs `helm template` for representative
  `values.yaml` cases (this is the gating quality bar).
- Cover gateway, adapter sub-charts (line/telegram/feishu), ingress/Service,
  ExternalSecrets variants, imagePullSecrets, persistence, message-processing modes
  — everything with a current chart test.

### Phase K3 — Migration & Helm deprecation
- `oabctl migrate --from-helm <release>` → emit `oab.dev/v1` manifest from chart values.
- Add `oabctl rollback` / `history`.
- Run both in parallel one release; mark charts deprecated once parity tests are green.

### Phase K4 — CRD + in-cluster operator (optional, ECS ADR Phase 3)
- Ship `OABService` CRD + reconciler for GitOps / self-healing.
- `oabctl apply` gains a CR-submit / `--server-side` mode. Reuses K0 rendering.

---

## 10. Risks

| # | Risk | Mitigation |
|---|------|------------|
| R1 | ECS and K8s rendering drift apart | Shared `openab-manifest` crate (K0); both backends call the same `render_config_toml()` |
| R2 | Helm feature parity is large (~22 KB values, 14 tests) | Golden-file `oabctl template` vs `helm template` diff tests are a **gating** criterion for K2 |
| R3 | Secret model divergence (ECS native vs ESO) | `platform.k8s.secretBackend: external\|native`; document ESO prerequisite for `external` |
| R4 | Losing Helm ecosystem (ArgoCD/Flux, `helm diff`, rollback) | `oabctl template` keeps GitOps tools working; add `rollback`/`history` (K3) |
| R5 | Breaking existing ECS manifests during schema refactor | One-release legacy shim folding top-level ECS fields into `platform.ecs` + deprecation warning |
| R6 | `--set` semantics differ between manifest-store and future CRD mode | Define per-backend semantics before shipping `--set` |

---

## 11. Alternatives Considered

| Alternative | Why not chosen |
|-------------|----------------|
| Keep Helm for K8s, `oabctl` for ECS | Two tools, two specs, two mental models; the stated goal is one spec / one tool |
| `oabctl` shells out to `helm`/`kubectl` | Reintroduces Go-template fragility and a Helm runtime dependency; loses typed validation |
| CRD + operator first (skip client-render) | Much larger lift (CRD lifecycle, RBAC, controller HA, finalizers); blocks Helm deprecation; not needed for parity |
| Generate static YAML for `kubectl apply` | No lifecycle (rollback/history/uninstall), no typed validation — a downgrade from Helm |

---

## 12. Open Questions

1. **ESO hard dependency?** Should `external` secret backend require ESO, or should
   `oabctl` optionally write native Secrets when ESO is absent?
2. **Gateway** — port the chart's gateway resources into `oabctl`, or keep gateway on
   Helm until K2 completes?
3. **Shared crate boundary** — does `openab-manifest` also absorb the main binary's
   config types, or stay operator-local for now?
4. **CRD timing** — do we ship K4 at all, or is client-render sufficient for the
   foreseeable roadmap?

---

## 13. Recommendation

Proceed with **Phase K0 + a K1 spike** as the first PR: extract the shared render
layer, refactor the schema to `platform.{ecs,k8s}` with target-aware validation
(no ECS behavior change), introduce the `Provisioner` trait, and prototype
`oabctl apply --target k8s` deploying the minimal Deployment + ConfigMap + PVC +
Secret for one Kiro agent against a real cluster. This proves "same spec → K8s"
end-to-end before investing in the parity long tail.
