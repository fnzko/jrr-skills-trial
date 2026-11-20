//! One queue's messages and the indexes that keep every operation cheap.
//!
//! A queue must stay fast with 100000 messages, so nothing here scans all of them. Messages are
//! stored by *position* (a per-queue counter giving arrival order), and three ordered indexes are
//! kept alongside:
//!
//! - `by_age` orders messages by `SentAt`, so retention pops expired messages from the front;
//! - `hidden` orders delayed and in-flight messages by the time they become visible;
//! - `eligible` holds, in arrival order, the messages `ReceiveMessage` may hand out right now.
//!
//! The queue never reads a clock. The owner calls [`Queue::settle`] whenever time moves or the
//! retention changes, and between those calls the indexes describe the present exactly.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::ops::Bound;

/// Width of the FIFO deduplication window, in seconds.
const DEDUPLICATION_WINDOW: u64 = 300;

/// A process-wide sequence issuing 1, 2, 3, ...
#[derive(Debug, Default)]
pub struct Counter(u64);

impl Counter {
    pub fn issue(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }

    /// The largest value issued so far, or 0.
    pub fn last(&self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetterConfig {
    pub queue_name: String,
    pub max_receive_count: u64,
}

/// The attributes of SPEC section 4, with defaults applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: u64,
    pub delay_seconds: u64,
    pub retention_seconds: u64,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetterConfig>,
}

impl Default for Attributes {
    fn default() -> Self {
        Self {
            visibility_timeout: 30,
            delay_seconds: 0,
            retention_seconds: 345_600,
            fifo: false,
            content_based_deduplication: false,
            dead_letter: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visibility {
    Visible,
    Delayed { until: u64 },
    InFlight { until: u64, receipt: u64 },
}

impl Visibility {
    fn hidden_until(self) -> Option<u64> {
        match self {
            Self::Visible => None,
            Self::Delayed { until } | Self::InFlight { until, .. } => Some(until),
        }
    }
}

/// A message as stored in a queue.
#[derive(Debug, Clone)]
pub struct Message {
    pub id: u64,
    pub body: String,
    pub sent_at: u64,
    pub group_id: Option<String>,
    pub receive_count: u64,
    pub first_received_at: Option<u64>,
    pub sequence_number: Option<u64>,
    visibility: Visibility,
}

impl Message {
    /// A message about to be added to a queue: never received, visible unless given a delay.
    pub fn new(id: u64, body: String, sent_at: u64, group_id: Option<String>) -> Self {
        Self {
            id,
            body,
            sent_at,
            group_id,
            receive_count: 0,
            first_received_at: None,
            sequence_number: None,
            visibility: Visibility::Visible,
        }
    }
}

/// What a delivery hands back to the caller of `ReceiveMessage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub message_id: u64,
    pub receipt: u64,
    pub body: String,
    pub receive_count: u64,
    pub sent_at: u64,
    pub first_received_at: u64,
    pub group_id: Option<String>,
    pub sequence_number: Option<u64>,
}

/// The outcome of [`Queue::receive`].
#[derive(Debug, Default)]
pub struct Received {
    pub deliveries: Vec<Delivery>,
    /// Messages that hit `MaxReceiveCount` and must go to the dead-letter queue, in order.
    pub dead_letters: Vec<Message>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// Why a receipt handle cannot be used on this queue right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandleExpired;

/// A FIFO send already accepted within the deduplication window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Original {
    pub message_id: u64,
    pub sequence_number: u64,
}

#[derive(Debug, Default)]
pub struct Queue {
    attributes: Attributes,
    messages: HashMap<u64, Message>,
    next_position: u64,
    sequence_numbers: Counter,
    by_age: BTreeSet<(u64, u64)>,
    hidden: BTreeSet<(u64, u64)>,
    eligible: BTreeSet<u64>,
    /// FIFO only: the positions of each group's messages; the first one is the group's head.
    groups: HashMap<String, BTreeSet<u64>>,
    /// Latest receipt of each in-flight message, mapped to its position.
    in_flight: HashMap<u64, u64>,
    deduplication: Deduplication,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            ..Self::default()
        }
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    /// Mutable access to the attributes. Call [`Queue::settle`] afterwards if the retention
    /// changed; the other attributes only affect later operations.
    pub fn attributes_mut(&mut self) -> &mut Attributes {
        &mut self.attributes
    }

    pub fn is_fifo(&self) -> bool {
        self.attributes.fifo
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn counts(&self) -> Counts {
        let in_flight = self.in_flight.len();
        Counts {
            visible: self.messages.len() - self.hidden.len(),
            in_flight,
            delayed: self.hidden.len() - in_flight,
        }
    }

    /// Brings the queue up to date with the clock: drops messages past retention and makes
    /// visible those whose delay or visibility timeout has ended.
    pub fn settle(&mut self, now: u64) {
        let retention = self.attributes.retention_seconds;
        while let Some(&(sent_at, position)) = self.by_age.first() {
            if now < sent_at + retention {
                break;
            }
            self.remove(position);
        }
        while let Some(&(until, position)) = self.hidden.first() {
            if now < until {
                break;
            }
            self.set_visibility(position, Visibility::Visible);
        }
    }

    /// Looks up a FIFO deduplication id; `Some` means the send is a duplicate of that message.
    pub fn find_duplicate(&mut self, deduplication_id: &str, now: u64) -> Option<Original> {
        self.deduplication.find(deduplication_id, now)
    }

    /// Adds a message at the end of the queue, hidden for `delay` seconds, and returns its
    /// sequence number on a FIFO queue. With a deduplication id, opens its window at `now`.
    pub fn push(
        &mut self,
        mut message: Message,
        delay: u64,
        deduplication_id: Option<String>,
        now: u64,
    ) -> Option<u64> {
        let position = self.next_position;
        self.next_position += 1;

        if self.attributes.fifo {
            message.sequence_number = Some(self.sequence_numbers.issue());
        }
        if let (Some(id), Some(sequence_number)) = (deduplication_id, message.sequence_number) {
            self.deduplication.record(
                id,
                now,
                Original {
                    message_id: message.id,
                    sequence_number,
                },
            );
        }
        if let Some(group) = &message.group_id {
            self.groups
                .entry(group.clone())
                .or_default()
                .insert(position);
        }
        self.by_age.insert((message.sent_at, position));
        message.visibility = Visibility::Visible;
        let sequence_number = message.sequence_number;
        self.messages.insert(position, message);

        if delay > 0 {
            self.set_visibility(position, Visibility::Delayed { until: now + delay });
        } else {
            self.refresh_eligibility(position);
        }
        sequence_number
    }

    /// Examines eligible messages in arrival order, delivering up to `max_messages` of them
    /// with visibility timeout `timeout`. With `max_receive_count`, messages already received
    /// that many times are taken out for the dead-letter queue instead.
    pub fn receive(
        &mut self,
        max_messages: usize,
        timeout: u64,
        max_receive_count: Option<u64>,
        receipts: &mut Counter,
        now: u64,
    ) -> Received {
        let mut received = Received::default();
        let mut cursor = Bound::Unbounded;
        while received.deliveries.len() < max_messages {
            let Some(&position) = self.eligible.range((cursor, Bound::Unbounded)).next() else {
                break;
            };
            cursor = Bound::Excluded(position);

            let message = &self.messages[&position];
            if max_receive_count.is_some_and(|max| message.receive_count >= max) {
                // Removing a FIFO head makes the next message of its group the head; it comes
                // later in arrival order, so this same scan still examines it.
                let mut message = self.remove(position);
                message.receive_count = 0;
                message.first_received_at = None;
                message.sequence_number = None;
                received.dead_letters.push(message);
                continue;
            }

            let receipt = receipts.issue();
            if timeout > 0 {
                let until = now + timeout;
                self.set_visibility(position, Visibility::InFlight { until, receipt });
            }
            // A zero timeout leaves the message visible and its receipt never usable.
            let message = self
                .messages
                .get_mut(&position)
                .expect("eligible messages are stored");
            message.receive_count += 1;
            let first_received_at = *message.first_received_at.get_or_insert(now);
            received.deliveries.push(Delivery {
                message_id: message.id,
                receipt,
                body: message.body.clone(),
                receive_count: message.receive_count,
                sent_at: message.sent_at,
                first_received_at,
                group_id: message.group_id.clone(),
                sequence_number: message.sequence_number,
            });
        }
        received
    }

    /// Deletes the in-flight message whose latest receipt is `receipt`.
    pub fn delete(&mut self, receipt: u64) -> Result<(), HandleExpired> {
        let position = *self.in_flight.get(&receipt).ok_or(HandleExpired)?;
        self.remove(position);
        Ok(())
    }

    /// Makes the in-flight message whose latest receipt is `receipt` visible again `timeout`
    /// seconds from `now`.
    pub fn change_visibility(
        &mut self,
        receipt: u64,
        timeout: u64,
        now: u64,
    ) -> Result<(), HandleExpired> {
        let position = *self.in_flight.get(&receipt).ok_or(HandleExpired)?;
        let visibility = if timeout == 0 {
            Visibility::Visible
        } else {
            Visibility::InFlight {
                until: now + timeout,
                receipt,
            }
        };
        self.set_visibility(position, visibility);
        Ok(())
    }

    /// Removes every message. Sequence numbers and deduplication windows carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_age.clear();
        self.hidden.clear();
        self.eligible.clear();
        self.groups.clear();
        self.in_flight.clear();
    }

    /// Removes one message from the queue and every index.
    fn remove(&mut self, position: u64) -> Message {
        let message = self
            .messages
            .remove(&position)
            .expect("indexes only refer to stored messages");
        self.by_age.remove(&(message.sent_at, position));
        self.eligible.remove(&position);
        if let Some(until) = message.visibility.hidden_until() {
            self.hidden.remove(&(until, position));
        }
        if let Visibility::InFlight { receipt, .. } = message.visibility {
            self.in_flight.remove(&receipt);
        }
        if let Some(group) = &message.group_id {
            let members = self
                .groups
                .get_mut(group)
                .expect("grouped messages are indexed");
            members.remove(&position);
            match members.first() {
                Some(&head) => self.refresh_eligibility(head),
                None => {
                    self.groups.remove(group);
                }
            }
        }
        message
    }

    /// Changes a stored message's visibility and keeps `hidden`, `in_flight` and `eligible` in
    /// step with it.
    fn set_visibility(&mut self, position: u64, visibility: Visibility) {
        let message = self
            .messages
            .get_mut(&position)
            .expect("indexes only refer to stored messages");
        let old = std::mem::replace(&mut message.visibility, visibility);

        if let Some(until) = old.hidden_until() {
            self.hidden.remove(&(until, position));
        }
        if let Visibility::InFlight { receipt, .. } = old {
            self.in_flight.remove(&receipt);
        }
        if let Some(until) = visibility.hidden_until() {
            self.hidden.insert((until, position));
        }
        if let Visibility::InFlight { receipt, .. } = visibility {
            self.in_flight.insert(receipt, position);
        }
        self.refresh_eligibility(position);
    }

    /// A message is eligible when visible and, in a FIFO queue, the head of its group.
    fn refresh_eligibility(&mut self, position: u64) {
        let message = &self.messages[&position];
        let is_head = message.group_id.as_ref().is_none_or(|group| {
            self.groups.get(group).and_then(|members| members.first()) == Some(&position)
        });
        if is_head && message.visibility == Visibility::Visible {
            self.eligible.insert(position);
        } else {
            self.eligible.remove(&position);
        }
    }
}

/// The deduplication ids a FIFO queue accepted within the last [`DEDUPLICATION_WINDOW`] seconds.
#[derive(Debug, Default)]
struct Deduplication {
    windows: HashMap<String, (u64, Original)>,
    /// Ids in the order their windows opened, which is also time order, for cheap expiry.
    opened: VecDeque<(u64, String)>,
}

impl Deduplication {
    fn find(&mut self, id: &str, now: u64) -> Option<Original> {
        self.expire(now);
        self.windows.get(id).map(|&(_, original)| original)
    }

    fn record(&mut self, id: String, now: u64, original: Original) {
        self.expire(now);
        self.opened.push_back((now, id.clone()));
        self.windows.insert(id, (now, original));
    }

    fn expire(&mut self, now: u64) {
        while let Some((opened_at, _)) = self.opened.front() {
            if now - opened_at < DEDUPLICATION_WINDOW {
                break;
            }
            let (opened_at, id) = self.opened.pop_front().expect("front exists");
            // The id may have opened a newer window since; only drop the window this entry made.
            if self.windows.get(&id).is_some_and(|&(t, _)| t == opened_at) {
                self.windows.remove(&id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        queue: Queue,
        ids: Counter,
        receipts: Counter,
        now: u64,
    }

    impl Fixture {
        fn new(attributes: Attributes) -> Self {
            Self {
                queue: Queue::new(attributes),
                ids: Counter::default(),
                receipts: Counter::default(),
                now: 0,
            }
        }

        fn standard() -> Self {
            Self::new(Attributes::default())
        }

        fn fifo() -> Self {
            Self::new(Attributes {
                fifo: true,
                ..Attributes::default()
            })
        }

        fn send(&mut self, body: &str, group: Option<&str>, delay: u64) -> u64 {
            let id = self.ids.issue();
            let message = Message::new(id, body.into(), self.now, group.map(str::to_owned));
            self.queue.push(message, delay, None, self.now);
            id
        }

        fn tick(&mut self, seconds: u64) {
            self.now += seconds;
            self.queue.settle(self.now);
        }

        fn receive(&mut self, max: usize, timeout: u64) -> Vec<Delivery> {
            let received = self
                .queue
                .receive(max, timeout, None, &mut self.receipts, self.now);
            assert!(received.dead_letters.is_empty());
            received.deliveries
        }

        fn bodies(&mut self, max: usize, timeout: u64) -> Vec<String> {
            self.receive(max, timeout)
                .into_iter()
                .map(|d| d.body)
                .collect()
        }
    }

    #[test]
    fn delivers_in_arrival_order_and_hides_in_flight() {
        let mut f = Fixture::standard();
        f.send("a", None, 0);
        f.send("b", None, 0);
        f.send("c", None, 0);
        assert_eq!(f.bodies(2, 30), ["a", "b"]);
        assert_eq!(f.bodies(10, 30), ["c"]);
        assert!(f.bodies(10, 30).is_empty());
        assert_eq!(
            f.queue.counts(),
            Counts {
                visible: 0,
                in_flight: 3,
                delayed: 0
            }
        );
        f.tick(30);
        // Timed-out messages keep their place in arrival order.
        assert_eq!(f.bodies(10, 30), ["a", "b", "c"]);
    }

    #[test]
    fn zero_timeout_keeps_message_visible_but_receipt_unusable() {
        let mut f = Fixture::standard();
        f.send("a", None, 0);
        let first = f.receive(10, 0);
        assert_eq!(first.len(), 1, "examined once per receive");
        assert_eq!(f.queue.delete(first[0].receipt), Err(HandleExpired));
        let second = f.receive(10, 0);
        assert_eq!(second[0].receive_count, 2);
        assert_eq!(second[0].first_received_at, 0);
    }

    #[test]
    fn delay_hides_until_it_ends() {
        let mut f = Fixture::standard();
        f.send("late", None, 10);
        f.send("now", None, 0);
        assert_eq!(f.queue.counts().delayed, 1);
        assert_eq!(f.bodies(10, 30), ["now"]);
        f.tick(9);
        assert!(f.bodies(10, 30).is_empty());
        f.tick(1);
        assert_eq!(f.bodies(10, 30), ["late"]);
    }

    #[test]
    fn retention_removes_messages_in_any_state() {
        let mut f = Fixture::standard();
        f.send("a", None, 0);
        f.receive(1, 1000);
        f.tick(10);
        f.send("b", None, 0);
        f.tick(345_590);
        assert_eq!(f.queue.len(), 1);
        assert!(f.queue.in_flight.is_empty());
        f.queue.attributes_mut().retention_seconds = 60;
        f.queue.settle(f.now);
        assert!(f.queue.is_empty());
    }

    #[test]
    fn delete_and_change_visibility_need_the_latest_receipt() {
        let mut f = Fixture::standard();
        f.send("a", None, 0);
        let old = f.receive(1, 5)[0].receipt;
        f.tick(5);
        let new = f.receive(1, 5)[0].receipt;
        assert_eq!(
            f.queue.change_visibility(old, 100, f.now),
            Err(HandleExpired)
        );
        assert_eq!(f.queue.change_visibility(new, 100, f.now), Ok(()));
        f.tick(99);
        assert!(f.receive(1, 5).is_empty());
        assert_eq!(f.queue.change_visibility(new, 0, f.now), Ok(()));
        assert_eq!(f.queue.delete(new), Err(HandleExpired));
        let latest = f.receive(1, 5)[0].receipt;
        assert_eq!(f.queue.delete(latest), Ok(()));
        assert!(f.queue.is_empty());
    }

    #[test]
    fn fifo_delivers_one_message_per_group_head() {
        let mut f = Fixture::fifo();
        f.send("a1", Some("a"), 0);
        f.send("a2", Some("a"), 0);
        f.send("b1", Some("b"), 0);
        assert_eq!(f.bodies(10, 30), ["a1", "b1"]);
        assert!(
            f.bodies(10, 30).is_empty(),
            "heads in flight block their groups"
        );
        f.tick(30);
        let deliveries = f.receive(1, 30);
        assert_eq!(deliveries[0].body, "a1");
        assert_eq!(deliveries[0].sequence_number, Some(1));
        f.queue.delete(deliveries[0].receipt).unwrap();
        assert_eq!(f.bodies(10, 30), ["a2", "b1"]);
    }

    #[test]
    fn fifo_delayed_head_blocks_its_group() {
        let mut f = Fixture::fifo();
        f.send("a1", Some("a"), 10);
        f.send("a2", Some("a"), 0);
        assert!(f.bodies(10, 30).is_empty());
        f.tick(10);
        assert_eq!(f.bodies(10, 30), ["a1"]);
    }

    #[test]
    fn dead_letter_candidates_are_removed_and_the_scan_continues() {
        let mut f = Fixture::fifo();
        f.send("a1", Some("a"), 0);
        f.send("a2", Some("a"), 0);
        f.receive(1, 0);
        let received = f.queue.receive(10, 30, Some(1), &mut f.receipts, f.now);
        assert_eq!(received.dead_letters.len(), 1);
        assert_eq!(received.dead_letters[0].body, "a1");
        assert_eq!(received.dead_letters[0].receive_count, 0);
        let bodies: Vec<_> = received.deliveries.iter().map(|d| &d.body).collect();
        assert_eq!(bodies, ["a2"]);
    }

    #[test]
    fn purge_keeps_sequence_numbers() {
        let mut f = Fixture::fifo();
        f.send("a", Some("g"), 0);
        f.queue.purge();
        f.send("b", Some("g"), 0);
        assert_eq!(f.receive(1, 30)[0].sequence_number, Some(2));
    }

    #[test]
    fn deduplication_window_is_not_extended_by_duplicates() {
        let mut dedup = Deduplication::default();
        let original = Original {
            message_id: 1,
            sequence_number: 1,
        };
        dedup.record("x".into(), 0, original);
        assert_eq!(dedup.find("x", 299), Some(original));
        assert_eq!(dedup.find("x", 300), None);
        let renewed = Original {
            message_id: 2,
            sequence_number: 2,
        };
        dedup.record("x".into(), 300, renewed);
        assert_eq!(dedup.find("x", 599), Some(renewed));
        assert_eq!(dedup.find("x", 600), None);
        assert!(dedup.opened.is_empty());
    }
}
