//! Message commands (SPEC section 6).

use serde_json::{Value, json};

use super::{Broker, Members, lookup, members};
use crate::error::{Error, ErrorCode, Result};
use crate::ids::{MessageId, ReceiptHandle};
use crate::queue::{Accepted, MAX_MESSAGES, Message, Queue, Step};
use crate::request::Params;

const MAX_BODY_BYTES: usize = 1024;
const MAX_ID_CHARS: usize = 128;

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

/// The ids a send to a FIFO queue carries.
struct FifoIds<'a> {
    group_id: &'a str,
    deduplication_id: &'a str,
}

fn received_json(message: &Message, handle: ReceiptHandle, fifo: bool) -> Value {
    let mut json = json!({
        "MessageId": message.id.to_string(),
        "ReceiptHandle": handle.to_string(),
        "Body": message.body,
        "ReceiveCount": message.receive_count,
        "SentAt": message.sent_at,
        "FirstReceivedAt": message.first_received_at,
    });
    if fifo {
        json["GroupId"] = json!(message.group_id);
        json["SequenceNumber"] = json!(message.sequence_number);
    }
    json
}

impl Broker {
    pub(super) fn send_message(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Body"])?;
        let name = params
            .string("QueueName")?
            .expect("presence checked by Params::require");
        let body = params
            .string("Body")?
            .expect("presence checked by Params::require");
        if !(1..=MAX_BODY_BYTES).contains(&body.len()) {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("`Body` must be 1 to {MAX_BODY_BYTES} bytes"),
            ));
        }
        let delay = params.integer("DelaySeconds", 0..=900)?;
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
        let fifo = if attributes.fifo {
            let group_id = group_id.ok_or_else(|| Error::missing_parameter("GroupId"))?;
            let deduplication_id = match deduplication_id {
                Some(id) => id,
                None if attributes.content_based_deduplication => body,
                None => return Err(Error::missing_parameter("DeduplicationId")),
            };
            if let Some(original) = queue.duplicate_of(deduplication_id, now) {
                return Ok(members([
                    ("MessageId", json!(original.message_id.to_string())),
                    ("SequenceNumber", json!(original.sequence_number)),
                ]));
            }
            Some(FifoIds {
                group_id,
                deduplication_id,
            })
        } else {
            None
        };
        if queue.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }

        let id = MessageId(self.next_message_id);
        self.next_message_id += 1;
        let delay = delay.unwrap_or(queue.attributes.delay_seconds);
        let sequence_number = queue.push(Message {
            id,
            body: body.to_owned(),
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group_id: fifo.as_ref().map(|ids| ids.group_id.to_owned()),
            sequence_number: None,
            delayed_until: now + delay,
            invisible_until: 0,
            handle: None,
        });
        let mut result = members([("MessageId", json!(id.to_string()))]);
        if let Some(ids) = fifo {
            let sequence_number = sequence_number.expect("a FIFO queue numbers every message");
            queue.record_deduplication(
                ids.deduplication_id,
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
        let name = params
            .string("QueueName")?
            .expect("presence checked by Params::require");
        let max_messages = params.integer("MaxMessages", 1..=10)?.unwrap_or(1);
        let visibility_timeout = params.integer("VisibilityTimeout", 0..=43_200)?;

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.attributes.visibility_timeout);
        let fifo = queue.attributes.fifo;
        let dead_letter_queue = queue
            .attributes
            .dead_letter
            .as_ref()
            .map(|dl| dl.queue.clone());

        let mut delivered = Vec::new();
        let mut dead_letters = Vec::new();
        for step in queue.plan_receive(now, max_messages as usize) {
            match step {
                Step::Deliver(key) => {
                    let handle = ReceiptHandle(self.next_receipt_handle);
                    self.next_receipt_handle += 1;
                    let message = queue.deliver(key, now, visibility_timeout, handle);
                    delivered.push(received_json(message, handle, fifo));
                }
                Step::DeadLetter(key) => {
                    dead_letters.extend(queue.remove(key));
                }
            }
        }

        if let Some(target) = dead_letter_queue.and_then(|q| self.queues.get_mut(&q)) {
            for message in dead_letters {
                target.push_dead_letter(message, now);
            }
        }
        Ok(members([("Messages", Value::Array(delivered))]))
    }

    pub(super) fn delete_message(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "ReceiptHandle"])?;
        let name = params
            .string("QueueName")?
            .expect("presence checked by Params::require");
        let handle = params
            .string("ReceiptHandle")?
            .expect("presence checked by Params::require");

        let now = self.now;
        let (queue, handle) = self.queue_and_handle(name, handle)?;
        queue
            .delete_in_flight(handle, now)
            .ok_or_else(handle_expired)?;
        Ok(Members::new())
    }

    pub(super) fn change_message_visibility(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
        let name = params
            .string("QueueName")?
            .expect("presence checked by Params::require");
        let handle = params
            .string("ReceiptHandle")?
            .expect("presence checked by Params::require");
        let timeout = params
            .integer("VisibilityTimeout", 0..=43_200)?
            .expect("presence checked by Params::require");

        let now = self.now;
        let (queue, handle) = self.queue_and_handle(name, handle)?;
        queue
            .in_flight_mut(handle, now)
            .ok_or_else(handle_expired)?
            .invisible_until = now + timeout;
        Ok(Members::new())
    }

    /// Looks up the queue and checks that the receipt handle was issued
    /// (SPEC section 6, rule 1).
    fn queue_and_handle(
        &mut self,
        queue_name: &str,
        handle: &str,
    ) -> Result<(&mut Queue, ReceiptHandle)> {
        let issued = self.next_receipt_handle;
        let queue = lookup(&mut self.queues, self.now, queue_name)?;
        let handle = ReceiptHandle::parse(handle)
            .filter(|handle| handle.0 < issued)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    "receipt handle was never issued",
                )
            })?;
        Ok((queue, handle))
    }
}

/// A handle that was issued but no longer stands for an in-flight message
/// (SPEC section 6, rule 2).
fn handle_expired() -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        "receipt handle no longer refers to an in-flight message",
    )
}
