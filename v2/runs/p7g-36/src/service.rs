//! The miniq service: process-wide state and the command handlers.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::attributes::AttributeUpdate;
use crate::error::{Code, Error, Result};
use crate::params::{Params, is_name_byte};
use crate::queue::{MAX_MESSAGES, Message, Queue};

const MAX_QUEUES: usize = 1000;
const FIFO_SUFFIX: &str = ".fifo";

type Members = Map<String, Value>;

#[derive(Debug)]
pub struct Service {
    now: u64,
    queues: HashMap<String, Queue>,
    /// Number of the next `MessageId`.
    next_message: u64,
    /// Number of the next `ReceiptHandle`.
    next_handle: u64,
}

impl Default for Service {
    fn default() -> Self {
        Self::new()
    }
}

impl Service {
    pub fn new() -> Self {
        Service {
            now: 0,
            queues: HashMap::new(),
            next_message: 1,
            next_handle: 1,
        }
    }

    /// Runs one input line (without its newline) and returns the single-line
    /// JSON response.
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        let response = match self.execute(line) {
            Ok(mut members) => {
                members.insert("ok".into(), Value::Bool(true));
                Value::Object(members)
            }
            Err(e) => json!({
                "ok": false,
                "error": {"code": e.code.as_str(), "message": e.message},
            }),
        };
        response.to_string()
    }

    fn execute(&mut self, line: &[u8]) -> Result<Members> {
        let text = std::str::from_utf8(line)
            .map_err(|_| Error::new(Code::InvalidRequest, "line is not valid UTF-8"))?;
        let value: Value = serde_json::from_str(text)
            .map_err(|e| Error::new(Code::InvalidRequest, format!("invalid JSON: {e}")))?;
        let Value::Object(members) = value else {
            return Err(Error::new(
                Code::InvalidRequest,
                "line is not a JSON object",
            ));
        };
        let command = members
            .get("Command")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::new(Code::InvalidRequest, "Command must be a string"))?;
        let p = Params::new(&members);
        match command {
            "CreateQueue" => self.create_queue(&p),
            "SetQueueAttributes" => self.set_queue_attributes(&p),
            "GetQueueAttributes" => self.get_queue_attributes(&p),
            "ListQueues" => self.list_queues(&p),
            "DeleteQueue" => self.delete_queue(&p),
            "PurgeQueue" => self.purge_queue(&p),
            "Tick" => self.tick(&p),
            "SendMessage" => self.send_message(&p),
            "ReceiveMessage" => self.receive_message(&p),
            "DeleteMessage" => self.delete_message(&p),
            "ChangeMessageVisibility" => self.change_message_visibility(&p),
            other => Err(Error::new(
                Code::InvalidRequest,
                format!("unknown command {other}"),
            )),
        }
    }

    /// Looks up a queue by exact name and applies pending retention.
    fn queue(&mut self, name: &str) -> Result<&mut Queue> {
        lookup(&mut self.queues, name, self.now)
    }

    // ---- queue commands -------------------------------------------------

    fn create_queue(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        check_queue_name(name)?;
        let update = match p.object("Attributes")? {
            Some(attributes) => AttributeUpdate::parse(&attributes)?,
            None => AttributeUpdate::default(),
        };
        let wanted = update.with_defaults();

        // 1-3: consistency of the request on its own.
        if name.ends_with(FIFO_SUFFIX) != wanted.fifo {
            return Err(Error::new(
                Code::QueueNameInvalid,
                "a queue is FIFO exactly when its name ends in .fifo",
            ));
        }
        if wanted.content_based_deduplication && !wanted.fifo {
            return Err(invalid("ContentBasedDeduplication requires a FIFO queue"));
        }
        update.check_dead_letter_pair()?;

        // 4: creating an existing queue again is fine if nothing differs.
        if let Some(existing) = self.queues.get(name) {
            return if existing.attributes == wanted {
                Ok(Members::new())
            } else {
                Err(Error::new(
                    Code::QueueAlreadyExists,
                    format!("queue {name} exists with different attributes"),
                ))
            };
        }
        if let Some(dead_letter) = &wanted.dead_letter {
            let dlq = self.queues.get(&dead_letter.queue).ok_or_else(|| {
                Error::new(
                    Code::QueueDoesNotExist,
                    format!("dead-letter queue {} does not exist", dead_letter.queue),
                )
            })?;
            if dlq.attributes.fifo != wanted.fifo {
                return Err(invalid("dead-letter queue must be of the same type"));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(Code::LimitExceeded, "too many queues"));
        }
        self.queues.insert(name.to_owned(), Queue::new(wanted));
        Ok(Members::new())
    }

    fn set_queue_attributes(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName", "Attributes"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        let attributes = p.object("Attributes")?.expect("required, so present");
        let update = AttributeUpdate::parse(&attributes)?;
        let now = self.now;
        let fifo = lookup(&mut self.queues, name, now)?.attributes.fifo;

        if update.fifo.is_some() {
            return Err(Error::new(
                Code::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if update.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid("ContentBasedDeduplication requires a FIFO queue"));
        }
        update.check_dead_letter_pair()?;
        if let Some(dlq_name) = &update.dead_letter_queue {
            let dlq = self.queues.get(dlq_name).ok_or_else(|| {
                Error::new(
                    Code::QueueDoesNotExist,
                    format!("dead-letter queue {dlq_name} does not exist"),
                )
            })?;
            if dlq_name == name || dlq.attributes.fifo != fifo {
                return Err(invalid(
                    "dead-letter queue must be another queue of the same type",
                ));
            }
        }
        let queue = self.queue(name)?;
        update.apply_to(&mut queue.attributes);
        queue.advance(now);
        Ok(Members::new())
    }

    fn get_queue_attributes(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        let now = self.now;
        let queue = self.queue(name)?;
        let a = &queue.attributes;
        let mut attributes = json!({
            "VisibilityTimeout": a.visibility_timeout,
            "DelaySeconds": a.delay_seconds,
            "RetentionSeconds": a.retention_seconds,
            "Fifo": a.fifo,
            "ContentBasedDeduplication": a.content_based_deduplication,
        });
        if let Some(d) = &a.dead_letter {
            attributes["DeadLetterQueue"] = json!(d.queue);
            attributes["MaxReceiveCount"] = json!(d.max_receive_count);
        }
        let counts = queue.counts(now);
        Ok(members(json!({
            "Attributes": attributes,
            "Counts": {
                "Visible": counts.visible,
                "InFlight": counts.in_flight,
                "Delayed": counts.delayed,
            },
        })))
    }

    fn list_queues(&mut self, p: &Params) -> Result<Members> {
        let prefix = p.string("Prefix")?.unwrap_or_default();
        let mut names: Vec<&String> = self
            .queues
            .keys()
            .filter(|name| name.starts_with(prefix))
            .collect();
        names.sort_unstable();
        Ok(members(json!({ "QueueNames": names })))
    }

    fn delete_queue(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        self.queue(name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            if queue
                .attributes
                .dead_letter
                .as_ref()
                .is_some_and(|d| d.queue == name)
            {
                queue.attributes.dead_letter = None;
            }
        }
        Ok(Members::new())
    }

    fn purge_queue(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        self.queue(name)?.purge();
        Ok(Members::new())
    }

    fn tick(&mut self, p: &Params) -> Result<Members> {
        p.require(&["Seconds"])?;
        let seconds = p.integer("Seconds", 0..=1_209_600)?.unwrap_or_default();
        self.now += seconds;
        Ok(members(json!({ "Now": self.now })))
    }

    // ---- message commands -----------------------------------------------

    fn send_message(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName", "Body"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        let body = p.string("Body")?.unwrap_or_default();
        if !(1..=1024).contains(&body.len()) {
            return Err(invalid("Body must be 1 to 1024 bytes"));
        }
        let delay = p.integer("DelaySeconds", 0..=900)?;
        let group_id = p.identifier("GroupId", 1..=128)?;
        let dedup_id = p.identifier("DeduplicationId", 1..=128)?;

        let now = self.now;
        let queue = lookup(&mut self.queues, name, now)?;
        let fifo = queue.attributes.fifo;

        if (!fifo && (group_id.is_some() || dedup_id.is_some())) || (fifo && delay.is_some()) {
            return Err(Error::new(
                Code::UnsupportedOperation,
                if fifo {
                    "FIFO queues do not support per-message delay"
                } else {
                    "standard queues do not support GroupId or DeduplicationId"
                },
            ));
        }
        let mut dedup_key: Option<&str> = None;
        if fifo {
            group_id.ok_or_else(|| missing("GroupId"))?;
            let key = match dedup_id {
                Some(id) => id,
                None if queue.attributes.content_based_deduplication => body,
                None => return Err(missing("DeduplicationId")),
            };
            if let Some((message_id, sequence)) = queue.duplicate_of(key, now) {
                return Ok(send_result(message_id, sequence));
            }
            dedup_key = Some(key);
        }
        if queue.len() >= MAX_MESSAGES {
            return Err(Error::new(
                Code::LimitExceeded,
                "queue holds too many messages",
            ));
        }

        let id = self.next_message;
        self.next_message += 1;
        let delay = delay.unwrap_or(queue.attributes.delay_seconds);
        let message = Message::new(id, body.to_owned(), now, delay, group_id.map(str::to_owned));
        let sequence = queue.add(message, now);
        if let Some(key) = dedup_key {
            queue.record_send(key.to_owned(), now, id, sequence);
        }
        Ok(send_result(id, sequence))
    }

    fn receive_message(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        let max = p.integer("MaxMessages", 1..=10)?.unwrap_or(1);
        let visibility = p.integer("VisibilityTimeout", 0..=43_200)?;

        let now = self.now;
        let queue = lookup(&mut self.queues, name, now)?;
        let visibility = visibility.unwrap_or(queue.attributes.visibility_timeout);
        let dead_letter_queue = queue
            .attributes
            .dead_letter
            .as_ref()
            .map(|d| d.queue.clone());
        let fifo = queue.attributes.fifo;
        let received = queue.receive(now, max as usize, visibility, &mut self.next_handle);

        if let Some(dlq) = dead_letter_queue.and_then(|n| self.queues.get_mut(&n)) {
            dlq.advance(now);
            for message in received.dead {
                dlq.add(message, now);
            }
        }

        let messages: Vec<Value> = received
            .delivered
            .iter()
            .map(|m| {
                let mut v = json!({
                    "MessageId": format!("m-{}", m.id),
                    "ReceiptHandle": format!("r-{}", m.handle().unwrap_or_default()),
                    "Body": m.body,
                    "ReceiveCount": m.receive_count,
                    "SentAt": m.sent_at,
                    "FirstReceivedAt": m.first_received_at,
                });
                if fifo {
                    v["GroupId"] = json!(m.group_id);
                    v["SequenceNumber"] = json!(m.sequence_number);
                }
                v
            })
            .collect();
        Ok(members(json!({ "Messages": messages })))
    }

    fn delete_message(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName", "ReceiptHandle"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        let handle = p.string("ReceiptHandle")?.unwrap_or_default();
        let now = self.now;
        let handle = self.issued_handle(handle)?;
        if !lookup(&mut self.queues, name, now)?.delete_by_handle(handle, now) {
            return Err(expired());
        }
        Ok(Members::new())
    }

    fn change_message_visibility(&mut self, p: &Params) -> Result<Members> {
        p.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
        let name = p.string("QueueName")?.unwrap_or_default();
        let handle = p.string("ReceiptHandle")?.unwrap_or_default();
        let timeout = p
            .integer("VisibilityTimeout", 0..=43_200)?
            .unwrap_or_default();
        let now = self.now;
        let handle = self.issued_handle(handle)?;
        let queue = lookup(&mut self.queues, name, now)?;
        if !queue.set_invisible_until(handle, now, now + timeout) {
            return Err(expired());
        }
        Ok(Members::new())
    }

    /// Parses `r-<n>` and checks that exactly this string was handed out.
    fn issued_handle(&self, text: &str) -> Result<u64> {
        text.strip_prefix("r-")
            .filter(|digits| !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|digits| digits.parse::<u64>().ok())
            .filter(|&n| n < self.next_handle)
            .ok_or_else(|| {
                Error::new(
                    Code::ReceiptHandleInvalid,
                    "receipt handle was never issued",
                )
            })
    }
}

fn lookup<'a>(
    queues: &'a mut HashMap<String, Queue>,
    name: &str,
    now: u64,
) -> Result<&'a mut Queue> {
    let queue = queues.get_mut(name).ok_or_else(|| {
        Error::new(
            Code::QueueDoesNotExist,
            format!("queue {name} does not exist"),
        )
    })?;
    queue.advance(now);
    Ok(queue)
}

/// A queue name is 1-80 characters of `A-Z a-z 0-9 _ -`, or 1-75 of them
/// followed by `.fifo`.
fn check_queue_name(name: &str) -> Result<()> {
    let stem = name.strip_suffix(FIFO_SUFFIX).unwrap_or(name);
    let stem_max = if stem.len() < name.len() { 75 } else { 80 };
    if (1..=stem_max).contains(&stem.len()) && stem.bytes().all(is_name_byte) {
        Ok(())
    } else {
        Err(Error::new(
            Code::QueueNameInvalid,
            "queue name must be 1-80 characters of A-Z a-z 0-9 _ - (plus an optional .fifo suffix)",
        ))
    }
}

fn send_result(message_id: u64, sequence: Option<u64>) -> Members {
    let mut result = members(json!({ "MessageId": format!("m-{message_id}") }));
    if let Some(n) = sequence {
        result.insert("SequenceNumber".into(), json!(n));
    }
    result
}

fn members(value: Value) -> Members {
    match value {
        Value::Object(m) => m,
        _ => unreachable!("members() is only called with object literals"),
    }
}

fn invalid(message: &str) -> Error {
    Error::new(Code::InvalidParameterValue, message)
}

fn missing(name: &str) -> Error {
    Error::new(
        Code::MissingParameter,
        format!("missing required parameter {name}"),
    )
}

fn expired() -> Error {
    Error::new(
        Code::ReceiptHandleExpired,
        "receipt handle is not the latest handle of an in-flight message",
    )
}
