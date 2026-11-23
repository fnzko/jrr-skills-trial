//! Message commands (SPEC section 6).

use serde_json::{Value, json};

use super::{Broker, Members, lookup, members};
use crate::attributes::{MAX_DELAY_SECONDS, MAX_VISIBILITY_TIMEOUT};
use crate::error::{Error, ErrorCode, Result};
use crate::ids::ReceiptHandle;
use crate::queue::{Accepted, MAX_MESSAGES, Message, Step};
use crate::request::Params;

const MAX_BODY_BYTES: usize = 1024;
const MAX_ID_CHARS: usize = 128;

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

/// The parameters of a send to a FIFO queue.
struct FifoSend<'a> {
    group_id: &'a str,
    deduplication_id: &'a str,
}

impl<'a> FifoSend<'a> {
    /// SPEC section 6, rules 2 and 3. Without a `DeduplicationId` the body
    /// itself is the deduplication id if the queue deduplicates by content.
    fn resolve(
        group_id: Option<&'a str>,
        deduplication_id: Option<&'a str>,
        body: &'a str,
        content_based_deduplication: bool,
    ) -> Result<Self> {
        let group_id = group_id.ok_or_else(|| Error::missing_parameter("GroupId"))?;
        let deduplication_id = match deduplication_id {
            Some(id) => id,
            None if content_based_deduplication => body,
            None => return Err(Error::missing_parameter("DeduplicationId")),
        };
        Ok(Self {
            group_id,
            deduplication_id,
        })
    }
}

impl Broker {
    pub(super) fn send_message(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Body"])?;
        let name = params.required_string("QueueName")?;
        let body = params.required_string("Body")?;
        if !(1..=MAX_BODY_BYTES).contains(&body.len()) {
            return Err(Error::invalid_parameter(format!(
                "`Body` must be 1 to {MAX_BODY_BYTES} bytes"
            )));
        }
        let delay = params.integer("DelaySeconds", 0..=MAX_DELAY_SECONDS)?;
        let group_id = params.identifier("GroupId", MAX_ID_CHARS)?;
        let deduplication_id = params.identifier("DeduplicationId", MAX_ID_CHARS)?;

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let attributes = &queue.attributes;
        if !attributes.fifo && (group_id.is_some() || deduplication_id.is_some()) {
            return Err(Error::unsupported(
                "`GroupId` and `DeduplicationId` need a FIFO queue",
            ));
        }
        if attributes.fifo && delay.is_some() {
            return Err(Error::unsupported(
                "FIFO queues do not support per-message delay",
            ));
        }
        let fifo = if attributes.fifo {
            let fifo = FifoSend::resolve(
                group_id,
                deduplication_id,
                body,
                attributes.content_based_deduplication,
            )?;
            if let Some(original) = queue.duplicate_of(fifo.deduplication_id, now) {
                return Ok(members([
                    ("MessageId", json!(original.message_id.to_string())),
                    ("SequenceNumber", json!(original.sequence_number)),
                ]));
            }
            Some(fifo)
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
        let delay = delay.unwrap_or(queue.attributes.delay_seconds);
        let message = Message::new(
            id,
            body,
            now,
            fifo.as_ref().map(|fifo| fifo.group_id),
            now + delay,
        );
        let sequence_number = queue.push(message);
        let mut result = members([("MessageId", json!(id.to_string()))]);
        if let Some(fifo) = fifo {
            let sequence_number = sequence_number.expect("a FIFO queue numbers its messages");
            queue.record_deduplication(
                fifo.deduplication_id,
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
        let visibility_timeout = params.integer("VisibilityTimeout", 0..=MAX_VISIBILITY_TIMEOUT)?;

        let now = self.now;
        let mut next_handle = self.next_receipt_handle;
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

        if let Some(target) = dead_letter_queue.and_then(|q| self.queues.get_mut(&q)) {
            for message in dead_letters {
                target.push_dead_letter(message, now);
            }
        }
        Ok(members([("Messages", Value::Array(delivered))]))
    }

    pub(super) fn delete_message(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "ReceiptHandle"])?;
        let name = params.required_string("QueueName")?;
        let handle = params.required_string("ReceiptHandle")?;

        let now = self.now;
        let issued = self.next_receipt_handle;
        let queue = lookup(&mut self.queues, now, name)?;
        let handle = parse_issued_handle(handle, issued)?;
        queue
            .remove_in_flight(handle, now)
            .ok_or_else(expired_handle)?;
        Ok(Members::new())
    }

    pub(super) fn change_message_visibility(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
        let name = params.required_string("QueueName")?;
        let handle = params.required_string("ReceiptHandle")?;
        let timeout = params.required_integer("VisibilityTimeout", 0..=MAX_VISIBILITY_TIMEOUT)?;

        let now = self.now;
        let issued = self.next_receipt_handle;
        let queue = lookup(&mut self.queues, now, name)?;
        let handle = parse_issued_handle(handle, issued)?;
        let message = queue
            .in_flight_mut(handle, now)
            .ok_or_else(expired_handle)?;
        message.invisible_until = now + timeout;
        Ok(Members::new())
    }
}

/// SPEC section 6, rule 1: `text` must be a handle below `next_unissued`.
fn parse_issued_handle(text: &str, next_unissued: ReceiptHandle) -> Result<ReceiptHandle> {
    ReceiptHandle::parse(text)
        .filter(|&handle| handle < next_unissued)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                "receipt handle was never issued",
            )
        })
}

/// SPEC section 6, rule 2.
fn expired_handle() -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        "receipt handle no longer refers to an in-flight message",
    )
}
