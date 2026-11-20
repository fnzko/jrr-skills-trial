//! Message commands (SPEC section 6).

use std::time::Duration;

use serde_json::{Value, json};

use super::{Broker, Members, lookup, members};
use crate::error::{Error, ErrorCode, Result};
use crate::ids::ReceiptHandle;
use crate::queue::{Accepted, MAX_MESSAGES, Message, Queue, Step};
use crate::request::{Params, missing};

const MAX_BODY_BYTES: usize = 1024;
const MAX_ID_CHARS: usize = 128;

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

fn received_json(message: &Message, handle: ReceiptHandle) -> Value {
    let mut json = json!({
        "MessageId": message.id.to_string(),
        "ReceiptHandle": handle.to_string(),
        "Body": message.body,
        "ReceiveCount": message.receive_count,
        "SentAt": message.sent_at.as_secs(),
        "FirstReceivedAt": message.first_received_at.map(|t| t.as_secs()),
    });
    if let Some(fifo) = &message.fifo {
        json["GroupId"] = json!(fifo.group_id);
        json["SequenceNumber"] = json!(fifo.sequence_number);
    }
    json
}

impl Broker {
    pub(super) fn send_message(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Body"])?;
        let name = params.required_string("QueueName")?;
        let body = params.required_string("Body")?;
        if !(1..=MAX_BODY_BYTES).contains(&body.len()) {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("`Body` must be 1 to {MAX_BODY_BYTES} bytes"),
            ));
        }
        let delay = params.seconds("DelaySeconds", 0..=900)?;
        let group_id = params.identifier("GroupId", MAX_ID_CHARS)?;
        let deduplication_id = params.identifier("DeduplicationId", MAX_ID_CHARS)?;

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let attributes = &queue.attributes;
        if !attributes.fifo && (group_id.is_some() || deduplication_id.is_some()) {
            return Err(unsupported(
                "`GroupId` and `DeduplicationId` need a FIFO queue",
            ));
        }
        if attributes.fifo && delay.is_some() {
            return Err(unsupported("FIFO queues do not support per-message delay"));
        }
        let fifo_ids = if attributes.fifo {
            let group_id = group_id.ok_or_else(|| missing("GroupId"))?;
            let deduplication_id = match deduplication_id {
                Some(id) => id,
                None if attributes.content_based_deduplication => body,
                None => return Err(missing("DeduplicationId")),
            };
            if let Some(original) = queue.duplicate_of(deduplication_id, now) {
                return Ok(members([
                    ("MessageId", json!(original.message_id.to_string())),
                    ("SequenceNumber", json!(original.sequence_number)),
                ]));
            }
            Some((group_id, deduplication_id))
        } else {
            None
        };
        if queue.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }

        let id = self.next_message_id.take_next();
        let delay = delay.unwrap_or(queue.attributes.delay);
        let group_id = fifo_ids.map(|(group, _)| group.to_owned());
        let sequence_number = queue.push(
            Message {
                id,
                body: body.to_owned(),
                sent_at: now,
                receive_count: 0,
                first_received_at: None,
                fifo: None,
                delayed_until: now + delay,
                invisible_until: Duration::ZERO,
                handle: None,
            },
            group_id,
        );
        let mut result = members([("MessageId", json!(id.to_string()))]);
        if let (Some((_, deduplication_id)), Some(sequence_number)) = (fifo_ids, sequence_number) {
            queue.record_deduplication(
                deduplication_id,
                Accepted {
                    accepted_at: now,
                    message_id: id,
                    sequence_number,
                },
            );
            result.insert("SequenceNumber".into(), json!(sequence_number));
        }
        Ok(result)
    }

    pub(super) fn receive_message(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName"])?;
        let name = params.required_string("QueueName")?;
        let max_messages = params.integer("MaxMessages", 1..=10)?.unwrap_or(1);
        let visibility_timeout = params.seconds("VisibilityTimeout", 0..=43_200)?;

        let now = self.now;
        let mut next_handle = self.next_receipt_handle;
        let queue = lookup(&mut self.queues, now, name)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.attributes.visibility_timeout);

        let mut delivered = Vec::new();
        let mut dead_letters = Vec::new();
        for step in queue.plan_receive(now, max_messages) {
            match step {
                Step::Deliver(key) => {
                    let handle = next_handle.take_next();
                    let message = queue.deliver(key, now, visibility_timeout, handle);
                    delivered.push(received_json(message, handle));
                }
                Step::DeadLetter(key) => {
                    dead_letters.extend(queue.remove(key));
                }
            }
        }
        self.next_receipt_handle = next_handle;

        if !dead_letters.is_empty() {
            self.move_to_dead_letter_queue(name, dead_letters);
        }
        Ok(members([("Messages", Value::Array(delivered))]))
    }

    /// Appends messages taken out of `source` to the dead-letter queue that
    /// `source` names, if it still has one.
    fn move_to_dead_letter_queue(&mut self, source: &str, messages: Vec<Message>) {
        let now = self.now;
        // The name is copied because the target is borrowed from the same map.
        let target_name = self
            .queues
            .get(source)
            .and_then(|q| q.attributes.dead_letter.as_ref())
            .map(|dl| dl.queue.clone());
        if let Some(target) = target_name.and_then(|name| self.queues.get_mut(&name)) {
            for message in messages {
                target.push_dead_letter(message, now);
            }
        }
    }

    pub(super) fn delete_message(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "ReceiptHandle"])?;
        let name = params.required_string("QueueName")?;
        let handle = params.required_string("ReceiptHandle")?;

        let now = self.now;
        let (queue, handle) = self.queue_and_handle(name, handle)?;
        queue
            .remove_in_flight(handle, now)
            .ok_or_else(handle_expired)?;
        Ok(Members::new())
    }

    pub(super) fn change_message_visibility(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
        let name = params.required_string("QueueName")?;
        let handle = params.required_string("ReceiptHandle")?;
        let timeout = params.required_seconds("VisibilityTimeout", 0..=43_200)?;

        let now = self.now;
        let (queue, handle) = self.queue_and_handle(name, handle)?;
        let message = queue
            .in_flight_mut(handle, now)
            .ok_or_else(handle_expired)?;
        message.invisible_until = now + timeout;
        Ok(Members::new())
    }

    /// Looks up the queue, then checks that the handle was issued
    /// (SPEC section 6, rule 1).
    fn queue_and_handle(
        &mut self,
        queue_name: &str,
        handle: &str,
    ) -> Result<(&mut Queue, ReceiptHandle)> {
        let issued = self.next_receipt_handle;
        let queue = lookup(&mut self.queues, self.now, queue_name)?;
        let handle = ReceiptHandle::parse(handle)
            .filter(|&h| h < issued)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    "receipt handle was never issued",
                )
            })?;
        Ok((queue, handle))
    }
}

/// SPEC section 6, rule 2.
fn handle_expired() -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        "receipt handle no longer refers to an in-flight message",
    )
}
