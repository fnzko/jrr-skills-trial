//! One queue: its attributes, its messages in arrival order and the
//! bookkeeping that keeps receipt handles and deduplication ids honest.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::config::{Config, QueueKind};
use crate::ids::{Counter, MessageId, ReceiptHandle, SequenceNumber};
use crate::message::{Delivery, GroupId, Message, MessageState};
use crate::time::{Instant, Seconds};

pub const MAX_MESSAGES: usize = 100_000;
const DEDUP_WINDOW_SECS: u64 = 300;

/// Arrival position of a message in its queue. Never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Slot(u64);

/// What the sender learns about an accepted message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accepted {
    pub id: MessageId,
    pub sequence: Option<SequenceNumber>,
}

#[derive(Clone, Copy, Debug)]
struct DedupEntry {
    accepted_at: Instant,
    accepted: Accepted,
}

/// The number of messages in each state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// The outcome of one `receive` scan.
#[derive(Debug)]
pub struct Received {
    pub delivered: Vec<Delivery>,
    /// Messages that left the queue for its dead-letter queue, in arrival order.
    pub dead: Vec<Message>,
}

#[derive(Debug)]
pub struct Queue {
    config: Config,
    messages: BTreeMap<Slot, Message>,
    next_slot: u64,
    sequences: Counter,
    dedup: HashMap<String, DedupEntry>,
    handles: HashMap<ReceiptHandle, Slot>,
    /// A lower bound of the `SentAt` of the messages held, so that `expire`
    /// can skip the scan while nothing can have expired. `None` when empty.
    oldest_sent_at: Option<Instant>,
}

impl Queue {
    pub fn new(config: Config) -> Queue {
        Queue {
            config,
            messages: BTreeMap::new(),
            next_slot: 0,
            sequences: Counter::new(),
            dedup: HashMap::new(),
            handles: HashMap::new(),
            oldest_sent_at: None,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Callers that change the retention period must call [`Queue::expire`] afterwards.
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    pub fn is_full(&self) -> bool {
        self.messages.len() >= MAX_MESSAGES
    }

    /// The earlier send of `dedup_id` if it is still inside its window.
    pub fn find_duplicate(&self, dedup_id: &str, now: Instant) -> Option<Accepted> {
        self.dedup
            .get(dedup_id)
            .filter(|entry| now.since(entry.accepted_at) < DEDUP_WINDOW_SECS)
            .map(|entry| entry.accepted)
    }

    /// Appends `message`, numbering it on a FIFO queue, and opens a
    /// deduplication window for `dedup_id` if one is given.
    pub fn add(&mut self, mut message: Message, dedup_id: Option<String>) -> Accepted {
        let sequence = match self.config.kind {
            QueueKind::Fifo { .. } => Some(self.sequences.next_sequence_number()),
            QueueKind::Standard => None,
        };
        message.set_sequence(sequence);
        let accepted = Accepted {
            id: message.id(),
            sequence,
        };
        if let Some(dedup_id) = dedup_id {
            self.dedup.insert(
                dedup_id,
                DedupEntry {
                    accepted_at: message.sent_at(),
                    accepted,
                },
            );
        }
        let sent_at = message.sent_at();
        self.oldest_sent_at = Some(
            self.oldest_sent_at
                .map_or(sent_at, |oldest| oldest.min(sent_at)),
        );
        let slot = Slot(self.next_slot);
        self.next_slot += 1;
        self.messages.insert(slot, message);
        accepted
    }

    /// Removes every message that has outlived the retention period.
    pub fn expire(&mut self, now: Instant) {
        let retention = self.config.retention;
        match self.oldest_sent_at {
            Some(oldest) if now >= oldest.after(retention) => {}
            _ => return,
        }
        let handles = &mut self.handles;
        let mut oldest_kept: Option<Instant> = None;
        self.messages.retain(|_, message| {
            if message.is_expired(now, retention) {
                if let Some(handle) = message.handle() {
                    handles.remove(&handle);
                }
                return false;
            }
            let sent_at = message.sent_at();
            oldest_kept = Some(oldest_kept.map_or(sent_at, |oldest| oldest.min(sent_at)));
            true
        });
        self.oldest_sent_at = oldest_kept;
    }

    /// Drops deduplication ids whose window has closed.
    pub fn forget_closed_dedup_windows(&mut self, now: Instant) {
        self.dedup
            .retain(|_, entry| now.since(entry.accepted_at) < DEDUP_WINDOW_SECS);
    }

    /// Removes every message. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.handles.clear();
        self.oldest_sent_at = None;
    }

    pub fn counts(&self, now: Instant) -> Counts {
        self.messages
            .values()
            .fold(Counts::default(), |mut counts, message| {
                match message.state(now) {
                    MessageState::Visible => counts.visible += 1,
                    MessageState::InFlight => counts.in_flight += 1,
                    MessageState::Delayed => counts.delayed += 1,
                }
                counts
            })
    }

    /// Delivers up to `max` eligible messages. On a FIFO queue a message is
    /// eligible only if no earlier message of its group is still queued.
    /// Messages that reached the dead-letter limit are taken out instead.
    pub fn receive(
        &mut self,
        now: Instant,
        max: usize,
        visibility: Seconds,
        handle_numbers: &mut Counter,
    ) -> Received {
        let receive_limit = self
            .config
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.max_receive_count);
        let mut delivered = Vec::new();
        let mut dead_slots = Vec::new();
        let mut groups_in_queue: HashSet<GroupId> = HashSet::new();
        for (&slot, message) in self.messages.iter_mut() {
            if delivered.len() >= max {
                break;
            }
            let group_blocked = message
                .group()
                .is_some_and(|group| groups_in_queue.contains(group));
            if !group_blocked && message.state(now) == MessageState::Visible {
                if receive_limit.is_some_and(|limit| message.receive_count() >= limit) {
                    dead_slots.push(slot);
                    continue;
                }
                let handle = handle_numbers.next_receipt_handle();
                let (delivery, replaced) = message.deliver(now, visibility, handle);
                if let Some(replaced) = replaced {
                    self.handles.remove(&replaced);
                }
                self.handles.insert(handle, slot);
                delivered.push(delivery);
            }
            if let Some(group) = message.group() {
                groups_in_queue.insert(group.clone());
            }
        }
        let dead = dead_slots
            .into_iter()
            .filter_map(|slot| self.remove(slot))
            .collect();
        Received { delivered, dead }
    }

    /// Deletes the message that `handle` is the latest handle of, if it is in
    /// flight. Returns whether it did.
    pub fn delete_in_flight(&mut self, handle: ReceiptHandle, now: Instant) -> bool {
        match self.in_flight_slot(handle, now) {
            Some(slot) => self.remove(slot).is_some(),
            None => false,
        }
    }

    /// Restarts the visibility timeout of the in-flight message `handle`
    /// belongs to. Returns whether there was one.
    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: Instant,
        visibility: Seconds,
    ) -> bool {
        let Some(slot) = self.in_flight_slot(handle, now) else {
            return false;
        };
        match self.messages.get_mut(&slot) {
            Some(message) => {
                message.restart_visibility(now, visibility);
                true
            }
            None => false,
        }
    }

    fn in_flight_slot(&self, handle: ReceiptHandle, now: Instant) -> Option<Slot> {
        let slot = *self.handles.get(&handle)?;
        let message = self.messages.get(&slot)?;
        (message.handle() == Some(handle) && message.state(now) == MessageState::InFlight)
            .then_some(slot)
    }

    fn remove(&mut self, slot: Slot) -> Option<Message> {
        let message = self.messages.remove(&slot)?;
        if let Some(handle) = message.handle() {
            self.handles.remove(&handle);
        }
        Some(message)
    }
}
