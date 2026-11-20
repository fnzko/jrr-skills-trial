//! The messages of one queue, in arrival order, with the indexes that keep
//! sending, expiring and handle lookups cheap for large queues.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::types::{
    Body, Counter, GroupId, MaxReceiveCount, MessageId, ReceiptHandle, Seconds, SequenceNumber,
    Timestamp,
};

/// The FIFO-only part of a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct FifoTag {
    pub group_id: GroupId,
    pub sequence_number: SequenceNumber,
}

#[derive(Debug)]
pub struct Message {
    pub id: MessageId,
    pub body: Body,
    pub sent_at: Timestamp,
    pub fifo: Option<FifoTag>,
    delivery: Delivery,
}

/// Delivery history of a message since it entered its current queue.
#[derive(Debug, Clone, Copy)]
enum Delivery {
    /// Not delivered yet; delayed while `now < visible_from`.
    Pending { visible_from: Timestamp },
    /// Delivered `count` times; in flight while `now < visible_from`.
    Received {
        count: u64,
        first_received_at: Timestamp,
        handle: ReceiptHandle,
        visible_from: Timestamp,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Delayed,
    InFlight,
    Visible,
}

impl Delivery {
    fn state(self, now: Timestamp) -> State {
        match self {
            Self::Pending { visible_from } if now < visible_from => State::Delayed,
            Self::Received { visible_from, .. } if now < visible_from => State::InFlight,
            Self::Pending { .. } | Self::Received { .. } => State::Visible,
        }
    }

    fn receive_count(self) -> u64 {
        match self {
            Self::Pending { .. } => 0,
            Self::Received { count, .. } => count,
        }
    }

    fn handle(self) -> Option<ReceiptHandle> {
        match self {
            Self::Pending { .. } => None,
            Self::Received { handle, .. } => Some(handle),
        }
    }
}

impl Message {
    /// A message that has not been delivered and stays delayed until `visible_from`.
    #[must_use]
    pub fn new(
        id: MessageId,
        body: Body,
        sent_at: Timestamp,
        visible_from: Timestamp,
        fifo: Option<FifoTag>,
    ) -> Self {
        Self {
            id,
            body,
            sent_at,
            fifo,
            delivery: Delivery::Pending { visible_from },
        }
    }

    /// Forgets the delivery history, as a move to a dead-letter queue requires,
    /// leaving the message visible from `now`.
    pub fn reset_delivery(&mut self, now: Timestamp) {
        self.delivery = Delivery::Pending { visible_from: now };
    }
}

/// One delivered message as `ReceiveMessage` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceivedMessage {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: Body,
    pub receive_count: u64,
    pub sent_at: Timestamp,
    pub first_received_at: Timestamp,
    #[serde(flatten)]
    pub fifo: Option<FifoTag>,
}

/// How many messages are in each state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// The parameters of one receive, resolved against the queue.
#[derive(Debug, Clone, Copy)]
pub struct ReceiveOptions {
    pub max_messages: usize,
    pub visibility_timeout: Seconds,
    /// The queue's `MaxReceiveCount` when it has a dead-letter configuration.
    pub dead_letter_after: Option<MaxReceiveCount>,
}

/// The receipt handle is not the latest handle of a message in flight here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the handle does not belong to a message in flight in this queue")]
pub struct NotInFlight;

/// Position of a message in arrival order; never reused within one store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Arrival(u64);

#[derive(Debug, Default)]
pub struct MessageStore {
    messages: BTreeMap<Arrival, Message>,
    /// Every stored message by `(sent_at, arrival)`, so expiry pops from the front.
    by_sent_at: BTreeSet<(Timestamp, Arrival)>,
    /// The latest handle of every stored message that has one.
    by_handle: HashMap<ReceiptHandle, Arrival>,
    next_arrival: u64,
}

impl MessageStore {
    #[must_use]
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Adds `message` at the end of arrival order.
    pub fn push(&mut self, message: Message) {
        let arrival = Arrival(self.next_arrival);
        self.next_arrival += 1;
        self.by_sent_at.insert((message.sent_at, arrival));
        if let Some(handle) = message.delivery.handle() {
            self.by_handle.insert(handle, arrival);
        }
        self.messages.insert(arrival, message);
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
        self.by_handle.clear();
    }

    /// Removes every message with `now >= sent_at + retention`.
    pub fn expire(&mut self, now: Timestamp, retention: Seconds) {
        let Some(cutoff) = now.minus(retention) else {
            return;
        };
        while let Some(&(sent_at, arrival)) = self.by_sent_at.first()
            && sent_at <= cutoff
        {
            self.remove(arrival);
        }
    }

    #[must_use]
    pub fn counts(&self, now: Timestamp) -> Counts {
        self.messages
            .values()
            .fold(Counts::default(), |mut counts, message| {
                match message.delivery.state(now) {
                    State::Visible => counts.visible += 1,
                    State::InFlight => counts.in_flight += 1,
                    State::Delayed => counts.delayed += 1,
                }
                counts
            })
    }

    /// Removes the message that `handle` holds in flight.
    pub fn delete(&mut self, handle: ReceiptHandle, now: Timestamp) -> Result<(), NotInFlight> {
        let (arrival, _) = self.in_flight(handle, now)?;
        self.remove(arrival);
        Ok(())
    }

    /// Keeps the message that `handle` holds in flight while `now < now + timeout`.
    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: Timestamp,
        timeout: Seconds,
    ) -> Result<(), NotInFlight> {
        let (_, visible_from) = self.in_flight(handle, now)?;
        *visible_from = now.plus(timeout);
        Ok(())
    }

    /// Examines messages in arrival order and delivers up to
    /// `options.max_messages` eligible ones. Eligible messages that have used up
    /// their deliveries are removed instead and returned for dead-lettering.
    pub fn receive(
        &mut self,
        now: Timestamp,
        options: ReceiveOptions,
        handles: &mut Counter,
    ) -> (Vec<ReceivedMessage>, Vec<Message>) {
        let mut delivered = Vec::new();
        let mut dead = Vec::new();
        // Groups whose earliest remaining message has been examined; later
        // messages of these groups are not eligible.
        let mut blocked_groups: HashSet<&GroupId> = HashSet::new();
        for (&arrival, message) in &mut self.messages {
            if delivered.len() == options.max_messages {
                break;
            }
            let Message {
                id,
                body,
                sent_at,
                fifo,
                delivery,
            } = message;
            let fifo: &Option<FifoTag> = fifo;
            let group = fifo.as_ref().map(|tag| &tag.group_id);
            let eligible = group.is_none_or(|group| !blocked_groups.contains(group))
                && delivery.state(now) == State::Visible;
            if eligible
                && options
                    .dead_letter_after
                    .is_some_and(|limit| limit.is_reached_by(delivery.receive_count()))
            {
                // A message that leaves no longer holds back its group.
                dead.push(arrival);
                continue;
            }
            if let Some(group) = group {
                blocked_groups.insert(group);
            }
            if !eligible {
                continue;
            }
            let handle = handles.next_receipt_handle();
            let (count, first_received_at) = match *delivery {
                Delivery::Pending { .. } => (1, now),
                Delivery::Received {
                    count,
                    first_received_at,
                    handle: previous,
                    ..
                } => {
                    self.by_handle.remove(&previous);
                    (count + 1, first_received_at)
                }
            };
            *delivery = Delivery::Received {
                count,
                first_received_at,
                handle,
                visible_from: now.plus(options.visibility_timeout),
            };
            self.by_handle.insert(handle, arrival);
            delivered.push(ReceivedMessage {
                message_id: *id,
                receipt_handle: handle,
                body: body.clone(),
                receive_count: count,
                sent_at: *sent_at,
                first_received_at,
                fifo: fifo.clone(),
            });
        }
        let dead = dead
            .into_iter()
            .map(|arrival| {
                self.remove(arrival)
                    .expect("dead-lettered message was found in this scan")
            })
            .collect();
        (delivered, dead)
    }

    /// The message for which `handle` is the latest handle and which is in
    /// flight, with the end of its visibility timeout.
    fn in_flight(
        &mut self,
        handle: ReceiptHandle,
        now: Timestamp,
    ) -> Result<(Arrival, &mut Timestamp), NotInFlight> {
        let arrival = *self.by_handle.get(&handle).ok_or(NotInFlight)?;
        match self.messages.get_mut(&arrival) {
            Some(Message {
                delivery:
                    Delivery::Received {
                        handle: latest,
                        visible_from,
                        ..
                    },
                ..
            }) if *latest == handle && now < *visible_from => Ok((arrival, visible_from)),
            _ => Err(NotInFlight),
        }
    }

    fn remove(&mut self, arrival: Arrival) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        self.by_sent_at.remove(&(message.sent_at, arrival));
        if let Some(handle) = message.delivery.handle() {
            self.by_handle.remove(&handle);
        }
        Some(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_DEAD_LETTER: Option<MaxReceiveCount> = None;

    struct Fixture {
        store: MessageStore,
        ids: Counter,
        handles: Counter,
        sequence: Counter,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                store: MessageStore::default(),
                ids: Counter::default(),
                handles: Counter::default(),
                sequence: Counter::default(),
            }
        }

        fn send(&mut self, body: &str, sent_at: u32, group: Option<&str>) {
            let fifo = group.map(|group| FifoTag {
                group_id: GroupId::parse(group.to_owned()).expect("test group ids are valid"),
                sequence_number: self.sequence.next_sequence_number(),
            });
            let at = at(sent_at);
            let body = Body::parse(body.to_owned()).expect("test bodies are valid");
            self.store
                .push(Message::new(self.ids.next_message_id(), body, at, at, fifo));
        }

        fn receive(
            &mut self,
            now: u32,
            max_messages: usize,
            dead_letter_after: Option<MaxReceiveCount>,
        ) -> (Vec<String>, Vec<String>) {
            let options = ReceiveOptions {
                max_messages,
                visibility_timeout: Seconds::new(10),
                dead_letter_after,
            };
            let (delivered, dead) = self.store.receive(at(now), options, &mut self.handles);
            (
                delivered
                    .into_iter()
                    .map(|m| m.body.as_str().to_owned())
                    .collect(),
                dead.into_iter()
                    .map(|m| m.body.as_str().to_owned())
                    .collect(),
            )
        }
    }

    fn at(seconds: u32) -> Timestamp {
        Timestamp::default().plus(Seconds::new(seconds))
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|&s| s.to_owned()).collect()
    }

    #[test]
    fn expiry_follows_sent_at_not_arrival_order() {
        let mut f = Fixture::new();
        f.send("late", 50, None);
        f.send("early", 10, None);
        f.store.expire(at(70), Seconds::new(60));
        assert_eq!(f.store.len(), 1);
        assert_eq!(
            f.receive(70, 10, NO_DEAD_LETTER),
            (strings(&["late"]), vec![])
        );
    }

    #[test]
    fn fifo_receive_takes_one_message_per_unblocked_group() {
        let mut f = Fixture::new();
        f.send("a1", 0, Some("a"));
        f.send("a2", 0, Some("a"));
        f.send("b1", 0, Some("b"));
        assert_eq!(f.receive(0, 1, NO_DEAD_LETTER), (strings(&["a1"]), vec![]));
        assert_eq!(f.receive(0, 10, NO_DEAD_LETTER), (strings(&["b1"]), vec![]));
        assert_eq!(
            f.receive(10, 10, NO_DEAD_LETTER),
            (strings(&["a1", "b1"]), vec![])
        );
    }

    #[test]
    fn dead_lettered_message_releases_its_group_within_the_same_scan() {
        let mut f = Fixture::new();
        f.send("a1", 0, Some("a"));
        f.send("a2", 0, Some("a"));
        let limit = MaxReceiveCount::new(1).ok();
        assert_eq!(f.receive(0, 10, limit), (strings(&["a1"]), vec![]));
        assert_eq!(
            f.receive(10, 10, limit),
            (strings(&["a2"]), strings(&["a1"]))
        );
        assert_eq!(f.store.len(), 1);
    }

    #[test]
    fn handles_stop_working_after_redelivery_timeout_or_removal() {
        let mut f = Fixture::new();
        f.send("m", 0, None);
        f.receive(0, 1, NO_DEAD_LETTER);
        let first = ReceiptHandle::parse("r-1").expect("r-1 is well formed");
        assert_eq!(
            f.store.change_visibility(first, at(5), Seconds::new(0)),
            Ok(())
        );
        assert_eq!(f.store.delete(first, at(5)), Err(NotInFlight));
        f.receive(5, 1, NO_DEAD_LETTER);
        assert_eq!(f.store.delete(first, at(5)), Err(NotInFlight));
        let second = ReceiptHandle::parse("r-2").expect("r-2 is well formed");
        assert_eq!(
            f.store.change_visibility(second, at(14), Seconds::new(100)),
            Ok(())
        );
        assert_eq!(f.store.delete(second, at(113)), Ok(()));
        assert_eq!(f.store.len(), 0);
    }

    #[test]
    fn counts_each_state() {
        let mut f = Fixture::new();
        f.send("in flight", 0, None);
        f.send("visible", 0, None);
        f.receive(0, 1, NO_DEAD_LETTER);
        let delayed = Body::parse("delayed".to_owned()).expect("valid body");
        f.store.push(Message::new(
            f.ids.next_message_id(),
            delayed,
            at(0),
            at(5),
            None,
        ));
        assert_eq!(
            f.store.counts(at(0)),
            Counts {
                visible: 1,
                in_flight: 1,
                delayed: 1
            }
        );
        assert_eq!(
            f.store.counts(at(10)),
            Counts {
                visible: 3,
                in_flight: 0,
                delayed: 0
            }
        );
    }
}
