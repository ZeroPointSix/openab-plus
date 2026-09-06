#!/usr/bin/env node

import fs from "node:fs"
import readline from "node:readline"

const logPath = process.argv[2] || "/tmp/openab-codeg-e2e-agent.log"
const sessionId = "codeg-e2e-acp-session"
let pendingPromptId = null

function trace(event) {
  fs.appendFileSync(logPath, `${JSON.stringify(event)}\n`, { encoding: "utf8" })
}

function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`)
}

function update(payload) {
  send({
    jsonrpc: "2.0",
    method: "session/update",
    params: { sessionId, update: payload },
  })
}

function promptText(prompt) {
  if (!Array.isArray(prompt)) return ""
  return prompt
    .filter((block) => block && block.type === "text")
    .map((block) => String(block.text || ""))
    .join("\n")
}

const pause = (milliseconds) =>
  new Promise((resolve) => setTimeout(resolve, milliseconds))

async function handle(request) {
  const method = request?.method
  const requestId = request?.id
  const params = request?.params || {}

  if (method === "initialize") {
    send({
      jsonrpc: "2.0",
      id: requestId,
      result: {
        agentInfo: { name: "openab-codeg-e2e-agent" },
        agentCapabilities: { loadSession: false },
      },
    })
    return
  }

  if (method === "session/new") {
    trace({ method })
    send({ jsonrpc: "2.0", id: requestId, result: { sessionId } })
    return
  }

  if (method === "session/prompt") {
    const text = promptText(params.prompt)
    trace({ method, text })
    if (text === "block") {
      pendingPromptId = requestId
      update({
        sessionUpdate: "agent_thought_chunk",
        content: { type: "text", text: "waiting for cancellation" },
      })
      return
    }

    update({
      sessionUpdate: "agent_thought_chunk",
      content: { type: "text", text: "checking context" },
    })
    await pause(800)
    update({
      sessionUpdate: "tool_call",
      toolCallId: "tool-1",
      title: "Inspect workspace",
      status: "running",
      rawInput: { path: "Cargo.toml" },
    })
    await pause(800)
    update({
      sessionUpdate: "tool_call_update",
      toolCallId: "tool-1",
      title: "Inspect workspace",
      status: "completed",
      content: [{ type: "text", text: "workspace inspected" }],
    })
    await pause(800)
    update({
      sessionUpdate: "agent_message_chunk",
      content: { type: "text", text: "control-plane reply" },
    })
    send({
      jsonrpc: "2.0",
      id: requestId,
      result: {
        stopReason: "end_turn",
        usage: { inputTokens: 1, outputTokens: 1, totalTokens: 2 },
      },
    })
    return
  }

  if (method === "session/cancel") {
    trace({ method })
    if (pendingPromptId !== null) {
      send({
        jsonrpc: "2.0",
        id: pendingPromptId,
        result: { stopReason: "cancelled" },
      })
      pendingPromptId = null
    }
  }
}

const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity })
for await (const line of lines) {
  if (!line.trim()) continue
  await handle(JSON.parse(line))
}
