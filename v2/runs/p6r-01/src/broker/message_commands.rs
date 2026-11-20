//! Message commands (SPEC section 6).

use serde_json::{Value, json};

use super::{Broker, Members, lookup, members};
use crate::error::{Error, ErrorCode, Result};
use crate::ids::{Arrival, ReceiptHandle};
use crate::queue::{Accepted, MAX_MESSAGES, Message, Queue, Step};
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
        "ReceiveCount": message.receive_count,
        "SentAt": message.sent_at,
        "FirstReceivedAt": message.first_received_at,
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
        let name = params
            .string("QueueName")?
            .expect("`require` has checked that it is present");
        let body = params
            .string("Body")?
            .expect("`require` has checked that it is present");
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
        let attributes = queue.attributes();
        if !attributes.fifo && (group_id.is_some() || deduplication_id.is_some()) {
            return Err(unsupported(
                "`GroupId` and `DeduplicationId` need a FIFO queue",
            ));
        }
        if attributes.fifo && delay.is_some() {
            return Err(unsupported("FIFO queues do not support per-message delay"));
        }
        let fifo_ids = if attributes.fifo {
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
        let delay = delay.unwrap_or(queue.attributes().delay_seconds);
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
        let group_id = fifo_ids.map(|(group, _)| group.to_owned());
        let sequence_number = queue.push(message, group_id);
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
        let name = params
            .string("QueueName")?
            .expect("`require` has checked that it is present");
        let max_messages = usize::try_from(params.integer("MaxMessages", 1..=10)?.unwrap_or(1))
            .expect("MaxMessages is at most 10");
        let visibility_timeout = params.integer("VisibilityTimeout", 0..=43_200)?;

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let visibility_timeout =
            visibility_timeout.unwrap_or(queue.attributes().visibility_timeout);
        let dead_letter_queue = queue
            .attributes()
            .dead_letter
            .as_ref()
            .map(|dl| dl.queue.clone());

        let mut delivered = Vec::new();
        let mut dead_letters = Vec::new();
        for step in queue.plan_receive(now, max_messages) {
            match step {
                Step::Deliver(key) => {
                    let handle = self.next_receipt_handle.take_next();
                    let message = queue.deliver(key, now, visibility_timeout, handle);
                    delivered.push(received_json(message, handle));
                }
                Step::DeadLetter(key) => {
                    dead_letters.push(queue.remove(key).expect("planned message exists"));
                }
            }
        }

        if let Some(target) = dead_letter_queue {
            let target = self
                .queues
                .get_mut(&target)
                .expect("DeleteQueue clears every dead-letter configuration that names the queue");
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
            .expect("`require` has checked that it is present");
        let handle = params
            .string("ReceiptHandle")?
            .expect("`require` has checked that it is present");

        let (queue, key) = self.in_flight_message(name, handle)?;
        queue.remove(key);
        Ok(Members::new())
    }

    pub(super) fn change_message_visibility(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
        let name = params
            .string("QueueName")?
            .expect("`require` has checked that it is present");
        let handle = params
            .string("ReceiptHandle")?
            .expect("`require` has checked that it is present");
        let timeout = params
            .integer("VisibilityTimeout", 0..=43_200)?
            .expect("`require` has checked that it is present");

        let now = self.now;
        let (queue, key) = self.in_flight_message(name, handle)?;
        queue
            .message_mut(key)
            .expect("find_in_flight returned a message that is in the queue")
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
            .filter(|&handle| handle < issued)
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
