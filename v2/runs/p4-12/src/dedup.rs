//! The per-queue FIFO deduplication window of SPEC section 6.

use std::collections::{HashMap, VecDeque};

/// How long, in seconds, an accepted deduplication id turns away later sends.
pub const WINDOW_SECONDS: u64 = 300;

/// What a duplicate send answers with: the identity of the message that opened the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Original {
    pub message_id: u64,
    pub sequence_number: u64,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    accepted_at: u64,
    original: Original,
}

/// Remembers the deduplication ids a queue accepted during the last [`WINDOW_SECONDS`].
///
/// Ids are kept independently of the messages themselves, so deleting a message does not free its
/// id. Expired ids are dropped lazily, oldest first, which keeps memory bounded by the send rate.
#[derive(Debug, Default)]
pub struct Deduplicator {
    entries: HashMap<String, Entry>,
    /// Accepted ids in acceptance order; acceptance times never decrease since the clock doesn't.
    accepted: VecDeque<(u64, String)>,
}

impl Deduplicator {
    /// Returns the original message if `id` was accepted less than [`WINDOW_SECONDS`] before `now`.
    pub fn find(&mut self, id: &str, now: u64) -> Option<Original> {
        self.forget_expired(now);
        self.entries.get(id).map(|entry| entry.original)
    }

    /// Records that `id` was accepted at `now`, opening a new window. Call only after [`find`]
    /// returned `None` for the same id and time.
    ///
    /// [`find`]: Self::find
    pub fn record(&mut self, id: String, now: u64, original: Original) {
        let entry = Entry {
            accepted_at: now,
            original,
        };
        self.entries.insert(id.clone(), entry);
        self.accepted.push_back((now, id));
    }

    fn forget_expired(&mut self, now: u64) {
        while let Some((accepted_at, _)) = self.accepted.front() {
            if now - accepted_at < WINDOW_SECONDS {
                break;
            }
            let (accepted_at, id) = self.accepted.pop_front().expect("front exists");
            // The id may have been accepted again since; only drop the entry this record created.
            if self
                .entries
                .get(&id)
                .is_some_and(|entry| entry.accepted_at == accepted_at)
            {
                self.entries.remove(&id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn original(message_id: u64) -> Original {
        Original {
            message_id,
            sequence_number: message_id,
        }
    }

    #[test]
    fn window_lasts_300_seconds_from_acceptance() {
        let mut dedup = Deduplicator::default();
        assert_eq!(dedup.find("a", 10), None);
        dedup.record("a".into(), 10, original(1));
        assert_eq!(dedup.find("a", 10), Some(original(1)));
        assert_eq!(dedup.find("a", 309), Some(original(1)));
        assert_eq!(dedup.find("a", 310), None);
    }

    #[test]
    fn reaccepted_id_opens_a_new_window() {
        let mut dedup = Deduplicator::default();
        dedup.record("a".into(), 0, original(1));
        dedup.record("b".into(), 100, original(2));
        // "a" is accepted again at 300 without an intervening find that would prune it first.
        dedup.record("a".into(), 300, original(3));
        assert_eq!(dedup.find("a", 350), Some(original(3)));
        assert_eq!(dedup.find("b", 350), Some(original(2)));
        assert_eq!(dedup.find("b", 400), None);
        assert_eq!(dedup.find("a", 599), Some(original(3)));
        assert_eq!(dedup.find("a", 600), None);
    }
}
