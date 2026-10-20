//! Bound one OpenCode V2 live segment to a read-only SQLite baseline.

use std::collections::{HashMap, HashSet};

use serde_json::Value;
use uuid::Uuid;

pub const MAX_CAPTURE_SESSIONS: usize = 1024;
pub const MAX_CAPTURE_EVENTS: usize = 4096;
pub const MAX_CAPTURE_EVENT_BYTES: usize = 256 * 1024;
pub const MAX_CAPTURE_BUFFER_BYTES: usize = 16 * 1024 * 1024;
const MAX_CAPTURE_ID_BYTES: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    AwaitingSubscription,
    Hydrating,
    Live,
    Gap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryKind {
    Seed,
    Live,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CapturedEvent {
    pub session_id: String,
    pub sequence: u64,
    pub kind: BoundaryKind,
    pub event: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureGap {
    SubscriptionNotConfirmed,
    InvalidBoundary,
    InvalidEvent,
    SequenceRegression,
    CapacityExceeded,
    ContinuityLost,
}

/// A server subscription and its read-only snapshot boundary.
pub struct OpenCodeV2CaptureEpoch {
    id: Uuid,
    phase: Phase,
    admitted_sessions: HashSet<String>,
    boundaries: HashMap<String, Option<u64>>,
    buffered: Vec<Value>,
    buffered_bytes: usize,
    seen_ids: HashSet<String>,
    last_sequences: HashMap<String, u64>,
}

impl OpenCodeV2CaptureEpoch {
    pub fn new(admitted_sessions: Vec<String>) -> Result<Self, CaptureGap> {
        let unique_sessions: HashSet<String> = admitted_sessions.iter().cloned().collect();
        if admitted_sessions.is_empty()
            || admitted_sessions.len() > MAX_CAPTURE_SESSIONS
            || unique_sessions.len() != admitted_sessions.len()
            || admitted_sessions
                .iter()
                .any(|id| id.is_empty() || id.len() > MAX_CAPTURE_ID_BYTES || id.contains('\0'))
        {
            return Err(CaptureGap::InvalidBoundary);
        }
        Ok(Self {
            id: Uuid::new_v4(),
            phase: Phase::AwaitingSubscription,
            admitted_sessions: unique_sessions,
            boundaries: HashMap::new(),
            buffered: Vec::new(),
            buffered_bytes: 0,
            seen_ids: HashSet::new(),
            last_sequences: HashMap::new(),
        })
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Confirm the feed's `server.connected` marker before snapshot work starts.
    pub fn confirm_subscription(&mut self) -> Result<(), CaptureGap> {
        if self.phase != Phase::AwaitingSubscription {
            return Err(if self.phase == Phase::Gap {
                CaptureGap::ContinuityLost
            } else {
                CaptureGap::SubscriptionNotConfirmed
            });
        }
        self.phase = Phase::Hydrating;
        Ok(())
    }

    /// Save the per-session sequence watermark from one SQLite read snapshot.
    /// Events buffered before this call are classified against these watermarks.
    pub fn finish_hydration(
        &mut self,
        boundaries: HashMap<String, Option<u64>>,
    ) -> Result<Vec<CapturedEvent>, CaptureGap> {
        if self.phase != Phase::Hydrating {
            return Err(if self.phase == Phase::Gap {
                CaptureGap::ContinuityLost
            } else {
                CaptureGap::SubscriptionNotConfirmed
            });
        }
        if boundaries.len() != self.admitted_sessions.len()
            || boundaries.keys().any(|id| {
                id.is_empty()
                    || id.len() > MAX_CAPTURE_ID_BYTES
                    || id.contains('\0')
                    || !self.admitted_sessions.contains(id)
            })
        {
            self.mark_gap();
            return Err(CaptureGap::InvalidBoundary);
        }
        self.boundaries = boundaries;
        let buffered = std::mem::take(&mut self.buffered);
        self.buffered_bytes = 0;
        self.seen_ids.clear();
        self.last_sequences.clear();
        let mut classified = Vec::with_capacity(buffered.len());
        for event in buffered {
            if let Some(event) = self.classify(event)? {
                classified.push(event);
            }
        }
        self.phase = Phase::Live;
        Ok(classified)
    }

    /// Buffer events until the snapshot boundary is ready, then classify each
    /// event by its aggregate sequence. Public sequence jumps are allowed.
    pub fn observe(&mut self, event: Value) -> Result<Option<CapturedEvent>, CaptureGap> {
        match self.phase {
            Phase::AwaitingSubscription => Err(CaptureGap::SubscriptionNotConfirmed),
            Phase::Hydrating => {
                let Some((event_id, session_id, sequence)) = self.event_identity(&event)? else {
                    return Ok(None);
                };
                let Some(bytes) = event_size(&event) else {
                    self.mark_gap();
                    return Err(CaptureGap::CapacityExceeded);
                };
                if self.buffered.len() >= MAX_CAPTURE_EVENTS
                    || self.buffered_bytes.saturating_add(bytes) > MAX_CAPTURE_BUFFER_BYTES
                {
                    self.mark_gap();
                    return Err(CaptureGap::CapacityExceeded);
                }
                if self.seen_ids.contains(event_id) {
                    return Ok(None);
                }
                if self.seen_ids.len() >= MAX_CAPTURE_EVENTS {
                    self.mark_gap();
                    return Err(CaptureGap::CapacityExceeded);
                }
                if self
                    .last_sequences
                    .get(session_id)
                    .is_some_and(|last| sequence <= *last)
                {
                    self.mark_gap();
                    return Err(CaptureGap::SequenceRegression);
                }
                self.buffered_bytes += bytes;
                self.seen_ids.insert(event_id.to_owned());
                self.last_sequences.insert(session_id.to_owned(), sequence);
                self.buffered.push(event);
                Ok(None)
            }
            Phase::Live => self.classify(event),
            Phase::Gap => Err(CaptureGap::ContinuityLost),
        }
    }

    pub fn mark_gap(&mut self) {
        self.phase = Phase::Gap;
        self.buffered.clear();
        self.buffered_bytes = 0;
        self.seen_ids.clear();
        self.last_sequences.clear();
    }

    fn classify(&mut self, event: Value) -> Result<Option<CapturedEvent>, CaptureGap> {
        let Some((event_id, session_id, sequence)) = self.event_identity(&event)? else {
            return Ok(None);
        };
        if event_size(&event).is_none() {
            self.mark_gap();
            return Err(CaptureGap::CapacityExceeded);
        }
        let boundary = self
            .boundaries
            .get(session_id)
            .copied()
            .expect("checked key");
        if self.seen_ids.contains(event_id) {
            return Ok(None);
        }
        if self.seen_ids.len() >= MAX_CAPTURE_EVENTS {
            self.mark_gap();
            return Err(CaptureGap::CapacityExceeded);
        }
        if self
            .last_sequences
            .get(session_id)
            .is_some_and(|last| sequence <= *last)
        {
            self.mark_gap();
            return Err(CaptureGap::SequenceRegression);
        }
        self.seen_ids.insert(event_id.to_owned());
        self.last_sequences.insert(session_id.to_owned(), sequence);
        Ok(Some(CapturedEvent {
            session_id: session_id.to_owned(),
            sequence,
            kind: if boundary.is_some_and(|watermark| sequence <= watermark) {
                BoundaryKind::Seed
            } else {
                BoundaryKind::Live
            },
            event,
        }))
    }

    fn event_identity<'a>(
        &mut self,
        event: &'a Value,
    ) -> Result<Option<(&'a str, &'a str, u64)>, CaptureGap> {
        let Some(durable) = event.get("durable").and_then(Value::as_object) else {
            return Ok(None);
        };
        let Some(session_id) = durable.get("aggregateID").and_then(Value::as_str) else {
            self.mark_gap();
            return Err(CaptureGap::InvalidEvent);
        };
        if !self.admitted_sessions.contains(session_id) {
            return Ok(None);
        }
        let (Some(event_id), Some(sequence)) = (
            event.get("id").and_then(Value::as_str),
            durable.get("seq").and_then(Value::as_u64),
        ) else {
            self.mark_gap();
            return Err(CaptureGap::InvalidEvent);
        };
        if event_id.is_empty()
            || event_id.len() > MAX_CAPTURE_ID_BYTES
            || session_id.is_empty()
            || session_id.len() > MAX_CAPTURE_ID_BYTES
            || session_id.contains('\0')
        {
            self.mark_gap();
            return Err(CaptureGap::InvalidEvent);
        }
        Ok(Some((event_id, session_id, sequence)))
    }
}

fn event_size(event: &Value) -> Option<usize> {
    use std::io::{self, Write};

    struct Counter {
        len: usize,
    }

    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.len = self
                .len
                .checked_add(bytes.len())
                .filter(|len| *len <= MAX_CAPTURE_EVENT_BYTES)
                .ok_or_else(|| io::Error::other("capture event exceeds byte limit"))?;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut counter = Counter { len: 0 };
    serde_json::to_writer(&mut counter, event).ok()?;
    Some(counter.len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(id: &str, session_id: &str, sequence: u64) -> Value {
        json!({
            "id": id,
            "type": "session.tool.success",
            "durable": {"aggregateID": session_id, "seq": sequence, "version": 1},
            "data": {"sessionID": session_id}
        })
    }

    #[test]
    fn subscription_precedes_hydration_and_classifies_buffered_boundary_events() {
        let mut epoch = OpenCodeV2CaptureEpoch::new(vec!["ses_a".into(), "ses_b".into()]).unwrap();
        assert_ne!(epoch.id(), Uuid::nil());
        assert_ne!(
            epoch.id(),
            OpenCodeV2CaptureEpoch::new(vec!["ses_a".into(), "ses_b".into()])
                .unwrap()
                .id()
        );
        assert!(matches!(
            epoch.observe(event("before", "ses_a", 1)),
            Err(CaptureGap::SubscriptionNotConfirmed)
        ));
        epoch.confirm_subscription().unwrap();
        assert_eq!(
            epoch.observe(json!({"type":"server.connected"})).unwrap(),
            None
        );
        assert_eq!(epoch.observe(event("old", "ses_a", 2)).unwrap(), None);
        assert_eq!(epoch.observe(event("new", "ses_a", 9)).unwrap(), None);
        assert_eq!(epoch.observe(event("other", "ses_b", 4)).unwrap(), None);
        assert_eq!(
            epoch
                .observe(json!({
                    "id":"unrelated",
                    "durable":{"aggregateID":"ses_untracked", "seq":1},
                    "payload":"x".repeat(MAX_CAPTURE_EVENT_BYTES + 1)
                }))
                .unwrap(),
            None
        );

        let events = epoch
            .finish_hydration(HashMap::from([
                ("ses_a".to_owned(), Some(2)),
                ("ses_b".to_owned(), Some(3)),
            ]))
            .unwrap();

        assert_eq!(events.len(), 3);
        assert_eq!(events[0].kind, BoundaryKind::Seed);
        assert_eq!(events[1].kind, BoundaryKind::Live);
        assert_eq!(events[2].kind, BoundaryKind::Live);
        let after = epoch.observe(event("later", "ses_a", 13)).unwrap().unwrap();
        assert_eq!(after.kind, BoundaryKind::Live);
    }

    #[test]
    fn public_sequence_jumps_are_not_gaps_but_regressions_stop_the_epoch() {
        let mut epoch = OpenCodeV2CaptureEpoch::new(vec!["ses_a".into()]).unwrap();
        epoch.confirm_subscription().unwrap();
        epoch
            .finish_hydration(HashMap::from([("ses_a".to_owned(), None)]))
            .unwrap();
        assert!(epoch.observe(event("first", "ses_a", 1)).unwrap().is_some());
        assert!(
            epoch
                .observe(event("later", "ses_a", 99))
                .unwrap()
                .is_some()
        );
        assert!(matches!(
            epoch.observe(event("regressed", "ses_a", 98)),
            Err(CaptureGap::SequenceRegression)
        ));
        assert_eq!(
            epoch.observe(event("after-gap", "ses_a", 100)),
            Err(CaptureGap::ContinuityLost)
        );
    }

    #[test]
    fn hydration_buffer_is_bounded_and_drops_partial_events_on_overflow() {
        let mut epoch = OpenCodeV2CaptureEpoch::new(vec!["ses_a".into()]).unwrap();
        epoch.confirm_subscription().unwrap();
        for sequence in 0..MAX_CAPTURE_EVENTS {
            epoch
                .observe(event(&format!("evt_{sequence}"), "ses_a", sequence as u64))
                .unwrap();
        }
        assert!(matches!(
            epoch.observe(event("overflow", "ses_a", MAX_CAPTURE_EVENTS as u64)),
            Err(CaptureGap::CapacityExceeded)
        ));
        assert_eq!(
            epoch.finish_hydration(HashMap::from([("ses_a".to_owned(), None)])),
            Err(CaptureGap::ContinuityLost)
        );
    }
}
