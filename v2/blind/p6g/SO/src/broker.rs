//! The whole service state and the execution of validated commands.
//!
//! This implements steps 4-5 of the error order: queue lookup, then the
//! numbered rules of each command.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::command::{Attributes, Command, SendMessage, is_fifo_name};
use crate::error::{Error, ErrorCode, Result};
use crate::queue::{Config, Delivery, Queue, Send, SendOutcome};

pub const MAX_QUEUES: usize = 1000;

/// The members of a success response, besides `"ok": true`.
pub type Members = Map<String, Value>;

#[derive(Debug)]
pub struct Broker {
    now: u64,
    queues: HashMap<String, Queue>,
    /// Number of the next `MessageId`.
    next_message_id: u64,
    /// Number of the next `ReceiptHandle`.
    next_handle: u64,
}

impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}

impl Broker {
    pub fn new() -> Self {
        Self {
            now: 0,
            queues: HashMap::new(),
            next_message_id: 1,
            next_handle: 1,
        }
    }

    pub fn execute(&mut self, command: Command) -> Result<Members> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref())),
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                lookup(&mut self.queues, self.now, &queue)?.purge();
                Ok(Members::new())
            }
            Command::Tick { seconds } => {
                self.now = self.now.saturating_add(u64::from(seconds));
                Ok(members([("Now", json!(self.now))]))
            }
            Command::SendMessage(send) => self.send_message(&send),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let (queue, key) = self.find_in_flight(&queue, &receipt_handle)?;
                queue.delete(key);
                Ok(Members::new())
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let now = self.now;
                let (queue, key) = self.find_in_flight(&queue, &receipt_handle)?;
                queue.set_visibility(key, now, visibility_timeout);
                Ok(Members::new())
            }
        }
    }

    fn create_queue(&mut self, name: String, attributes: &Attributes) -> Result<Members> {
        let fifo = attributes.fifo.unwrap_or(false);
        if is_fifo_name(&name) != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in `.fifo` exactly when the queue is a FIFO queue",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(content_dedup_on_standard());
        }
        check_dead_letter_pair(attributes)?;

        let config = Config::from_attributes(attributes);
        if let Some(existing) = self.queues.get(&name) {
            return if existing.config == config {
                Ok(Members::new())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue `{name}` exists with different attributes"),
                ))
            };
        }
        if let Some(dead_letter) = &config.dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| queue_missing(&dead_letter.queue))?;
            if target.config.fifo != fifo {
                return Err(dead_letter_type_mismatch());
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        self.queues.insert(name, Queue::new(config));
        Ok(Members::new())
    }

    fn set_queue_attributes(&mut self, name: &str, attributes: &Attributes) -> Result<Members> {
        let now = self.now;
        let fifo = lookup(&mut self.queues, now, name)?.config.fifo;
        if attributes.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(content_dedup_on_standard());
        }
        check_dead_letter_pair(attributes)?;
        if let Some(target_name) = &attributes.dead_letter_queue {
            let target = self
                .queues
                .get(target_name)
                .ok_or_else(|| queue_missing(target_name))?;
            if target_name == name || target.config.fifo != fifo {
                return Err(Error::new(
                    ErrorCode::InvalidParameterValue,
                    "`DeadLetterQueue` must be another queue of the same type",
                ));
            }
        }

        let queue = lookup(&mut self.queues, now, name)?;
        queue.config.apply(attributes);
        if attributes.retention_seconds.is_some() {
            queue.invalidate_sweep();
            queue.expire(now);
        }
        Ok(Members::new())
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Members> {
        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let config = &queue.config;
        let mut attributes = Members::new();
        attributes.insert("VisibilityTimeout".into(), json!(config.visibility_timeout));
        attributes.insert("DelaySeconds".into(), json!(config.delay_seconds));
        attributes.insert("RetentionSeconds".into(), json!(config.retention_seconds));
        attributes.insert("Fifo".into(), json!(config.fifo));
        attributes.insert(
            "ContentBasedDeduplication".into(),
            json!(config.content_based_deduplication),
        );
        if let Some(dead_letter) = &config.dead_letter {
            attributes.insert("DeadLetterQueue".into(), json!(dead_letter.queue));
            attributes.insert(
                "MaxReceiveCount".into(),
                json!(dead_letter.max_receive_count),
            );
        }
        let counts = queue.counts(now);
        Ok(members([
            ("Attributes", Value::Object(attributes)),
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

    fn list_queues(&self, prefix: Option<&str>) -> Members {
        let prefix = prefix.unwrap_or("");
        let mut names: Vec<&str> = self
            .queues
            .keys()
            .map(String::as_str)
            .filter(|name| name.starts_with(prefix))
            .collect();
        // `str` ordering is bytewise, as the spec requires.
        names.sort_unstable();
        members([("QueueNames", json!(names))])
    }

    fn delete_queue(&mut self, name: &str) -> Result<Members> {
        lookup(&mut self.queues, self.now, name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            if queue
                .config
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue == name)
            {
                queue.config.dead_letter = None;
            }
        }
        Ok(Members::new())
    }

    fn send_message(&mut self, send: &SendMessage) -> Result<Members> {
        let queue = lookup(&mut self.queues, self.now, &send.queue)?;
        let outcome = queue.send(
            self.now,
            self.next_message_id,
            Send {
                body: &send.body,
                delay_seconds: send.delay_seconds,
                group_id: send.group_id.as_deref(),
                deduplication_id: send.deduplication_id.as_deref(),
            },
        )?;
        let (message_id, sequence_number) = match outcome {
            SendOutcome::Accepted { sequence_number } => {
                let id = self.next_message_id;
                self.next_message_id += 1;
                (id, sequence_number)
            }
            SendOutcome::Duplicate {
                message_id,
                sequence_number,
            } => (message_id, Some(sequence_number)),
        };
        let mut result = members([("MessageId", json!(format!("m-{message_id}")))]);
        if let Some(n) = sequence_number {
            result.insert("SequenceNumber".into(), json!(n));
        }
        Ok(result)
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: usize,
        visibility_timeout: Option<u32>,
    ) -> Result<Members> {
        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let dead_letter_queue = queue.config.dead_letter.as_ref().map(|d| d.queue.clone());
        let received = queue.receive(now, max_messages, visibility_timeout, &mut self.next_handle);

        if let Some(target) = dead_letter_queue
            && let Ok(target) = lookup(&mut self.queues, now, &target)
        {
            for message in received.dead_letters {
                target.accept_dead_letter(now, message);
            }
        }
        let messages: Vec<Value> = received.deliveries.iter().map(delivery_json).collect();
        Ok(members([("Messages", Value::Array(messages))]))
    }

    /// Resolves a receipt handle to the in-flight message it grants access to.
    fn find_in_flight(&mut self, queue: &str, handle: &str) -> Result<(&mut Queue, u64)> {
        let now = self.now;
        let queue = lookup(&mut self.queues, now, queue)?;
        let handle = parse_handle(handle, self.next_handle).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                "the receipt handle was never issued",
            )
        })?;
        let key = queue.find_in_flight(now, handle)?;
        Ok((queue, key))
    }
}

/// Finds a queue and applies retention to it, or fails with `QueueDoesNotExist`.
fn lookup<'a>(
    queues: &'a mut HashMap<String, Queue>,
    now: u64,
    name: &str,
) -> Result<&'a mut Queue> {
    let queue = queues.get_mut(name).ok_or_else(|| queue_missing(name))?;
    queue.expire(now);
    Ok(queue)
}

/// Parses `r-<n>` in its canonical spelling, and only if it was issued.
fn parse_handle(handle: &str, next_handle: u64) -> Option<u64> {
    let digits = handle.strip_prefix("r-")?;
    if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|&n| n < next_handle)
}

fn delivery_json(delivery: &Delivery) -> Value {
    let mut message = Members::new();
    message.insert(
        "MessageId".into(),
        json!(format!("m-{}", delivery.message_id)),
    );
    message.insert(
        "ReceiptHandle".into(),
        json!(format!("r-{}", delivery.receipt_handle)),
    );
    message.insert("Body".into(), json!(delivery.body));
    message.insert("ReceiveCount".into(), json!(delivery.receive_count));
    message.insert("SentAt".into(), json!(delivery.sent_at));
    message.insert("FirstReceivedAt".into(), json!(delivery.first_received_at));
    if let Some(group) = &delivery.group_id {
        message.insert("GroupId".into(), json!(group));
    }
    if let Some(n) = delivery.sequence_number {
        message.insert("SequenceNumber".into(), json!(n));
    }
    Value::Object(message)
}

fn members<const N: usize>(entries: [(&str, Value); N]) -> Members {
    entries
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect()
}

fn check_dead_letter_pair(attributes: &Attributes) -> Result<()> {
    if attributes.dead_letter_queue.is_some() != attributes.max_receive_count.is_some() {
        return Err(Error::new(
            ErrorCode::InvalidParameterValue,
            "`DeadLetterQueue` and `MaxReceiveCount` must be given together",
        ));
    }
    Ok(())
}

fn content_dedup_on_standard() -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        "`ContentBasedDeduplication` requires a FIFO queue",
    )
}

fn dead_letter_type_mismatch() -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        "a dead-letter queue must have the same type as its source queue",
    )
}

fn queue_missing(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue `{name}` does not exist"),
    )
}
