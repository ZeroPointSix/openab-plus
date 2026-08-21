import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(
  resolve(scriptDir, "../web/src/pages/ProfilesPage.tsx"),
  "utf8",
);

assert(
  !source.includes("PERMISSION_CONFIG_FIELDS"),
  "profile UI must not inject hardcoded permission fields",
);
assert(
  !source.includes("approval_policy") && !source.includes("sandbox_mode"),
  "agent-specific options must come from the runtime schema",
);
assert.match(
  source,
  /adminApi\.profileSchema\(agentType as string\)/,
  "profile UI must request the selected agent's config schema",
);
assert.match(
  source,
  /queryKey: \['profileSchema', agentType\]/,
  "profile schema requests must be cached by agent type",
);
assert.match(
  source,
  /dynamicFields\.map\(fieldFor\)/,
  "profile form must render the remaining fields returned by the runtime schema",
);
assert.match(
  source,
  /field\.options\?\.length/,
  "runtime enum options must render as selects",
);
assert.match(
  source,
  /\['boolean', 'bool'\]\.includes\(field\.kind\)/,
  "runtime boolean fields must be supported",
);
assert.match(
  source,
  /\['number', 'integer'\]\.includes\(field\.kind\)/,
  "runtime numeric fields must be supported",
);
assert.match(
  source,
  /来源：\{schemaQuery\.data\?\.source\}/,
  "the active schema source must remain visible to administrators",
);
assert.match(
  source,
  /name="provider"/,
  "profile UI must expose a provider selector for ZER-568 v2",
);
assert.match(
  source,
  /服务商/,
  "profile UI must label the provider control in Chinese",
);
assert.match(
  source,
  /disabled: !supportedEfforts\.has\(value\)/,
  "unsupported thinking levels must be greyed out",
);
assert.match(
  source,
  /adminApi\.dryRunCliConfig/,
  "profile save must dry-run CLI config before writing profiles",
);
assert.match(
  source,
  /adminApi\.restoreCliConfig\('codex'\)/,
  "profile UI must offer one-click restore for Codex CLI config",
);
assert.match(
  source,
  /adminApi\.restoreCliConfig\('claude'\)/,
  "profile UI must offer one-click restore for Claude CLI config",
);
assert.match(
  source,
  /BUILTIN_MODELS/,
  "profile UI must keep a builtin model catalog fallback",
);

console.log("profile UI schema smoke passed");
