# ADR: Codeg Frontend Facade Module

- **Status:** Proposed
- **Date:** 2026-09-03
- **Author:** @ZeroPointSix
- **Related:** [ADR: ACP Server with WebSocket Transport](./acp-server-websocket.md), [ADR: Separate Binaries with Opt-In Unified Build](./unified-binary.md), [ADR: Multi-Platform Adapters](./multi-platform-adapters.md)
- **Implementation:** ZER-944 (umbrella), ZER-945 (phase 1)

---

## 1. Context & Problem

openab-plus needs a production-grade coding-agent frontend: file tree, diff review, embedded terminal, conversation/session management, tool-call timeline. The existing `web/` admin UI (React + Vite + TanStack Query) is an operations console, not a coding workbench.

A survey of the open-source landscape produced one hard finding:

> No project satisfies all three of: production-grade UI, native ACP client, permissive license.

A contract adaptation layer is therefore unavoidable no matter which frontend we adopt. Given that, we picked the most complete UI and accepted the adaptation cost: [xintaofei/codeg](https://github.com/xintaofei/codeg) (Apache-2.0, Next.js 16 + React 19 + Tailwind 4 + monaco + xterm + tiptap, `output: 'export'` static export). openab-gateway remains the backend; we borrow only the frontend.

### Spike evidence (measured, not assumed)

A sandbox spike built Codeg to static output and served it behind a ~130-line fake backend that answered every `POST /api/*` with `{}` or `[]`:

- `pnpm build` succeeded; 28 routes all statically prerendered; `out/` ~75 MB
- The full workbench rendered with **zero frontend code changes** (30 interactive elements present in the accessibility tree)
- Cold start requested only **22 endpoints**, not the ~405 commands the client can theoretically issue
- Only visible defect: `Agent connection: Disconnected`, because the spike implemented the HTTP half only

Zero changes were needed because Codeg's transport derives its baseUrl from `window.location.origin`: whoever serves the bundle receives its calls.

### The actual problem

Codeg's wire shape is house-specific and unrelated to ACP:

- Flat RPC `POST /api/<command>` (not REST), returning bare JSON with no envelope
- Token in `localStorage['codeg_token']`, sent as `Authorization: Bearer`
- WebSocket at `/ws/events` with subprotocols `['codeg-events', 'codeg-token.<base64url>']`
- The server must emit `{channel: '__ready__'}` immediately on connect, or the client blocks on `subscribe()` forever
- 11 of the 22 cold-start endpoints back features we do not want (research, experts, office plugins, automations, update checks) and will return empty arrays permanently

This ADR answers one question: do those shapes belong in gateway core, in a separate middleware process, or somewhere else?

## 2. Decision

Add a single dedicated module in this repository, `crates/codeg-facade`, mounted onto the gateway's existing axum listener behind a default-off feature flag. Same process, same binary, **not a separate deployable**. Dependencies are one-way (`codeg-facade` -> gateway core); reverse dependencies are forbidden. Dropping the integration means deleting one directory.

```text
                    one process, one binary
+---------------------------------------------------------------+
| openab-gateway (axum listener)                                |
|                                                               |
|  GET   /  (static bundle)   --> codeg-facade  [feature: codeg]|
|  POST  /api/<command>       --> codeg-facade                  |
|  GET   /ws/events    (WS)   --> codeg-facade                  |
|                                     |                         |
|                                     | one-way calls           |
|                                     v                         |
|  *     /api/v1/*            --> gateway core --ACP--> agents  |
|  GET   /acp          (WS)   --> gateway core                  |
+---------------------------------------------------------------+
```

### The split line

> Capability that is semantically ours -> change gateway source, expose it under `/api/v1`.
> Shape that is merely Codeg's -> put it in the facade, never in core.

| Concern | Owner |
|---|---|
| 11 decorative cold-start endpoints (permanent empty arrays) | facade |
| `codeg_token` <-> admin Bearer auth bridge | facade |
| Flat `POST /api/<command>` routing and bare-JSON responses | facade |
| Static bundle hosting and fallback order | facade |
| `/ws/events` handshake, `codeg-events` subprotocol, `__ready__` frame | facade |
| attach/detach frames <-> our existing `sequence` cursor | facade |
| List sessions / list agents / agent skills / workspace files | gateway core, under `/api/v1` |
| ACP session lifecycle, event stream, permission arbitration | gateway core (already our job) |

### Both ends change

The forked frontend also changes, but only in its data layer: the transport module plus four scattered raw-`fetch` call sites (6 spots total, 3 of them needed for phase 1). All 415 `getTransport().call()` sites funnel through one place, only 7 files import `@tauri-apps/api/core`, and just 5 call `invoke()` directly, so the 635 files under `src/components` are never touched.

Each side changes only its own layer. Neither side absorbs the other's shape.

## 3. Codeg wire surface (owned by the facade)

**HTTP**

- `POST {origin}/api/{command}`, `Authorization: Bearer {token}`, body `JSON.stringify(args ?? {})`
- Success returns the payload directly, with no envelope
- Non-2xx throws `{code, message}`, falling back to `{code: 'network_error', message: 'HTTP <status>'}`
- 401 clears the token and redirects to `/login`; 5xx and network failures deliberately keep the token and enter the workspace
- Client call timeout 60s

**WebSocket**

- URL: origin with `http` swapped for `ws`, path `/ws/events`
- Subprotocols: `['codeg-events', 'codeg-token.' + base64url(token)]` (base64 with `+`->`-`, `/`->`_`, padding stripped)
- Server must send `{channel: '__ready__'}` on connect. This is a hard requirement, not a nicety
- Client frames: `{action: 'attach', subscription_id, connection_id, since_seq}`, `{action: 'detach', subscription_id}`
- Server frames: `snapshot`, `replay`, `event`, `detached`, `pong`
- Reconnect backoff: 1s initial, 32s cap

## 4. Phasing

| Phase | Scope | Tracking |
|---|---|---|
| 1 | Host the bundle, pass the login gate, answer all 22 cold-start endpoints with 200, open the WS and send `__ready__`. Usability explicitly out of scope | ZER-945 |
| 2 | Light up the connection: `acp_connect` / `acp_prompt` / `acp_cancel` / `acp_respond_permission` / `acp_get_session_snapshot`, plus attach/snapshot/replay/event frames. This means energising the `tool_call`, `session/request_permission`, `fs/*` and `terminal/*` families currently marked inert in `crates/openab-gateway/src/adapters/acp_schema.rs` | ZER-944 |
| 3 | Trim unwanted routes and components (pet, canvas, tasks, automations, forge, science, officecli, experts, project-boot, commit, merge, stash, push), converging the surface from ~405 commands to 90-120 | ZER-944 |
| 4 | Unify the contract on ACP v1 plus a `zds-ext-v1` extension namespace, with multi-client fan-out and permission-response arbitration | future ADR |

Phase 2 work is owed by the gateway regardless of which frontend we use; it is not a cost of borrowing Codeg's UI.

## 5. Alternatives Considered

### Option 1: Stitch the Codeg shape directly into gateway core

- Pros: fewest moving parts; no feature flag; no new crate
- Cons: carves another project's house rules into our foundation, including 11 endpoints that are permanently empty. Swapping frontends, syncing with upstream, or serving a native ACP client later turns into archaeology inside core

### Option 2: A separate middleware process translating Codeg <-> ACP

- Pros: strong isolation; independently deployable and replaceable
- Cons: one more deployable, bidirectional WebSocket proxying, two auth hops, an extra process in local development. The only thing bought is process-level isolation, and frontend and backend already share a trust domain and lifecycle. The facade crate already delivers code-level isolation at none of that cost

### Option 3: Rewrite paths at the edge (nginx) or in a service worker

- Pros: no backend work at all
- Cons: only rewrites paths, and the mismatch is not in paths. It is in frame structure and envelope semantics. Ineffective

### Option 4: Register openab-gateway as a custom ACP agent inside Codeg

- Pros: no facade at all; Codeg stays stock
- Cons: inverts ownership. Codeg becomes the platform and openab-plus becomes a plugin inside it, which is the opposite of the product intent

### Option 5: Vendor Codeg's `src-tauri/src/web/router.rs`

- Pros: instant contract compatibility
- Cons: 450 registered routes bound to Codeg's Tauri command layer. Measured and rejected: we would inherit its entire backend to serve 22 cold-start calls

### Option 6: Dedicated in-repo, in-process facade module (adopted)

- Pros: code-level isolation with single-binary operations; same-origin hosting needs no frontend address changes; deletable in one directory; keeps core ACP-pure
- Cons: requires discipline about the split line, and a feature flag to maintain

## 6. Consequences

- **Positive:** gateway core stays ACP-native and frontend-agnostic. Junk endpoints never reach core. Same-origin hosting keeps the frontend diff near zero, minimising merge conflicts against a fast-moving upstream. The integration is reversible by deletion
- **Negative:** two HTTP namespaces coexist in one listener (flat RPC and REST), which must be mounted so they cannot shadow each other. Contributors must know which side of the split line a change belongs to
- **Mitigation:** default-off feature flag; one-way dependency enforced at the crate boundary; the split line stated in this ADR and repeated in ZER-945's working rules

## 7. Open Questions

1. Does the facade expose `/ws/events` natively, or reuse `GET /acp` and change the single `new WebSocket` call site in the fork? Phase 1 assumes the former to keep the frontend diff at zero
2. Should the 11 decorative endpoints eventually 404 instead of returning empty arrays, once their entry points are trimmed in phase 3?
3. Where does `upload_attachment` (multipart, bypasses the transport) belong? Shape is Codeg's, but storage is ours
4. Is `crates/codeg-facade` the right name, or should it be generalised now that a second borrowed frontend is plausible?

## References

- ZER-944 (umbrella), ZER-945 (phase 1 breakdown with the endpoint tables)
- [xintaofei/codeg](https://github.com/xintaofei/codeg), Apache-2.0
- `crates/openab-gateway/src/adapters/acp_schema.rs`, generated from ACP schema v1.19.0
- [ADR: ACP Server with WebSocket Transport](./acp-server-websocket.md)
