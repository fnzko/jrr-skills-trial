//! The command dispatcher and the process-wide state (clock and counters).

use std::collections::BTreeMap;
use std::ops::Bound;

use serde_json::{Map, Value, json};

use crate::attributes::{AttributeSet, Attributes};
use crate::error::{Error, ErrorCode, Result};
use crate::params::Params;
use crate::queue::{Accepted, MAX_QUEUES, Message, Queue, is_fifo_name, is_valid_queue_name};

type Members = Map<String, Value>;

pub struct Service {
    now: i64,
    /// Messages accepted so far; the next `MessageId` is one more.
    messages_accepted: u64,
    /// Receipt handles issued so far; the next handle is `r-<n+1>`.
    deliveries: u64,
    /// Ordered by name; `String` ordering is byte-wise, as `ListQueues` requires.
    queues: BTreeMap<String, Queue>,
}

impl Default for Service {
    fn default() -> Self {
        Self::new()
    }
}

impl Service {
    pub fn new() -> Self {
        Self {
            now: 0,
            messages_accepted: 0,
            deliveries: 0,
            queues: BTreeMap::new(),
        }
    }

    /// Runs one input line (without its `\n`) and returns the response line.
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        let response = match self.execute(line) {
            Ok(mut members) => {
                members.insert("ok".into(), Value::Bool(true));
                Value::Object(members)
            }
            Err(err) => json!({
                "ok": false,
                "error": { "code": err.code.as_str(), "message": err.message },
            }),
        };
        response.to_string()
    }

    fn execute(&mut self, line: &[u8]) -> Result<Members> {
        let request = parse_request(line)?;
        let command = match request.get("Command") {
            Some(Value::String(c)) => c.as_str(),
            _ => return Err(invalid_request("Command must be a string")),
        };
        let params = Params::new(&request);
        match command {
            "CreateQueue" => self.create_queue(&params),
            "SetQueueAttributes" => self.set_queue_attributes(&params),
            "GetQueueAttributes" => self.get_queue_attributes(&params),
            "ListQueues" => self.list_queues(&params),
            "DeleteQueue" => self.delete_queue(&params),
            "PurgeQueue" => self.purge_queue(&params),
            "SendMessage" => self.send_message(&params),
            "ReceiveMessage" => self.receive_message(&params),
            "DeleteMessage" => self.delete_message(&params),
            "ChangeMessageVisibility" => self.change_message_visibility(&params),
            "Tick" => self.tick(&params),
            other => Err(invalid_request(format!("unknown command {other:?}"))),
        }
    }

    /// Finds a queue for a command (rule 4 of the error order), dropping any
    /// messages whose retention has run out.
    fn queue_mut(&mut self, name: &str) -> Result<&mut Queue> {
        lookup(&mut self.queues, self.now, name)
    }

    fn create_queue(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        if !is_valid_queue_name(name) {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("invalid queue name {name:?}"),
            ));
        }
        let set = parse_attributes(p)?.unwrap_or_default();
        let attributes = Attributes::from_set(&set);
        if is_fifo_name(name) != attributes.fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "the .fifo suffix and the Fifo attribute must agree",
            ));
        }
        if attributes.content_based_deduplication && !attributes.fifo {
            return Err(invalid_value(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        if set.has_partial_dead_letter() {
            return Err(invalid_value(
                "DeadLetterQueue and MaxReceiveCount go together",
            ));
        }
        if let Some(existing) = self.queues.get(name) {
            return if existing.attributes == attributes {
                Ok(Members::new())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {name:?} exists with different attributes"),
                ))
            };
        }
        if let Some(dead_letter) = &attributes.dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| does_not_exist(&dead_letter.queue))?;
            if target.attributes.fifo != attributes.fifo {
                return Err(invalid_value(
                    "the dead-letter queue must be of the same type",
                ));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(ErrorCode::LimitExceeded, "too many queues"));
        }
        self.queues.insert(name.to_owned(), Queue::new(attributes));
        Ok(Members::new())
    }

    fn set_queue_attributes(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName", "Attributes"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        let set = parse_attributes(p)?.unwrap_or_default();
        let fifo = self.queue_mut(name)?.attributes.fifo;
        if set.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue never changes",
            ));
        }
        if set.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid_value(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        if set.has_partial_dead_letter() {
            return Err(invalid_value(
                "DeadLetterQueue and MaxReceiveCount go together",
            ));
        }
        if let Some(target_name) = &set.dead_letter_queue {
            let target = self
                .queues
                .get(target_name)
                .ok_or_else(|| does_not_exist(target_name))?;
            if target_name == name || target.attributes.fifo != fifo {
                return Err(invalid_value("invalid dead-letter queue"));
            }
        }
        let now = self.now;
        let queue = self.queue_mut(name)?;
        queue.attributes.apply(&set);
        queue.expire(now);
        Ok(Members::new())
    }

    fn get_queue_attributes(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        let now = self.now;
        let queue = self.queue_mut(name)?;
        let counts = queue.counts(now);
        Ok(members([
            ("Attributes", queue.attributes.to_json()),
            (
                "Counts",
                json!({
                    "Visible": counts.visible,
                    "InFlight": counts.in_flight,
                    "Delayed": counts.delayed,
                }),
            ),
        ]))
    }

    fn list_queues(&mut self, p: &Params) -> Result<Members> {
        let prefix = p.string("Prefix")?.unwrap_or_default();
        let names: Vec<&String> = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name)
            .take_while(|name| name.starts_with(prefix))
            .collect();
        Ok(members([("QueueNames", json!(names))]))
    }

    fn delete_queue(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        self.queue_mut(name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            if queue
                .attributes
                .dead_letter
                .as_ref()
                .is_some_and(|dl| dl.queue == name)
            {
                queue.attributes.dead_letter = None;
            }
        }
        Ok(Members::new())
    }

    fn purge_queue(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        self.queue_mut(name)?.purge();
        Ok(Members::new())
    }

    fn send_message(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName", "Body"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        let body = p.string_bytes("Body", 1..=1024)?.unwrap_or_default();
        let delay = p.int("DelaySeconds", 0..=900)?;
        let group_id = p.identifier("GroupId")?;
        let dedup_id = p.identifier("DeduplicationId")?;
        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let fifo = queue.attributes.fifo;
        if !fifo && (group_id.is_some() || dedup_id.is_some()) {
            return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
        }
        if fifo && delay.is_some() {
            return Err(unsupported(
                "DelaySeconds per message is not supported on a FIFO queue",
            ));
        }
        let mut dedup_key = None;
        if fifo {
            if group_id.is_none() {
                return Err(missing("GroupId"));
            }
            if dedup_id.is_none() && !queue.attributes.content_based_deduplication {
                return Err(missing("DeduplicationId"));
            }
            let key = dedup_id.unwrap_or(body);
            if let Some(original) = queue.duplicate_of(key, now) {
                return Ok(members([
                    ("MessageId", json!(format!("m-{}", original.message_id))),
                    ("SequenceNumber", json!(original.sequence)),
                ]));
            }
            dedup_key = Some(key);
        }
        if queue.is_full() {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "queue holds too many messages",
            ));
        }
        let delay = delay.unwrap_or(queue.attributes.delay_seconds);
        self.messages_accepted += 1;
        let id = self.messages_accepted;
        let sequence = queue.push(Message {
            id,
            body: body.to_owned(),
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group_id: group_id.map(str::to_owned),
            sequence: None,
            delayed_until: now + delay,
            in_flight_until: 0,
            receipt: None,
        });
        let mut result = members([("MessageId", json!(format!("m-{id}")))]);
        if let (Some(key), Some(sequence)) = (dedup_key, sequence) {
            queue.remember(
                key,
                Accepted {
                    at: now,
                    message_id: id,
                    sequence,
                },
            );
            result.insert("SequenceNumber".into(), json!(sequence));
        }
        Ok(result)
    }

    fn receive_message(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        let max = p.int("MaxMessages", 1..=10)?.unwrap_or(1);
        let visibility = p.int("VisibilityTimeout", 0..=43_200)?;
        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let visibility = visibility.unwrap_or(queue.attributes.visibility_timeout);
        let fifo = queue.attributes.fifo;
        let dead_letter = queue
            .attributes
            .dead_letter
            .as_ref()
            .map(|d| d.queue.clone());
        let (delivered, dead) = queue.receive(now, max as usize, visibility, &mut self.deliveries);
        if let Some(target) = dead_letter {
            // The target exists: deleting a queue clears every reference to it.
            let target = lookup(&mut self.queues, now, &target)?;
            for message in dead {
                target.push(message);
            }
            target.expire(now);
        }
        let messages: Vec<Value> = delivered
            .iter()
            .map(|m| {
                let mut v = json!({
                    "MessageId": format!("m-{}", m.id),
                    "ReceiptHandle": format!("r-{}", m.receipt.unwrap_or_default()),
                    "Body": m.body,
                    "ReceiveCount": m.receive_count,
                    "SentAt": m.sent_at,
                    "FirstReceivedAt": m.first_received_at,
                });
                if fifo {
                    v["GroupId"] = json!(m.group_id);
                    v["SequenceNumber"] = json!(m.sequence);
                }
                v
            })
            .collect();
        Ok(members([("Messages", Value::Array(messages))]))
    }

    fn delete_message(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName", "ReceiptHandle"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        let handle = p.string("ReceiptHandle")?.unwrap_or_default();
        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let index = find_in_flight(queue, handle, self.deliveries, now)?;
        queue.messages.remove(index);
        Ok(Members::new())
    }

    fn change_message_visibility(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        let handle = p.string("ReceiptHandle")?.unwrap_or_default();
        let timeout = p.int("VisibilityTimeout", 0..=43_200)?.unwrap_or_default();
        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let index = find_in_flight(queue, handle, self.deliveries, now)?;
        queue.messages[index].in_flight_until = now + timeout;
        Ok(Members::new())
    }

    fn tick(&mut self, p: &Params) -> Result<Members> {
        p.require(&["Seconds"])?;
        let seconds = p.int("Seconds", 0..=1_209_600)?.unwrap_or(0);
        self.now += seconds;
        Ok(members([("Now", json!(self.now))]))
    }
}

fn invalid_value(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

/// Rule 4 of the error order: finds the queue and drops expired messages.
/// A free function so callers can keep borrowing other fields of `Service`.
fn lookup<'q>(
    queues: &'q mut BTreeMap<String, Queue>,
    now: i64,
    name: &str,
) -> Result<&'q mut Queue> {
    let queue = queues.get_mut(name).ok_or_else(|| does_not_exist(name))?;
    queue.expire(now);
    Ok(queue)
}

/// Applies the receipt handle rules (SPEC section 6): a handle that was never
/// issued is invalid; one that is not the latest handle of an in-flight message
/// of this queue is expired. Returns the message's position.
fn find_in_flight(queue: &Queue, handle: &str, issued: u64, now: i64) -> Result<usize> {
    let number = parse_handle(handle)
        .filter(|n| (1..=issued).contains(n))
        .ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("receipt handle {handle:?} was never issued"),
            )
        })?;
    queue.find_in_flight(number, now).ok_or_else(|| {
        Error::new(
            ErrorCode::ReceiptHandleExpired,
            format!("receipt handle {handle:?} is no longer usable"),
        )
    })
}

/// Parses exactly the canonical spelling `r-<n>` (no sign, no leading zeros).
fn parse_handle(handle: &str) -> Option<u64> {
    let digits = handle.strip_prefix("r-")?;
    if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn unsupported(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

fn missing(name: &str) -> Error {
    Error::new(ErrorCode::MissingParameter, format!("{name} is required"))
}

fn does_not_exist(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

fn parse_attributes(p: &Params) -> Result<Option<AttributeSet>> {
    p.object("Attributes")?.map(AttributeSet::parse).transpose()
}

fn invalid_request(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

fn parse_request(line: &[u8]) -> Result<Map<String, Value>> {
    match serde_json::from_slice::<Value>(line) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(invalid_request("line is not a JSON object")),
        Err(err) => Err(invalid_request(format!("line is not valid JSON: {err}"))),
    }
}

fn members<const N: usize>(pairs: [(&str, Value); N]) -> Members {
    pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(service: &mut Service, line: &str) -> Value {
        serde_json::from_str(&service.handle_line(line.as_bytes())).unwrap()
    }

    fn code(response: &Value) -> &str {
        assert_eq!(response["ok"], false, "{response}");
        response["error"]["code"].as_str().unwrap()
    }

    #[test]
    fn rejects_malformed_lines_and_carries_on() {
        let mut s = Service::new();
        for line in [
            "",
            "   ",
            "[]",
            "42",
            "{",
            r#"{"Command": 1}"#,
            r#"{"Command": null}"#,
            "{}",
            r#"{"Command": "Nope"}"#,
            r#"{"Command":"Tick","Seconds":1} x"#,
        ] {
            assert_eq!(code(&run(&mut s, line)), "InvalidRequest", "{line:?}");
        }
        assert_eq!(code(&s_invalid_utf8()), "InvalidRequest");
        assert_eq!(run(&mut s, r#"{"Command":"Tick","Seconds":0}"#)["Now"], 0);
    }

    fn s_invalid_utf8() -> Value {
        let mut s = Service::new();
        serde_json::from_str(&s.handle_line(b"{\"Command\": \"\xff\"}")).unwrap()
    }

    #[test]
    fn tick_advances_the_clock() {
        let mut s = Service::new();
        assert_eq!(
            run(&mut s, r#"{"Command":"Tick","Seconds":5}"#),
            json!({"ok": true, "Now": 5})
        );
        assert_eq!(
            run(&mut s, r#"{"Command":"Tick","Seconds":7,"Extra":1}"#)["Now"],
            12
        );
    }

    #[test]
    fn tick_validates_seconds() {
        let mut s = Service::new();
        assert_eq!(
            code(&run(&mut s, r#"{"Command":"Tick"}"#)),
            "MissingParameter"
        );
        assert_eq!(
            code(&run(&mut s, r#"{"Command":"Tick","Seconds":null}"#)),
            "MissingParameter"
        );
        for bad in ["-1", "1209601", "1.0", "1e0", "\"1\"", "true"] {
            let line = format!(r#"{{"Command":"Tick","Seconds":{bad}}}"#);
            assert_eq!(code(&run(&mut s, &line)), "InvalidParameterValue", "{bad}");
        }
        assert_eq!(
            run(&mut s, r#"{"Command":"Tick","Seconds":1209600}"#)["Now"],
            1_209_600
        );
    }
}
