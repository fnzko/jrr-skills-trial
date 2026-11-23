//! A message and the state it is in.

use std::sync::Arc;

use crate::error::Result;
use crate::ids::{MessageId, ReceiptHandle, SequenceNumber};
use crate::params::invalid;
use crate::time::{Instant, Seconds};

/// Where a message stands at a given moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageState {
    Delayed,
    InFlight,
    Visible,
}

/// Checks the character rule shared by group and deduplication ids: 1 to 128
/// characters from `A-Z a-z 0-9 _ -`.
pub fn validate_token<'a>(name: &str, text: &'a str) -> Result<&'a str> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    if text.is_empty() || text.len() > 128 || !text.chars().all(allowed) {
        return Err(invalid(format!(
            "{name} must be 1 to 128 characters from A-Z a-z 0-9 _ -"
        )));
    }
    Ok(text)
}

/// The message group of a FIFO message. Cheap to clone: queues remember
/// groups while they scan.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GroupId(Arc<str>);

impl GroupId {
    pub fn parse(text: &str) -> Result<GroupId> {
        validate_token("GroupId", text).map(|text| GroupId(Arc::from(text)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a consumer sees of one delivery.
#[derive(Clone, Debug)]
pub struct Delivery {
    pub id: MessageId,
    pub handle: ReceiptHandle,
    pub body: String,
    pub receive_count: u32,
    pub sent_at: Instant,
    pub first_received_at: Instant,
    pub group: Option<GroupId>,
    pub sequence: Option<SequenceNumber>,
}

#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: Instant,
    group: Option<GroupId>,
    sequence: Option<SequenceNumber>,
    receive_count: u32,
    first_received_at: Option<Instant>,
    delayed_until: Instant,
    in_flight_until: Instant,
    handle: Option<ReceiptHandle>,
}

impl Message {
    pub fn new(
        id: MessageId,
        body: String,
        sent_at: Instant,
        delay: Seconds,
        group: Option<GroupId>,
    ) -> Message {
        Message {
            id,
            body,
            sent_at,
            group,
            sequence: None,
            receive_count: 0,
            first_received_at: None,
            delayed_until: sent_at.after(delay),
            in_flight_until: Instant::START,
            handle: None,
        }
    }

    pub fn id(&self) -> MessageId {
        self.id
    }

    pub fn sent_at(&self) -> Instant {
        self.sent_at
    }

    pub fn group(&self) -> Option<&GroupId> {
        self.group.as_ref()
    }

    pub fn receive_count(&self) -> u32 {
        self.receive_count
    }

    /// The handle of the latest delivery, if any. It is only usable while
    /// the message is in flight.
    pub fn handle(&self) -> Option<ReceiptHandle> {
        self.handle
    }

    pub fn set_sequence(&mut self, sequence: Option<SequenceNumber>) {
        self.sequence = sequence;
    }

    pub fn state(&self, now: Instant) -> MessageState {
        if now < self.delayed_until {
            MessageState::Delayed
        } else if now < self.in_flight_until {
            MessageState::InFlight
        } else {
            MessageState::Visible
        }
    }

    /// True once the retention period, counted from `SentAt`, has passed.
    pub fn is_expired(&self, now: Instant, retention: Seconds) -> bool {
        now >= self.sent_at.after(retention)
    }

    /// Records a delivery. Returns what the consumer sees and the handle of
    /// the previous delivery, which stops being usable.
    pub fn deliver(
        &mut self,
        now: Instant,
        visibility: Seconds,
        handle: ReceiptHandle,
    ) -> (Delivery, Option<ReceiptHandle>) {
        self.receive_count += 1;
        let first_received_at = *self.first_received_at.get_or_insert(now);
        self.in_flight_until = now.after(visibility);
        let replaced = self.handle.replace(handle);
        let delivery = Delivery {
            id: self.id,
            handle,
            body: self.body.clone(),
            receive_count: self.receive_count,
            sent_at: self.sent_at,
            first_received_at,
            group: self.group.clone(),
            sequence: self.sequence,
        };
        (delivery, replaced)
    }

    /// Ends the current visibility timeout `visibility` after `now`.
    pub fn restart_visibility(&mut self, now: Instant, visibility: Seconds) {
        self.in_flight_until = now.after(visibility);
    }

    /// The message as it arrives in a dead-letter queue: same identity and
    /// age, but never received and not delayed.
    pub fn into_dead_letter(self) -> Message {
        Message {
            sequence: None,
            receive_count: 0,
            first_received_at: None,
            delayed_until: Instant::START,
            in_flight_until: Instant::START,
            handle: None,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::Counter;

    fn message(delay: u32) -> Message {
        let id = Counter::new().next_message_id();
        Message::new(id, "x".into(), Instant::START, Seconds::new(delay), None)
    }

    #[test]
    fn delay_then_visible() {
        let message = message(5);
        let at = |secs| Instant::START.after(Seconds::new(secs));
        assert_eq!(message.state(at(4)), MessageState::Delayed);
        assert_eq!(message.state(at(5)), MessageState::Visible);
    }

    #[test]
    fn delivery_puts_message_in_flight_until_timeout() {
        let mut message = message(0);
        let handle = Counter::new().next_receipt_handle();
        let now = Instant::START.after(Seconds::new(10));
        assert_eq!(message.deliver(now, Seconds::new(3), handle).1, None);
        assert_eq!(
            message.state(now.after(Seconds::new(2))),
            MessageState::InFlight
        );
        assert_eq!(
            message.state(now.after(Seconds::new(3))),
            MessageState::Visible
        );
        assert_eq!(message.deliver(now, Seconds::ZERO, handle).1, Some(handle));
        assert_eq!(message.state(now), MessageState::Visible);
    }

    #[test]
    fn expiry_is_inclusive_of_the_retention_boundary() {
        let message = message(0);
        let retention = Seconds::new(60);
        assert!(!message.is_expired(Instant::START.after(Seconds::new(59)), retention));
        assert!(message.is_expired(Instant::START.after(Seconds::new(60)), retention));
    }

    #[test]
    fn token_rules() {
        assert!(validate_token("t", "aZ09_-").is_ok());
        assert!(validate_token("t", &"a".repeat(128)).is_ok());
        for bad in ["", "a b", "é", &"a".repeat(129)] {
            assert!(validate_token("t", bad).is_err(), "{bad:?}");
        }
    }
}
