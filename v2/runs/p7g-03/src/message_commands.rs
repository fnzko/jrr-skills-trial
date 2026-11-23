//! Message commands (SPEC section 6).

use serde_json::{Map, Value, json};

use crate::error::{Code, Error, Result};
use crate::params::{Params, is_identifier};
use crate::queue::Queue;
use crate::queue::{Action, Message, State};
use crate::service::{Service, lookup, object};

const MAX_MESSAGES_PER_QUEUE: usize = 100_000;
const MAX_BODY_BYTES: usize = 1024;
const MAX_VISIBILITY_TIMEOUT: i64 = 43_200;

fn format_message_id(n: u64) -> String {
    format!("m-{n}")
}

/// Reads an optional identifier parameter (`GroupId`, `DeduplicationId`).
fn identifier<'a>(params: &Params<'a>, name: &str) -> Result<Option<&'a str>> {
    match params.string(name)? {
        Some(s) if !is_identifier(s, 128) => Err(Error::invalid_value(format!(
            "{name} must be 1 to 128 characters of A-Z a-z 0-9 _ -"
        ))),
        other => Ok(other),
    }
}

/// Parses a receipt handle: `r-<n>` with `n` written without leading zeros
/// and already issued, i.e. `1 <= n <= issued`.
fn parse_receipt_handle(handle: &str, issued: usize) -> Result<u64> {
    handle
        .strip_prefix("r-")
        .filter(|n| !n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|n| n.parse::<u64>().ok())
        .filter(|&n| n >= 1 && n <= issued as u64)
        .ok_or_else(|| {
            Error::new(
                Code::ReceiptHandleInvalid,
                format!("receipt handle {handle:?} was never issued"),
            )
        })
}

impl Service {
    pub(crate) fn send_message(&mut self, params: &Params) -> Result<Map<String, Value>> {
        params.require("QueueName")?;
        params.require("Body")?;
        let name = params.required_string("QueueName")?;
        let body = params.required_string("Body")?;
        if !(1..=MAX_BODY_BYTES).contains(&body.len()) {
            return Err(Error::invalid_value("Body must be 1 to 1024 bytes"));
        }
        let delay = params.integer("DelaySeconds", 0..=900)?;
        let group_id = identifier(params, "GroupId")?;
        let deduplication_id = identifier(params, "DeduplicationId")?;

        let now = self.now;
        let next_id = self.messages_accepted + 1;
        let queue = self.queue_mut(name)?;
        let fifo = queue.attributes.fifo;

        if !fifo && (group_id.is_some() || deduplication_id.is_some()) {
            return Err(Error::new(
                Code::UnsupportedOperation,
                "GroupId and DeduplicationId need a FIFO queue",
            ));
        }
        if fifo && delay.is_some() {
            return Err(Error::new(
                Code::UnsupportedOperation,
                "DelaySeconds per message is not supported on a FIFO queue",
            ));
        }
        let mut dedup_key = None;
        if fifo {
            if group_id.is_none() {
                return Err(Error::missing("GroupId"));
            }
            if deduplication_id.is_none() && !queue.attributes.content_based_deduplication {
                return Err(Error::missing("DeduplicationId"));
            }
            let id = deduplication_id.unwrap_or(body);
            if let Some((message_id, sequence)) = queue.duplicate_of(id, now) {
                return Ok(object([
                    ("MessageId", json!(format_message_id(message_id))),
                    ("SequenceNumber", json!(sequence)),
                ]));
            }
            dedup_key = Some(id.to_owned());
        }
        if queue.len() >= MAX_MESSAGES_PER_QUEUE {
            return Err(Error::new(
                Code::LimitExceeded,
                "the queue holds too many messages",
            ));
        }

        let delay = delay.map_or(queue.attributes.delay_seconds, |d| d as u64);
        let (_, sequence) = queue.push(Message {
            id: next_id,
            body: body.to_owned(),
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group_id: group_id.map(str::to_owned),
            sequence_number: None,
            delayed_until: now + delay,
            in_flight_until: 0,
            receipt_handle: None,
        });
        if let (Some(id), Some(sequence)) = (dedup_key, sequence) {
            queue.remember(id, now, next_id, sequence);
        }
        self.messages_accepted = next_id;

        let mut result = object([("MessageId", json!(format_message_id(next_id)))]);
        if let Some(sequence) = sequence {
            result.insert("SequenceNumber".into(), json!(sequence));
        }
        Ok(result)
    }

    pub(crate) fn receive_message(&mut self, params: &Params) -> Result<Map<String, Value>> {
        params.require("QueueName")?;
        let name = params.required_string("QueueName")?;
        let max_messages = params.integer("MaxMessages", 1..=10)?.unwrap_or(1) as usize;
        let visibility_timeout = params.integer("VisibilityTimeout", 0..=MAX_VISIBILITY_TIMEOUT)?;

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let uid = queue.uid;
        let visibility_timeout =
            visibility_timeout.map_or(queue.attributes.visibility_timeout, |v| v as u64);
        let dead_letter = queue
            .attributes
            .dead_letter
            .as_ref()
            .map(|d| d.queue.clone());

        let mut delivered = Vec::new();
        let mut dead = Vec::new();
        for (key, action) in queue.plan_receive(now, max_messages) {
            match action {
                Action::Deliver => {
                    self.deliveries.push((uid, key));
                    let handle = self.deliveries.len() as u64;
                    let message = queue.get_mut(key).expect("planned message exists");
                    message.receive_count += 1;
                    message.first_received_at.get_or_insert(now);
                    message.in_flight_until = now + visibility_timeout;
                    message.receipt_handle = Some(handle);
                    delivered.push(message_json(message, handle));
                }
                Action::MoveToDeadLetter => {
                    dead.push(queue.remove(key).expect("planned message exists"));
                }
            }
        }

        if !dead.is_empty() {
            let target =
                dead_letter.expect("only a queue with a dead-letter configuration moves messages");
            let target = self
                .queues
                .get_mut(&target)
                .expect("DeleteQueue clears references to a deleted queue");
            for mut message in dead {
                message.receive_count = 0;
                message.first_received_at = None;
                message.delayed_until = 0;
                message.in_flight_until = 0;
                message.receipt_handle = None;
                target.push(message);
            }
            // Retention counts from `SentAt`, so a message can arrive expired.
            target.expire(now);
        }
        Ok(object([("Messages", Value::Array(delivered))]))
    }

    pub(crate) fn delete_message(&mut self, params: &Params) -> Result<Map<String, Value>> {
        params.require("QueueName")?;
        params.require("ReceiptHandle")?;
        let name = params.required_string("QueueName")?;
        let handle = params.required_string("ReceiptHandle")?;
        let (queue, key) = self.in_flight_message(name, handle)?;
        queue.remove(key);
        Ok(Map::new())
    }

    pub(crate) fn change_message_visibility(
        &mut self,
        params: &Params,
    ) -> Result<Map<String, Value>> {
        params.require("QueueName")?;
        params.require("ReceiptHandle")?;
        params.require("VisibilityTimeout")?;
        let name = params.required_string("QueueName")?;
        let handle = params.required_string("ReceiptHandle")?;
        let timeout = params
            .integer("VisibilityTimeout", 0..=MAX_VISIBILITY_TIMEOUT)?
            .unwrap_or(0) as u64;
        let now = self.now;
        let (queue, key) = self.in_flight_message(name, handle)?;
        queue
            .get_mut(key)
            .expect("message was just found")
            .in_flight_until = now + timeout;
        Ok(Map::new())
    }

    /// Resolves a receipt handle to the queue `name` and the arrival key of the
    /// message it is the latest handle of, provided that message is in flight.
    fn in_flight_message(&mut self, name: &str, handle: &str) -> Result<(&mut Queue, u64)> {
        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let number = parse_receipt_handle(handle, self.deliveries.len())?;
        let (uid, key) = self.deliveries[number as usize - 1];
        let current = uid == queue.uid
            && queue.get(key).is_some_and(|m| {
                m.receipt_handle == Some(number) && m.state(now) == State::InFlight
            });
        if current {
            Ok((queue, key))
        } else {
            Err(Error::new(
                Code::ReceiptHandleExpired,
                format!("receipt handle {handle:?} is no longer usable"),
            ))
        }
    }
}

fn message_json(message: &Message, handle: u64) -> Value {
    let mut m = Map::new();
    m.insert("MessageId".into(), format_message_id(message.id).into());
    m.insert("ReceiptHandle".into(), format!("r-{handle}").into());
    m.insert("Body".into(), message.body.clone().into());
    m.insert("ReceiveCount".into(), message.receive_count.into());
    m.insert("SentAt".into(), message.sent_at.into());
    m.insert("FirstReceivedAt".into(), message.first_received_at.into());
    if let Some(group) = &message.group_id {
        m.insert("GroupId".into(), group.clone().into());
    }
    if let Some(sequence) = message.sequence_number {
        m.insert("SequenceNumber".into(), sequence.into());
    }
    Value::Object(m)
}
