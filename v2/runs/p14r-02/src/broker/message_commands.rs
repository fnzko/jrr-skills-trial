//! Message commands (SPEC section 6).

use serde_json::{Value, json};

use super::{Broker, Members, lookup, members};
use crate::attributes::Attributes;
use crate::error::{Error, ErrorCode, Result};
use crate::ids::{ArrivalKey, ReceiptHandle};
use crate::queue::{Accepted, MAX_MESSAGES, Message, Step};
use crate::request::{Params, missing};

const MAX_BODY_BYTES: usize = 1024;
const MAX_ID_CHARS: usize = 128;

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
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

/// The identifiers a FIFO send needs, after the checks of SPEC section 6.
struct FifoIds<'a> {
    group_id: &'a str,
    deduplication_id: &'a str,
}

impl<'a> FifoIds<'a> {
    /// `None` for a standard queue; an error if the send's parameters do not
    /// fit the queue's type.
    fn resolve(
        attributes: &Attributes,
        group_id: Option<&'a str>,
        deduplication_id: Option<&'a str>,
        delay: Option<u64>,
        body: &'a str,
    ) -> Result<Option<Self>> {
        if !attributes.fifo {
            if group_id.is_some() || deduplication_id.is_some() {
                return Err(unsupported(
                    "`GroupId` and `DeduplicationId` need a FIFO queue",
                ));
            }
            return Ok(None);
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
        Ok(Some(Self {
            group_id,
            deduplication_id,
        }))
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
        let delay = params.integer("DelaySeconds", 0..=900)?;
        let group_id = params.identifier("GroupId", MAX_ID_CHARS)?;
        let deduplication_id = params.identifier("DeduplicationId", MAX_ID_CHARS)?;

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let fifo_ids =
            FifoIds::resolve(&queue.attributes, group_id, deduplication_id, delay, body)?;
        if let Some(ids) = &fifo_ids
            && let Some(original) = queue.duplicate_of(ids.deduplication_id, now)
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

        let id = self.next_message_id.take_next();
        let delay = delay.unwrap_or(queue.attributes.delay_seconds);
        let sequence_number = queue.push(Message {
            id,
            body: body.to_owned(),
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group_id: fifo_ids.as_ref().map(|ids| ids.group_id.to_owned()),
            sequence_number: None,
            delayed_until: now + delay,
            invisible_until: 0,
            handle: None,
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
            result.insert("SequenceNumber".into(), json!(sequence_number));
        }
        Ok(result)
    }

    pub(super) fn receive_message(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName"])?;
        let name = params.required_string("QueueName")?;
        let max_messages = params.integer("MaxMessages", 1..=10)?.unwrap_or(1);
        let visibility_timeout = params.integer("VisibilityTimeout", 0..=43_200)?;

        let now = self.now;
        let mut next_handle = self.next_receipt_handle;
        let queue = lookup(&mut self.queues, now, name)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.attributes.visibility_timeout);
        let fifo = queue.attributes.fifo;

        let mut delivered = Vec::new();
        let mut dead_letters = Vec::new();
        for step in queue.plan_receive(now, max_messages as usize) {
            match step {
                Step::Deliver(key) => {
                    let handle = next_handle.take_next();
                    let message = queue.deliver(key, now, visibility_timeout, handle);
                    delivered.push(received_json(message, handle, fifo));
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
        let timeout = params.required_integer("VisibilityTimeout", 0..=43_200)?;

        let key = self.in_flight_message(name, handle)?;
        let now = self.now;
        if let Some(message) = self.queue_mut(name)?.message_mut(key) {
            message.invisible_until = now + timeout;
        }
        Ok(Members::new())
    }

    /// Appends `messages` taken from queue `source` to its dead-letter queue.
    /// They are dropped if the queue has no dead-letter queue left.
    fn move_to_dead_letter_queue(&mut self, source: &str, messages: Vec<Message>) {
        let now = self.now;
        let target = self
            .queues
            .get(source)
            .and_then(|queue| queue.attributes.dead_letter.as_ref())
            .map(|dead_letter| dead_letter.queue.clone());
        let Some(target) = target.and_then(|target| self.queues.get_mut(&target)) else {
            return;
        };
        for message in messages {
            target.push_dead_letter(message, now);
        }
    }

    /// Resolves a receipt handle to the arrival index of the in-flight
    /// message it currently stands for (SPEC section 6, rules 1 and 2).
    fn in_flight_message(&mut self, queue_name: &str, handle: &str) -> Result<ArrivalKey> {
        let now = self.now;
        let issued = self.next_receipt_handle;
        let queue = self.queue_mut(queue_name)?;
        let handle = ReceiptHandle::parse(handle)
            .filter(|&n| n < issued)
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
