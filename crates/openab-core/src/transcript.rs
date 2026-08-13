use crate::session_event::SessionStreamBus;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::env;
use std::sync::{Arc, Mutex};

pub const DEFAULT_TRANSCRIPT_CAPACITY: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptEntry {
    /// Stable per-session identity. Every revision of a streamed assistant or
    /// tool entry keeps this value so a client can upsert rather than append.
    pub entry_id: String,
    /// A per-session mutation sequence. A merged text entry or tool upsert gets
    /// a fresh sequence so snapshot `after` requests can replay its latest form.
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub role: TranscriptRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone)]
struct TranscriptEntryDraft {
    role: TranscriptRole,
    content: Option<String>,
    tool_call: Option<Value>,
    tool_result: Option<Value>,
    tool_call_id: Option<String>,
    status: Option<String>,
}

/// Complete information for one ACP tool_call or tool_call_update notification.
/// `payload` is intentionally not normalized: ACP agents use agent-specific
/// fields for raw input, output, terminal output, and file diffs.
#[derive(Debug, Clone)]
pub struct ToolTranscriptUpdate {
    pub tool_call_id: String,
    pub title: String,
    pub status: String,
    pub completed: bool,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptEvent {
    /// The global sequence used by the shared `/sessions/events` SSE cursor.
    pub sequence: u64,
    pub session_id: String,
    pub entry: TranscriptEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptSnapshot {
    pub session_id: String,
    pub entries: Vec<TranscriptEntry>,
    pub overflowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_sequence: Option<u64>,
    pub next_sequence: u64,
    /// The generation for the shared SSE cursor captured with this snapshot.
    /// Together with `stream_next_sequence`, clients can replay the tiny window
    /// between snapshot/tail retrieval and live SSE subscription.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_generation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_next_sequence: Option<u64>,
}

#[derive(Debug)]
struct TranscriptSession {
    capacity: usize,
    next_sequence: u64,
    next_entry_id: u64,
    /// The current, de-duplicated display state. Assistant text chunks and a
    /// tool's lifecycle share one visible entry each.
    entries: VecDeque<TranscriptEntry>,
    /// Bounded mutation history used only for `after` replay. It can contain
    /// multiple revisions of the same visible entry so reconnecting clients can
    /// apply an upsert without retrieving a full snapshot.
    events: VecDeque<TranscriptEntry>,
}

impl TranscriptSession {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            next_sequence: 1,
            next_entry_id: 1,
            entries: VecDeque::new(),
            events: VecDeque::new(),
        }
    }

    fn allocate_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        sequence
    }

    fn allocate_entry_id(&mut self) -> String {
        let entry_id = format!("entry-{}", self.next_entry_id);
        self.next_entry_id += 1;
        entry_id
    }

    fn push_event(&mut self, entry: TranscriptEntry) {
        while self.events.len() >= self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(entry);
    }

    fn push_entry(&mut self, entry: TranscriptEntry) {
        while self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    fn snapshot(&self, session_id: &str, after: Option<u64>) -> TranscriptSnapshot {
        let (entries, oldest_sequence) = match after {
            Some(after) => (
                self.events
                    .iter()
                    .filter(|entry| entry.sequence > after)
                    .cloned()
                    .collect(),
                self.events.front().map(|entry| entry.sequence),
            ),
            None => (
                self.entries.iter().cloned().collect(),
                self.entries.front().map(|entry| entry.sequence),
            ),
        };
        let overflowed = after.is_some_and(|after| {
            oldest_sequence.is_some_and(|oldest| after < oldest.saturating_sub(1))
        });

        TranscriptSnapshot {
            session_id: session_id.to_string(),
            entries,
            overflowed,
            oldest_sequence,
            next_sequence: self.next_sequence,
            stream_generation: None,
            stream_next_sequence: None,
        }
    }
}

#[derive(Debug, Default)]
struct TranscriptStoreInner {
    sessions: HashMap<String, TranscriptSession>,
}

/// In-memory, per-session transcript storage.
///
/// The store is intentionally independent from `SessionEventBus`: lifecycle
/// status events stay low-volume while ACP text chunks are retained here. The
/// shared stream bus only serializes SSE cursor allocation across those two
/// independent producers.
#[derive(Clone)]
pub struct SessionTranscriptStore {
    capacity: usize,
    stream: SessionStreamBus,
    inner: Arc<Mutex<TranscriptStoreInner>>,
}

impl SessionTranscriptStore {
    pub fn new(capacity: usize, stream: SessionStreamBus) -> Self {
        Self {
            capacity: capacity.max(1),
            stream,
            inner: Arc::new(Mutex::new(TranscriptStoreInner::default())),
        }
    }

    pub fn capacity_from_env() -> usize {
        env::var("OPENAB_TRANSCRIPT_HISTORY_CAPACITY")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|capacity| *capacity > 0)
            .unwrap_or(DEFAULT_TRANSCRIPT_CAPACITY)
    }

    pub fn snapshot(&self, session_id: &str, after: Option<u64>) -> TranscriptSnapshot {
        let (mut snapshot, generation, stream_next_sequence) = self.stream.capture_cursor(|| {
            self.inner
                .lock()
                .expect("session transcript store lock")
                .sessions
                .get(session_id)
                .map(|session| session.snapshot(session_id, after))
                .unwrap_or(TranscriptSnapshot {
                    session_id: session_id.to_string(),
                    entries: Vec::new(),
                    overflowed: false,
                    oldest_sequence: None,
                    next_sequence: 1,
                    stream_generation: None,
                    stream_next_sequence: None,
                })
        });
        snapshot.stream_generation = Some(generation);
        snapshot.stream_next_sequence = Some(stream_next_sequence);
        snapshot
    }

    pub fn record_user_text(
        &self,
        session_id: &str,
        content: impl Into<String>,
    ) -> TranscriptEvent {
        self.append_entry(
            session_id,
            TranscriptEntryDraft {
                role: TranscriptRole::User,
                content: Some(content.into()),
                tool_call: None,
                tool_result: None,
                tool_call_id: None,
                status: Some("completed".to_string()),
            },
        )
    }

    pub fn record_system_text(
        &self,
        session_id: &str,
        content: impl Into<String>,
        status: impl Into<String>,
    ) -> TranscriptEvent {
        self.append_entry(
            session_id,
            TranscriptEntryDraft {
                role: TranscriptRole::System,
                content: Some(content.into()),
                tool_call: None,
                tool_result: None,
                tool_call_id: None,
                status: Some(status.into()),
            },
        )
    }

    /// Merge each ACP `agent_message_chunk` into the active assistant entry.
    /// A new mutation sequence is emitted for each update, but the full snapshot
    /// contains one display entry instead of one line per token.
    pub fn append_assistant_text(
        &self,
        session_id: &str,
        content: impl Into<String>,
    ) -> TranscriptEvent {
        let content = content.into();
        self.mutate(session_id, move |session| {
            let sequence = session.allocate_sequence();
            let now = Utc::now();
            if let Some(entry) = session.entries.back_mut().filter(|entry| {
                entry.role == TranscriptRole::Assistant
                    && entry.status.as_deref() == Some("streaming")
                    && entry.tool_call_id.is_none()
            }) {
                entry.sequence = sequence;
                entry.timestamp = now;
                entry
                    .content
                    .get_or_insert_with(String::new)
                    .push_str(&content);
                let event = entry.clone();
                session.push_event(event.clone());
                return event;
            }

            let entry = TranscriptEntry {
                entry_id: session.allocate_entry_id(),
                sequence,
                timestamp: now,
                role: TranscriptRole::Assistant,
                content: Some(content),
                tool_call: None,
                tool_result: None,
                tool_call_id: None,
                status: Some("streaming".to_string()),
            };
            session.push_entry(entry.clone());
            session.push_event(entry.clone());
            entry
        })
    }

    pub fn append_thinking(&self, session_id: &str, content: impl Into<String>) -> TranscriptEvent {
        self.append_entry(
            session_id,
            TranscriptEntryDraft {
                role: TranscriptRole::Assistant,
                content: Some(content.into()),
                tool_call: None,
                tool_result: None,
                tool_call_id: None,
                status: Some("thinking".to_string()),
            },
        )
    }

    pub fn upsert_tool_call(
        &self,
        session_id: &str,
        update: ToolTranscriptUpdate,
    ) -> TranscriptEvent {
        self.mutate(session_id, move |session| {
            let sequence = session.allocate_sequence();
            let now = Utc::now();
            let tool_call_id = update.tool_call_id;
            let title = update.title;
            let status = update.status;
            let payload = tool_call_payload(update.payload, &tool_call_id, &title);
            if let Some(entry) = session.entries.iter_mut().find(|entry| {
                entry.role == TranscriptRole::Tool
                    && entry.tool_call_id.as_deref() == Some(tool_call_id.as_str())
            }) {
                entry.sequence = sequence;
                entry.timestamp = now;
                if !title.is_empty() {
                    entry.content = Some(title.clone());
                }
                entry.tool_call = Some(merge_tool_payload(entry.tool_call.take(), payload.clone()));
                entry.status = Some(status.clone());
                if update.completed {
                    entry.tool_result = Some(merge_tool_payload(entry.tool_result.take(), payload));
                }
                let event = entry.clone();
                session.push_event(event.clone());
                return event;
            }

            let entry = TranscriptEntry {
                entry_id: session.allocate_entry_id(),
                sequence,
                timestamp: now,
                role: TranscriptRole::Tool,
                content: (!title.is_empty()).then_some(title),
                tool_call: Some(payload.clone()),
                tool_result: update.completed.then_some(payload),
                tool_call_id: Some(tool_call_id),
                status: Some(status),
            };
            session.push_entry(entry.clone());
            session.push_event(entry.clone());
            entry
        })
    }

    pub fn finish_assistant_turn(&self, session_id: &str) -> Option<TranscriptEvent> {
        self.try_mutate(session_id, |session| {
            let sequence = session.allocate_sequence();
            let now = Utc::now();
            let entry = session.entries.back_mut().filter(|entry| {
                entry.role == TranscriptRole::Assistant
                    && entry.status.as_deref() == Some("streaming")
                    && entry.tool_call_id.is_none()
            })?;
            entry.sequence = sequence;
            entry.timestamp = now;
            entry.status = Some("completed".to_string());
            let event = entry.clone();
            session.push_event(event.clone());
            Some(event)
        })
    }

    fn append_entry(&self, session_id: &str, draft: TranscriptEntryDraft) -> TranscriptEvent {
        self.mutate(session_id, move |session| {
            let entry = TranscriptEntry {
                entry_id: session.allocate_entry_id(),
                sequence: session.allocate_sequence(),
                timestamp: Utc::now(),
                role: draft.role,
                content: draft.content,
                tool_call: draft.tool_call,
                tool_result: draft.tool_result,
                tool_call_id: draft.tool_call_id,
                status: draft.status,
            };
            session.push_entry(entry.clone());
            session.push_event(entry.clone());
            entry
        })
    }

    fn mutate<F>(&self, session_id: &str, mutate: F) -> TranscriptEvent
    where
        F: FnOnce(&mut TranscriptSession) -> TranscriptEntry,
    {
        let session_id = session_id.to_string();
        self.stream.publish_transcript(move |stream_sequence| {
            let entry = {
                let mut inner = self.inner.lock().expect("session transcript store lock");
                let session = inner
                    .sessions
                    .entry(session_id.clone())
                    .or_insert_with(|| TranscriptSession::new(self.capacity));
                mutate(session)
            };
            TranscriptEvent {
                sequence: stream_sequence,
                session_id,
                entry,
            }
        })
    }

    fn try_mutate<F>(&self, session_id: &str, mutate: F) -> Option<TranscriptEvent>
    where
        F: FnOnce(&mut TranscriptSession) -> Option<TranscriptEntry>,
    {
        let session_id = session_id.to_string();
        let mut inner = self.inner.lock().expect("session transcript store lock");
        let session = inner.sessions.get_mut(&session_id)?;
        let entry = mutate(session)?;
        drop(inner);
        Some(
            self.stream
                .publish_transcript(move |stream_sequence| TranscriptEvent {
                    sequence: stream_sequence,
                    session_id,
                    entry,
                }),
        )
    }
}

fn tool_call_payload(mut payload: Value, tool_call_id: &str, title: &str) -> Value {
    if let Value::Object(fields) = &mut payload {
        fields
            .entry("toolCallId".to_string())
            .or_insert_with(|| Value::String(tool_call_id.to_string()));
        if !title.is_empty() {
            fields.insert("title".to_string(), Value::String(title.to_string()));
        }
    }
    payload
}

fn merge_tool_payload(existing: Option<Value>, update: Value) -> Value {
    match (existing, update) {
        (Some(Value::Object(mut existing)), Value::Object(update)) => {
            existing.extend(update);
            Value::Object(existing)
        }
        (_, update) => update,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store(capacity: usize) -> SessionTranscriptStore {
        SessionTranscriptStore::new(capacity, SessionStreamBus::new(32))
    }

    #[test]
    fn merges_assistant_chunks_into_one_snapshot_entry() {
        let store = store(8);
        let first = store.append_assistant_text("session", "hello");
        let second = store.append_assistant_text("session", " world");

        assert!(second.sequence > first.sequence);
        assert_eq!(first.entry.entry_id, second.entry.entry_id);
        let snapshot = store.snapshot("session", None);
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].entry_id, first.entry.entry_id);
        assert_eq!(snapshot.entries[0].content.as_deref(), Some("hello world"));
        assert_eq!(snapshot.entries[0].sequence, 2);

        let replay = store.snapshot("session", Some(1));
        assert_eq!(replay.entries.len(), 1);
        assert_eq!(replay.entries[0].content.as_deref(), Some("hello world"));
        assert_eq!(replay.entries[0].sequence, 2);
    }

    #[test]
    fn upserts_tool_calls_by_stable_identifier_without_losing_raw_payload() {
        let store = store(8);
        let first = store.upsert_tool_call(
            "session",
            ToolTranscriptUpdate {
                tool_call_id: "tool-1".into(),
                title: "Terminal".into(),
                status: "running".into(),
                completed: false,
                payload: json!({
                    "sessionUpdate": "tool_call",
                    "rawInput": {"command": "git status"}
                }),
            },
        );
        let second = store.upsert_tool_call(
            "session",
            ToolTranscriptUpdate {
                tool_call_id: "tool-1".into(),
                title: "".into(),
                status: "completed".into(),
                completed: true,
                payload: json!({
                    "sessionUpdate": "tool_call_update",
                    "content": [{"type": "text", "text": "clean"}],
                    "diff": {"path": "src/lib.rs", "before": "old", "after": "new"}
                }),
            },
        );

        assert!(second.sequence > first.sequence);
        assert_eq!(first.entry.entry_id, second.entry.entry_id);
        let snapshot = store.snapshot("session", None);
        assert_eq!(snapshot.entries.len(), 1);
        let entry = &snapshot.entries[0];
        assert_eq!(entry.tool_call_id.as_deref(), Some("tool-1"));
        assert_eq!(entry.status.as_deref(), Some("completed"));
        assert_eq!(
            entry.tool_call.as_ref().unwrap()["rawInput"]["command"],
            "git status"
        );
        assert_eq!(
            entry.tool_call.as_ref().unwrap()["diff"]["path"],
            "src/lib.rs"
        );
        assert_eq!(
            entry.tool_result.as_ref().unwrap()["content"][0]["text"],
            "clean"
        );
        assert_eq!(entry.tool_result.as_ref().unwrap()["diff"]["after"], "new");
    }

    #[test]
    fn reports_history_gap_after_bounded_replay_overflow() {
        let store = store(2);
        store.record_user_text("session", "one");
        store.record_user_text("session", "two");
        store.record_user_text("session", "three");

        let replay = store.snapshot("session", Some(0));
        assert!(replay.overflowed);
        assert_eq!(replay.oldest_sequence, Some(2));
        assert_eq!(replay.next_sequence, 4);
        assert_eq!(
            replay
                .entries
                .iter()
                .map(|entry| entry.content.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("two"), Some("three")]
        );
    }

    #[test]
    fn snapshot_captures_shared_stream_cursor() {
        let store = store(8);
        store.record_user_text("session", "one");

        let snapshot = store.snapshot("session", None);

        assert_eq!(
            snapshot.stream_generation.as_deref(),
            Some(store.stream.generation())
        );
        assert_eq!(snapshot.stream_next_sequence, Some(2));
    }

    #[test]
    fn configured_capacity_requires_a_positive_integer() {
        std::env::set_var("OPENAB_TRANSCRIPT_HISTORY_CAPACITY", "17");
        assert_eq!(SessionTranscriptStore::capacity_from_env(), 17);
        std::env::set_var("OPENAB_TRANSCRIPT_HISTORY_CAPACITY", "0");
        assert_eq!(
            SessionTranscriptStore::capacity_from_env(),
            DEFAULT_TRANSCRIPT_CAPACITY
        );
        std::env::remove_var("OPENAB_TRANSCRIPT_HISTORY_CAPACITY");
    }
}
