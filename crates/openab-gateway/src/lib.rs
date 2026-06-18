pub mod adapters;
pub(crate) mod media;
pub mod schema;
pub mod store;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, Mutex, Semaphore};

// --- Reply token cache for LINE hybrid Reply/Push dispatch ---

/// Cache entry for LINE reply tokens: (replyToken, insertion_time).
pub type ReplyTokenCache = Arc<std::sync::Mutex<HashMap<String, (String, Instant)>>>;

/// Maximum age (in seconds) before a cached reply token is considered expired.
pub const REPLY_TOKEN_TTL_SECS: u64 = 50;

/// Maximum number of cached reply tokens.
pub const REPLY_TOKEN_CACHE_MAX: usize = 10_000;

/// Maximum number of post-ack LINE webhook payloads processed concurrently.
pub const LINE_WEBHOOK_CONCURRENCY_MAX: usize = 8;

// --- App state (shared across all adapters) ---

pub struct AppState {
    pub telegram_bot_token: Option<String>,
    pub telegram_secret_token: Option<String>,
    pub telegram_rich_messages: bool,
    pub line_channel_secret: Option<String>,
    pub line_access_token: Option<String>,
    #[cfg(feature = "teams")]
    pub teams: Option<adapters::teams::TeamsAdapter>,
    pub teams_service_urls: Mutex<HashMap<String, (String, Instant)>>,
    #[cfg(feature = "feishu")]
    pub feishu: Option<adapters::feishu::FeishuAdapter>,
    #[cfg(feature = "googlechat")]
    pub google_chat: Option<adapters::googlechat::GoogleChatAdapter>,
    #[cfg(feature = "wecom")]
    pub wecom: Option<adapters::wecom::WecomAdapter>,
    pub ws_token: Option<String>,
    pub event_tx: broadcast::Sender<String>,
    pub reply_token_cache: ReplyTokenCache,
    pub line_webhook_semaphore: Arc<Semaphore>,
    pub client: reqwest::Client,
}
