#!/usr/bin/env python3
"""agentcore-acp: ACP↔WebSocket bridge for Amazon Bedrock AgentCore Runtime.

Bridges OAB's ACP JSON-RPC protocol to AgentCore's InvokeAgentRuntimeCommandShell
WebSocket API.  Opens a persistent PTY shell in the microVM and runs kiro-cli in
native ACP mode, forwarding JSON-RPC messages bidirectionally.

Usage:
    agentcore-acp --runtime-arn ARN --region REGION
"""

import argparse
import asyncio
import json
import re
import sys
import hashlib

from bedrock_agentcore.runtime import AgentCoreRuntimeClient, ReconnectConfig, ShellChannel

# ---------------------------------------------------------------------------
# ACP stdio helpers
# ---------------------------------------------------------------------------


def write_stdout(msg: dict):
    """Write a JSON-RPC message to stdout (toward OAB)."""
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


def write_response(id, result=None, error=None):
    msg = {"jsonrpc": "2.0", "id": id}
    if error is not None:
        msg["error"] = error
    else:
        msg["result"] = result if result is not None else {}
    write_stdout(msg)


def write_notification(method: str, params: dict):
    msg = {"jsonrpc": "2.0", "method": method, "params": params}
    write_stdout(msg)


def emit_text_chunk(text: str):
    write_notification("session/update", {
        "update": {
            "sessionUpdate": "agent_message_chunk",
            "content": {"text": text},
        }
    })


# ---------------------------------------------------------------------------
# Sender context parsing
# ---------------------------------------------------------------------------

SENDER_CTX_RE = re.compile(
    r"<sender_context>\s*(.*?)\s*</sender_context>", re.DOTALL
)


def _extract_json_object(text: str) -> dict | None:
    start = text.find("{")
    if start == -1:
        return None
    depth = 0
    in_string = False
    escape = False
    for i in range(start, len(text)):
        c = text[i]
        if escape:
            escape = False
            continue
        if c == "\\":
            escape = True
            continue
        if c == '"' and not escape:
            in_string = not in_string
            continue
        if in_string:
            continue
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                try:
                    return json.loads(text[start : i + 1])
                except json.JSONDecodeError:
                    return None
    return None


def extract_session_id_from_prompt(blocks: list) -> str | None:
    """Parse <sender_context> to build deterministic session ID (≥33 chars)."""
    for block in blocks:
        if isinstance(block, dict) and block.get("type") == "text":
            m = SENDER_CTX_RE.search(block.get("text", ""))
            if m:
                ctx = _extract_json_object(m.group(1))
                if ctx:
                    platform = ctx.get("channel", "unknown")
                    thread_id = ctx.get("thread_id") or ctx.get("channel_id", "")
                    sid = f"oab-{platform}-thread-{thread_id}"
                    if len(sid) < 33:
                        sid = sid + "0" * (33 - len(sid))
                    return sid
    return None


def derive_shell_id(session_id: str) -> str:
    """Deterministic shell_id from session_id (1-128 chars, alphanumeric/underscore/hyphen)."""
    h = hashlib.sha256(session_id.encode()).hexdigest()[:16]
    return f"acp-{h}"


def strip_sender_context(blocks: list) -> str:
    parts = []
    for block in blocks:
        if isinstance(block, dict) and block.get("type") == "text":
            text = block.get("text", "")
            text = SENDER_CTX_RE.sub("", text).strip()
            if text:
                parts.append(text)
    return "\n".join(parts)


# ---------------------------------------------------------------------------
# Shell session manager
# ---------------------------------------------------------------------------

KIRO_ACP_CMD = "kiro-cli acp --trust-all-tools\n"
# Marker we look for to confirm kiro-cli ACP mode is ready
ACP_READY_MARKER = '"jsonrpc"'


class ShellSession:
    """Manages a persistent shell connection to a microVM running kiro-cli ACP."""

    def __init__(self, client: AgentCoreRuntimeClient, runtime_arn: str,
                 session_id: str, shell_id: str):
        self.client = client
        self.runtime_arn = runtime_arn
        self.session_id = session_id
        self.shell_id = shell_id
        self.shell = None
        self._reader_task: asyncio.Task | None = None
        self._line_buffer = ""
        self._response_queue: asyncio.Queue = asyncio.Queue()
        self._ready = asyncio.Event()
        self._initialized = False

    async def connect(self):
        """Open shell and start kiro-cli ACP if not already running."""
        reconnect_config = ReconnectConfig(max_retries=3, base_delay=1.0)
        self.shell = await self.client.open_shell(
            self.runtime_arn,
            session_id=self.session_id,
            shell_id=self.shell_id,
            reconnect_config=reconnect_config,
        ).__aenter__()

        # Start reader task
        self._reader_task = asyncio.create_task(self._read_loop())

        if not self.shell.reconnected:
            # Fresh shell — launch kiro-cli in ACP mode
            await self.shell.send(KIRO_ACP_CMD)
            # Wait for kiro-cli to be ready (first JSON-RPC output)
            try:
                await asyncio.wait_for(self._ready.wait(), timeout=30.0)
            except asyncio.TimeoutError:
                pass  # proceed anyway — kiro-cli may output differently
        else:
            # Reconnected — kiro-cli should still be running
            self._ready.set()
        self._initialized = True

    async def _read_loop(self):
        """Read frames from shell and route JSON-RPC messages."""
        try:
            async for frame in self.shell:
                if frame.channel == ShellChannel.STDOUT:
                    self._line_buffer += frame.text
                    while "\n" in self._line_buffer:
                        line, self._line_buffer = self._line_buffer.split("\n", 1)
                        line = line.strip()
                        if not line:
                            continue
                        # Try to parse as JSON-RPC
                        try:
                            msg = json.loads(line)
                        except json.JSONDecodeError:
                            continue  # skip non-JSON PTY output (prompts, ANSI, etc.)

                        if not self._ready.is_set():
                            self._ready.set()

                        # Route: if it has "id" it's a response; if "method" it's a notification
                        if "id" in msg and "method" not in msg:
                            await self._response_queue.put(msg)
                        elif "method" in msg:
                            # Forward notification to OAB stdout
                            write_stdout(msg)
        except Exception:
            pass  # connection closed

    async def send_and_wait(self, msg: dict, timeout: float = 300.0) -> dict | None:
        """Send a JSON-RPC request to kiro-cli and wait for the response."""
        data = json.dumps(msg) + "\n"
        await self.shell.send(data)

        if msg.get("id") is None:
            return None  # notification, no response expected

        # Wait for response with matching id
        try:
            while True:
                resp = await asyncio.wait_for(self._response_queue.get(), timeout=timeout)
                if resp.get("id") == msg.get("id"):
                    return resp
                # Not our response — forward it (shouldn't happen in practice)
                write_stdout(resp)
        except asyncio.TimeoutError:
            return None

    async def send_notification(self, msg: dict):
        """Send a JSON-RPC notification (no response expected)."""
        data = json.dumps(msg) + "\n"
        await self.shell.send(data)

    async def close(self):
        if self._reader_task:
            self._reader_task.cancel()
        if self.shell:
            try:
                await self.shell.__aexit__(None, None, None)
            except Exception:
                pass


# ---------------------------------------------------------------------------
# ACP Adapter (async)
# ---------------------------------------------------------------------------


class AcpAdapter:
    def __init__(self, runtime_arn: str, region: str):
        self.runtime_arn = runtime_arn
        self.region = region
        self.client = AgentCoreRuntimeClient(region=region)
        self.sessions: dict[str, ShellSession] = {}  # acp_session_id → ShellSession
        self._next_id = 1000  # internal JSON-RPC ids for kiro-cli

    def _alloc_id(self) -> int:
        self._next_id += 1
        return self._next_id

    async def _get_or_create_shell(self, acp_sid: str, blocks: list) -> ShellSession:
        """Get existing shell session or create a new one."""
        if acp_sid in self.sessions:
            session = self.sessions[acp_sid]
            if session._initialized:
                return session

        # Derive runtime session_id and shell_id from prompt context
        runtime_sid = extract_session_id_from_prompt(blocks)
        if not runtime_sid:
            runtime_sid = f"oab-fallback-{acp_sid}"
            if len(runtime_sid) < 33:
                runtime_sid = runtime_sid + "0" * (33 - len(runtime_sid))
        shell_id = derive_shell_id(runtime_sid)

        session = ShellSession(self.client, self.runtime_arn, runtime_sid, shell_id)
        self.sessions[acp_sid] = session
        await session.connect()
        return session

    async def handle_session_new(self, id, params: dict):
        import time
        acp_sid = f"agentcore-{int(time.time() * 1000)}"
        self.sessions[acp_sid] = None  # placeholder until first prompt
        write_response(id, {"sessionId": acp_sid})

    async def handle_session_prompt(self, id, params: dict):
        acp_sid = params.get("sessionId", "")
        blocks = params.get("prompt", [])

        # Cold-start notification
        cold_start_task = None
        if acp_sid not in self.sessions or self.sessions[acp_sid] is None:
            async def _cold_start():
                await asyncio.sleep(3.0)
                emit_text_chunk("⏳ Starting agent environment...")
            cold_start_task = asyncio.create_task(_cold_start())

        try:
            session = await self._get_or_create_shell(acp_sid, blocks)
        except Exception as e:
            if cold_start_task:
                cold_start_task.cancel()
            write_response(id, error={"code": -32000, "message": f"shell connect failed: {e}"})
            return

        if cold_start_task:
            cold_start_task.cancel()

        # Forward prompt to kiro-cli as session/prompt
        kiro_id = self._alloc_id()
        kiro_msg = {
            "jsonrpc": "2.0",
            "id": kiro_id,
            "method": "session/prompt",
            "params": params,
        }
        resp = await session.send_and_wait(kiro_msg)

        if resp is None:
            write_response(id, error={"code": -32000, "message": "timeout waiting for agent response"})
        elif "error" in resp:
            write_response(id, error=resp["error"])
        else:
            write_response(id, resp.get("result", {"type": "success"}))

    async def handle_session_load(self, id, params: dict):
        acp_sid = params.get("sessionId", "")
        # Just store — actual connection happens on first prompt
        if acp_sid not in self.sessions:
            self.sessions[acp_sid] = None
        write_response(id, {"sessionId": acp_sid})

    async def handle_cancel(self, params: dict):
        acp_sid = params.get("sessionId", "")
        session = self.sessions.get(acp_sid)
        if session and session._initialized:
            await session.send_notification({
                "jsonrpc": "2.0",
                "method": "session/cancel",
                "params": params,
            })

    async def handle_request_permission(self, id, params: dict):
        write_response(id, {"approved": True})

    async def run(self):
        """Main loop: read JSON-RPC from stdin, dispatch."""
        loop = asyncio.get_event_loop()
        reader = asyncio.StreamReader()
        protocol = asyncio.StreamReaderProtocol(reader)
        await loop.connect_read_pipe(lambda: protocol, sys.stdin)

        while True:
            line = await reader.readline()
            if not line:
                break
            line = line.decode().strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue

            method = msg.get("method")
            params = msg.get("params", {})
            msg_id = msg.get("id")

            if method == "session/new":
                await self.handle_session_new(msg_id, params)
            elif method == "session/prompt":
                await self.handle_session_prompt(msg_id, params)
            elif method in ("session/cancel", "cancel"):
                await self.handle_cancel(params)
            elif method == "session/load":
                await self.handle_session_load(msg_id, params)
            elif method == "session/request_permission":
                if msg_id is not None:
                    await self.handle_request_permission(msg_id, params)
            elif msg_id is not None:
                write_response(msg_id, error={"code": -32601, "message": f"unknown method: {method}"})


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(description="ACP↔WebSocket bridge for AgentCore Runtime")
    parser.add_argument("--runtime-arn", required=True, help="AgentCore Runtime ARN")
    parser.add_argument("--region", default="us-east-1", help="AWS region")
    parser.add_argument(
        "--cancel-strategy",
        choices=["noop", "stop"],
        default="stop",
        help="(ignored, kept for CLI compat)",
    )
    args = parser.parse_args()

    adapter = AcpAdapter(runtime_arn=args.runtime_arn, region=args.region)
    asyncio.run(adapter.run())


if __name__ == "__main__":
    main()
