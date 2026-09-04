#!/usr/bin/env node

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const fixtureUrl = new URL(
  "../docs/fixtures/codeg-openab-minimum-contract.v1.json",
  import.meta.url,
);
const fixture = JSON.parse(await readFile(fixtureUrl, "utf8"));

const expectedEndpoints = new Map([
  ["GET /api/v1/sessions", 200],
  ["POST /api/v1/sessions", 201],
  ["GET /api/v1/sessions/{session_id}", 200],
  ["GET /api/v1/sessions/{session_id}/transcript", 200],
  ["GET /api/v1/sessions/events", 200],
  ["POST /api/v1/sessions/{session_id}/messages", 202],
  ["POST /api/v1/sessions/{session_id}/cancel", 204],
]);

assert.equal(fixture.contract, "openab.codeg.session.v1");
assert.equal(fixture.fixture_version, 1);
assert.equal(fixture.endpoints.length, 7, "契约必须恰好包含 7 个接口");

const actualEndpoints = new Map(
  fixture.endpoints.map((endpoint) => [
    `${endpoint.method} ${endpoint.path}`,
    endpoint.success_status,
  ]),
);
assert.deepEqual(actualEndpoints, expectedEndpoints, "7 个接口或成功状态码发生漂移");

for (const endpoint of fixture.endpoints) {
  assert.ok(endpoint.path.startsWith("/api/v1/"), `${endpoint.id} 必须属于 /api/v1`);
  assert.ok(!endpoint.path.includes("?"), `${endpoint.id} 不得在路径中携带 token/query`);
  assert.ok(endpoint.follow_up.startsWith("ZER-"), `${endpoint.id} 缺少后续 issue`);
}

assert.deepEqual(
  fixture.auth,
  {
    required: true,
    header: "Authorization",
    scheme: "Bearer",
    fetch_sse: true,
    query_token: false,
    token: "fixture-admin-token",
  },
  "鉴权边界发生漂移",
);

const allCases = [
  ...Object.values(fixture.write_cases.messages),
  ...Object.values(fixture.write_cases.cancel),
];
for (const testCase of allCases) {
  assert.equal(testCase.request.method, "POST");
  assert.equal(
    testCase.request.headers.Authorization,
    "Bearer fixture-admin-token",
    "写接口必须使用 Authorization Bearer",
  );
  assert.ok(!testCase.request.path.includes("?"), "token 不得进入写接口 URL");
  if (testCase.response.body !== null) {
    assert.deepEqual(
      Object.keys(testCase.response.body),
      ["error"],
      "错误响应必须保持最小 {error} 形状",
    );
  }
}

const messages = fixture.write_cases.messages;
assert.deepEqual(Object.keys(messages.accepted.request.body), ["text"]);
assert.ok(messages.accepted.request.body.text.trim().length > 0);
assert.equal(messages.accepted.response.status, 202);
assert.equal(messages.accepted.response.body, null, "202 响应不得重复返回最终文本");
assert.equal(messages.empty_text.request.body.text.trim(), "");
assert.equal(messages.empty_text.response.status, 400);
assert.equal(messages.missing_session.response.status, 404);
assert.equal(messages.busy.response.status, 409, "并发发送必须返回明确的 conflict");
assert.equal(messages.pre_accept_failure.response.status, 500);
assert.equal(messages.agent_error_after_accept.response.status, 202);
assert.equal(messages.agent_error_after_accept.response.body, null);

function validateSequencedEvents(events, expectedSessionId) {
  let previous = -1;
  for (const frame of events) {
    assert.ok(frame.data.sequence > previous, "SSE 全局 sequence 必须严格递增");
    previous = frame.data.sequence;
    const [generation, sequence] = frame.id.split(":");
    assert.match(generation, /^[0-9a-f]{32}$/);
    assert.equal(Number(sequence), frame.data.sequence, "SSE id 必须匹配 data.sequence");
    if (frame.event === "transcript") {
      assert.equal(frame.data.session_id, expectedSessionId);
      assert.ok(frame.data.entry.entry_id);
      assert.ok(Number.isInteger(frame.data.entry.sequence));
    } else {
      assert.equal(frame.data.event, frame.event);
      assert.equal(frame.data.snapshot.session_id, expectedSessionId);
    }
  }
}

const acceptedEvents = messages.accepted.sse_result;
validateSequencedEvents(acceptedEvents, fixture.session.id);

const transcriptEntries = acceptedEvents
  .filter((frame) => frame.event === "transcript")
  .map((frame) => frame.data.entry);
assert.ok(
  transcriptEntries.some(
    (entry) =>
      entry.role === "user" &&
      entry.status === "completed" &&
      entry.content === messages.accepted.request.body.text,
  ),
  "SSE 必须包含被接受的 user 文本",
);
assert.ok(
  transcriptEntries.some(
    (entry) => entry.role === "assistant" && entry.status === "thinking",
  ),
  "fixture 必须覆盖 thinking",
);

const toolEntries = transcriptEntries.filter((entry) => entry.role === "tool");
assert.equal(toolEntries.length, 2, "fixture 必须覆盖 tool upsert");
assert.equal(toolEntries[0].entry_id, toolEntries[1].entry_id);
assert.equal(toolEntries[0].tool_call_id, toolEntries[1].tool_call_id);
assert.equal(toolEntries[0].status, "in_progress");
assert.equal(toolEntries[1].status, "completed");
assert.ok(toolEntries[1].tool_result);

const assistantText = transcriptEntries.filter(
  (entry) => entry.role === "assistant" && entry.status !== "thinking",
);
assert.equal(assistantText.length, 2, "fixture 必须覆盖 assistant 流式 upsert");
assert.equal(assistantText[0].entry_id, assistantText[1].entry_id);
assert.equal(assistantText[0].status, "streaming");
assert.equal(assistantText[1].status, "completed");

const agentErrorEvents = messages.agent_error_after_accept.sse_result;
validateSequencedEvents(agentErrorEvents, fixture.session.id);
assert.ok(
  agentErrorEvents.some(
    (frame) =>
      frame.event === "error" &&
      frame.data.snapshot.status === "error" &&
      frame.data.snapshot.last_error,
  ),
  "202 后的 Agent 错误必须通过 lifecycle error SSE 呈现",
);

const cancel = fixture.write_cases.cancel;
for (const state of ["running", "idle"]) {
  assert.equal(cancel[state].request.body, null, "cancel 请求不得要求 JSON 正文");
  assert.equal(cancel[state].response.status, 204, "cancel 必须对运行中和空闲 session 幂等");
  assert.equal(cancel[state].response.body, null);
  assert.equal(cancel[state].sse_result.dedicated_cancel_event, false);
  assert.deepEqual(cancel[state].sse_result.guaranteed_events, []);
}
assert.deepEqual(cancel.running.response, cancel.idle.response);
assert.equal(cancel.missing_session.response.status, 404);
assert.equal(cancel.send_failure.response.status, 500);
validateSequencedEvents(cancel.running.sse_result.possible_follow_up, fixture.session.id);

const cursorReset = fixture.recovery.cursor_reset;
assert.equal(cursorReset.event, "cursor_reset");
assert.equal(cursorReset.id, `${cursorReset.data.current_generation}:0`);
assert.equal(
  cursorReset.data.action,
  "refetch /api/v1/sessions before continuing the stream",
);
assert.equal(fixture.recovery.history_unavailable.event, "error");
assert.equal(fixture.recovery.receiver_lagged.event, "error");

const requiredUnsupported = [
  "attachments",
  "terminal",
  "fs",
  "permission",
  "git",
  "persistent_transcript",
  "cross_machine_migration",
  "remote_runtime",
];
for (const capability of requiredUnsupported) {
  assert.ok(fixture.unsupported.includes(capability), `缺少 unsupported: ${capability}`);
}

console.log(
  `Codeg/OpenAB contract fixture valid: ${fixture.endpoints.length} endpoints, ` +
    `${acceptedEvents.length} accepted-turn SSE frames, cancel is idempotent.`,
);
