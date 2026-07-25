import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(resolve(scriptDir, "../web/profiles.js"), "utf8");

assert(!source.includes("PERMISSION_CONFIG_FIELDS"), "profile UI must not inject hardcoded permission fields");

const context = {
  console,
  apiMaybe: async () => null,
  clearTimeout,
  setTimeout,
  window: { clearTimeout, setTimeout, confirm: () => true },
  escapeHtml(value) {
    return String(value ?? "").replace(/[&<>"']/g, (char) => ({
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      '"': "&quot;",
      "'": "&#39;",
    }[char]));
  },
};

vm.createContext(context);
vm.runInContext(source, context, { filename: "web/profiles.js" });

function fieldById(fields, id) {
  return fields.find((field) => field.id === id);
}

const profile = { default_model: "", reasoning_effort: "", config_options: {} };

const liveSchema = {
  source: "agent-session-config-options",
  fields: [
    { id: "approval_policy", label: "Approval policy", kind: "enum", options: ["live-approval"] },
    { id: "sandbox_mode", label: "Sandbox mode", kind: "enum", options: ["live-sandbox"] },
  ],
};

const liveFields = context.profileConfigFields(liveSchema, profile);
assert.deepEqual(fieldById(liveFields, "approval_policy")?.options, ["live-approval"]);
assert.deepEqual(fieldById(liveFields, "sandbox_mode")?.options, ["live-sandbox"]);
assert.equal(fieldById(liveFields, "network_access"), undefined);
assert.match(context.schemaSubtitle(liveSchema, "codex"), /agent-session-config-options/);

const fallbackSchema = {
  source: "profile-store-fallback",
  fields: [
    { id: "network_access", label: "Network access", kind: "enum", options: ["fallback-disabled", "fallback-enabled"] },
  ],
};

const fallbackFields = context.profileConfigFields(fallbackSchema, profile);
assert.deepEqual(fieldById(fallbackFields, "network_access")?.options, ["fallback-disabled", "fallback-enabled"]);
assert.equal(fieldById(fallbackFields, "approval_policy"), undefined);
assert.match(context.schemaSubtitle(fallbackSchema, "opencode"), /profile-store-fallback/);

const unavailableFields = context.profileConfigFields(null, profile);
assert.ok(fieldById(unavailableFields, "model"));
assert.ok(fieldById(unavailableFields, "reasoning_effort"));
assert.equal(fieldById(unavailableFields, "approval_policy"), undefined);
assert.match(context.schemaSubtitle(null, "codex"), /未获取到配置 schema/);

const schemaCalls = [];
context.apiMaybe = async (path) => {
  schemaCalls.push(path);
  return { source: `schema-call-${schemaCalls.length}`, fields: [] };
};

const cachedSchema = await context.fetchProfileSchema("codex");
const reusedSchema = await context.fetchProfileSchema("codex");
assert.equal(schemaCalls.length, 1);
assert.equal(reusedSchema, cachedSchema);

const refreshedSchema = await context.fetchProfileSchema("codex", { force: true });
assert.equal(schemaCalls.length, 2);
assert.equal(refreshedSchema.source, "schema-call-2");

console.log("profile UI schema smoke passed");
