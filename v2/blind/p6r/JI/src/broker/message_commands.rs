//! Message commands (SPEC section 6).

use serde_json::{Value, json};

use super::{Broker, Members, lookup, members};
use crate::attributes::{DELAY_RANGE, VISIBILITY_TIMEOUT_RANGE};
use crate::error::{Error, ErrorCode, Result};
use crate::ids::ReceiptHandle;
use crate::queue::{Accepted, Arrival, MAX_MESSAGES, Message, Queue, Step};
use crate::request::Params;
use crate::time::Instant;

const MAX_BODY_BYTES: usize = 1024;
const MAX_ID_CHARS: usize = 128;

/// The FIFO-only identifiers of a send.
#[derive(Clone, Copy)]
struct FifoIds<'a> {
    group_id: &'a str,
    deduplication_id: &'a str,
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

fn missing(name: &str) -> Error {
    Error::new(ErrorCode::MissingParameter, format!("`{name}` is required"))
}

fn received_json(message: &Message, handle: ReceiptHandle, fifo: bool) -> Value {
    let mut json = json!({
        "MessageId": message.id.to_string(),
        "ReceiptHandle": handle.to_string(),
        "Body": message.body,
        "ReceiveCount": message.receive_count,
        "SentAt": message.sent_at.get(),
        "FirstReceivedAt": message.first_received_at.map(Instant::get),
    });
    if fifo {
        json["GroupId"] = json!(message.fifo.as_ref().map(|f| &f.group_id));
        json["SequenceNumber"] = json!(message.fifo.as_ref().map(|f| f.sequence_number.get()));
    }
    json
}

impl Broker {
    pub(super) fn send_message(&mut self, params: Params<'_>) -> Result<Members> {
        let [name, body] = params.require(["QueueName", "Body"])?;
        let name = name.string()?;
        let body = body.string()?;
        if !(1..=MAX_BODY_BYTES).contains(&body.len()) {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("`Body` must be 1 to {MAX_BODY_BYTES} bytes"),
            ));
        }
        let delay = params.seconds("DelaySeconds", DELAY_RANGE)?;
        let group_id = params.identifier("GroupId", MAX_ID_CHARS)?;
        let deduplication_id = params.identifier("DeduplicationId", MAX_ID_CHARS)?;

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let attributes = &queue.attributes;
        if !attributes.kind.is_fifo() && (group_id.is_some() || deduplication_id.is_some()) {
            return Err(unsupported(
                "`GroupId` and `DeduplicationId` need a FIFO queue",
            ));
        }
        if attributes.kind.is_fifo() && delay.is_some() {
            return Err(unsupported("FIFO queues do not support per-message delay"));
        }
        let fifo = if attributes.kind.is_fifo() {
            let group_id = group_id.ok_or_else(|| missing("GroupId"))?;
            let deduplication_id = match deduplication_id {
                Some(id) => id,
                None if attributes.kind.content_based_deduplication() => body,
                None => return Err(missing("DeduplicationId")),
            };
            if let Some(original) = queue.duplicate_of(deduplication_id, now) {
                return Ok(members([
                    ("MessageId", json!(original.message_id.to_string())),
                    ("SequenceNumber", json!(original.sequence_number.get())),
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

        let id = self.next_message_id;
        self.next_message_id = id.next();
        let delay = delay.unwrap_or(queue.attributes.delay);
        let message = Message::new(id, body, now, now + delay);
        let sequence_number = queue.push(message, fifo.map(|f| f.group_id.to_owned()));
        let mut result = members([("MessageId", json!(id.to_string()))]);
        if let Some(fifo) = fifo {
            let sequence_number =
                sequence_number.expect("a message pushed with a group id gets a sequence number");
            queue.record_deduplication(
                fifo.deduplication_id,
                Accepted {
                    accepted_at: now,
                    message_id: id,
                    sequence_number,
                },
            );
            result.insert("SequenceNumber".into(), json!(sequence_number.get()));
        }
        Ok(result)
    }

    pub(super) fn receive_message(&mut self, params: Params<'_>) -> Result<Members> {
        let [name] = params.require(["QueueName"])?;
        let name = name.string()?;
        let max_messages = params.integer("MaxMessages", 1..=10)?.unwrap_or(1);
        let visibility_timeout = params.seconds("VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE)?;

        let now = self.now;
        let mut next_handle = self.next_receipt_handle;
        let queue = lookup(&mut self.queues, now, name)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.attributes.visibility_timeout);
        let fifo = queue.attributes.kind.is_fifo();
        let dead_letter_queue = queue
            .attributes
            .dead_letter
            .as_ref()
            .map(|dl| dl.queue.clone());

        let mut delivered = Vec::new();
        let mut dead_letters = Vec::new();
        for step in queue.plan_receive(now, max_messages) {
            match step {
                Step::Deliver(key) => {
                    let handle = next_handle;
                    next_handle = handle.next();
                    let message = queue.deliver(key, now, visibility_timeout, handle);
                    delivered.push(received_json(message, handle, fifo));
                }
                Step::DeadLetter(key) => {
                    dead_letters.extend(queue.remove(key));
                }
            }
        }
        self.next_receipt_handle = next_handle;

        if let Some(target) = dead_letter_queue.and_then(|q| self.queues.get_mut(q.as_str())) {
            for message in dead_letters {
                target.push_dead_letter(message, now);
            }
        }
        Ok(members([("Messages", Value::Array(delivered))]))
    }

    pub(super) fn delete_message(&mut self, params: Params<'_>) -> Result<Members> {
        let [name, handle] = params.require(["QueueName", "ReceiptHandle"])?;
        let name = name.string()?;
        let handle = handle.string()?;

        let (queue, key) = self.in_flight_message(name, handle)?;
        queue.remove(key);
        Ok(Members::new())
    }

    pub(super) fn change_message_visibility(&mut self, params: Params<'_>) -> Result<Members> {
        let [name, handle, timeout] =
            params.require(["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
        let name = name.string()?;
        let handle = handle.string()?;
        let timeout = timeout.seconds(VISIBILITY_TIMEOUT_RANGE)?;

        let now = self.now;
        let (queue, key) = self.in_flight_message(name, handle)?;
        queue
            .message_mut(key)
            .expect("find_in_flight returned a key of a stored message")
            .invisible_until = now + timeout;
        Ok(Members::new())
    }

    /// Resolves a receipt handle to the queue and the arrival index of the
    /// in-flight message it currently stands for (SPEC section 6, rules 1
    /// and 2).
    fn in_flight_message(
        &mut self,
        queue_name: &str,
        handle: &str,
    ) -> Result<(&mut Queue, Arrival)> {
        let now = self.now;
        let issued = self.next_receipt_handle;
        let queue = lookup(&mut self.queues, now, queue_name)?;
        let handle = ReceiptHandle::parse(handle)
            .filter(|&n| n < issued)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    "receipt handle was never issued",
                )
            })?;
        let key = queue.find_in_flight(handle, now).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleExpired,
                "receipt handle no longer refers to an in-flight message",
            )
        })?;
        Ok((queue, key))
    }
}
