use crate::session_snapshot::SessionSnapshot;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use uuid::Uuid;

const DEFAULT_HISTORY_CAPACITY: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    #[serde(rename = "session.created")]
    SessionCreated,
    StatusChanged,
    ConfigChanged,
    Error,
    ProfileChanged,
    Exited,
}

impl SessionEventKind {
    pub fn as_sse_event(&self) -> &'static str {
        match self {
            Self::SessionCreated => "session.created",
            Self::StatusChanged => "status_changed",
            Self::ConfigChanged => "config_changed",
            Self::Error => "error",
            Self::ProfileChanged => "profile_changed",
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionEventReplay {
    pub events: Vec<SessionEvent>,
    pub overflowed: bool,
    pub oldest_sequence: Option<u64>,
    pub next_sequence: u64,
}

pub struct SessionEventSubscription {
    pub receiver: broadcast::Receiver<SessionEvent>,
    pub replay: SessionEventReplay,
}

#[derive(Debug)]
struct SessionEventHistory {
    capacity: usize,
    events: VecDeque<SessionEvent>,
}

impl SessionEventHistory {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            events: VecDeque::new(),
        }
    }

    fn push(&mut self, event: SessionEvent) {
        while self.events.len() >= self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    fn replay_after(&self, last_sequence: u64, next_sequence: u64) -> SessionEventReplay {
        let oldest_sequence = self.events.front().map(|event| event.sequence);
        let overflowed = match oldest_sequence {
            Some(oldest_sequence) => last_sequence < oldest_sequence.saturating_sub(1),
            None => false,
        };
        let events = self
            .events
            .iter()
            .filter(|event| event.sequence > last_sequence)
            .cloned()
            .collect();

        SessionEventReplay {
            events,
            overflowed,
            oldest_sequence,
            next_sequence,
        }
    }
}

#[derive(Clone)]
pub struct SessionEventBus {
    tx: broadcast::Sender<SessionEvent>,
    generation: Arc<str>,
    next_sequence: Arc<AtomicU64>,
    history: Arc<Mutex<SessionEventHistory>>,
}

impl Default for SessionEventBus {
    fn default() -> Self {
        Self::new(DEFAULT_HISTORY_CAPACITY)
    }
}

impl SessionEventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(1));
        Self {
            tx,
            generation: Uuid::new_v4().simple().to_string().into(),
            next_sequence: Arc::new(AtomicU64::new(1)),
            history: Arc::new(Mutex::new(SessionEventHistory::new(capacity))),
        }
    }

    pub fn generation(&self) -> &str {
        self.generation.as_ref()
    }

    pub fn event_id(&self, sequence: u64) -> String {
        format!("{}:{sequence}", self.generation())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.tx.subscribe()
    }

    pub fn subscribe_after(&self, last_sequence: Option<u64>) -> SessionEventSubscription {
        let history = self.history.lock().expect("session event history lock");
        let replay = last_sequence
            .map(|last_sequence| {
                history.replay_after(last_sequence, self.next_sequence.load(Ordering::Relaxed))
            })
            .unwrap_or_default();
        let receiver = self.tx.subscribe();

        SessionEventSubscription { receiver, replay }
    }

    pub fn replay_after(&self, last_sequence: u64) -> SessionEventReplay {
        self.history
            .lock()
            .expect("session event history lock")
            .replay_after(last_sequence, self.next_sequence.load(Ordering::Relaxed))
    }

    pub fn publish(&self, event: SessionEventKind, snapshot: SessionSnapshot) -> SessionEvent {
        let mut history = self.history.lock().expect("session event history lock");
        let published = SessionEvent {
            sequence: self.next_sequence.fetch_add(1, Ordering::Relaxed),
            event,
            snapshot,
        };
        history.push(published.clone());
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
        assert_eq!(
            SessionEventKind::SessionCreated.as_sse_event(),
            "session.created"
        );
        assert_eq!(
            SessionEventKind::StatusChanged.as_sse_event(),
            "status_changed"
        );
        assert_eq!(
            SessionEventKind::ConfigChanged.as_sse_event(),
            "config_changed"
        );
        assert_eq!(SessionEventKind::Error.as_sse_event(), "error");
        assert_eq!(
            SessionEventKind::ProfileChanged.as_sse_event(),
            "profile_changed"
        );
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

    #[test]
    fn event_ids_include_a_stable_bus_generation() {
        let bus = SessionEventBus::new(8);
        let clone = bus.clone();
        let replacement = SessionEventBus::new(8);

        assert_eq!(bus.generation(), clone.generation());
        assert_ne!(bus.generation(), replacement.generation());
        assert_eq!(bus.event_id(42), format!("{}:42", bus.generation()));
    }

    #[test]
    fn replay_after_returns_events_after_last_sequence() {
        let bus = SessionEventBus::new(8);
        let first = bus.publish(SessionEventKind::SessionCreated, snapshot());
        let mut updated = snapshot();
        updated.set_status(SessionStatus::Running);
        let second = bus.publish(SessionEventKind::StatusChanged, updated);

        let replay = bus.replay_after(first.sequence);

        assert!(!replay.overflowed);
        assert_eq!(replay.events, vec![second]);
        assert_eq!(replay.oldest_sequence, Some(first.sequence));
    }

    #[test]
    fn replay_after_reports_history_overflow() {
        let bus = SessionEventBus::new(2);
        let first = bus.publish(SessionEventKind::SessionCreated, snapshot());
        let mut running = snapshot();
        running.set_status(SessionStatus::Running);
        let second = bus.publish(SessionEventKind::StatusChanged, running);
        let mut exited = snapshot();
        exited.set_status(SessionStatus::Exited);
        let third = bus.publish(SessionEventKind::Exited, exited);
        let next_sequence = third.sequence + 1;

        let replay = bus.replay_after(first.sequence.saturating_sub(1));

        assert!(replay.overflowed);
        assert_eq!(replay.events, vec![second, third]);
        assert_eq!(replay.oldest_sequence, Some(first.sequence + 1));
        assert_eq!(replay.next_sequence, next_sequence);
    }
}
