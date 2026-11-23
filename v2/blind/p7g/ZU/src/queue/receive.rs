//! Delivery of messages and everything that works on a delivered message.

use std::ops::Bound;

use serde::Serialize;

use super::{FifoPlacement, Message, Queue, Slot, State};
use crate::error::{Error, ErrorCode, Result};
use crate::types::{
    Body, MaxMessages, MessageId, QueueName, ReceiptHandle, Time, VisibilityTimeout,
};

/// One message as `ReceiveMessage` reports it.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
    message_id: MessageId,
    receipt_handle: ReceiptHandle,
    body: Body,
    receive_count: u32,
    sent_at: Time,
    first_received_at: Time,
    #[serde(flatten)]
    placement: Option<FifoPlacement>,
}

/// Messages that left a queue for its dead-letter queue.
#[derive(Debug)]
pub struct DeadLettered {
    pub target: QueueName,
    pub messages: Vec<Message>,
}

#[derive(Debug)]
pub struct Received {
    pub deliveries: Vec<Delivery>,
    pub dead_lettered: Option<DeadLettered>,
}

fn expired_handle() -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        "the receipt handle does not belong to a message that is in flight in this queue",
    )
}

impl Queue {
    /// Examines the deliverable messages once each in arrival order, until `max_messages` have
    /// been delivered. A message that already used up its receives moves to the dead-letter
    /// queue instead, and does not count.
    pub fn receive(
        &mut self,
        now: Time,
        max_messages: MaxMessages,
        visibility_timeout: VisibilityTimeout,
        next_handle: &mut ReceiptHandle,
    ) -> Received {
        let mut deliveries = Vec::new();
        let mut dead_lettered: Option<DeadLettered> = None;
        let mut remaining = max_messages.get();
        let mut after = Bound::Unbounded;
        // `ready` changes while we walk it (a moved message can make its successor
        // deliverable), so the walk resumes from the last slot instead of holding an iterator.
        while remaining > 0
            && let Some(&slot) = self.ready.range((after, Bound::Unbounded)).next()
        {
            after = Bound::Excluded(slot);
            let message = self
                .messages
                .get_mut(&slot)
                .expect("ready slots always hold a message");
            if let Some(dead_letter) = &self.attributes.dead_letter
                && message.receive_count >= dead_letter.max_receive_count.get()
            {
                let batch = dead_lettered.get_or_insert_with(|| DeadLettered {
                    target: dead_letter.queue.clone(),
                    messages: Vec::new(),
                });
                let moved = self.remove(slot);
                batch.messages.push(moved);
                continue;
            }
            let handle = *next_handle;
            *next_handle = handle.next();
            if let Some(previous) = message.handle.replace(handle) {
                self.handles.remove(&previous);
            }
            self.handles.insert(handle, slot);
            message.receive_count += 1;
            let first_received_at = *message.first_received_at.get_or_insert(now);
            deliveries.push(Delivery {
                message_id: message.id,
                receipt_handle: handle,
                body: message.body.clone(),
                receive_count: message.receive_count,
                sent_at: message.sent_at,
                first_received_at,
                placement: message.placement.clone(),
            });
            self.hide_for(slot, now, visibility_timeout);
            remaining -= 1;
        }
        Received {
            deliveries,
            dead_lettered,
        }
    }

    /// Takes a message moved from another queue: it starts over as a new, visible message.
    pub fn accept_dead_letter(&mut self, now: Time, mut message: Message) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.state = State::Visible;
        message.handle = None;
        if let Some(placement) = &mut message.placement {
            placement.sequence_number = self.next_sequence;
            self.next_sequence = self.next_sequence.next();
        }
        if now < message.sent_at.after(self.attributes.retention) {
            self.push(message);
        }
    }

    pub fn delete(&mut self, handle: ReceiptHandle) -> Result<()> {
        let slot = self.in_flight_slot(handle)?;
        self.remove(slot);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        now: Time,
        handle: ReceiptHandle,
        visibility_timeout: VisibilityTimeout,
    ) -> Result<()> {
        let slot = self.in_flight_slot(handle)?;
        self.hide_for(slot, now, visibility_timeout);
        Ok(())
    }

    /// In flight until `now + visibility_timeout`; with a timeout of 0 that is already over.
    fn hide_for(&mut self, slot: Slot, now: Time, visibility_timeout: VisibilityTimeout) {
        let until = now.after(visibility_timeout);
        let state = if until > now {
            State::InFlight { until }
        } else {
            State::Visible
        };
        self.set_state(slot, state);
    }

    /// The message `handle` was issued for, if that is still its latest handle and the message
    /// is in flight right now.
    fn in_flight_slot(&self, handle: ReceiptHandle) -> Result<Slot> {
        self.handles
            .get(&handle)
            .copied()
            .filter(|slot| {
                self.messages
                    .get(slot)
                    .is_some_and(|message| matches!(message.state, State::InFlight { .. }))
            })
            .ok_or_else(expired_handle)
    }
}
