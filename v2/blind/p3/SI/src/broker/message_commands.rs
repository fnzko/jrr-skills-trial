//! Message commands (SPEC section 6).

use std::sync::Arc;

use serde_json::{Value, json};

use super::{Broker, Members, lookup, members, queue_name};
use crate::attributes::Attributes;
use crate::error::{Error, ErrorCode, Result};
use crate::ids::{ArrivalIndex, ReceiptHandle};
use crate::queue::{Accepted, MAX_MESSAGES, Message, NewMessage, Step};
use crate::request::Params;

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
        "ReceiveCount": message.receive_count(),
        "SentAt": message.sent_at,
        "FirstReceivedAt": message.first_received_at(),
    });
    if let Some(position) = &message.fifo {
        json["GroupId"] = json!(position.group_id);
        json["SequenceNumber"] = Value::from(position.sequence_number);
    }
    json
}

/// The identifiers a FIFO send carries.
struct FifoIds<'a> {
    group_id: &'a str,
    deduplication_id: &'a str,
}

impl<'a> FifoIds<'a> {
    /// Applies SPEC section 6 rules on `GroupId`, `DeduplicationId` and
    /// `DelaySeconds` for this queue type; `None` for a standard queue.
    fn resolve(
        attributes: &Attributes,
        group_id: Option<&'a str>,
        deduplication_id: Option<&'a str>,
        has_delay: bool,
        body: &'a str,
    ) -> Result<Option<Self>> {
        if !attributes.fifo {
            return if group_id.is_some() || deduplication_id.is_some() {
                Err(unsupported(
                    "`GroupId` and `DeduplicationId` need a FIFO queue",
                ))
            } else {
                Ok(None)
            };
        }
        if has_delay {
            return Err(unsupported("FIFO queues do not support per-message delay"));
        }
        let group_id = group_id.ok_or_else(|| Error::missing_parameter("GroupId"))?;
        let deduplication_id = match deduplication_id {
            Some(id) => id,
            None if attributes.content_based_deduplication => body,
            None => return Err(Error::missing_parameter("DeduplicationId")),
        };
        Ok(Some(Self {
            group_id,
            deduplication_id,
        }))
    }
}

impl Broker {
    pub(super) fn send_message(&mut self, params: Params<'_>) -> Result<Members> {
        let [name, body] = params.require(["QueueName", "Body"])?;
        let (name, body) = (name.string()?, body.string()?);
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
        let fifo_ids = FifoIds::resolve(
            &queue.attributes,
            group_id,
            deduplication_id,
            delay.is_some(),
            body,
        )?;
        if let Some(original) = fifo_ids
            .as_ref()
            .and_then(|ids| queue.duplicate_of(ids.deduplication_id, now))
        {
            return Ok(members([
                ("MessageId", json!(original.message_id.to_string())),
                ("SequenceNumber", Value::from(original.sequence_number)),
            ]));
        }
        if queue.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }

        let id = self.next_message_id.take();
        let delay = delay.unwrap_or(queue.attributes.delay_seconds);
        let sequence_number = queue.push(NewMessage {
            id,
            body: body.to_owned(),
            sent_at: now,
            group_id: fifo_ids.as_ref().map(|ids| ids.group_id.to_owned()),
            delayed_until: now + delay,
        });
        let mut result = members([("MessageId", json!(id.to_string()))]);
        if let (Some(ids), Some(sequence_number)) = (fifo_ids, sequence_number) {
            queue.record_deduplication(
                ids.deduplication_id,
                Accepted {
                    accepted_at: now,
                    message_id: id,
                    sequence_number,
                },
            );
            result.insert("SequenceNumber".into(), Value::from(sequence_number));
        }
        Ok(result)
    }

    pub(super) fn receive_message(&mut self, params: Params<'_>) -> Result<Members> {
        let name = queue_name(params)?;
        let max_messages = params.integer("MaxMessages", 1..=10)?.unwrap_or(1);
        let visibility_timeout = params.integer("VisibilityTimeout", 0..=43_200)?;

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.attributes.visibility_timeout);
        let dead_letter_queue = queue
            .attributes
            .dead_letter
            .as_ref()
            .map(|dl| Arc::clone(&dl.queue));

        let mut delivered = Vec::new();
        let mut dead_letters = Vec::new();
        for step in queue.plan_receive(now, max_messages) {
            match step {
                Step::Deliver(key) => {
                    let handle = self.next_receipt_handle.take();
                    let message = queue.deliver(key, now, visibility_timeout, handle);
                    delivered.push(received_json(message, handle));
                }
                Step::DeadLetter(key) => {
                    dead_letters.extend(queue.remove(key));
                }
            }
        }

        if let Some(target) = dead_letter_queue.and_then(|q| self.queues.get_mut(&*q)) {
            for message in dead_letters {
                target.push_dead_letter(message, now);
            }
        }
        Ok(members([("Messages", Value::Array(delivered))]))
    }

    pub(super) fn delete_message(&mut self, params: Params<'_>) -> Result<Members> {
        let [name, handle] = params.require(["QueueName", "ReceiptHandle"])?;
        let (name, handle) = (name.string()?, handle.string()?);

        let key = self.in_flight_message(name, handle)?;
        self.queue_mut(name)?.remove(key);
        Ok(Members::new())
    }

    pub(super) fn change_message_visibility(&mut self, params: Params<'_>) -> Result<Members> {
        let [name, handle, timeout] =
            params.require(["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
        let (name, handle) = (name.string()?, handle.string()?);
        let timeout = timeout.integer(0..=43_200)?;

        let key = self.in_flight_message(name, handle)?;
        let now = self.now;
        if let Some(message) = self.queue_mut(name)?.message_mut(key) {
            message.invisible_until = now + timeout;
        }
        Ok(Members::new())
    }

    /// Resolves a receipt handle to the arrival index of the in-flight
    /// message it currently stands for (SPEC section 6, rules 1 and 2).
    fn in_flight_message(&mut self, queue_name: &str, handle: &str) -> Result<ArrivalIndex> {
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
