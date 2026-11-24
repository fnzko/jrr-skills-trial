//! Commands that send, receive, delete and re-time messages.

use serde_json::{Value, json};

use crate::broker::{Broker, Fields, fields};
use crate::error::{Error, ErrorCode, Result};
use crate::params::Params;
use crate::queue::{MAX_MESSAGES, Message, Queue, State, is_valid_identifier};

const IDENTIFIER_RULE: &str = "1 to 128 characters from A-Z a-z 0-9 _ -";

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

impl Broker {
    pub(crate) fn send_message(&mut self, params: &Params) -> Result<Fields> {
        params.require(&["QueueName", "Body"])?;
        let name = params.string("QueueName")?.expect("required");
        let body = params
            .string_where(
                "Body",
                |body| (1..=1024).contains(&body.len()),
                "1 to 1024 bytes long",
            )?
            .expect("required");
        let delay = params.integer("DelaySeconds", 0..=900)?;
        let group_id = params.string_where("GroupId", is_valid_identifier, IDENTIFIER_RULE)?;
        let dedup_id =
            params.string_where("DeduplicationId", is_valid_identifier, IDENTIFIER_RULE)?;

        let now = self.now;
        let message_id = self.messages_accepted + 1;
        let queue = self.queue_mut(name)?;
        let fifo = queue.settings.fifo;
        if !fifo && (group_id.is_some() || dedup_id.is_some()) {
            return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
        }
        if fifo && delay.is_some() {
            return Err(unsupported(
                "DelaySeconds cannot be set per message on a FIFO queue",
            ));
        }
        let mut dedup_key = None;
        if fifo {
            if group_id.is_none() {
                return Err(missing("GroupId"));
            }
            if dedup_id.is_none() && !queue.settings.content_based_deduplication {
                return Err(missing("DeduplicationId"));
            }
            let key = dedup_id.unwrap_or(body);
            if let Some(original) = queue.duplicate_of(key, now) {
                return Ok(fields([
                    ("MessageId", json!(format!("m-{}", original.message_id))),
                    ("SequenceNumber", json!(original.sequence_number)),
                ]));
            }
            dedup_key = Some(key);
        }
        if queue.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }

        let delay = delay.map_or(queue.settings.delay_seconds, |d| d as u32);
        let message = Message::new(
            message_id,
            body.to_owned(),
            now,
            delay,
            group_id.map(str::to_owned),
        );
        let (_, sequence) = queue.push(message);
        let mut result = fields([("MessageId", json!(format!("m-{message_id}")))]);
        if let Some(sequence) = sequence {
            result.insert("SequenceNumber".into(), json!(sequence));
        }
        if let (Some(key), Some(sequence)) = (dedup_key, sequence) {
            queue.remember_dedup_id(key, now, message_id, sequence);
        }
        self.messages_accepted = message_id;
        Ok(result)
    }

    pub(crate) fn receive_message(&mut self, params: &Params) -> Result<Fields> {
        params.require(&["QueueName"])?;
        let name = params.string("QueueName")?.expect("required");
        let max = params.integer("MaxMessages", 1..=10)?.unwrap_or(1) as usize;
        let visibility = params.integer("VisibilityTimeout", 0..=43_200)?;

        let now = self.now;
        let first_handle = self.handles.len() as u64 + 1;
        let queue = self.queue_mut(name)?;
        let visibility = visibility.map_or(queue.settings.visibility_timeout, |v| v as u32);
        let dead_letter_queue = queue.settings.dead_letter.as_ref().map(|d| d.queue.clone());
        let queue_id = queue.id;
        let received = queue.receive(now, max, visibility, first_handle);

        self.handles
            .extend(received.delivered.iter().map(|(key, _)| (queue_id, *key)));
        if let Some(target) = dead_letter_queue.and_then(|name| self.queues.get_mut(&name)) {
            target.expire(now);
            for message in received.dead {
                target.push_dead_letter(message, now);
            }
        }

        let messages = received
            .delivered
            .iter()
            .map(|(_, message)| delivery_json(message))
            .collect();
        Ok(fields([("Messages", Value::Array(messages))]))
    }

    pub(crate) fn delete_message(&mut self, params: &Params) -> Result<Fields> {
        params.require(&["QueueName", "ReceiptHandle"])?;
        let name = params.string("QueueName")?.expect("required");
        let handle = params.string("ReceiptHandle")?.expect("required");
        let (queue, key) = self.in_flight_message(name, handle)?;
        queue.remove(key);
        Ok(Fields::new())
    }

    pub(crate) fn change_message_visibility(&mut self, params: &Params) -> Result<Fields> {
        params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
        let name = params.string("QueueName")?.expect("required");
        let handle = params.string("ReceiptHandle")?.expect("required");
        let timeout = params
            .integer("VisibilityTimeout", 0..=43_200)?
            .expect("required");
        let now = self.now;
        let (queue, key) = self.in_flight_message(name, handle)?;
        if let Some(message) = queue.message_mut(key) {
            message.in_flight_until = now + timeout as u64;
        }
        Ok(Fields::new())
    }

    /// Finds the message `handle` currently stands for in queue `name`; fails unless
    /// the handle was issued and is the latest one of a message that is in flight.
    fn in_flight_message(&mut self, name: &str, handle: &str) -> Result<(&mut Queue, u64)> {
        let now = self.now;
        let issued = parse_handle(handle, self.handles.len() as u64)
            .map(|number| (number, self.handles[number as usize - 1]));
        let queue = self.queue_mut(name)?;
        let (number, (queue_id, key)) = issued.ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("{handle:?} was never issued"),
            )
        })?;
        let usable = queue_id == queue.id
            && queue.message(key).is_some_and(|message| {
                message.latest_handle == Some(number) && message.state(now) == State::InFlight
            });
        if usable {
            Ok((queue, key))
        } else {
            Err(Error::new(
                ErrorCode::ReceiptHandleExpired,
                format!("{handle:?} no longer identifies a message in flight in this queue"),
            ))
        }
    }
}

/// The number of a handle `r-<n>` written exactly as miniq issues them, if issued.
fn parse_handle(handle: &str, issued: u64) -> Option<u64> {
    let digits = handle.strip_prefix("r-")?;
    if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|n| (1..=issued).contains(n))
}

fn delivery_json(message: &Message) -> Value {
    let mut object = json!({
        "MessageId": format!("m-{}", message.id),
        "ReceiptHandle": format!("r-{}", message.latest_handle.unwrap_or_default()),
        "Body": message.body,
        "ReceiveCount": message.receive_count,
        "SentAt": message.sent_at,
        "FirstReceivedAt": message.first_received_at,
    });
    if let Some(group) = &message.group_id {
        object["GroupId"] = json!(group);
    }
    if let Some(sequence) = message.sequence_number {
        object["SequenceNumber"] = json!(sequence);
    }
    object
}

fn missing(name: &str) -> Error {
    Error::new(
        ErrorCode::MissingParameter,
        format!("{name} is required on a FIFO queue"),
    )
}
