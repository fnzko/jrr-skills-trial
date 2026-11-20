//! One queue: its messages in arrival order, plus the indexes that let receive find eligible,
//! visible messages without scanning delayed and in-flight ones.
//!
//! Visibility depends on the clock, so every method that can change it takes `now`. The indexes
//! say which side of the clock a message is on: `visible` holds messages whose `visible_at` has
//! passed, `waiting` holds the rest, keyed by when they wake. `Broker::tick` calls `promote`
//! for every queue, so the two stay in step with the clock.

use crate::attributes::QueueConfig;
use crate::ids::{GroupId, MessageId, ReceiptHandle, SequenceNumber};
use crate::time::{Instant, Seconds};
use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Bound;

/// A FIFO duplicate is one with the same deduplication id accepted less than this long ago.
const DEDUP_WINDOW: Seconds = Seconds::new(300);

/// A message's place in its queue's arrival order. Numbers are never reused within a queue.
/// For a FIFO queue this is the message's SequenceNumber.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Arrival(u64);

/// A message as it sits in a queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: Instant,
    /// The message is visible from this instant on. It is set to the send time plus the delay,
    /// and then to the end of each visibility timeout.
    visible_at: Instant,
    receive_count: u64,
    first_received_at: Option<Instant>,
    /// The handle of the latest delivery, if there has been one.
    receipt: Option<ReceiptHandle>,
    fifo: Option<FifoPosition>,
}

/// The group and sequence number of a message in a FIFO queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FifoPosition {
    group: GroupId,
    sequence: SequenceNumber,
}

impl FifoPosition {
    pub fn group(&self) -> &GroupId {
        &self.group
    }

    pub fn sequence(&self) -> SequenceNumber {
        self.sequence
    }
}

/// The state of a message at some instant (SPEC section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageState {
    Delayed,
    InFlight,
    Visible,
}

impl Message {
    pub fn id(&self) -> MessageId {
        self.id
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn sent_at(&self) -> Instant {
        self.sent_at
    }

    pub fn receive_count(&self) -> u64 {
        self.receive_count
    }

    /// None until the first delivery. A delivered message always has it.
    pub fn first_received_at(&self) -> Option<Instant> {
        self.first_received_at
    }

    pub fn receipt(&self) -> Option<ReceiptHandle> {
        self.receipt
    }

    pub fn fifo(&self) -> Option<&FifoPosition> {
        self.fifo.as_ref()
    }

    /// A message that was never delivered and is not yet visible is delayed; one that was
    /// delivered and is not yet visible is in flight.
    pub fn state(&self, now: Instant) -> MessageState {
        if now >= self.visible_at {
            MessageState::Visible
        } else if self.receive_count == 0 {
            MessageState::Delayed
        } else {
            MessageState::InFlight
        }
    }
}

/// A message being added to a queue, before the queue gives it an arrival and, in a FIFO queue,
/// a sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enqueue {
    id: MessageId,
    body: String,
    sent_at: Instant,
    visible_at: Instant,
    group: Option<GroupId>,
}

impl Enqueue {
    /// `group` is Some exactly when the target queue is FIFO. `Queue::append` checks this.
    pub fn new(
        id: MessageId,
        body: String,
        sent_at: Instant,
        visible_at: Instant,
        group: Option<GroupId>,
    ) -> Enqueue {
        Enqueue {
            id,
            body,
            sent_at,
            visible_at,
            group,
        }
    }

    /// The entry for moving `message` to a dead-letter queue. It keeps its id, body, send time
    /// and group, and is visible at once.
    pub fn dead_lettered(message: Message, now: Instant) -> Enqueue {
        Enqueue {
            id: message.id,
            body: message.body,
            sent_at: message.sent_at,
            visible_at: now,
            group: message.fifo.map(|position| position.group),
        }
    }
}

/// A send that was accepted by a FIFO queue, kept for deduplication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accepted {
    id: MessageId,
    sequence: SequenceNumber,
    accepted_at: Instant,
}

impl Accepted {
    pub fn new(id: MessageId, sequence: SequenceNumber, accepted_at: Instant) -> Accepted {
        Accepted {
            id,
            sequence,
            accepted_at,
        }
    }

    pub fn id(&self) -> MessageId {
        self.id
    }

    pub fn sequence(&self) -> SequenceNumber {
        self.sequence
    }
}

/// The messages a receive delivered and the ones it moved to a dead-letter queue.
#[derive(Debug, Default)]
pub struct Received {
    pub delivered: Vec<Arrival>,
    pub dead_lettered: Vec<Message>,
}

/// How many messages are in each state at some instant.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub visible: u64,
    pub in_flight: u64,
    pub delayed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DedupKey(String);

impl Borrow<str> for DedupKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[derive(Debug)]
pub struct Queue {
    config: QueueConfig,
    messages: BTreeMap<Arrival, Message>,
    /// The arrival the next append gets. Starts at 1, so the first FIFO message has sequence 1.
    next_arrival: u64,
    /// Eligible messages whose `visible_at` has passed. Eligible means: any message of a
    /// standard queue, and the head of its group in a FIFO queue.
    visible: BTreeSet<Arrival>,
    /// Eligible messages not yet visible, by the instant they become visible.
    waiting: BTreeSet<(Instant, Arrival)>,
    /// Every message by send time, so retention can drop the oldest first.
    by_sent: BTreeSet<(Instant, Arrival)>,
    /// The arrivals of each group in a FIFO queue, oldest first. The first is the group's head.
    groups: HashMap<GroupId, BTreeSet<Arrival>>,
    /// The latest handle of each message that has been delivered and is still in the queue.
    receipts: HashMap<ReceiptHandle, Arrival>,
    /// The accepted FIFO sends. Entries outlive their messages, as the spec requires.
    accepted: HashMap<DedupKey, Accepted>,
}

impl Queue {
    pub fn new(config: QueueConfig) -> Queue {
        Queue {
            config,
            messages: BTreeMap::new(),
            next_arrival: 1,
            visible: BTreeSet::new(),
            waiting: BTreeSet::new(),
            by_sent: BTreeSet::new(),
            groups: HashMap::new(),
            receipts: HashMap::new(),
            accepted: HashMap::new(),
        }
    }

    pub fn config(&self) -> &QueueConfig {
        &self.config
    }

    /// Replaces the attributes. A shorter retention takes effect at once: the caller runs
    /// `expire` next.
    pub fn set_config(&mut self, config: QueueConfig) {
        self.config = config;
    }

    /// The number of messages in the queue, in any state.
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    pub fn counts(&self, now: Instant) -> Counts {
        let mut counts = Counts::default();
        for message in self.messages.values() {
            match message.state(now) {
                MessageState::Visible => counts.visible += 1,
                MessageState::InFlight => counts.in_flight += 1,
                MessageState::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    pub fn message(&self, arrival: Arrival) -> &Message {
        self.messages
            .get(&arrival)
            .expect("an arrival in an index always names a message in the queue")
    }

    /// The first arrival after `after` that is visible and eligible.
    pub fn next_visible_after(&self, after: Bound<Arrival>) -> Option<Arrival> {
        self.visible
            .range((after, Bound::Unbounded))
            .next()
            .copied()
    }

    /// Adds a message at the end of the queue. Returns its SequenceNumber in a FIFO queue.
    pub fn append(&mut self, entry: Enqueue, now: Instant) -> Option<SequenceNumber> {
        let arrival = Arrival(self.next_arrival);
        self.next_arrival = self
            .next_arrival
            .checked_add(1)
            .expect("arrival numbers cannot run out in a u64");
        let fifo = match (self.config.fifo(), entry.group) {
            (true, Some(group)) => Some(FifoPosition {
                group,
                sequence: SequenceNumber::issued(arrival.0),
            }),
            (false, None) => None,
            _ => unreachable!(
                "the broker gives a FIFO queue a group on every send and a standard queue none"
            ),
        };
        let sequence = fifo.as_ref().map(|position| position.sequence);
        let group = fifo.as_ref().map(|position| position.group.clone());
        let visible_at = entry.visible_at;
        self.by_sent.insert((entry.sent_at, arrival));
        self.messages.insert(
            arrival,
            Message {
                id: entry.id,
                body: entry.body,
                sent_at: entry.sent_at,
                visible_at,
                receive_count: 0,
                first_received_at: None,
                receipt: None,
                fifo,
            },
        );
        if let Some(group) = group {
            self.groups.entry(group).or_default().insert(arrival);
        }
        if self.is_eligible(arrival) {
            self.index(arrival, visible_at, now);
        }
        sequence
    }

    /// Removes every message whose retention has passed.
    pub fn expire(&mut self, now: Instant) {
        let retention = self.config.retention_seconds();
        while let Some(&(sent_at, arrival)) = self.by_sent.first() {
            if now < sent_at + retention {
                break;
            }
            self.remove(arrival, now);
        }
    }

    /// Makes visible every waiting message whose time has come.
    pub fn promote(&mut self, now: Instant) {
        while let Some(&(visible_at, arrival)) = self.waiting.first() {
            if visible_at > now {
                break;
            }
            self.waiting.remove(&(visible_at, arrival));
            self.visible.insert(arrival);
        }
    }

    /// Removes every message, in any state. Sequence numbers and deduplication entries carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.visible.clear();
        self.waiting.clear();
        self.by_sent.clear();
        self.groups.clear();
        self.receipts.clear();
    }

    /// Delivers up to `max_messages` eligible messages in arrival order. Each one that has used up
    /// its delivery attempts goes to the dead-letter queue instead, and does not count. `issue`
    /// hands out a fresh receipt handle for each delivery.
    pub fn receive(
        &mut self,
        now: Instant,
        max_messages: u32,
        visibility_timeout: Seconds,
        mut issue: impl FnMut() -> ReceiptHandle,
    ) -> Received {
        let mut received = Received::default();
        let mut delivered: u32 = 0;
        let mut after = Bound::Unbounded;
        while delivered < max_messages {
            let Some(arrival) = self.next_visible_after(after) else {
                break;
            };
            after = Bound::Excluded(arrival);
            let message = self.message(arrival);
            let exhausted = self.config.dead_letter().is_some_and(|dead_letter| {
                message.receive_count >= u64::from(dead_letter.max_receive_count())
            });
            if exhausted {
                received.dead_lettered.push(self.remove(arrival, now));
            } else {
                self.deliver(arrival, now, visibility_timeout, &mut issue);
                received.delivered.push(arrival);
                delivered += 1;
            }
        }
        received
    }

    /// The arrival of the message that `receipt` is the latest handle of, if that message is in
    /// flight at `now`.
    pub fn in_flight(&self, receipt: ReceiptHandle, now: Instant) -> Option<Arrival> {
        let arrival = *self.receipts.get(&receipt)?;
        (now < self.message(arrival).visible_at).then_some(arrival)
    }

    /// Takes a message out of the queue, whatever its state.
    pub fn remove(&mut self, arrival: Arrival, now: Instant) -> Message {
        let message = self
            .messages
            .remove(&arrival)
            .expect("an arrival in an index always names a message in the queue");
        self.by_sent.remove(&(message.sent_at, arrival));
        self.unindex(arrival, message.visible_at);
        if let Some(receipt) = message.receipt {
            self.receipts.remove(&receipt);
        }
        if let Some(position) = &message.fifo {
            if let Some(next_head) = self.leave_group(&position.group, arrival) {
                let visible_at = self.message(next_head).visible_at;
                self.index(next_head, visible_at, now);
            }
        }
        message
    }

    /// Moves the end of a message's visibility timeout. Used by both a delivery and
    /// ChangeMessageVisibility.
    pub fn set_visible_at(&mut self, arrival: Arrival, visible_at: Instant, now: Instant) {
        let message = self
            .messages
            .get_mut(&arrival)
            .expect("an arrival in an index always names a message in the queue");
        let previous = std::mem::replace(&mut message.visible_at, visible_at);
        self.unindex(arrival, previous);
        if self.is_eligible(arrival) {
            self.index(arrival, visible_at, now);
        }
    }

    /// The most recent accepted send with this deduplication key, if it is still inside the
    /// window at `now`.
    pub fn recent_duplicate(&self, key: &str, now: Instant) -> Option<Accepted> {
        self.accepted
            .get(key)
            .copied()
            .filter(|accepted| now < accepted.accepted_at + DEDUP_WINDOW)
    }

    pub fn record_accepted(&mut self, key: &str, accepted: Accepted) {
        self.accepted.insert(DedupKey(key.to_owned()), accepted);
    }

    fn is_eligible(&self, arrival: Arrival) -> bool {
        match &self.message(arrival).fifo {
            None => true,
            Some(position) => {
                self.groups
                    .get(&position.group)
                    .and_then(|members| members.first())
                    == Some(&arrival)
            }
        }
    }

    fn index(&mut self, arrival: Arrival, visible_at: Instant, now: Instant) {
        if visible_at <= now {
            self.visible.insert(arrival);
        } else {
            self.waiting.insert((visible_at, arrival));
        }
    }

    fn unindex(&mut self, arrival: Arrival, visible_at: Instant) {
        self.visible.remove(&arrival);
        self.waiting.remove(&(visible_at, arrival));
    }

    /// Takes `arrival` out of its group. Returns the group's new head when `arrival` was the head
    /// and other members remain, since that member now becomes eligible.
    fn leave_group(&mut self, group: &GroupId, arrival: Arrival) -> Option<Arrival> {
        let members = self
            .groups
            .get_mut(group)
            .expect("a FIFO message's group is indexed while the message is queued");
        let was_head = members.first() == Some(&arrival);
        members.remove(&arrival);
        match members.first().copied() {
            None => {
                self.groups.remove(group);
                None
            }
            Some(head) if was_head => Some(head),
            Some(_) => None,
        }
    }

    fn deliver(
        &mut self,
        arrival: Arrival,
        now: Instant,
        visibility_timeout: Seconds,
        issue: &mut impl FnMut() -> ReceiptHandle,
    ) {
        let receipt = issue();
        let message = self
            .messages
            .get_mut(&arrival)
            .expect("an arrival in an index always names a message in the queue");
        message.receive_count += 1;
        message.first_received_at.get_or_insert(now);
        if let Some(previous) = message.receipt.replace(receipt) {
            self.receipts.remove(&previous);
        }
        self.receipts.insert(receipt, arrival);
        self.set_visible_at(arrival, now + visibility_timeout, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::{AttributeUpdate, DeadLetter};
    use crate::ids::QueueName;
    use serde_json::{json, Value};

    fn config(members: Value) -> QueueConfig {
        let update = AttributeUpdate::parse(Some(&members)).expect("test attributes are valid");
        let dead_letter = update
            .dead_letter()
            .expect("test dead-letter pair is complete")
            .map(|request| {
                let queue = QueueName::parse(request.queue()).expect("test queue name is valid");
                DeadLetter::from_request(queue, request)
            });
        QueueConfig::defaults().updated(&update, dead_letter)
    }

    fn at(seconds: u32) -> Instant {
        Instant::START + Seconds::new(seconds)
    }

    /// Appends a message sent at `now`, with the queue's default delay.
    fn send(queue: &mut Queue, now: Instant, body: &str, group: Option<&str>) {
        let id = MessageId::issued(1);
        let delay = queue.config().delay_seconds();
        let entry = Enqueue::new(
            id,
            body.to_owned(),
            now,
            now + delay,
            group.map(|g| GroupId::parse(g).expect("test group is valid")),
        );
        queue.append(entry, now);
    }

    fn receipts() -> impl FnMut() -> ReceiptHandle {
        let mut next = 0;
        move || {
            next += 1;
            ReceiptHandle::issued(next)
        }
    }

    fn bodies(queue: &Queue, received: &Received) -> Vec<String> {
        received
            .delivered
            .iter()
            .map(|&arrival| queue.message(arrival).body().to_owned())
            .collect()
    }

    #[test]
    fn standard_queue_delivers_in_arrival_order_and_hides_in_flight_messages() {
        let mut queue = Queue::new(config(json!({"VisibilityTimeout": 10})));
        for body in ["a", "b", "c"] {
            send(&mut queue, at(0), body, None);
        }
        let first = queue.receive(at(0), 2, Seconds::new(10), receipts());
        assert_eq!(bodies(&queue, &first), ["a", "b"]);
        let second = queue.receive(at(5), 5, Seconds::new(10), receipts());
        assert_eq!(bodies(&queue, &second), ["c"]);
        assert_eq!(
            queue.counts(at(5)),
            Counts {
                visible: 0,
                in_flight: 3,
                delayed: 0
            }
        );
    }

    #[test]
    fn expired_visibility_returns_the_message_to_its_arrival_position() {
        let mut queue = Queue::new(config(json!({})));
        send(&mut queue, at(0), "a", None);
        send(&mut queue, at(0), "b", None);
        let first = queue.receive(at(0), 1, Seconds::new(10), receipts());
        assert_eq!(bodies(&queue, &first), ["a"]);
        queue.promote(at(10));
        let again = queue.receive(at(10), 1, Seconds::new(10), receipts());
        assert_eq!(bodies(&queue, &again), ["a"]);
        assert_eq!(queue.message(again.delivered[0]).receive_count(), 2);
    }

    #[test]
    fn delayed_messages_wait_for_promote_and_then_join_the_visible_order() {
        let mut queue = Queue::new(config(json!({"DelaySeconds": 5})));
        send(&mut queue, at(0), "late", None);
        assert_eq!(queue.counts(at(4)).delayed, 1);
        let early = queue.receive(at(4), 1, Seconds::new(30), receipts());
        assert!(early.delivered.is_empty());
        queue.promote(at(5));
        let ready = queue.receive(at(5), 1, Seconds::new(30), receipts());
        assert_eq!(bodies(&queue, &ready), ["late"]);
    }

    #[test]
    fn zero_visibility_timeout_makes_the_message_visible_again_at_once() {
        let mut queue = Queue::new(config(json!({})));
        send(&mut queue, at(0), "a", None);
        let first = queue.receive(at(0), 1, Seconds::new(0), receipts());
        let receipt = queue
            .message(first.delivered[0])
            .receipt()
            .expect("delivered");
        assert_eq!(queue.in_flight(receipt, at(0)), None);
        let again = queue.receive(at(0), 1, Seconds::new(0), receipts());
        assert_eq!(bodies(&queue, &again), ["a"]);
    }

    #[test]
    fn fifo_group_delivers_only_its_head_until_the_head_leaves() {
        let mut queue = Queue::new(config(
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ));
        send(&mut queue, at(0), "g1-first", Some("g1"));
        send(&mut queue, at(0), "g1-second", Some("g1"));
        send(&mut queue, at(0), "g2-first", Some("g2"));
        let first = queue.receive(at(0), 10, Seconds::new(30), receipts());
        assert_eq!(bodies(&queue, &first), ["g1-first", "g2-first"]);

        let blocked = queue.receive(at(1), 10, Seconds::new(30), receipts());
        assert!(blocked.delivered.is_empty(), "head is in flight");

        queue.remove(first.delivered[0], at(1));
        let next = queue.receive(at(1), 10, Seconds::new(30), receipts());
        assert_eq!(bodies(&queue, &next), ["g1-second"]);
    }

    #[test]
    fn fifo_sequence_numbers_count_messages_from_one() {
        let mut queue = Queue::new(config(
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ));
        let entry = |body: &str| {
            Enqueue::new(
                MessageId::issued(1),
                body.to_owned(),
                at(0),
                at(0),
                GroupId::parse("g"),
            )
        };
        assert_eq!(
            queue.append(entry("a"), at(0)),
            Some(SequenceNumber::issued(1))
        );
        assert_eq!(
            queue.append(entry("b"), at(0)),
            Some(SequenceNumber::issued(2))
        );
    }

    #[test]
    fn retention_removes_messages_and_shortening_it_removes_them_at_once() {
        let mut queue = Queue::new(config(json!({"RetentionSeconds": 100})));
        send(&mut queue, at(0), "old", None);
        queue.expire(at(99));
        assert_eq!(queue.message_count(), 1);
        queue.expire(at(100));
        assert_eq!(queue.message_count(), 0);

        send(&mut queue, at(100), "young", None);
        send(&mut queue, at(150), "newer", None);
        queue.set_config(config(json!({"RetentionSeconds": 60})));
        queue.expire(at(160));
        assert_eq!(queue.message_count(), 1, "only the newer one survives");
    }

    #[test]
    fn receive_moves_messages_past_max_receive_count_to_the_dead_letter_list() {
        let mut queue = Queue::new(config(
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
        ));
        send(&mut queue, at(0), "a", None);
        send(&mut queue, at(0), "b", None);
        let first = queue.receive(at(0), 1, Seconds::new(30), receipts());
        assert_eq!(bodies(&queue, &first), ["a"]);
        let second = queue.receive(at(0), 1, Seconds::new(30), receipts());
        assert_eq!(bodies(&queue, &second), ["b"]);
        queue.promote(at(30));
        let third = queue.receive(at(30), 10, Seconds::new(30), receipts());
        assert!(third.delivered.is_empty());
        let moved: Vec<&str> = third.dead_lettered.iter().map(Message::body).collect();
        assert_eq!(moved, ["a", "b"]);
    }

    #[test]
    fn in_flight_accepts_only_the_latest_handle_of_a_message_still_in_flight() {
        let mut queue = Queue::new(config(json!({})));
        send(&mut queue, at(0), "a", None);
        let mut issued = receipts();
        let first = queue.receive(at(0), 1, Seconds::new(5), &mut issued);
        let old = queue
            .message(first.delivered[0])
            .receipt()
            .expect("delivered");
        queue.promote(at(5));
        let second = queue.receive(at(5), 1, Seconds::new(5), &mut issued);
        let latest = queue
            .message(second.delivered[0])
            .receipt()
            .expect("delivered");
        assert_eq!(queue.in_flight(old, at(5)), None);
        assert_eq!(queue.in_flight(latest, at(5)), Some(second.delivered[0]));
        assert_eq!(queue.in_flight(latest, at(10)), None);
    }

    #[test]
    fn purge_empties_the_queue_but_keeps_sequence_numbers() {
        let mut queue = Queue::new(config(
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ));
        send(&mut queue, at(0), "a", Some("g"));
        queue.purge();
        assert_eq!(queue.message_count(), 0);
        let entry = Enqueue::new(
            MessageId::issued(2),
            "b".into(),
            at(0),
            at(0),
            GroupId::parse("g"),
        );
        assert_eq!(queue.append(entry, at(0)), Some(SequenceNumber::issued(2)));
    }
}
