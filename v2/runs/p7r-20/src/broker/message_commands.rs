//! Message commands (SPEC section 6).

use serde_json::{Value, json};

use super::{Broker, Members, lookup, members};
use crate::error::{Error, ErrorCode, Result};
use crate::ids::ReceiptHandle;
use crate::limits;
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
        "SentAt": message.sent_at,
        "FirstReceivedAt": message.first_received_at,
    });
    if let Some(place) = &message.fifo {
        json["GroupId"] = json!(place.group_id);
        json["SequenceNumber"] = json!(place.sequence_number);
    }
    json
}

/// What a send needs from its queue's type (SPEC section 6, rules 1 to 3).
#[derive(Clone, Copy)]
enum SendTarget<'a> {
    Standard,
    Fifo {
        group_id: &'a str,
        deduplication_id: &'a str,
    },
}

impl<'a> SendTarget<'a> {
    fn resolve(
        queue: &Queue,
        body: &'a str,
        delay: Option<u64>,
        group_id: Option<&'a str>,
        deduplication_id: Option<&'a str>,
    ) -> Result<Self> {
        let attributes = &queue.attributes;
        if !attributes.fifo {
            if group_id.is_some() || deduplication_id.is_some() {
                return Err(unsupported(
                    "`GroupId` and `DeduplicationId` need a FIFO queue",
                ));
            }
            return Ok(Self::Standard);
        }
        if delay.is_some() {
            return Err(unsupported("FIFO queues do not support per-message delay"));
        }
        let group_id = group_id.ok_or_else(|| missing("GroupId"))?;
        let deduplication_id = match deduplication_id {
            Some(id) => id,
            None if attributes.content_based_deduplication => body,
            None => return Err(missing("DeduplicationId")),
        };
        Ok(Self::Fifo {
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
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("`Body` must be 1 to {MAX_BODY_BYTES} bytes"),
            ));
        }
        let delay = params.integer("DelaySeconds", limits::DELAY_SECONDS)?;
        let group_id = params.identifier("GroupId", MAX_ID_CHARS)?;
        let deduplication_id = params.identifier("DeduplicationId", MAX_ID_CHARS)?;

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let target = SendTarget::resolve(queue, body, delay, group_id, deduplication_id)?;
        if let SendTarget::Fifo {
            deduplication_id, ..
        } = target
            && let Some(original) = queue.duplicate_of(deduplication_id, now)
        {
            return Ok(members([
                ("MessageId", json!(original.message_id.to_string())),
                ("SequenceNumber", json!(original.sequence_number)),
            ]));
        }
        if queue.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }

        let id = self.next_message_id;
        self.next_message_id = id.next();
        let delay = delay.unwrap_or(queue.attributes.delay_seconds);
        let message = Message {
            id,
            body: body.to_owned(),
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            fifo: None,
            delayed_until: now + delay,
            invisible_until: 0,
            handle: None,
        };
        let mut result = members([("MessageId", json!(id.to_string()))]);
        match target {
            SendTarget::Standard => queue.push(message),
            SendTarget::Fifo {
                group_id,
                deduplication_id,
            } => {
                let sequence_number = queue.push_fifo(message, group_id.to_owned());
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
        }
        Ok(result)
    }

    pub(super) fn receive_message(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName"])?;
        let name = params.required_string("QueueName")?;
        let max_messages = params
            .integer("MaxMessages", limits::RECEIVE_BATCH)?
            .unwrap_or(1);
        let visibility_timeout = params.integer("VisibilityTimeout", limits::VISIBILITY_TIMEOUT)?;

        let now = self.now;
        let mut next_handle = self.next_receipt_handle;
        let queue = lookup(&mut self.queues, now, name)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.attributes.visibility_timeout);
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
                    delivered.push(received_json(message, handle));
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

        let key = self.in_flight_message(name, handle)?;
        self.queue_mut(name)?.remove(key);
        Ok(Members::new())
    }

    pub(super) fn change_message_visibility(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
        let name = params.required_string("QueueName")?;
        let handle = params.required_string("ReceiptHandle")?;
        let timeout = params.required_integer("VisibilityTimeout", limits::VISIBILITY_TIMEOUT)?;

        let key = self.in_flight_message(name, handle)?;
        let now = self.now;
        self.queue_mut(name)?
            .message_mut(key)
            .expect("in_flight_message found this key in the same queue")
            .invisible_until = now + timeout;
        Ok(Members::new())
    }

    /// Resolves a receipt handle to the arrival index of the in-flight
    /// message it currently stands for (SPEC section 6, rules 1 and 2).
    fn in_flight_message(&mut self, queue_name: &str, handle: &str) -> Result<u64> {
        let now = self.now;
        let issued = self.next_receipt_handle;
        let queue = self.queue_mut(queue_name)?;
        let handle = ReceiptHandle::parse(handle)
            .filter(|&handle| handle < issued)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    "receipt handle was never issued",
                )
            })?;
        queue.find_in_flight(handle, now).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleExpired,
                "receipt handle no longer refers to an in-flight message",
            )
        })
    }
}
