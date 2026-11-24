//! Message commands (SPEC section 6).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::{Broker, Members, lookup, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::queue::{Accepted, MAX_MESSAGES, Message, Step};
use crate::request::Params;

const MAX_BODY_BYTES: usize = 1024;
const MAX_ID_CHARS: usize = 128;
const MAX_BATCH_ENTRIES: usize = 10;
const MAX_BATCH_ID_CHARS: usize = 80;

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

/// One entry of a batch request whose `Id` has passed the batch rules.
struct BatchEntry<'a> {
    id: &'a str,
    params: Params<'a>,
}

/// Applies the numbered rules shared by both batch commands.
fn batch_entries(entries: &[Value]) -> Result<Vec<BatchEntry<'_>>> {
    if entries.is_empty() {
        return Err(Error::new(
            ErrorCode::EmptyBatchRequest,
            "`Entries` has no entry",
        ));
    }
    if entries.len() > MAX_BATCH_ENTRIES {
        return Err(Error::new(
            ErrorCode::TooManyEntriesInBatchRequest,
            format!("`Entries` has more than {MAX_BATCH_ENTRIES} entries"),
        ));
    }
    let parsed = entries
        .iter()
        .map(|entry| {
            let members = entry
                .as_object()
                .ok_or_else(|| invalid_entry_id("is not an object"))?;
            let params = Params::new(members);
            let id = members
                .get("Id")
                .and_then(Value::as_str)
                .filter(|id| naming::is_identifier(id, MAX_BATCH_ID_CHARS))
                .ok_or_else(|| invalid_entry_id("has no valid `Id`"))?;
            Ok(BatchEntry { id, params })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if let Some(duplicate) = parsed.iter().find(|entry| !seen.insert(entry.id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            format!("entry `Id` `{}` appears twice", duplicate.id),
        ));
    }
    Ok(parsed)
}

fn invalid_entry_id(problem: &str) -> Error {
    Error::new(
        ErrorCode::InvalidBatchEntryId,
        format!("a batch entry {problem}"),
    )
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

impl Broker {
    pub(super) fn send_message(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Body"])?;
        let name = params.string("QueueName")?.expect("required");
        self.send_to_queue(name, params)
    }

    /// `SendMessage` from the queue lookup on. `params` holds the single
    /// command's parameters other than `QueueName`, and `Body` is present.
    fn send_to_queue(&mut self, name: &str, params: Params<'_>) -> Result<Members> {
        let body = params.string("Body")?.expect("required");
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
        if !u64::try_from(body.len()).is_ok_and(|len| len <= attributes.max_message_bytes) {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!(
                    "`Body` must be at most {} bytes in this queue",
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

        let id = self.next_message_id;
        self.next_message_id += 1;
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

    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, name, entry| {
            entry.require(&["Body"])?;
            broker.send_to_queue(name, entry)
        })
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, name, entry| {
            entry.require(&["ReceiptHandle"])?;
            broker.delete_from_queue(name, entry)
        })
    }

    /// Validates a batch request and handles its entries in order with
    /// `handle`, which acts as the single command would from step 2 on.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        handle: fn(&mut Self, &str, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.required_string("QueueName")?;
        let entries = params.required_array("Entries")?;
        self.queue_mut(name)?;
        let entries = batch_entries(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for BatchEntry { id, params } in entries {
            match handle(self, name, params) {
                Ok(mut result) => {
                    result.insert("Id".into(), json!(id));
                    successful.push(Value::Object(result));
                }
                Err(error) => failed.push(json!({ "Id": id, "error": error.to_json() })),
            }
        }
        Ok(members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
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
        self.delete_from_queue(name, params)
    }

    /// `DeleteMessage` from the queue lookup on. `params` holds the single
    /// command's parameters other than `QueueName`, and `ReceiptHandle` is
    /// present.
    fn delete_from_queue(&mut self, name: &str, params: Params<'_>) -> Result<Members> {
        let handle = params.string("ReceiptHandle")?.expect("required");

        let key = self.in_flight_message(name, handle)?;
        self.queue_mut(name)?.remove(key);
        Ok(Members::new())
    }

    pub(super) fn change_message_visibility(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
        let name = params.string("QueueName")?.expect("required");
        let handle = params.string("ReceiptHandle")?.expect("required");
        let timeout = params
            .integer("VisibilityTimeout", 0..=43_200)?
            .expect("required");

        let key = self.in_flight_message(name, handle)?;
        let now = self.now;
        if let Some(message) = self.queue_mut(name)?.message_mut(key) {
            message.invisible_until = now + timeout;
        }
        Ok(Members::new())
    }

    /// Resolves a receipt handle to the arrival index of the in-flight
    /// message it currently stands for (SPEC section 6, rules 1 and 2).
    fn in_flight_message(&mut self, queue_name: &str, handle: &str) -> Result<u64> {
        let now = self.now;
        let issued = self.next_receipt_handle;
        let queue = self.queue_mut(queue_name)?;
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
}
