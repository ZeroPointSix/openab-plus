use crate::session_snapshot::SessionSnapshot;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    #[serde(rename = "session.created")]
    SessionCreated,
    StatusChanged,
    ConfigChanged,
    Error,
    Exited,
}

impl SessionEventKind {
    pub fn as_sse_event(&self) -> &'static str {
        match self {
            Self::SessionCreated => "session.created",
            Self::StatusChanged => "status_changed",
            Self::ConfigChanged => "config_changed",
            Self::Error => "error",
            Self::Exited => "exited",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionEvent {
    pub sequence: u64,
    pub event: SessionEventKind,
    pub snapshot: SessionSnapshot,
}

#[derive(Clone)]
pub struct SessionEventBus {
    tx: broadcast::Sender<SessionEvent>,
    next_sequence: Arc<AtomicU64>,
}

impl Default for SessionEventBus {
    fn default() -> Self {
        Self::new(512)
    }
}

impl SessionEventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(1));
        Self {
            tx,
            next_sequence: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.tx.subscribe()
    }

    pub fn publish(&self, event: SessionEventKind, snapshot: SessionSnapshot) -> SessionEvent {
        let published = SessionEvent {
            sequence: self.next_sequence.fetch_add(1, Ordering::Relaxed),
            event,
            snapshot,
        };
        let _ = self.tx.send(published.clone());
        published
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_snapshot::{SessionSnapshot, SessionStatus};

    fn snapshot() -> SessionSnapshot {
        SessionSnapshot::new(
            "slack:T1".into(),
            "codex".into(),
            "/workspace".into(),
            None,
            None,
            Some("gpt-5".into()),
            None,
        )
    }

    #[test]
    fn event_names_match_sse_contract() {
        assert_eq!(SessionEventKind::SessionCreated.as_sse_event(), "session.created");
        assert_eq!(SessionEventKind::StatusChanged.as_sse_event(), "status_changed");
        assert_eq!(SessionEventKind::ConfigChanged.as_sse_event(), "config_changed");
        assert_eq!(SessionEventKind::Error.as_sse_event(), "error");
        assert_eq!(SessionEventKind::Exited.as_sse_event(), "exited");
    }

    #[test]
    fn publish_assigns_monotonic_sequence_numbers() {
        let bus = SessionEventBus::new(8);
        let first = bus.publish(SessionEventKind::SessionCreated, snapshot());
        let mut updated = snapshot();
        updated.set_status(SessionStatus::Running);
        let second = bus.publish(SessionEventKind::StatusChanged, updated);
        assert_eq!(first.sequence + 1, second.sequence);
    }
}
