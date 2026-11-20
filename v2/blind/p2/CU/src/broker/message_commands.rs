//! Message commands (SPEC section 6).

use serde_json::{Value, json};

use super::{Broker, Members, lookup, members};
use crate::error::{Error, ErrorCode, Result};
use crate::queue::{Accepted, MAX_MESSAGES, Message, Queue, Step};
use crate::request::Params;

const MAX_BODY_BYTES: usize = 1024;
const MAX_ID_CHARS: usize = 128;

fn message_id(n: u64) -> String {
    format!("m-{n}")
}

fn receipt_handle(n: u64) -> String {
    format!("r-{n}")
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

fn missing(name: &str) -> Error {
    Error::new(ErrorCode::MissingParameter, format!("`{name}` is required"))
}

/// Parses `r-<n>` exactly as it was issued: no sign, no leading zeros.
fn parse_receipt_handle(text: &str) -> Option<u64> {
    let digits = text.strip_prefix("r-")?;
    if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn received_json(message: &Message, handle: u64, fifo: bool) -> Value {
    let mut json = json!({
        "MessageId": message_id(message.id),
        "ReceiptHandle": receipt_handle(handle),
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

/// The parameters of one send, validated (SPEC section 2, checks 2 and 3).
pub(super) struct SendArgs<'a> {
    body: &'a str,
    delay: Option<u64>,
    group_id: Option<&'a str>,
    deduplication_id: Option<&'a str>,
}

impl<'a> SendArgs<'a> {
    /// Reads `Body`, `DelaySeconds`, `GroupId` and `DeduplicationId`; the
    /// caller has checked that the required parameters are present.
    pub(super) fn parse(params: Params<'a>) -> Result<Self> {
        params.require(&["Body"])?;
        let body = params.string("Body")?.expect("required");
        if !(1..=MAX_BODY_BYTES).contains(&body.len()) {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("`Body` must be 1 to {MAX_BODY_BYTES} bytes"),
            ));
        }
        Ok(Self {
            body,
            delay: params.integer("DelaySeconds", 0..=900)?,
            group_id: params.identifier("GroupId", MAX_ID_CHARS)?,
            deduplication_id: params.identifier("DeduplicationId", MAX_ID_CHARS)?,
        })
    }
}

/// The counter `SendMessage` draws `MessageId`s from.
pub(super) struct IdCounter<'a>(pub &'a mut u64);

/// Sends one message to a queue that was already looked up: the numbered
/// rules of `SendMessage` (SPEC section 6).
pub(super) fn send_to_queue(
    queue: &mut Queue,
    next_message_id: IdCounter<'_>,
    now: u64,
    args: SendArgs<'_>,
) -> Result<Members> {
    let SendArgs {
        body,
        delay,
        group_id,
        deduplication_id,
    } = args;
    let attributes = &queue.attributes;
    if body.len() as u64 > attributes.max_message_bytes {
        return Err(Error::new(
            ErrorCode::InvalidParameterValue,
            format!(
                "`Body` is longer than the queue's MaxMessageBytes ({})",
                attributes.max_message_bytes
            ),
        ));
    }
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
                ("MessageId", json!(message_id(original.message_id))),
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

    let counter = next_message_id.0;
    let id = *counter;
    *counter += 1;
    let delay = delay.unwrap_or(queue.attributes.delay_seconds);
    let sequence_number = queue.push(Message {
        id,
        body: body.to_owned(),
        sent_at: now,
        receive_count: 0,
        first_received_at: None,
        group_id: fifo_ids.map(|(group, _)| group.to_owned()),
        sequence_number: None,
        delayed_until: now + delay,
        invisible_until: 0,
        handle: None,
    });
    let mut result = members([("MessageId", json!(message_id(id)))]);
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

/// Deletes the message a receipt handle stands for, in a queue that was
/// already looked up (SPEC section 6, rules 1 and 2).
pub(super) fn delete_in_queue(
    queue: &mut Queue,
    issued: u64,
    now: u64,
    handle: &str,
) -> Result<Members> {
    let key = resolve_handle(queue, issued, now, handle)?;
    queue.remove(key);
    Ok(Members::new())
}

/// Resolves a receipt handle to the arrival index of the in-flight message
/// it currently stands for (SPEC section 6, rules 1 and 2).
fn resolve_handle(queue: &Queue, issued: u64, now: u64, handle: &str) -> Result<u64> {
    let handle = parse_receipt_handle(handle)
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

impl Broker {
    pub(super) fn send_message(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Body"])?;
        let name = params.string("QueueName")?.expect("required");
        let args = SendArgs::parse(params)?;

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        send_to_queue(queue, IdCounter(&mut self.next_message_id), now, args)
    }

    pub(super) fn receive_message(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName"])?;
        let name = params.string("QueueName")?.expect("required");
        let max_messages = params.integer("MaxMessages", 1..=10)?.unwrap_or(1);
        let visibility_timeout = params.integer("VisibilityTimeout", 0..=43_200)?;

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
        for step in queue.plan_receive(now, max_messages as usize) {
            match step {
                Step::Deliver(key) => {
                    let handle = next_handle;
                    next_handle += 1;
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
        let name = params.string("QueueName")?.expect("required");
        let handle = params.string("ReceiptHandle")?.expect("required");

        let (now, issued) = (self.now, self.next_receipt_handle);
        let queue = lookup(&mut self.queues, now, name)?;
        delete_in_queue(queue, issued, now, handle)
    }

    pub(super) fn change_message_visibility(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
        let name = params.string("QueueName")?.expect("required");
        let handle = params.string("ReceiptHandle")?.expect("required");
        let timeout = params
            .integer("VisibilityTimeout", 0..=43_200)?
            .expect("required");

        let (now, issued) = (self.now, self.next_receipt_handle);
        let queue = lookup(&mut self.queues, now, name)?;
        let key = resolve_handle(queue, issued, now, handle)?;
        if let Some(message) = queue.message_mut(key) {
            message.invisible_until = now + timeout;
        }
        Ok(Members::new())
    }
}
