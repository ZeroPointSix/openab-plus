#!/usr/bin/env node
/**
 * Smoke test for ZER-176 session admin API + SSE contract.
 * Usage: BASE_URL=http://127.0.0.1:18080 TOKEN=test-admin node scripts/session-admin-smoke.mjs
 */
const BASE_URL = process.env.BASE_URL || "http://127.0.0.1:18080";
const TOKEN = process.env.TOKEN || process.env.GATEWAY_ADMIN_TOKEN || "test-admin";

function authHeaders() {
  return { Authorization: `Bearer ${TOKEN}` };
}

function parseSSEChunk(chunk) {
  const lines = chunk.split(/\r?\n/);
  const dataLines = [];
  let event = "";
  for (const line of lines) {
    if (line.startsWith("event:")) event = line.slice(6).trim();
    if (line.startsWith("data:")) dataLines.push(line.slice(5).trimStart());
  }
  return { event, data: dataLines.join("\n") };
}

function consumeSSEBuffer(buffer) {
  const normalized = buffer.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const chunks = normalized.split("\n\n");
  const rest = chunks.pop() || "";
  const events = [];
  for (const chunk of chunks) {
    const parsed = parseSSEChunk(chunk);
    if (parsed.data) {
      let payload = parsed.data;
      try {
        payload = JSON.parse(parsed.data);
      } catch {
        // keep raw
      }
      events.push({ event: parsed.event || "message", payload });
    }
  }
  return { events, rest };
}

async function fetchSessions() {
  const response = await fetch(`${BASE_URL}/api/v1/sessions`, { headers: authHeaders() });
  if (!response.ok) {
    throw new Error(`GET /api/v1/sessions failed: ${response.status}`);
  }
  const data = await response.json();
  return Array.isArray(data) ? data : data.sessions || [];
}

async function waitForSSEOpen({ timeoutMs = 5000 } = {}) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(`${BASE_URL}/api/v1/sessions/events`, {
      headers: authHeaders(),
      signal: controller.signal,
    });
    if (!response.ok) {
      throw new Error(`SSE connect failed: ${response.status}`);
    }
    const contentType = response.headers.get("content-type") || "";
    if (!contentType.includes("text/event-stream")) {
      throw new Error(`unexpected SSE content-type: ${contentType}`);
    }
  } finally {
    clearTimeout(timer);
    controller.abort();
  }
}

async function waitForSSEEvent({ timeoutMs = 8000, predicate }) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(`${BASE_URL}/api/v1/sessions/events`, {
      headers: authHeaders(),
      signal: controller.signal,
    });
    if (!response.ok) {
      throw new Error(`SSE connect failed: ${response.status}`);
    }
    if (!response.body) {
      throw new Error("SSE response has no body");
    }

    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const { events, rest } = consumeSSEBuffer(buffer);
      buffer = rest;
      for (const entry of events) {
        if (!predicate || predicate(entry)) return entry;
      }
    }
    throw new Error("SSE stream ended before expected event");
  } finally {
    clearTimeout(timer);
    controller.abort();
  }
}

async function main() {
  const health = await fetch(`${BASE_URL}/health`);
  if (!health.ok) throw new Error(`health check failed: ${health.status}`);
  console.log("✓ /health");

  const sessions = await fetchSessions();
  console.log(`✓ GET /api/v1/sessions (${sessions.length} sessions)`);

  await waitForSSEOpen();
  console.log("✓ SSE /api/v1/sessions/events connected");

  if (process.env.WAIT_FOR_EVENT === "1") {
    const event = await waitForSSEEvent({
      timeoutMs: 12000,
      predicate: (entry) => entry.payload?.snapshot || entry.payload?.error,
    });
    console.log(`✓ SSE event: ${event.event}`);
  }

  const after = await fetchSessions();
  console.log(`✓ resync GET /api/v1/sessions (${after.length} sessions)`);
  console.log("session-admin smoke passed");
}

main().catch((error) => {
  console.error(`session-admin smoke failed: ${error.message}`);
  process.exit(1);
});
