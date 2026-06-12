//! AgentCore ACP bridge — stdin/stdout subprocess that bridges ACP JSON-RPC
//! to AgentCore's InvokeAgentRuntimeCommandShell WebSocket API.
//!
//! Invoked as: `openab --agentcore-bridge --runtime-arn ARN --region REGION`
//!
//! Opens a persistent PTY shell in the microVM, launches `kiro-cli acp
//! --trust-all-tools`, and forwards JSON-RPC bidirectionally.

use anyhow::{anyhow, Result};
use aws_credential_types::provider::ProvideCredentials;
use aws_sigv4::http_request::{sign, SignableBody, SignableRequest, SigningSettings};
use aws_sigv4::sign::v4;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::http;
use tokio_tungstenite::tungstenite::protocol::Message;
use tracing::{debug, error, info, warn};

const KIRO_ACP_CMD: &str = "stty -echo 2>/dev/null; exec kiro-cli acp --trust-all-tools\n";

/// WebSocket binary frame channel bytes (1-byte prefix protocol).
const CHANNEL_STDIN: u8 = 0x00;
const CHANNEL_STDOUT: u8 = 0x01;
const CHANNEL_STDERR: u8 = 0x02;

/// Extract a complete JSON object from a line that may have PTY prefix noise.
/// Uses brace-counting to find matching `{}` pairs, robust against partial JSON
/// or embedded `{` in prompt text.
fn extract_json_object(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let mut start = None;
    for i in 0..bytes.len() {
        if bytes[i] == b'{' {
            start = Some(i);
            break;
        }
    }
    let start = start?;

    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;

    for i in start..bytes.len() {
        let c = bytes[i];
        if escape {
            escape = false;
            continue;
        }
        if c == b'\\' && in_string {
            escape = true;
            continue;
        }
        if c == b'"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
            if depth == 0 {
                let candidate = &line[start..=i];
                // Validate it's actually valid JSON
                if serde_json::from_str::<Value>(candidate).is_ok() {
                    return Some(candidate.to_string());
                }
                // Not valid — try next `{`
                return extract_json_object(&line[start + 1..]);
            }
        }
    }
    None
}

/// Entry point for the agentcore bridge subprocess.
pub async fn run_bridge(runtime_arn: &str, region: &str) -> Result<()> {
    let stdin = BufReader::new(tokio::io::stdin());
    let stdout = tokio::io::stdout();

    let mut bridge = Bridge::new(runtime_arn, region, stdin, stdout);
    bridge.run().await
}

struct Bridge<R, W> {
    runtime_arn: String,
    region: String,
    stdin: R,
    stdout: W,
    sessions: HashMap<String, ShellHandle>,
    next_id: u64,
}

struct ShellHandle {
    /// Sender for writing to the WebSocket (stdin of shell)
    ws_write: Arc<Mutex<WsSink>>,
    /// Buffered output from kiro-cli (stdout of shell via WebSocket)
    line_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    /// Pump task handle
    _pump: tokio::task::JoinHandle<()>,
    /// Runtime session ID used for this shell
    runtime_session_id: String,
}

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

impl<R, W> Bridge<R, W>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    fn new(runtime_arn: &str, region: &str, stdin: R, stdout: W) -> Self {
        Self {
            runtime_arn: runtime_arn.to_string(),
            region: region.to_string(),
            stdin,
            stdout,
            sessions: HashMap::new(),
            next_id: 1000,
        }
    }

    fn alloc_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    async fn write_msg(&mut self, msg: &Value) -> Result<()> {
        let data = serde_json::to_string(msg)?;
        self.stdout.write_all(data.as_bytes()).await?;
        self.stdout.write_all(b"\n").await?;
        self.stdout.flush().await?;
        Ok(())
    }

    async fn write_response(&mut self, id: &Value, result: Value) -> Result<()> {
        self.write_msg(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
            .await
    }

    async fn write_error(&mut self, id: &Value, code: i32, message: &str) -> Result<()> {
        self.write_msg(
            &json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        )
        .await
    }

    async fn run(&mut self) -> Result<()> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = self.stdin.read_line(&mut line).await?;
            if n == 0 {
                break; // EOF
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let msg: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
            let id = msg.get("id").cloned().unwrap_or(Value::Null);
            let params = msg.get("params").cloned().unwrap_or(json!({}));

            // Skip messages without a method (e.g. stray responses) — same fix as Python F1
            if method.is_empty() {
                continue;
            }

            match method {
                "initialize" => {
                    self.write_response(
                        &id,
                        json!({
                            "protocolVersion": 1,
                            "agentInfo": {"name": "agentcore-shell-bridge", "version": "0.2.0"},
                            "agentCapabilities": {"loadSession": true}
                        }),
                    )
                    .await?;
                }
                "session/new" => {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    let acp_sid = format!("agentcore-{ts}");
                    self.write_response(&id, json!({"sessionId": acp_sid}))
                        .await?;
                }
                "session/load" => {
                    let acp_sid = params
                        .get("sessionId")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    self.write_response(&id, json!({"sessionId": acp_sid}))
                        .await?;
                }
                "session/prompt" => {
                    self.handle_prompt(&id, &params).await?;
                }
                "session/cancel" | "cancel" => {
                    self.handle_cancel(&params).await;
                }
                "session/destroy" | "session/stop" => {
                    let acp_sid = params
                        .get("sessionId")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    self.sessions.remove(&acp_sid);
                    if id != Value::Null {
                        self.write_response(&id, json!({})).await?;
                    }
                }
                "session/request_permission" => {
                    if id != Value::Null {
                        self.write_response(&id, json!({"approved": true})).await?;
                    }
                }
                _ => {
                    if id != Value::Null {
                        self.write_error(&id, -32601, &format!("unknown method: {method}"))
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn handle_prompt(&mut self, id: &Value, params: &Value) -> Result<()> {
        let acp_sid = params
            .get("sessionId")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        // Ensure shell is connected
        if !self.sessions.contains_key(&acp_sid) {
            let runtime_sid = self.derive_runtime_session_id(params);
            match self.open_shell(&runtime_sid).await {
                Ok(handle) => {
                    self.sessions.insert(acp_sid.clone(), handle);
                }
                Err(e) => {
                    self.write_error(id, -32000, &format!("shell connect failed: {e}"))
                        .await?;
                    return Ok(());
                }
            }
        }

        let shell = self.sessions.get_mut(&acp_sid).unwrap();

        // Forward prompt to kiro-cli as JSON-RPC
        let kiro_id = self.alloc_id();
        let kiro_msg = json!({
            "jsonrpc": "2.0",
            "id": kiro_id,
            "method": "session/prompt",
            "params": params,
        });
        let data = format!("{}\n", serde_json::to_string(&kiro_msg)?);
        {
            let mut w = shell.ws_write.lock().await;
            let mut frame = Vec::with_capacity(1 + data.len());
            frame.push(CHANNEL_STDIN);
            frame.extend_from_slice(data.as_bytes());
            let _ = w.send(Message::Binary(frame.into())).await;
        }

        // Read responses/notifications from kiro-cli until we get the response for our id
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(300), shell.line_rx.recv())
                .await
            {
                Ok(Some(line)) => {
                    let msg: Value = match serde_json::from_str(&line) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    // Check if it's a response to our request
                    if msg.get("id").and_then(|i| i.as_u64()) == Some(kiro_id) {
                        // Forward as our response
                        if let Some(err) = msg.get("error") {
                            self.write_msg(&json!({"jsonrpc": "2.0", "id": id, "error": err}))
                                .await?;
                        } else {
                            let result = msg
                                .get("result")
                                .cloned()
                                .unwrap_or(json!({"type": "success"}));
                            self.write_response(id, result).await?;
                        }
                        break;
                    }
                    // It's a notification — forward directly to OAB
                    if msg.get("method").is_some() {
                        self.write_msg(&msg).await?;
                    }
                }
                Ok(None) => {
                    self.write_error(id, -32000, "shell connection closed")
                        .await?;
                    self.sessions.remove(&acp_sid);
                    break;
                }
                Err(_) => {
                    self.write_error(id, -32000, "timeout waiting for agent response")
                        .await?;
                    break;
                }
            }
        }
        Ok(())
    }

    async fn handle_cancel(&mut self, params: &Value) {
        let acp_sid = params
            .get("sessionId")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        if let Some(shell) = self.sessions.get(&acp_sid.to_string()) {
            let cancel_msg = json!({
                "jsonrpc": "2.0",
                "method": "session/cancel",
                "params": params,
            });
            let data = format!(
                "{}\n",
                serde_json::to_string(&cancel_msg).unwrap_or_default()
            );
            let mut frame = Vec::with_capacity(1 + data.len());
            frame.push(CHANNEL_STDIN);
            frame.extend_from_slice(data.as_bytes());
            let mut w = shell.ws_write.lock().await;
            let _ = w.send(Message::Binary(frame.into())).await;
        }
    }

    fn derive_runtime_session_id(&self, params: &Value) -> String {
        // Try to extract from sender_context in prompt blocks
        if let Some(blocks) = params.get("prompt").and_then(|p| p.as_array()) {
            for block in blocks {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    if let Some(start) = text.find("<sender_context>") {
                        if let Some(end) = text.find("</sender_context>") {
                            let ctx_str = &text[start + 16..end];
                            if let Ok(ctx) = serde_json::from_str::<Value>(ctx_str.trim()) {
                                let platform = ctx
                                    .get("channel")
                                    .and_then(|c| c.as_str())
                                    .unwrap_or("unknown");
                                let thread_id = ctx
                                    .get("thread_id")
                                    .or_else(|| ctx.get("channel_id"))
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("");
                                let mut sid = format!("oab-{platform}-thread-{thread_id}");
                                while sid.len() < 33 {
                                    sid.push('0');
                                }
                                return sid;
                            }
                        }
                    }
                }
            }
        }
        // Fallback
        let mut sid = format!("oab-fallback-{}", uuid::Uuid::new_v4());
        while sid.len() < 33 {
            sid.push('0');
        }
        sid
    }

    async fn open_shell(&self, session_id: &str) -> Result<ShellHandle> {
        let request = build_signed_request(&self.runtime_arn, session_id, &self.region).await?;

        let (ws_stream, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| anyhow!("WebSocket connect failed: {e}"))?;

        info!(session_id, "AgentCore shell connected");

        let (ws_write, mut ws_read) = ws_stream.split();
        let ws_write = Arc::new(Mutex::new(ws_write));

        // Send kiro-cli launch command
        {
            let mut frame = Vec::with_capacity(1 + KIRO_ACP_CMD.len());
            frame.push(CHANNEL_STDIN);
            frame.extend_from_slice(KIRO_ACP_CMD.as_bytes());
            let mut w = ws_write.lock().await;
            w.send(Message::Binary(frame.into()))
                .await
                .map_err(|e| anyhow!("failed to send launch cmd: {e}"))?;
        }

        // Channel for forwarding parsed JSON-RPC lines
        let (line_tx, line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        // Spawn reader pump
        let pump = tokio::spawn(async move {
            let mut buf = String::new();
            while let Some(Ok(msg)) = ws_read.next().await {
                match msg {
                    Message::Binary(data) => {
                        if data.len() < 2 {
                            continue;
                        }
                        if data[0] == CHANNEL_STDOUT {
                            // stdout
                            if let Ok(s) = std::str::from_utf8(&data[1..]) {
                                buf.push_str(s);
                                while let Some(nl) = buf.find('\n') {
                                    let line = buf[..nl].to_string();
                                    buf = buf[nl + 1..].to_string();
                                    let trimmed = line.trim().to_string();
                                    if trimmed.is_empty() {
                                        continue;
                                    }
                                    // Extract JSON object using brace-counting (handles PTY prefix noise)
                                    if let Some(json_str) = extract_json_object(&trimmed) {
                                        if line_tx.send(json_str).is_err() {
                                            return; // receiver dropped — exit pump
                                        }
                                    }
                                }
                            }
                        } else if data[0] == CHANNEL_STDERR {
                            // stderr — log
                            if let Ok(s) = std::str::from_utf8(&data[1..]) {
                                let t = s.trim();
                                if !t.is_empty() {
                                    eprintln!("[agentcore] {t}");
                                }
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        });

        Ok(ShellHandle {
            ws_write,
            line_rx,
            _pump: pump,
            runtime_session_id: session_id.to_string(),
        })
    }
}

/// Build a WebSocket upgrade request with SigV4 Authorization header.
async fn build_signed_request(
    arn: &str,
    session_id: &str,
    region: &str,
) -> Result<http::Request<()>> {
    let config = aws_config::from_env()
        .region(aws_config::Region::new(region.to_string()))
        .load()
        .await;

    let creds = config
        .credentials_provider()
        .ok_or_else(|| anyhow!("No AWS credentials found"))?
        .provide_credentials()
        .await
        .map_err(|e| anyhow!("Failed to get credentials: {e}"))?;

    let identity = creds.into();

    let encoded_arn = urlencoding::encode(arn);
    let host = format!("bedrock-agentcore.{region}.amazonaws.com");
    let path = format!("/runtimes/{encoded_arn}/ws/shells");

    // Deterministic shell_id from session_id
    let hash = Sha256::digest(session_id.as_bytes());
    let shell_id = format!("oab-{}", hex::encode(&hash[..8]));

    let query = format!(
        "qualifier=DEFAULT&runtimeSessionId={}&shellId={shell_id}",
        urlencoding::encode(session_id),
    );

    let uri = format!("https://{host}{path}?{query}");

    let mut settings = SigningSettings::default();
    settings.expires_in = None;

    let signing_params = v4::SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name("bedrock-agentcore")
        .time(SystemTime::now())
        .settings(settings)
        .build()?;

    let headers = [
        ("host", host.as_str()),
        ("x-amzn-bedrock-agentcore-runtime-session-id", session_id),
    ];

    let signable = SignableRequest::new("GET", &uri, headers.into_iter(), SignableBody::empty())?;

    let (instructions, _sig) = sign(signable, &signing_params.into())?.into_parts();

    let wss_uri = format!("wss://{host}{path}?{query}");
    let mut builder = http::Request::builder()
        .method("GET")
        .uri(&wss_uri)
        .header("host", &host)
        .header("x-amzn-bedrock-agentcore-runtime-session-id", session_id)
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header(
            "sec-websocket-key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        );

    for (name, value) in instructions.headers() {
        builder = builder.header(name, value);
    }

    Ok(builder.body(())?)
}
