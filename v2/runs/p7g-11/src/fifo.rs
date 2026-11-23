//! State only FIFO queues have: sequence numbers and the deduplication window.

use std::collections::{HashMap, VecDeque};

use serde::Serialize;

use crate::types::{Body, DeduplicationId, GroupId, MessageId, Time};

/// Seconds during which a deduplication id refuses a second send.
const DEDUPLICATION_WINDOW: u32 = 300;

/// The identity a FIFO send is deduplicated by: the explicit id, or else the body.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DedupKey(String);

impl DedupKey {
    pub fn from_id(id: &DeduplicationId) -> Self {
        Self(id.as_str().to_owned())
    }

    pub fn from_body(body: &Body) -> Self {
        Self(body.as_str().to_owned())
    }
}

/// What a FIFO message carries besides a standard one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct FifoMeta {
    pub group_id: GroupId,
    pub sequence_number: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Original {
    pub message_id: MessageId,
    pub sequence_number: u64,
}

#[derive(Debug, Clone, Copy)]
struct Accepted {
    at: Time,
    original: Original,
}

#[derive(Debug, Default)]
pub struct FifoState {
    sequence: u64,
    accepted: HashMap<DedupKey, Accepted>,
    /// Acceptance times in clock order, to drop entries whose window has closed.
    by_time: VecDeque<(Time, DedupKey)>,
}

impl FifoState {
    pub fn next_sequence(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    /// The message that already used `key` within the window ending after `now`.
    pub fn find_duplicate(&self, key: &DedupKey, now: Time) -> Option<Original> {
        self.accepted
            .get(key)
            .filter(|entry| now < entry.at.plus(DEDUPLICATION_WINDOW))
            .map(|entry| entry.original)
    }

    pub fn record(&mut self, key: DedupKey, now: Time, original: Original) {
        self.forget_closed_windows(now);
        self.by_time.push_back((now, key.clone()));
        self.accepted.insert(key, Accepted { at: now, original });
    }

    fn forget_closed_windows(&mut self, now: Time) {
        while let Some((at, _)) = self.by_time.front() {
            if now < at.plus(DEDUPLICATION_WINDOW) {
                break;
            }
            let Some((at, key)) = self.by_time.pop_front() else {
                break;
            };
            // A newer acceptance of the same key owns the map entry now.
            if self.accepted.get(&key).is_some_and(|entry| entry.at == at) {
                self.accepted.remove(&key);
            }
        }
    }
}
