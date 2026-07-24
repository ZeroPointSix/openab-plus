#!/usr/bin/env node
/**
 * Full local smoke for ZER-177 profile admin after merge.
 * Usage:
 *   BASE_URL=http://127.0.0.1:18080 TOKEN=test-admin node scripts/local-profile-full-test.mjs
 */
const BASE_URL = process.env.BASE_URL || "http://127.0.0.1:18080";
const TOKEN = process.env.TOKEN || process.env.GATEWAY_ADMIN_TOKEN || "test-admin";
const MODEL = process.env.TEST_MODEL || "deepseek/deepseek-v4-flash";
const API_BASE = process.env.TEST_API_BASE || "https://api.tokengo.com/v1";

function authHeaders(extra = {}) {
  return { Authorization: `Bearer ${TOKEN}`, ...extra };
}

async function request(path, options = {}) {
  const response = await fetch(`${BASE_URL}${path}`, {
    ...options,
    headers: authHeaders(options.headers || {}),
  });
  const text = await response.text();
  let payload = null;
  if (text) {
    try {
      payload = JSON.parse(text);
    } catch {
      payload = text;
    }
  }
  return { response, payload };
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function main() {
  const suffix = Date.now();
  const profileId = `local-full-${suffix}`;

  for (const asset of ["/admin", "/admin/profiles.js", "/admin/profiles.css"]) {
    const assetRes = await fetch(`${BASE_URL}${asset}`);
    assert(assetRes.ok, `asset ${asset} returned ${assetRes.status}`);
  }

  const profilesJs = await (await fetch(`${BASE_URL}/admin/profiles.js`)).text();
  for (const marker of ["profileListHtml", "ensureDynamicFieldsFresh", "profileNoticeHtml"]) {
    assert(profilesJs.includes(marker), `profiles.js missing ${marker}`);
  }

  const profile = {
    id: profileId,
    name: `Local Full Test ${suffix}`,
    agent_type: "codex",
    enabled: true,
    default_model: MODEL,
    reasoning_effort: "medium",
    env_refs: {
      OPENAI_API_KEY: "${OPENAI_API_KEY}",
      OPENAI_BASE_URL: "${OPENAI_BASE_URL}",
    },
    config_options: {
      approval_policy: "never",
      sandbox_mode: "workspace-write",
      network_access: "enabled",
    },
    timeout_secs: 300,
    recovery_strategy: "resume_session",
  };

  let result = await request("/api/v1/agent-profiles", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(profile),
  });
  assert(result.response.ok, `create failed: ${result.response.status} ${JSON.stringify(result.payload)}`);

  result = await request(`/api/v1/agent-profiles/${encodeURIComponent(profileId)}`);
  assert(result.response.ok, "get profile failed");
  assert(result.payload.default_model === MODEL, "default_model mismatch");
  assert(result.payload.env_refs?.OPENAI_API_KEY === "${OPENAI_API_KEY}", "env_refs mismatch");

  result = await request(`/api/v1/agents/${encodeURIComponent(profile.agent_type)}/config-schema`);
  assert(result.response.ok, "config-schema failed");
  assert(Array.isArray(result.payload.fields), "config-schema fields missing");
  assert(result.payload.fields.some((field) => field.id === "model"), "config-schema missing model");

  result = await request(`/api/v1/agent-profiles/${encodeURIComponent(profileId)}/validate`, { method: "POST" });
  assert(result.response.ok, "validate request failed");
  assert(result.payload.ok, `validate failed: ${JSON.stringify(result.payload.errors || [])}`);

  result = await request(`/api/v1/agent-profiles/default/${encodeURIComponent(profileId)}`, { method: "PUT" });
  assert(result.response.ok, "set default failed");

  result = await request("/api/v1/agent-profiles/default");
  assert(result.payload.default_profile === profileId, "default profile not set");

  result = await request(`/api/v1/agent-profiles/${encodeURIComponent(profileId)}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ ...profile, reasoning_effort: "high" }),
  });
  assert(result.response.ok, "update failed");

  result = await request(`/api/v1/agent-profiles/${encodeURIComponent(profileId)}`);
  assert(result.payload.reasoning_effort === "high", "reasoning_effort not updated");

  result = await request(`/api/v1/agent-profiles/${encodeURIComponent(profileId)}`, { method: "DELETE" });
  assert(result.response.ok, "delete failed");

  result = await request(`/api/v1/agent-profiles/${encodeURIComponent(profileId)}`);
  assert(result.response.status === 404, "deleted profile should 404");

  console.log(JSON.stringify({
    ok: true,
    profileId,
    model: MODEL,
    apiBase: API_BASE,
    checks: [
      "admin assets",
      "profiles.js markers",
      "profile create/get/update/delete",
      "config-schema",
      "validate",
      "default profile",
    ],
  }, null, 2));
}

main().catch((error) => {
  console.error(error.stack || error.message || String(error));
  process.exit(1);
});
