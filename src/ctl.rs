//! `openab set/get` IPC over Unix domain socket.
//!
//! Architecture (like consul/vault):
//! - `openab run` spawns a UnixListener at a well-known path.
//! - `openab set key value` connects, sends a JSON request, reads the response.
//!
//! Phase 1 supported keys:
//! - `thread.name` — rename the current Discord/Slack thread

use crate::adapter::{ChannelRef, ChatAdapter};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, error, info, warn};

/// Default socket path. Overridable via `OPENAB_SOCK` env var.
pub fn socket_path() -> PathBuf {
    std::env::var("OPENAB_SOCK")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/openab.sock"))
}

// ─── Protocol ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub action: Action,
    pub key: String,
    pub value: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Set,
    Get,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

// ─── Server (runs inside `openab run`) ──────────────────────────────────────

/// Handler trait — `openab run` provides the concrete implementation that
/// can access Discord/Slack adapters.
#[async_trait::async_trait]
pub trait CtlHandler: Send + Sync + 'static {
    async fn handle_set(&self, key: &str, value: &str) -> Response;
    async fn handle_get(&self, key: &str) -> Response;
}

/// Start the control socket server. Call this from `openab run` startup.
/// Returns a JoinHandle; abort it on shutdown.
pub fn spawn_server(
    handler: std::sync::Arc<dyn CtlHandler>,
) -> tokio::task::JoinHandle<()> {
    let path = socket_path();
    tokio::spawn(async move {
        // Remove stale socket file
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => l,
            Err(e) => {
                error!(path = %path.display(), error = %e, "failed to bind control socket");
                return;
            }
        };
        info!(path = %path.display(), "control socket listening");

        loop {
            let (stream, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    warn!(error = %e, "control socket accept error");
                    continue;
                }
            };
            let handler = handler.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_conn(stream, &*handler).await {
                    debug!(error = %e, "control socket connection error");
                }
            });
        }
    })
}

async fn handle_conn(
    stream: UnixStream,
    handler: &dyn CtlHandler,
) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    if let Some(line) = lines.next_line().await? {
        let req: Request = serde_json::from_str(&line)?;
        let resp = match req.action {
            Action::Set => {
                let val = req.value.as_deref().unwrap_or("");
                handler.handle_set(&req.key, val).await
            }
            Action::Get => handler.handle_get(&req.key).await,
        };
        let mut buf = serde_json::to_vec(&resp)?;
        buf.push(b'\n');
        writer.write_all(&buf).await?;
    }
    Ok(())
}

// ─── Client (used by `openab set/get` subcommands) ──────────────────────────

/// Concrete handler for `openab run` — dispatches to platform adapters.
pub struct RuntimeHandler {
    adapter: Arc<dyn ChatAdapter>,
    shard: Arc<std::sync::OnceLock<serenity::gateway::ShardMessenger>>,
}

impl RuntimeHandler {
    pub fn new(
        adapter: Arc<dyn ChatAdapter>,
        shard: Arc<std::sync::OnceLock<serenity::gateway::ShardMessenger>>,
    ) -> Self {
        Self { adapter, shard }
    }

    fn channel_ref() -> Option<ChannelRef> {
        let channel_id = std::env::var("OPENAB_THREAD_ID")
            .or_else(|_| std::env::var("OPENAB_CHANNEL_ID"))
            .ok()?;
        let platform = std::env::var("OPENAB_PLATFORM").unwrap_or_else(|_| "discord".into());
        Some(ChannelRef {
            platform,
            channel_id,
            thread_id: None,
            parent_id: None,
            origin_event_id: None,
        })
    }
}

#[async_trait::async_trait]
impl CtlHandler for RuntimeHandler {
    async fn handle_set(&self, key: &str, value: &str) -> Response {
        match key {
            "thread.name" => {
                let Some(channel) = Self::channel_ref() else {
                    return Response {
                        ok: false,
                        message: "OPENAB_THREAD_ID or OPENAB_CHANNEL_ID not set".into(),
                        value: None,
                    };
                };
                match self.adapter.rename_thread(&channel, value).await {
                    Ok(()) => Response {
                        ok: true,
                        message: format!("thread renamed to: {value}"),
                        value: None,
                    },
                    Err(e) => Response {
                        ok: false,
                        message: format!("rename failed: {e}"),
                        value: None,
                    },
                }
            }
            "thread.archived" => {
                let Some(channel) = Self::channel_ref() else {
                    return Response {
                        ok: false,
                        message: "OPENAB_THREAD_ID or OPENAB_CHANNEL_ID not set".into(),
                        value: None,
                    };
                };
                let archived = match value {
                    "true" | "1" | "yes" => true,
                    "false" | "0" | "no" => false,
                    _ => {
                        return Response {
                            ok: false,
                            message: format!("invalid value: {value} (expected true/false)"),
                            value: None,
                        };
                    }
                };
                match self.adapter.archive_thread(&channel, archived).await {
                    Ok(()) => Response {
                        ok: true,
                        message: format!(
                            "thread {}",
                            if archived { "archived" } else { "unarchived" }
                        ),
                        value: None,
                    },
                    Err(e) => Response {
                        ok: false,
                        message: format!("archive failed: {e}"),
                        value: None,
                    },
                }
            }
            "agent.status" => {
                let Some(shard) = self.shard.get() else {
                    return Response {
                        ok: false,
                        message: "shard not ready (Discord not connected yet)".into(),
                        value: None,
                    };
                };
                use serenity::gateway::ActivityData;
                use serenity::model::user::OnlineStatus;
                let activity = if value.is_empty() {
                    None
                } else {
                    Some(ActivityData::custom(value))
                };
                shard.set_presence(activity, OnlineStatus::Online);
                Response {
                    ok: true,
                    message: if value.is_empty() {
                        "status cleared".into()
                    } else {
                        format!("status set to: {value}")
                    },
                    value: None,
                }
            }
            _ => Response {
                ok: false,
                message: format!("unknown key: {key}"),
                value: None,
            },
        }
    }

    async fn handle_get(&self, key: &str) -> Response {
        match key {
            "thread.name" | "thread.archived" | "agent.status" => Response {
                ok: false,
                message: format!("{key} get not yet supported"),
                value: None,
            },
            _ => Response {
                ok: false,
                message: format!("unknown key: {key}"),
                value: None,
            },
        }
    }
}

pub async fn send_request(req: &Request) -> anyhow::Result<Response> {
    let path = socket_path();
    let stream = UnixStream::connect(&path).await.map_err(|e| {
        anyhow::anyhow!(
            "cannot connect to openab at {}: {} (is `openab run` running?)",
            path.display(),
            e
        )
    })?;
    let (reader, mut writer) = stream.into_split();
    let mut buf = serde_json::to_vec(req)?;
    buf.push(b'\n');
    writer.write_all(&buf).await?;
    writer.shutdown().await?;

    let mut lines = BufReader::new(reader).lines();
    let line = lines
        .next_line()
        .await?
        .ok_or_else(|| anyhow::anyhow!("no response from openab"))?;
    let resp: Response = serde_json::from_str(&line)?;
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serialization() {
        let req = Request {
            action: Action::Set,
            key: "thread.name".into(),
            value: Some("hello".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.action, Action::Set);
        assert_eq!(parsed.key, "thread.name");
        assert_eq!(parsed.value.as_deref(), Some("hello"));
    }

    #[tokio::test]
    async fn server_client_roundtrip() {
        struct MockHandler;
        #[async_trait::async_trait]
        impl CtlHandler for MockHandler {
            async fn handle_set(&self, key: &str, value: &str) -> Response {
                Response {
                    ok: true,
                    message: format!("{key} = {value}"),
                    value: None,
                }
            }
            async fn handle_get(&self, key: &str) -> Response {
                Response {
                    ok: true,
                    message: String::new(),
                    value: Some(format!("val-of-{key}")),
                }
            }
        }

        // Use a temp path to avoid conflicts
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("test.sock");
        std::env::set_var("OPENAB_SOCK", sock.to_str().unwrap());

        let handler = std::sync::Arc::new(MockHandler);
        let server = spawn_server(handler);
        // Give server a moment to bind
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Test set
        let resp = send_request(&Request {
            action: Action::Set,
            key: "thread.name".into(),
            value: Some("hello world".into()),
        })
        .await
        .unwrap();
        assert!(resp.ok);
        assert_eq!(resp.message, "thread.name = hello world");

        // Test get
        let resp = send_request(&Request {
            action: Action::Get,
            key: "thread.name".into(),
            value: None,
        })
        .await
        .unwrap();
        assert!(resp.ok);
        assert_eq!(resp.value.as_deref(), Some("val-of-thread.name"));

        server.abort();
        std::env::remove_var("OPENAB_SOCK");
    }
}
