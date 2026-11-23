//! One queue: its messages in arrival order, their delivery state, retention and deduplication.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::attributes::QueueAttributes;
use crate::types::{GroupId, IdGenerator, MessageId, ReceiptHandle, Seconds, SequenceNumber, Time};

/// A queue holds at most this many messages, in any state.
pub const MAX_MESSAGES: usize = 100_000;

/// How long a deduplication id keeps rejecting repeated sends.
const DEDUPLICATION_WINDOW: Seconds = Seconds(300);

/// Position in a queue's arrival order. Never reused within one queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageState {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug)]
struct InFlight {
    handle: ReceiptHandle,
    until: Time,
}

#[derive(Debug)]
struct FifoPosition {
    group: GroupId,
    sequence: SequenceNumber,
}

/// A stored message. It carries `fifo` exactly when its queue is a FIFO queue.
#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: Time,
    fifo: Option<FifoPosition>,
    receive_count: u32,
    first_received_at: Option<Time>,
    delayed_until: Time,
    in_flight: Option<InFlight>,
}

impl Message {
    fn state(&self, now: Time) -> MessageState {
        if now < self.delayed_until {
            MessageState::Delayed
        } else if self.in_flight.as_ref().is_some_and(|f| now < f.until) {
            MessageState::InFlight
        } else {
            MessageState::Visible
        }
    }

    /// The message as it enters a dead-letter queue: same identity and content, counters reset.
    pub fn into_incoming(self) -> Incoming {
        Incoming {
            id: self.id,
            body: self.body,
            sent_at: self.sent_at,
            delayed_until: Time::START,
            group: self.fifo.map(|fifo| fifo.group),
        }
    }
}

/// A message about to be added to a queue. `group` is present exactly for FIFO queues.
#[derive(Debug)]
pub struct Incoming {
    pub id: MessageId,
    pub body: String,
    pub sent_at: Time,
    pub delayed_until: Time,
    pub group: Option<GroupId>,
}

/// What the sender learns about an added message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accepted {
    pub message_id: MessageId,
    pub sequence: Option<SequenceNumber>,
}

#[derive(Debug)]
struct Remembered {
    at: Time,
    accepted: Accepted,
}

/// Number of messages in each state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// A message as handed to a consumer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivered {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: String,
    pub receive_count: u32,
    pub sent_at: Time,
    pub first_received_at: Time,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<GroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}

/// Messages a receive moved out of the queue, and where they go.
#[derive(Debug)]
pub struct DeadLettered {
    pub queue: String,
    pub messages: Vec<Message>,
}

#[derive(Debug)]
pub struct Received {
    pub delivered: Vec<Delivered>,
    pub dead_lettered: Option<DeadLettered>,
}

/// The handle does not belong to a message that is in flight in this queue right now.
#[derive(Debug, PartialEq, Eq)]
pub struct NotInFlight;

#[derive(Debug)]
enum Step {
    Deliver(Arrival),
    DeadLetter(Arrival),
}

#[derive(Debug)]
pub struct Queue {
    attributes: QueueAttributes,
    messages: BTreeMap<Arrival, Message>,
    /// The same messages ordered by send time, so retention only looks at the oldest ones.
    by_sent_at: BTreeSet<(Time, Arrival)>,
    /// Latest handle of every message that has been delivered and still exists.
    handles: HashMap<ReceiptHandle, Arrival>,
    deduplication: HashMap<String, Remembered>,
    next_arrival: Arrival,
    next_sequence: SequenceNumber,
}

impl Queue {
    pub fn new(attributes: QueueAttributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            by_sent_at: BTreeSet::new(),
            handles: HashMap::new(),
            deduplication: HashMap::new(),
            next_arrival: Arrival(0),
            next_sequence: SequenceNumber::FIRST,
        }
    }

    pub fn attributes(&self) -> &QueueAttributes {
        &self.attributes
    }

    pub fn attributes_mut(&mut self) -> &mut QueueAttributes {
        &mut self.attributes
    }

    pub fn is_full(&self) -> bool {
        self.messages.len() >= MAX_MESSAGES
    }

    pub fn counts(&self, now: Time) -> Counts {
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

    /// Removes every message that is too old at `now`. Every operation calls this first, which
    /// makes retention hold at every moment without a timer.
    pub fn expire(&mut self, now: Time) {
        let retention = self.attributes.retention;
        while let Some(&(sent_at, arrival)) = self.by_sent_at.first() {
            if now < sent_at.plus(retention) {
                break;
            }
            self.remove(arrival);
        }
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
        self.handles.clear();
    }

    fn remove(&mut self, arrival: Arrival) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        self.by_sent_at.remove(&(message.sent_at, arrival));
        if let Some(in_flight) = &message.in_flight {
            self.handles.remove(&in_flight.handle);
        }
        Some(message)
    }

    /// The outcome of an earlier send with the same deduplication id, if its window is open.
    pub fn find_duplicate(&self, key: &str, now: Time) -> Option<Accepted> {
        self.deduplication
            .get(key)
            .filter(|remembered| now < remembered.at.plus(DEDUPLICATION_WINDOW))
            .map(|remembered| remembered.accepted)
    }

    /// Opens a deduplication window for `key`, replacing an expired one.
    pub fn remember(&mut self, key: String, now: Time, accepted: Accepted) {
        self.deduplication
            .insert(key, Remembered { at: now, accepted });
    }

    /// Adds a message at the end of the queue.
    pub fn append(&mut self, incoming: Incoming) -> Accepted {
        let arrival = self.next_arrival;
        self.next_arrival = Arrival(arrival.0 + 1);
        let fifo = incoming.group.map(|group| {
            let sequence = self.next_sequence;
            self.next_sequence = sequence.next();
            FifoPosition { group, sequence }
        });
        let accepted = Accepted {
            message_id: incoming.id,
            sequence: fifo.as_ref().map(|fifo| fifo.sequence),
        };
        self.by_sent_at.insert((incoming.sent_at, arrival));
        self.messages.insert(
            arrival,
            Message {
                id: incoming.id,
                body: incoming.body,
                sent_at: incoming.sent_at,
                fifo,
                receive_count: 0,
                first_received_at: None,
                delayed_until: incoming.delayed_until,
                in_flight: None,
            },
        );
        accepted
    }

    /// Delivers up to `limit` eligible messages in arrival order, each invisible for `visibility`.
    pub fn receive(
        &mut self,
        now: Time,
        limit: usize,
        visibility: Seconds,
        ids: &mut IdGenerator,
    ) -> Received {
        let plan = self.plan_receive(now, limit);
        let mut delivered = Vec::new();
        let mut dead = Vec::new();
        for step in plan {
            match step {
                Step::DeadLetter(arrival) => dead.extend(self.remove(arrival)),
                Step::Deliver(arrival) => {
                    let handle = ids.next_handle();
                    let message = self
                        .messages
                        .get_mut(&arrival)
                        .expect("a planned arrival is still stored: nothing ran in between");
                    let previous = message.in_flight.replace(InFlight {
                        handle,
                        until: now.plus(visibility),
                    });
                    message.receive_count = message.receive_count.saturating_add(1);
                    let first_received_at = *message.first_received_at.get_or_insert(now);
                    delivered.push(Delivered {
                        message_id: message.id,
                        receipt_handle: handle,
                        body: message.body.clone(),
                        receive_count: message.receive_count,
                        sent_at: message.sent_at,
                        first_received_at,
                        group_id: message.fifo.as_ref().map(|f| f.group.clone()),
                        sequence_number: message.fifo.as_ref().map(|f| f.sequence),
                    });
                    if let Some(previous) = previous {
                        self.handles.remove(&previous.handle);
                    }
                    self.handles.insert(handle, arrival);
                }
            }
        }
        let dead_lettered = match &self.attributes.dead_letter {
            Some(dead_letter) if !dead.is_empty() => Some(DeadLettered {
                queue: dead_letter.queue.clone(),
                messages: dead,
            }),
            _ => None,
        };
        Received {
            delivered,
            dead_lettered,
        }
    }

    /// Decides what a receive does, without changing anything. The scan visits each message once,
    /// in arrival order, and remembers which groups already have an earlier message in the queue.
    fn plan_receive(&self, now: Time, limit: usize) -> Vec<Step> {
        let mut plan = Vec::new();
        let mut delivering = 0;
        let mut groups_in_queue: HashSet<&GroupId> = HashSet::new();
        for (&arrival, message) in &self.messages {
            if delivering == limit {
                break;
            }
            let group = message.fifo.as_ref().map(|fifo| &fifo.group);
            if group.is_some_and(|group| groups_in_queue.contains(group)) {
                continue;
            }
            if message.state(now) != MessageState::Visible {
                groups_in_queue.extend(group);
                continue;
            }
            let too_often = self
                .attributes
                .dead_letter
                .as_ref()
                .is_some_and(|d| message.receive_count >= d.max_receive_count);
            if too_often {
                // The message leaves the queue, so it no longer holds back its group.
                plan.push(Step::DeadLetter(arrival));
            } else {
                groups_in_queue.extend(group);
                plan.push(Step::Deliver(arrival));
                delivering += 1;
            }
        }
        plan
    }

    fn find_in_flight(&self, handle: ReceiptHandle, now: Time) -> Result<Arrival, NotInFlight> {
        let arrival = *self.handles.get(&handle).ok_or(NotInFlight)?;
        let message = self.messages.get(&arrival).ok_or(NotInFlight)?;
        let current = message
            .in_flight
            .as_ref()
            .is_some_and(|f| f.handle == handle);
        if current && message.state(now) == MessageState::InFlight {
            Ok(arrival)
        } else {
            Err(NotInFlight)
        }
    }

    pub fn delete(&mut self, handle: ReceiptHandle, now: Time) -> Result<(), NotInFlight> {
        let arrival = self.find_in_flight(handle, now)?;
        self.remove(arrival);
        Ok(())
    }

    /// Makes the message invisible for `timeout` counted from `now`.
    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: Time,
        timeout: Seconds,
    ) -> Result<(), NotInFlight> {
        let arrival = self.find_in_flight(handle, now)?;
        let in_flight = self
            .messages
            .get_mut(&arrival)
            .and_then(|message| message.in_flight.as_mut())
            .ok_or(NotInFlight)?;
        in_flight.until = now.plus(timeout);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::{AttributePatch, DeadLetter};

    fn standard() -> Queue {
        Queue::new(QueueAttributes::from_patch(
            &AttributePatch::default(),
            None,
        ))
    }

    fn fifo() -> Queue {
        let patch = AttributePatch {
            fifo: Some(true),
            ..AttributePatch::default()
        };
        Queue::new(QueueAttributes::from_patch(&patch, None))
    }

    fn incoming(ids: &mut IdGenerator, body: &str, sent_at: u64, group: Option<&str>) -> Incoming {
        Incoming {
            id: ids.next_message(),
            body: body.into(),
            sent_at: Time::START.plus(Seconds(u32::try_from(sent_at).unwrap())),
            delayed_until: Time::START,
            group: group.map(|g| GroupId::parse(g.into()).unwrap()),
        }
    }

    fn at(seconds: u32) -> Time {
        Time::START.plus(Seconds(seconds))
    }

    fn bodies(received: &Received) -> Vec<&str> {
        received.delivered.iter().map(|d| d.body.as_str()).collect()
    }

    #[test]
    fn a_message_keeps_its_place_after_its_visibility_timeout_ends() {
        let mut ids = IdGenerator::new();
        let mut queue = standard();
        for body in ["a", "b", "c"] {
            queue.append(incoming(&mut ids, body, 0, None));
        }
        let first = queue.receive(at(0), 1, Seconds(10), &mut ids);
        assert_eq!(bodies(&first), ["a"]);
        let second = queue.receive(at(0), 2, Seconds(10), &mut ids);
        assert_eq!(bodies(&second), ["b", "c"]);
        let again = queue.receive(at(10), 3, Seconds(10), &mut ids);
        assert_eq!(bodies(&again), ["a", "b", "c"]);
        let counts: Vec<u32> = again.delivered.iter().map(|d| d.receive_count).collect();
        assert_eq!(counts, [2, 2, 2]);
    }

    #[test]
    fn fifo_delivers_one_message_per_group_and_blocks_a_group_with_a_message_in_flight() {
        let mut ids = IdGenerator::new();
        let mut queue = fifo();
        for (body, group) in [("a1", "a"), ("a2", "a"), ("b1", "b")] {
            queue.append(incoming(&mut ids, body, 0, Some(group)));
        }
        assert_eq!(
            bodies(&queue.receive(at(0), 10, Seconds(30), &mut ids)),
            ["a1", "b1"]
        );
        assert_eq!(
            bodies(&queue.receive(at(1), 10, Seconds(30), &mut ids)),
            Vec::<&str>::new()
        );
        assert_eq!(
            bodies(&queue.receive(at(30), 10, Seconds(30), &mut ids)),
            ["a1", "b1"]
        );
    }

    #[test]
    fn delivered_fifo_messages_carry_group_and_sequence() {
        let mut ids = IdGenerator::new();
        let mut queue = fifo();
        queue.append(incoming(&mut ids, "x", 0, Some("g")));
        let received = queue.receive(at(0), 1, Seconds(5), &mut ids);
        let delivered = &received.delivered[0];
        assert_eq!(
            delivered.group_id,
            Some(GroupId::parse("g".into()).unwrap())
        );
        assert_eq!(delivered.sequence_number, Some(SequenceNumber::FIRST));
        assert_eq!(delivered.first_received_at, at(0));
    }

    #[test]
    fn messages_over_the_receive_limit_are_dead_lettered_and_release_their_group() {
        let mut ids = IdGenerator::new();
        let mut queue = fifo();
        queue.attributes_mut().dead_letter = Some(DeadLetter {
            queue: "dead.fifo".into(),
            max_receive_count: 1,
        });
        queue.append(incoming(&mut ids, "a1", 0, Some("a")));
        queue.append(incoming(&mut ids, "a2", 0, Some("a")));
        assert_eq!(
            bodies(&queue.receive(at(0), 1, Seconds(0), &mut ids)),
            ["a1"]
        );
        let second = queue.receive(at(0), 1, Seconds(0), &mut ids);
        assert_eq!(bodies(&second), ["a2"]);
        let moved = second.dead_lettered.expect("a1 was received once already");
        assert_eq!(moved.queue, "dead.fifo");
        assert_eq!(moved.messages.len(), 1);
        assert_eq!(queue.messages.len(), 1);
    }

    #[test]
    fn retention_removes_messages_whatever_their_state() {
        let mut ids = IdGenerator::new();
        let mut queue = standard();
        queue.append(incoming(&mut ids, "old", 0, None));
        queue.append(incoming(&mut ids, "new", 100, None));
        let first = queue.receive(at(100), 1, Seconds(1000), &mut ids);
        assert_eq!(bodies(&first), ["old"]);
        let handle = first.delivered[0].receipt_handle;
        queue.attributes_mut().retention = Seconds(100);
        queue.expire(at(100));
        assert_eq!(queue.messages.len(), 1);
        assert_eq!(queue.delete(handle, at(100)), Err(NotInFlight));
        queue.attributes_mut().retention = Seconds(1000);
        queue.expire(at(100));
        assert_eq!(queue.messages.len(), 1);
    }

    #[test]
    fn handles_stop_working_after_redelivery_or_visibility_change() {
        let mut ids = IdGenerator::new();
        let mut queue = standard();
        queue.append(incoming(&mut ids, "a", 0, None));
        let first = queue.receive(at(0), 1, Seconds(10), &mut ids).delivered[0].receipt_handle;
        assert_eq!(queue.change_visibility(first, at(5), Seconds(20)), Ok(()));
        assert_eq!(
            queue.receive(at(24), 1, Seconds(10), &mut ids).delivered,
            []
        );
        let second = queue.receive(at(25), 1, Seconds(10), &mut ids).delivered[0].receipt_handle;
        assert_eq!(queue.delete(first, at(25)), Err(NotInFlight));
        assert_eq!(queue.change_visibility(second, at(26), Seconds(0)), Ok(()));
        assert_eq!(queue.delete(second, at(26)), Err(NotInFlight));
    }

    #[test]
    fn deduplication_window_is_not_extended_by_duplicates() {
        let mut ids = IdGenerator::new();
        let mut queue = fifo();
        let accepted = queue.append(incoming(&mut ids, "a", 0, Some("g")));
        queue.remember("k".into(), at(0), accepted);
        assert_eq!(queue.find_duplicate("k", at(299)), Some(accepted));
        assert_eq!(queue.find_duplicate("k", at(300)), None);
        assert_eq!(queue.find_duplicate("other", at(0)), None);
    }

    #[test]
    fn counts_split_messages_by_state() {
        let mut ids = IdGenerator::new();
        let mut queue = standard();
        let mut delayed = incoming(&mut ids, "d", 0, None);
        delayed.delayed_until = at(50);
        queue.append(delayed);
        queue.append(incoming(&mut ids, "a", 0, None));
        queue.append(incoming(&mut ids, "b", 0, None));
        queue.receive(at(0), 1, Seconds(10), &mut ids);
        assert_eq!(
            queue.counts(at(0)),
            Counts {
                visible: 1,
                in_flight: 1,
                delayed: 1
            }
        );
    }
}
