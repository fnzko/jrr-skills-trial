//! One queue: its messages in arrival order and everything that depends on time.
//!
//! Time never advances inside this module. Callers pass `now` and must call
//! [`Queue::expire`] before reading or changing a queue, so retention holds at every moment.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::attributes::{Attributes, QueueKind};
use crate::error::{Error, ErrorCode};
use crate::types::{
    DeduplicationId, GroupId, IdGenerator, MessageId, ReceiptHandle, Seconds, SequenceNumber, Time,
};

pub const MAX_MESSAGES: usize = 100_000;
const DEDUPLICATION_WINDOW: Seconds = Seconds(300);

/// Position in the order messages were added to this queue; never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival(u64);

/// Why a message is not available right now, if it is not.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Hold {
    Free,
    Delayed { until: Time },
    InFlight { until: Time, handle: ReceiptHandle },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct FifoPlacement {
    pub group_id: GroupId,
    pub sequence_number: SequenceNumber,
}

#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: Time,
    receive_count: u32,
    first_received_at: Option<Time>,
    fifo: Option<FifoPlacement>,
    hold: Hold,
}

impl Message {
    fn state(&self, now: Time) -> State {
        match self.hold {
            Hold::Delayed { until } if now < until => State::Delayed,
            Hold::InFlight { until, .. } if now < until => State::InFlight,
            _ => State::Visible,
        }
    }

    fn is_in_flight_with(&self, handle: ReceiptHandle, now: Time) -> bool {
        matches!(self.hold, Hold::InFlight { until, handle: latest } if now < until && latest == handle)
    }
}

/// A message as handed to a consumer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: String,
    pub receive_count: u32,
    pub sent_at: Time,
    pub first_received_at: Time,
    #[serde(flatten)]
    pub fifo: Option<FifoPlacement>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// What a `SendMessage` asks of one queue, after parameter validation.
#[derive(Debug)]
pub struct SendRequest {
    pub body: String,
    pub delay: Option<Seconds>,
    pub group: Option<GroupId>,
    pub deduplication_id: Option<DeduplicationId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub message_id: MessageId,
    pub sequence_number: Option<SequenceNumber>,
}

/// The outcome of one receive: what to hand out, and what must move to the dead-letter queue.
#[derive(Debug)]
pub struct Received {
    pub deliveries: Vec<Delivery>,
    pub dead: Vec<Message>,
}

/// A FIFO send that opened a deduplication window.
#[derive(Debug, Clone, Copy)]
struct Accepted {
    at: Time,
    message_id: MessageId,
    sequence_number: SequenceNumber,
}

#[derive(Debug)]
pub struct Queue {
    attributes: Attributes,
    messages: BTreeMap<Arrival, Message>,
    /// Mirrors `messages`, ordered by send time, so expiry never scans the queue.
    by_age: BTreeSet<(Time, Arrival)>,
    /// Latest handles handed out by this queue; entries may be stale and are checked on use.
    handles: HashMap<ReceiptHandle, Arrival>,
    /// Keyed by deduplication id; outlives the messages it was accepted for.
    deduplication: HashMap<String, Accepted>,
    next_arrival: u64,
    next_sequence: SequenceNumber,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            by_age: BTreeSet::new(),
            handles: HashMap::new(),
            deduplication: HashMap::new(),
            next_arrival: 0,
            next_sequence: SequenceNumber::FIRST,
        }
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    pub fn attributes_mut(&mut self) -> &mut Attributes {
        &mut self.attributes
    }

    pub fn is_fifo(&self) -> bool {
        self.attributes.kind.is_fifo()
    }

    /// Removes every message whose retention has run out.
    pub fn expire(&mut self, now: Time) {
        while let Some(&(sent_at, arrival)) = self.by_age.first() {
            if now < sent_at + self.attributes.retention {
                break;
            }
            // `by_age` mirrors `messages`, so this removes the entry that was just read.
            self.remove(arrival);
        }
    }

    pub fn counts(&self, now: Time) -> Counts {
        let mut counts = Counts::default();
        for message in self.messages.values() {
            match message.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_age.clear();
        self.handles.clear();
    }

    fn remove(&mut self, arrival: Arrival) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        self.by_age.remove(&(message.sent_at, arrival));
        if let Hold::InFlight { handle, .. } = message.hold {
            self.handles.remove(&handle);
        }
        Some(message)
    }

    fn push(&mut self, message: Message) {
        let arrival = Arrival(self.next_arrival);
        self.next_arrival += 1;
        self.by_age.insert((message.sent_at, arrival));
        self.messages.insert(arrival, message);
    }

    pub fn send(
        &mut self,
        now: Time,
        ids: &mut IdGenerator,
        request: SendRequest,
    ) -> Result<Sent, Error> {
        let SendRequest {
            body,
            delay,
            group,
            deduplication_id,
        } = request;

        let (group, delay) = match self.attributes.kind {
            QueueKind::Standard => {
                if group.is_some() || deduplication_id.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                (None, delay.unwrap_or(self.attributes.delay))
            }
            QueueKind::Fifo {
                content_based_deduplication,
            } => {
                if delay.is_some() {
                    return Err(unsupported("DelaySeconds is not supported on a FIFO queue"));
                }
                let group = group.ok_or_else(|| Error::missing("GroupId"))?;
                if deduplication_id.is_none() && !content_based_deduplication {
                    return Err(Error::missing("DeduplicationId"));
                }
                (Some(group), self.attributes.delay)
            }
        };

        let key = deduplication_id
            .as_ref()
            .map_or(body.as_str(), DeduplicationId::as_str);
        if group.is_some()
            && let Some(original) = self
                .deduplication
                .get(key)
                .filter(|original| now < original.at + DEDUPLICATION_WINDOW)
        {
            return Ok(Sent {
                message_id: original.message_id,
                sequence_number: Some(original.sequence_number),
            });
        }

        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("queue already holds {MAX_MESSAGES} messages"),
            ));
        }

        let message_id = ids.next_message();
        let fifo = group.map(|group_id| FifoPlacement {
            group_id,
            sequence_number: self.next_sequence.take(),
        });
        if let Some(placement) = &fifo {
            self.deduplication.insert(
                key.to_owned(),
                Accepted {
                    at: now,
                    message_id,
                    sequence_number: placement.sequence_number,
                },
            );
        }
        let sent = Sent {
            message_id,
            sequence_number: fifo.as_ref().map(|placement| placement.sequence_number),
        };
        self.push(Message {
            id: message_id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            fifo,
            hold: if delay == Seconds(0) {
                Hold::Free
            } else {
                Hold::Delayed { until: now + delay }
            },
        });
        Ok(sent)
    }

    pub fn receive(
        &mut self,
        now: Time,
        max_messages: usize,
        visibility_timeout: Seconds,
        ids: &mut IdGenerator,
    ) -> Received {
        let mut deliveries = Vec::new();
        let mut dead = Vec::new();
        // Groups with an earlier message that stays in the queue; their later messages wait.
        let mut blocked_groups: HashSet<GroupId> = HashSet::new();

        for (&arrival, message) in &mut self.messages {
            if deliveries.len() == max_messages {
                break;
            }
            if let Some(placement) = &message.fifo
                && blocked_groups.contains(&placement.group_id)
            {
                continue;
            }
            if message.state(now) == State::Visible {
                let exhausted = self
                    .attributes
                    .dead_letter
                    .as_ref()
                    .is_some_and(|config| message.receive_count >= config.max_receive_count);
                if exhausted {
                    dead.push(arrival);
                    continue;
                }
                if let Hold::InFlight { handle: old, .. } = message.hold {
                    self.handles.remove(&old);
                }
                let handle = ids.next_handle();
                message.receive_count += 1;
                let first_received_at = *message.first_received_at.get_or_insert(now);
                message.hold = Hold::InFlight {
                    until: now + visibility_timeout,
                    handle,
                };
                self.handles.insert(handle, arrival);
                deliveries.push(Delivery {
                    message_id: message.id,
                    receipt_handle: handle,
                    body: message.body.clone(),
                    receive_count: message.receive_count,
                    sent_at: message.sent_at,
                    first_received_at,
                    fifo: message.fifo.clone(),
                });
            }
            if let Some(placement) = &message.fifo {
                blocked_groups.insert(placement.group_id.clone());
            }
        }

        let dead = dead
            .into_iter()
            .filter_map(|arrival| self.remove(arrival))
            .collect();
        Received { deliveries, dead }
    }

    /// Adds a message moved from another queue as a fresh, visible message.
    ///
    /// It keeps its id, body, send time and group. Ordinary limits and deduplication
    /// do not apply; retention still counts from the original send time.
    pub fn accept_dead_letter(&mut self, moved: Message, now: Time) {
        let fifo = moved.fifo.map(|placement| FifoPlacement {
            group_id: placement.group_id,
            sequence_number: self.next_sequence.take(),
        });
        let message = Message {
            receive_count: 0,
            first_received_at: None,
            fifo,
            hold: Hold::Free,
            ..moved
        };
        if now < message.sent_at + self.attributes.retention {
            self.push(message);
        }
    }

    pub fn delete(&mut self, handle: ReceiptHandle, now: Time) -> Result<(), Error> {
        let arrival = self.live_arrival(handle, now)?;
        self.remove(arrival);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: Time,
        timeout: Seconds,
    ) -> Result<(), Error> {
        let arrival = self.live_arrival(handle, now)?;
        let message = self
            .messages
            .get_mut(&arrival)
            .expect("live_arrival only returns arrivals of messages in this queue");
        message.hold = Hold::InFlight {
            until: now + timeout,
            handle,
        };
        Ok(())
    }

    /// The message `handle` still controls: its latest handle, and the message is in flight.
    fn live_arrival(&self, handle: ReceiptHandle, now: Time) -> Result<Arrival, Error> {
        self.handles
            .get(&handle)
            .copied()
            .filter(|arrival| {
                self.messages
                    .get(arrival)
                    .is_some_and(|message| message.is_in_flight_with(handle, now))
            })
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleExpired,
                    format!("receipt handle {handle} is not in use for this queue"),
                )
            })
    }
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::{AttributeSet, DeadLetter};
    use crate::types::QueueName;

    fn standard() -> Queue {
        Queue::new(Attributes::from_request(&AttributeSet::default(), None))
    }

    fn fifo(content_based: bool) -> Queue {
        let request = AttributeSet {
            fifo: Some(true),
            content_based_deduplication: Some(content_based),
            ..AttributeSet::default()
        };
        Queue::new(Attributes::from_request(&request, None))
    }

    fn plain(body: &str) -> SendRequest {
        SendRequest {
            body: body.to_owned(),
            delay: None,
            group: None,
            deduplication_id: None,
        }
    }

    fn grouped(body: &str, group: &str) -> SendRequest {
        SendRequest {
            group: Some(GroupId::parse(group).unwrap()),
            ..plain(body)
        }
    }

    fn bodies(received: &Received) -> Vec<&str> {
        received
            .deliveries
            .iter()
            .map(|d| d.body.as_str())
            .collect()
    }

    #[test]
    fn receives_in_arrival_order_and_hides_delivered_messages() {
        let mut ids = IdGenerator::default();
        let mut queue = standard();
        for body in ["a", "b", "c"] {
            queue.send(Time(0), &mut ids, plain(body)).unwrap();
        }
        let first = queue.receive(Time(0), 2, Seconds(30), &mut ids);
        assert_eq!(bodies(&first), ["a", "b"]);
        let second = queue.receive(Time(0), 10, Seconds(30), &mut ids);
        assert_eq!(bodies(&second), ["c"]);
        assert_eq!(
            queue.counts(Time(0)),
            Counts {
                visible: 0,
                in_flight: 3,
                delayed: 0
            }
        );
    }

    #[test]
    fn message_returns_to_its_place_when_visibility_ends() {
        let mut ids = IdGenerator::default();
        let mut queue = standard();
        queue.send(Time(0), &mut ids, plain("a")).unwrap();
        queue.send(Time(0), &mut ids, plain("b")).unwrap();
        queue.receive(Time(0), 1, Seconds(10), &mut ids);
        assert_eq!(
            bodies(&queue.receive(Time(9), 1, Seconds(10), &mut ids)),
            ["b"]
        );
        let again = queue.receive(Time(10), 1, Seconds(10), &mut ids);
        assert_eq!(bodies(&again), ["a"]);
        assert_eq!(again.deliveries[0].receive_count, 2);
        assert_eq!(again.deliveries[0].first_received_at, Time(0));
    }

    #[test]
    fn delayed_message_becomes_visible_at_the_delay() {
        let mut ids = IdGenerator::default();
        let mut queue = standard();
        let request = SendRequest {
            delay: Some(Seconds(5)),
            ..plain("a")
        };
        queue.send(Time(2), &mut ids, request).unwrap();
        assert_eq!(queue.counts(Time(6)).delayed, 1);
        assert!(
            queue
                .receive(Time(6), 1, Seconds(30), &mut ids)
                .deliveries
                .is_empty()
        );
        assert_eq!(
            bodies(&queue.receive(Time(7), 1, Seconds(30), &mut ids)),
            ["a"]
        );
    }

    #[test]
    fn retention_removes_messages_in_every_state() {
        let mut ids = IdGenerator::default();
        let mut queue = standard();
        queue.send(Time(0), &mut ids, plain("a")).unwrap();
        queue.receive(Time(0), 1, Seconds(1000), &mut ids);
        queue.send(Time(30), &mut ids, plain("b")).unwrap();
        queue.attributes_mut().retention = Seconds(60);
        queue.expire(Time(59));
        assert_eq!(
            queue.counts(Time(59)).in_flight + queue.counts(Time(59)).visible,
            2
        );
        queue.expire(Time(60));
        assert_eq!(
            queue.counts(Time(60)),
            Counts {
                visible: 1,
                in_flight: 0,
                delayed: 0
            }
        );
        queue.attributes_mut().retention = Seconds(1_000_000);
        queue.expire(Time(61));
        assert_eq!(queue.counts(Time(61)).visible, 1);
    }

    #[test]
    fn handle_works_until_redelivery_or_timeout() {
        let mut ids = IdGenerator::default();
        let mut queue = standard();
        queue.send(Time(0), &mut ids, plain("a")).unwrap();
        let first = queue.receive(Time(0), 1, Seconds(10), &mut ids).deliveries[0].receipt_handle;
        let second = queue.receive(Time(10), 1, Seconds(10), &mut ids).deliveries[0].receipt_handle;
        assert_eq!(
            queue.delete(first, Time(10)).unwrap_err().code,
            ErrorCode::ReceiptHandleExpired
        );
        assert_eq!(
            queue.delete(second, Time(20)).unwrap_err().code,
            ErrorCode::ReceiptHandleExpired
        );
        queue.delete(second, Time(19)).unwrap();
        assert_eq!(queue.counts(Time(19)), Counts::default());
    }

    #[test]
    fn zero_visibility_makes_the_handle_unusable() {
        let mut ids = IdGenerator::default();
        let mut queue = standard();
        queue.send(Time(0), &mut ids, plain("a")).unwrap();
        let handle = queue.receive(Time(0), 1, Seconds(0), &mut ids).deliveries[0].receipt_handle;
        assert_eq!(
            queue.delete(handle, Time(0)).unwrap_err().code,
            ErrorCode::ReceiptHandleExpired
        );
    }

    #[test]
    fn change_visibility_replaces_the_end_of_the_timeout() {
        let mut ids = IdGenerator::default();
        let mut queue = standard();
        queue.send(Time(0), &mut ids, plain("a")).unwrap();
        let handle = queue.receive(Time(0), 1, Seconds(10), &mut ids).deliveries[0].receipt_handle;
        queue
            .change_visibility(handle, Time(5), Seconds(20))
            .unwrap();
        assert_eq!(queue.counts(Time(24)).in_flight, 1);
        assert_eq!(queue.counts(Time(25)).visible, 1);
        queue
            .change_visibility(handle, Time(6), Seconds(0))
            .unwrap();
        assert_eq!(queue.counts(Time(6)).visible, 1);
    }

    #[test]
    fn standard_queue_rejects_group_and_fifo_rejects_delay() {
        let mut ids = IdGenerator::default();
        let err = standard()
            .send(Time(0), &mut ids, grouped("a", "g"))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::UnsupportedOperation);
        let delayed = SendRequest {
            delay: Some(Seconds(0)),
            ..grouped("a", "g")
        };
        let err = fifo(true).send(Time(0), &mut ids, delayed).unwrap_err();
        assert_eq!(err.code, ErrorCode::UnsupportedOperation);
    }

    #[test]
    fn fifo_requires_group_and_deduplication_source() {
        let mut ids = IdGenerator::default();
        let err = fifo(true).send(Time(0), &mut ids, plain("a")).unwrap_err();
        assert_eq!(err, Error::missing("GroupId"));
        let err = fifo(false)
            .send(Time(0), &mut ids, grouped("a", "g"))
            .unwrap_err();
        assert_eq!(err, Error::missing("DeduplicationId"));
    }

    #[test]
    fn duplicate_send_returns_the_original_and_window_is_not_extended() {
        let mut ids = IdGenerator::default();
        let mut queue = fifo(true);
        let first = queue.send(Time(0), &mut ids, grouped("a", "g")).unwrap();
        let duplicate = queue
            .send(Time(299), &mut ids, grouped("a", "other"))
            .unwrap();
        assert_eq!(duplicate, first);
        let reopened = queue.send(Time(300), &mut ids, grouped("a", "g")).unwrap();
        assert_ne!(reopened.message_id, first.message_id);
        assert_eq!(reopened.sequence_number, Some(SequenceNumber(2)));
    }

    #[test]
    fn deduplication_outlives_the_message() {
        let mut ids = IdGenerator::default();
        let mut queue = fifo(true);
        let first = queue.send(Time(0), &mut ids, grouped("a", "g")).unwrap();
        queue.purge();
        let again = queue.send(Time(1), &mut ids, grouped("a", "g")).unwrap();
        assert_eq!(again, first);
        assert_eq!(queue.counts(Time(1)), Counts::default());
    }

    #[test]
    fn explicit_deduplication_id_wins_over_the_body() {
        let mut ids = IdGenerator::default();
        let mut queue = fifo(true);
        let with_id = |body: &str, id: &str| SendRequest {
            deduplication_id: Some(DeduplicationId::parse(id).unwrap()),
            ..grouped(body, "g")
        };
        let first = queue.send(Time(0), &mut ids, with_id("a", "x")).unwrap();
        let same_id = queue.send(Time(1), &mut ids, with_id("b", "x")).unwrap();
        let other_id = queue.send(Time(1), &mut ids, with_id("a", "y")).unwrap();
        assert_eq!(same_id, first);
        assert_ne!(other_id.message_id, first.message_id);
    }

    #[test]
    fn fifo_delivers_one_message_per_group_per_receive() {
        let mut ids = IdGenerator::default();
        let mut queue = fifo(true);
        for (body, group) in [("a1", "a"), ("a2", "a"), ("b1", "b"), ("a3", "a")] {
            queue.send(Time(0), &mut ids, grouped(body, group)).unwrap();
        }
        let first = queue.receive(Time(0), 10, Seconds(30), &mut ids);
        assert_eq!(bodies(&first), ["a1", "b1"]);
        let second = queue.receive(Time(0), 10, Seconds(30), &mut ids);
        assert!(second.deliveries.is_empty());
        let handle = first.deliveries[0].receipt_handle;
        queue.delete(handle, Time(1)).unwrap();
        let third = queue.receive(Time(1), 10, Seconds(30), &mut ids);
        assert_eq!(bodies(&third), ["a2"]);
    }

    #[test]
    fn exhausted_messages_move_to_the_dead_letter_queue() {
        let mut ids = IdGenerator::default();
        let mut queue = standard();
        queue.attributes_mut().dead_letter = Some(DeadLetter {
            queue: QueueName::parse("dlq").unwrap(),
            max_receive_count: 1,
        });
        let mut dlq = standard();
        queue.send(Time(0), &mut ids, plain("a")).unwrap();
        queue.send(Time(0), &mut ids, plain("b")).unwrap();
        queue.receive(Time(0), 1, Seconds(0), &mut ids);
        let received = queue.receive(Time(1), 1, Seconds(30), &mut ids);
        assert_eq!(bodies(&received), ["b"]);
        assert_eq!(received.dead.len(), 1);
        for message in received.dead {
            dlq.accept_dead_letter(message, Time(1));
        }
        let dead = dlq.receive(Time(1), 1, Seconds(30), &mut ids);
        assert_eq!(bodies(&dead), ["a"]);
        assert_eq!(dead.deliveries[0].receive_count, 1);
        assert_eq!(dead.deliveries[0].sent_at, Time(0));
    }

    #[test]
    fn dead_letter_unblocks_later_messages_of_its_group() {
        let mut ids = IdGenerator::default();
        let mut queue = fifo(true);
        queue.attributes_mut().dead_letter = Some(DeadLetter {
            queue: QueueName::parse("dlq.fifo").unwrap(),
            max_receive_count: 1,
        });
        queue.send(Time(0), &mut ids, grouped("a1", "g")).unwrap();
        queue.send(Time(0), &mut ids, grouped("a2", "g")).unwrap();
        queue.receive(Time(0), 1, Seconds(0), &mut ids);
        let received = queue.receive(Time(0), 10, Seconds(30), &mut ids);
        assert_eq!(bodies(&received), ["a2"]);
        assert_eq!(received.dead.len(), 1);
    }

    #[test]
    fn dead_letter_arrival_past_retention_is_dropped_but_takes_a_sequence_number() {
        let mut ids = IdGenerator::default();
        let mut source = fifo(true);
        source.attributes_mut().dead_letter = Some(DeadLetter {
            queue: QueueName::parse("dlq.fifo").unwrap(),
            max_receive_count: 1,
        });
        source.send(Time(0), &mut ids, grouped("old", "g")).unwrap();
        source.receive(Time(0), 1, Seconds(0), &mut ids);
        let moved = source.receive(Time(100), 1, Seconds(30), &mut ids).dead;

        let mut dlq = fifo(true);
        dlq.attributes_mut().retention = Seconds(60);
        for message in moved {
            dlq.accept_dead_letter(message, Time(100));
        }
        assert_eq!(dlq.counts(Time(100)), Counts::default());
        let next = dlq.send(Time(100), &mut ids, grouped("new", "g")).unwrap();
        assert_eq!(next.sequence_number, Some(SequenceNumber(2)));
    }

    #[test]
    fn queue_refuses_message_beyond_the_limit() {
        let mut ids = IdGenerator::default();
        let mut queue = standard();
        for _ in 0..MAX_MESSAGES {
            queue.send(Time(0), &mut ids, plain("x")).unwrap();
        }
        let err = queue.send(Time(0), &mut ids, plain("x")).unwrap_err();
        assert_eq!(err.code, ErrorCode::LimitExceeded);
        queue.purge();
        queue.send(Time(0), &mut ids, plain("x")).unwrap();
    }
}
