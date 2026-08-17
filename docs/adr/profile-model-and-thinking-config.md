# ADR: Profile Model and Thinking Level Configuration

- **Status:** Proposed
- **Date:** 2026-08-17
- **Author:** @guanshangshui
- **Linear:** ZER-568 (parent ZER-565)
- **Related:** ZER-707 (local daemon, host CLIs), ZER-139 (admin session entry), ZER-404 (session observation)

---

## 1. Context & Problem Statement

Users want the Admin web UI to work like the Cursor and Factory Droid web UI. The UI must show a model dropdown and a thinking-level control. The user makes a selection, and the next session starts with those values.

The OpenAB service is different from a desktop tool. The gateway is a daemon, and it runs many ACP sessions at the same time (`[pool] max_sessions = 10`). An ACP CLI reads its configuration one time, at process start. Therefore a live change of the model of a running session is not possible without a restart of the process.

The scope is small on purpose. The feature has two halves: **a backend configuration interface, and a frontend switch.** Section 2, D7 states what stays out.

### Current state

| Area | Current state | Gap |
|---|---|---|
| `POST /api/v1/sessions` | Accepts `CreateSessionOverrides { working_dir, model, reasoning_effort, config_options }` | The web client does not send these fields |
| `web/src/pages/ProfilesPage.tsx` | `default_model` and `reasoning_effort` are `ProFormText` free-text inputs | No dropdown, and no list of permitted values |
| `DynamicConfigFields` | Shows a select control when `AgentConfigField.options` is not empty | No component fills `options` for the model field |
| `AgentProfile` | Has `default_model`, `reasoning_effort`, `env_refs`, `config_options` | No provider identity, no base URL, and no API-key surface |
| `crates/openab-core/src/profile_store.rs` | One file `config/agent-profiles.toml`, atomic write, `.bak`, rollback | One file for all agent types, and no `schema_version` |
| `SessionSnapshot` | Returns `model`, `reasoning_effort`, `metadata_source` | The values are not a record of the applied configuration, so a restart can use different values |

### Requirement summary

1. One configuration file per agent type. The file is the source of truth.
2. A model dropdown and a thinking-level control in the Admin UI.
3. An API-key field per provider. The value is not readable after you save it.
4. A change applies to the next session. It does not change a running session.

---

## 2. Decision

| # | Decision |
|---|---|
| **D1** | Configuration files are the source of truth. Use one file per agent type: `config/profiles.d/<agent>.toml`. Add `config/providers.toml` for provider data. |
| **D2** | To apply a profile, write the configuration files of the CLI in the user home directory (`~/.codex/config.toml`, `~/.claude/settings.json`, and equivalent). This is the same method as cc-switch. A per-agent apply lock and an applied-configuration snapshot make the method safe for concurrent sessions. |
| **D3** | Use seven thinking levels: `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`. Map each level to a native value with a per-model table. A `null` value means that the model does not support the level. |
| **D4** | Build the provider layer in phase P0. The user gives the API key one time, and more than one agent can use it. |
| **D5** | P0 gives a renderer to `codex` and `claude` only. The other types in `COMMON_AGENT_TYPES` (`gemini`, `opencode`, `kiro`, `cursor`, `hermes`) fall back to argv and environment variables. |
| **D6** | P0 gives a blank provider form. A provider preset list is a P1 item. |
| **D7** | **Scope guard.** This feature is a configuration switch. It does not include a cost multiplier, a rate limit, a quota, usage metering, or billing data. We take the configuration mechanism of cc-switch, and we do not take its commercial surface. |

### D7 in detail

The reference projects hold features that a product needs, and that this feature does not need. The following items are out of scope, and a reviewer must reject a PR that adds them under this ADR:

| Out of scope | Reason |
|---|---|
| A cost multiplier per model (the `2x` and `0.56x` labels of the Factory UI) | Display of a commercial term. Not a configuration value. |
| A rate limit or a quota per provider | Requires metering and an enforcement path. A separate feature. |
| Usage counting, a token ledger, or a spend report | The same. |
| A provider health check with a latency ranking or automatic failover | A routing feature. Some cc-switch forks add this, and we do not. |
| An API-key pool with a rotation policy | The same. |

`POST /api/v1/providers/{id}/test` in P1 is not in this list. It sends one request to confirm that a credential works, and it stores nothing.

---

## 3. Prior Art & Industry Research

### 3.1 cc-switch (farion1231/cc-switch, 127.6k stars, Rust + Tauri + TypeScript)

cc-switch is the primary reference. It solves the same problem for a desktop user: select a provider and a model for Claude Code, Codex, Gemini, and other CLIs. We take its configuration mechanism and its file-write discipline. We do not take its commercial or routing features, per D7.

**Source of truth and apply step.** cc-switch keeps its own data in `~/.cc-switch/config.json` (`get_app_config_path()`). To make a provider active, it writes the configuration file of the target CLI (`~/.claude/settings.json`, or `~/.codex/config.toml` with `auth.json`). One applier module exists per CLI: `codex_config.rs` (210 KB), `hermes_config.rs` (89 KB), `claude_desktop_config.rs` (80 KB). **We adopt this two-level model: our profile store is the source of truth, and an applier writes the CLI file.**

**Write discipline (`src-tauri/src/config.rs`).** We adopt all of the following:

| Practice | Detail |
|---|---|
| `atomic_write` | Write `.tmp.<pid>.<ts>.<counter>`, then rename. On Windows use `ReplaceFileW` with a `fs::rename` fallback. Handle `ERROR_NOT_SUPPORTED` (50) for a WSL UNC path. On failure, delete the temporary file and keep the original file. |
| `atomic_write_private` | The same, and force Unix mode `0600`. cc-switch uses this function for every file that holds a credential. |
| `sort_json_keys` | Sort keys recursively for a byte-identical output. A test asserts that a different insertion order gives the same file. |
| Do not trust `$HOME` | Use `dirs::home_dir()`. The code comment states that a Git, Cygwin, or MSYS shell can inject `HOME`, and that the result looks like data loss to the user. |
| Test isolation | `CC_SWITCH_TEST_HOME` redirects all paths in a test. |
| Directory override | `get_claude_override_dir()` lets the user move the target directory. |
| Legacy fallback | A v3.10.3 path fallback keeps an old installation readable. |

**Model catalog (`src/config/piModelCatalog.ts`).** The key is `vendor/model`. The value holds `capabilities { name, reasoning, input, contextWindow, maxTokens }`. The helper `piModel(catalogKey, { id, thinkingProfile, ...overrides })` separates the logical model identity from the real model ID of one provider. **We adopt the identity split, because the same model has a different ID on a different gateway. We do not adopt the full capability record in P0.** Section 4.5 defines the reduced entry.

**Thinking levels (`src/config/piThinkingProfiles.ts`).** `PI_THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh", "max"]`. Each profile holds a map from a level to a native value:

```ts
deepseekV4:           { map: { minimal: null, low: null, medium: null, high: "high", max: "max" } }
geminiLowHigh:        { map: { low: "LOW", high: "HIGH" } }
openaiResponsesGpt56: { map: { off: "none", low: "low", medium: "medium", high: "high", xhigh: "xhigh", max: "max" } }
```

The value semantics are exact: `null` means that the model does not support the level, and the UI must disable it. A missing key means unknown. An empty object `{}` means "use the default of the model". A binding record `{ catalogKey, api, profileId, modelCompat? }` selects the profile, where `api` is `anthropic-messages`, `openai-responses`, or `google-generative-ai`. **The map is therefore keyed on (model x API protocol), not on the model alone.** We adopt the level list, the three-state value semantics, and the protocol dimension.

**Capability registry (`src-tauri/src/model_capabilities.rs`).** `ImageInputCapability` is `Supported`, `Unsupported`, or `Unknown`. The code separates `Unknown` from `Supported` on purpose. `is_confirmed_text_only_model` matches an exact suffix, not a prefix, because `glm-5.2` is text-only and `glm-5.2v` must stay image-capable. `normalize_model_id` removes a `models/` prefix, removes the `[1M]` context marker, lowercases the value, and takes the last segment of `vendor/model`. An unconfirmed variant always passes. **We adopt the fail-open rule and the exact-suffix match, because our own list must never block a new model.** The image and context fields themselves are not needed for a model switch.

**Provider presets (`src/config/universalProviderPresets.ts`).** `UniversalProviderPreset` holds `name`, `providerType`, `defaultApps { claude, codex, gemini }`, and `defaultModels { claude: { model, haikuModel, sonnetModel, opusModel }, codex: { model, reasoningEffort }, gemini: { model } }`. The signature `createUniversalProviderFromPreset(preset, id, baseUrl, apiKey, customName)` shows the contract: **a preset gives default values only, and the user gives the base URL and the API key.** We adopt this shape for `providers.toml`. Per D6, P0 ships no preset data, only the blank form.

**Where cc-switch does not fit.** cc-switch has exactly one active provider for the machine. Our gateway runs concurrent sessions. Section 4.7 defines the rules that make the same write method safe for us.

### 3.2 ccstatusline (sirmalloc/ccstatusline, 12.4k stars)

`src/types/Settings.ts` keeps `CURRENT_VERSION = 3`, keeps `SettingsSchema_v1` for migration, gives every field a `.default()`, and derives `DEFAULT_SETTINGS = SettingsSchema.parse({})`. **We adopt this versioned-schema-with-defaults pattern for `schema_version` in `profiles.d`.** `src/types/ClaudeSettings.ts` confirms that `effortLevel` is a first-class field of `~/.claude/settings.json`.

### 3.3 OpenClaw and Hermes Agent - required, not yet done

`docs/adr/pr-contribution-guidelines.md` requires research of OpenClaw and Hermes Agent for an architectural change. This ADR does not contain that research yet. The implementation PR must add:

- OpenClaw: how it stores a per-agent model and provider, and how it isolates a concurrent session.
- Hermes Agent: how its gateway holds provider credentials, and its precedence rules.

cc-switch is a closer match to this problem, because it is the only reference that writes the configuration file of a third-party CLI. The two mandatory references still must be documented before the code merges.

---

## 4. Design

### 4.1 File layout

```text
config/
├── config.toml              # existing gateway configuration
├── providers.toml           # new: provider records (base URL + credential reference)
└── profiles.d/              # new: one file per agent type
    ├── codex.toml
    ├── claude.toml
    └── cursor.toml
```

`config/agent-profiles.toml` stays readable. On the first load, the store copies each profile into `profiles.d/<agent_type>.toml`, and then renames the old file to `agent-profiles.toml.migrated`.

### 4.2 `profiles.d/<agent>.toml`

```toml
schema_version = 1
default_profile = "codex-high"

[[profiles]]
id = "codex-high"
name = "Codex - high effort"
agent_type = "codex"
enabled = true
command = "codex-acp"
provider = "newapi"
default_model = "openai/gpt-5.6-sol"   # catalog key, not the raw model ID
thinking = "high"                      # one of the seven levels
workdir_strategy = "profile_default"
working_dir = "/home/agent/work"
inherit_env = true

[profiles.config_options]
approval_policy = "never"
```

Notes:

- `default_model` holds a catalog key. The applier converts the key to the real model ID of the provider.
- `thinking` replaces the free-text `reasoning_effort` field. `reasoning_effort` stays an accepted alias for one release.
- The file never holds a secret. Only `providers.toml` refers to a credential.

### 4.3 `providers.toml`

```toml
schema_version = 1

[[providers]]
id = "newapi"
name = "NewAPI"
provider_type = "openai_compatible"
base_url = "https://api.example.com/v1"
api_key_ref = "exec://credstore backend.prod.newapi_key"

[providers.default_models]
codex = { model = "gpt-5.6-sol", thinking = "high" }
claude = { model = "claude-sonnet-5" }
```

The record holds four fields and a default-model map. It holds no rate limit, no cost multiplier, and no quota, per D7.

`api_key_ref` uses the reference syntax of `docs/secrets-management.md` (`exec://<script> <key>`, `aws-sm://<id>#<key>`, `${secrets.x}`). The gateway resolves the reference in memory, and it never writes the plaintext value to `providers.toml`. Resolution is fail-closed: if the reference does not resolve, the session does not start.

### 4.4 Thinking levels

```text
off < minimal < low < medium < high < xhigh < max
```

The gateway holds a map for each `(agent_type, model, api_format)` triple:

| Level | `openai-responses` (GPT-5.6) | `anthropic-messages` (Sonnet 5) | `google-generative-ai` | DeepSeek V4 |
|---|---|---|---|---|
| `off` | `none` | `null` | `null` | `null` |
| `minimal` | `minimal` | `1024` tokens | `null` | `null` |
| `low` | `low` | `4096` tokens | `LOW` | `null` |
| `medium` | `medium` | `8192` tokens | `null` | `null` |
| `high` | `high` | `16384` tokens | `HIGH` | `high` |
| `xhigh` | `xhigh` | `32768` tokens | `null` | `null` |
| `max` | `max` | `64000` tokens | `null` | `max` |

Rules:

- `null` means unsupported. `GET /api/v1/agents/{agent}/config-schema` marks the level as disabled, and the UI shows it greyed out.
- A missing entry means unknown. The gateway passes the level through and records a warning. It does not block the session (fail-open, per section 3.1).
- The token values in the Anthropic column are a proposal. Confirm them against the live ACP schema before merge. See section 9, item Q3.

### 4.5 Model list, in three levels

1. **Live ACP schema first.** If `pool.config_schema_for_agent` returns a model field with `options`, use those values. This is the true list of the installed CLI.
2. **Catalog fallback.** If the CLI gives no list, use the catalog.
3. **Free text always permitted.** The dropdown accepts a value that is not in the list. The gateway does not reject an unknown model.

A catalog entry holds three things only:

```text
catalog key      "openai/gpt-5.6-sol"       stable identity used by a profile
display name     "GPT-5.6 Sol"              shown in the dropdown
real model ID    per provider               sent to the CLI
thinking profile ID of the map in 4.4       decides which levels are enabled
```

The entry holds no context window, no image flag, no maximum tokens, and no price. A model switch does not need them. Add a field later only when a caller needs it.

### 4.6 Apply pipeline

```text
profiles.d/<agent>.toml  (source of truth)
        │
        ├── merge entry overrides   (POST /api/v1/sessions body)
        │     precedence: system default < default profile < named profile < entry override
        ▼
  resolve provider  ──────►  secrets.rs resolves api_key_ref in memory
        │
        ▼
  per-agent renderer  (crates/openab-gateway/src/adapters/<agent>.rs, new)
        │   catalog key   -> real model ID
        │   thinking level -> native value
        │   P0: codex and claude only (D5)
        ▼
  three injection channels, in this order of preference
        1. argv flag        (--model)            preferred, no file write
        2. environment var  (OPENAI_API_KEY)     preferred for a credential
        3. CLI config file  (~/.codex/config.toml)  only for a value with no flag and no variable
        ▼
  AcpConnection::spawn
        ▼
  record the applied values in SessionSnapshot
```

Only a value that has no flag and no environment variable reaches the file. This keeps the number of file writes low, and it keeps most sessions independent. An agent type with no renderer (D5) uses channel 1 and channel 2 only, so it never writes a file.

### 4.7 Concurrency and user-file safety rules

D2 writes a file that the user owns, and that other sessions read. These rules are mandatory.

| # | Rule | Reason |
|---|---|---|
| R1 | Hold a per-agent-type apply lock from the first file write until `spawn` returns. | Prevents session B from overwriting the file between the write of session A and the process start of session A. |
| R2 | Write the resolved values into `SessionSnapshot`. A restart (`recovery_strategy = restart_process`) re-applies the snapshot. It never reads the current file. | Without this rule, a restarted session silently gets the configuration of another session. |
| R3 | Merge. Do not replace. The applier owns a declared key set only, and it keeps every unknown key. | The user has their own settings in `~/.codex/config.toml`. |
| R4 | Copy the file to `<file>.openab.bak` before the first write of the process. Add `openab config restore-cli-config` to undo. | The user must be able to get their file back. |
| R5 | Use `atomic_write`, and `atomic_write_private` with mode `0600` for any file that holds a credential. Sort keys for a deterministic output. | Adopted from cc-switch, section 3.1. |
| R6 | Resolve the home directory with `dirs::home_dir()`. Do not read `$HOME`. Support `OPENAB_TEST_HOME` for tests. | Adopted from cc-switch. A wrong home directory looks like data loss. |
| R7 | If two live sessions of one agent type need a different file value, the UI shows a warning on the second session. The second session still starts. | Honest behaviour is better than a silent conflict. |
| R8 | Keep `[apply] target = "user_home"` in `config.toml`, and reserve the value `"isolated"`. `"isolated"` writes to `$OPENAB_HOME/agents/<profile_id>/`, and points `CODEX_HOME` or `CLAUDE_CONFIG_DIR` at that directory. | R7 is a compromise. If a conflict becomes common, we change the default without a schema change. |

### 4.8 API changes

| Endpoint | Change |
|---|---|
| `GET /api/v1/agents/{agent}/config-schema` | Add `options` and `disabled_options` to the model field. Add the thinking field with the seven levels, and the disabled set for the selected model. |
| `GET /api/v1/providers` | New. Returns every provider. The API key is masked, for example `sk-...4f2a`. |
| `PUT /api/v1/providers/{id}` | New. The `api_key` field is write-only. The response never returns the value. |
| `DELETE /api/v1/providers/{id}` | New. Rejected while a profile still refers to the provider. |
| `POST /api/v1/providers/{id}/test` | P1, optional. Sends one cheap request to `base_url` to confirm that the credential works. Stores no result. |
| `POST /api/v1/sessions` | No shape change. The web client starts to send `model`, `reasoning_effort`, and `config_options`. |
| `GET /api/v1/sessions/{id}` | `SessionSnapshot` gains `applied_provider`, `applied_model_id`, and `applied_thinking`. `metadata_source` stays. |
| `GET/PUT /api/v1/agent-profiles` | No shape change. The store reads and writes `profiles.d`. |

`apply_policy` stays `new_session` for the model field and the thinking field. A field is `runtime` only when the ACP schema sets `apply_after_start = true`.

### 4.9 Web UI

- `ProfilesPage.tsx`: replace the `default_model` `ProFormText` with a searchable select. Replace `reasoning_effort` with a seven-step segmented control. A disabled level shows a reason on hover.
- New `ProvidersPage.tsx`: a list of providers, and a blank form with name, base URL, and API key (D6). The key field shows the masked value and a Replace action. No preset picker in P0.
- `SessionWorkbenchPage.tsx` and `NewAgentWizard.tsx`: add a model control and a thinking control to the session-start form, and send the values as overrides.
- Session header: show the applied provider, model, and thinking level from the snapshot.

---

## 5. Phases

| Phase | Scope | Acceptance |
|---|---|---|
| **P0** | `profiles.d` split with migration and `schema_version`; `providers.toml` with `api_key_ref`; seven-level thinking with the map; model dropdown from the live schema with a catalog fallback; provider CRUD API and a blank provider form; renderers for `codex` and `claude` only; R1 to R6. | A user selects a provider, a model, and a thinking level in the UI, starts a session, and the snapshot shows the same values. Two concurrent sessions with a different profile both start, and each one uses its own values. An agent type with no renderer still starts, and it uses argv and environment values. |
| **P1** | Provider preset list; `POST /api/v1/providers/{id}/test`; `openab config restore-cli-config`; a wider catalog with the fail-open normaliser; the R7 conflict warning. | A wrong API key gives a clear message before the session starts. A preset fills the base URL and the default models, and the user adds only the key. |
| **P2** | `[apply] target = "isolated"`; the ZER-707 path that pushes a profile change to a live session, which needs a restart of the ACP process. | A profile change reaches a live session, and the transcript records the restart. |

---

## 6. Why This Approach

- **A file, not a database.** The team already runs `profile_store.rs` with an atomic write, a `.bak` file, and a rollback. A file is reviewable, and it works with a Git workflow.
- **One file per agent type.** The user asked for this. It also removes a class of merge conflict, because a change to the Codex profile never touches the Claude file.
- **Write the file of the CLI.** The user chose this method instead of an isolated directory. It matches cc-switch, and the behaviour is easy to inspect: the user opens `~/.codex/config.toml` and sees the result. Section 4.7 pays the cost of this choice.
- **A provider layer in P0.** An API key per profile means that the user types the same key many times. The cc-switch preset contract shows that a preset, a base URL, and a key are enough.
- **Two renderers, not seven.** `codex` and `claude` are the two agent types that need a file write. The other five have a flag or a variable for the same setting, so a renderer for them adds code with no gain.
- **A small record, not a catalog product.** D7 keeps the feature at the size of the requirement: a backend configuration interface, and a frontend switch.

---

## 7. Alternatives Considered

| Option | Verdict |
|---|---|
| **A1** An isolated configuration directory per profile, with `CODEX_HOME` pointed at it | Rejected for P0, kept as R8. It removes every rule of section 4.7, and it never touches a file that the user owns, but it loses the visible cc-switch behaviour that the owner asked for. |
| **A2** argv and environment variables only, with no file write | Rejected as a general rule, and adopted for the five agent types with no renderer (D5). It fails for `codex`, because Codex reads `approval_policy` from `config.toml` only. |
| **A3** A live switch inside a running session | Out of scope, and ZER-568 states this. An ACP CLI reads its configuration at start, so a live switch needs a process restart. That work is the P2 item that ZER-707 tracks. |
| **A4** A free-text model field, which is the current state | Rejected. The user asked for a dropdown, and a typing error currently fails only when the session starts. |
| **A5** A full model catalog with capability flags, prices, and multipliers, as cc-switch and the Factory UI show | Rejected by D7. It turns a configuration switch into a catalogue product, and it needs data that we cannot keep current. |

---

## 8. Risks and Mitigations

| Risk | Severity | Mitigation |
|---|---|---|
| We overwrite a setting that the user wrote by hand in `~/.codex/config.toml`. | High | R3 merge, R4 backup, and a restore command. |
| Two sessions of one agent type need a different file value. | Medium | R1 lock and R2 snapshot remove the corruption. R7 makes the remaining ambiguity visible. R8 is the exit. |
| Our thinking map becomes stale when a provider adds a level. | Medium | Fail-open for an unknown key, and the live ACP schema has priority over the catalog. |
| An API key leaks into a log or a configuration file. | High | Store a reference only, resolve in memory, `0600` on any credential file, mask the value in every response, and keep the existing sanitised-environment rule. |
| The migration from `agent-profiles.toml` loses a profile. | High | Copy, do not move. Keep the old file as `.migrated`, and add a round-trip test. |
| Scope grows into routing, metering, or billing. | Medium | D7 lists the rejected items, so a reviewer has a written rule. |

---

## 9. Questions

### Resolved (2026-08-17, by the issue owner)

| # | Question | Answer |
|---|---|---|
| Q1 | Which agent types get a renderer in P0? | `codex` and `claude` only. The other five fall back to argv and environment variables. Now D5. |
| Q2 | Does P0 ship a provider preset list? | No. P0 ships a blank form, and the preset list is a P1 item. Now D6. |
| Q4 | Does `providers.toml` need a rate limit or a cost multiplier? | No. The feature is a configuration switch, and it takes no commercial feature from the reference projects. Now D7. |

### Remaining

| # | Question | Owner |
|---|---|---|
| Q3 | Is the Anthropic column of section 4.4 correct, or does the ACP layer accept a level name directly? Check `crates/openab-core/src/acp/protocol.rs` and the upstream `claude-code-acp` schema. | Implementation PR 1 |
| Q5 | Does the OpenClaw and Hermes Agent research (section 3.3) change any rule of section 4.7? | Implementation PR 1 |

---

## Consequences

- **Positive:** The Admin UI reaches parity with the Cursor and Factory model picker for the part that matters, which is the switch itself. A key is entered one time. A profile change is predictable, because it applies at the next session.
- **Negative:** The gateway now writes a file that the user owns. This adds the apply lock, the snapshot, the merge, and the backup work. A concurrent conflict is still possible, and R7 only reports it.
- **Mitigation:** R8 keeps the isolated-directory design one configuration value away.

## References

- Linear ZER-568, ZER-565, ZER-707, ZER-139, ZER-404
- [farion1231/cc-switch](https://github.com/farion1231/cc-switch): `src-tauri/src/config.rs`, `src-tauri/src/model_capabilities.rs`, `src/config/piModelCatalog.ts`, `src/config/piThinkingProfiles.ts`, `src/config/universalProviderPresets.ts`
- [sirmalloc/ccstatusline](https://github.com/sirmalloc/ccstatusline): `src/types/Settings.ts`, `src/types/ClaudeSettings.ts`
- `docs/secrets-management.md`, `docs/adr/pr-contribution-guidelines.md`, `docs/review-contract.md`
- `crates/openab-core/src/profile_store.rs`, `crates/openab-gateway/src/agent_profile_admin.rs`, `crates/openab-gateway/src/session_admin.rs`
