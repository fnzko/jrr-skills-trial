//! The service state and the command handlers: queue lookup (check 4 of SPEC section 2), the
//! numbered rules that span queues, the clock and the process-wide identifiers.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::error::{Error, ErrorCode, Result};
use crate::queue::{Counter, Queue, QueueAttributes, format_message_id};
use crate::request::{self, AttributeChanges, Command, FIFO_SUFFIX, SendMessage};

/// The most queues that may exist at once.
pub const MAX_QUEUES: usize = 1000;

/// The whole state of a miniq process.
#[derive(Debug, Default)]
pub struct Service {
    now: u64,
    /// Queues by name; a `BTreeMap` so that listing comes out in byte order.
    queues: BTreeMap<String, Queue>,
    message_ids: Counter,
    receipt_handles: Counter,
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }

    /// Executes one command line (without its trailing `\n`) and returns the response line (also
    /// without `\n`).
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        let response = match request::parse(line).and_then(|command| self.execute(command)) {
            Ok(mut members) => {
                members.insert("ok".to_owned(), Value::Bool(true));
                Value::Object(members)
            }
            Err(error) => json!({ "ok": false, "error": error }),
        };
        response.to_string()
    }

    /// Runs a parsed command and returns the result members of its success response.
    fn execute(&mut self, command: Command) -> Result<Map<String, Value>> {
        match command {
            Command::CreateQueue {
                queue_name,
                attributes,
            } => self.create_queue(queue_name, &attributes).map(no_members),
            Command::SetQueueAttributes {
                queue_name,
                attributes,
            } => self
                .set_queue_attributes(&queue_name, &attributes)
                .map(no_members),
            Command::GetQueueAttributes { queue_name } => {
                let queue = self.queue(&queue_name)?;
                Ok(members(json!({
                    "Attributes": queue.attributes(),
                    "Counts": queue.counts(),
                })))
            }
            Command::ListQueues { prefix } => {
                let prefix = prefix.as_deref().unwrap_or("");
                let names: Vec<&String> = self
                    .queues
                    .range(prefix.to_owned()..)
                    .map(|(name, _)| name)
                    .take_while(|name| name.starts_with(prefix))
                    .collect();
                Ok(members(json!({ "QueueNames": names })))
            }
            Command::DeleteQueue { queue_name } => self.delete_queue(&queue_name).map(no_members),
            Command::PurgeQueue { queue_name } => {
                self.queue_mut(&queue_name)?.purge();
                Ok(Map::new())
            }
            Command::Tick { seconds } => {
                self.now += seconds;
                for queue in self.queues.values_mut() {
                    queue.advance(self.now);
                }
                Ok(members(json!({ "Now": self.now })))
            }
            Command::SendMessage(send) => self.send_message(send),
            Command::ReceiveMessage {
                queue_name,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue_name, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue_name,
                receipt_handle,
            } => {
                let (issued, now) = (self.receipt_handles.issued(), self.now);
                let queue = self.queue_mut(&queue_name)?;
                queue.delete_message(parse_receipt_handle(&receipt_handle, issued)?, now)?;
                Ok(Map::new())
            }
            Command::ChangeMessageVisibility {
                queue_name,
                receipt_handle,
                visibility_timeout,
            } => {
                let (issued, now) = (self.receipt_handles.issued(), self.now);
                let queue = self.queue_mut(&queue_name)?;
                let receipt = parse_receipt_handle(&receipt_handle, issued)?;
                queue.change_visibility(receipt, visibility_timeout, now)?;
                Ok(Map::new())
            }
        }
    }

    fn queue(&self, name: &str) -> Result<&Queue> {
        self.queues
            .get(name)
            .ok_or_else(|| Error::queue_does_not_exist(name))
    }

    fn queue_mut(&mut self, name: &str) -> Result<&mut Queue> {
        self.queues
            .get_mut(name)
            .ok_or_else(|| Error::queue_does_not_exist(name))
    }

    fn create_queue(&mut self, name: String, changes: &AttributeChanges) -> Result<()> {
        let mut attributes = QueueAttributes::default();
        attributes.apply(changes);

        if name.ends_with(FIFO_SUFFIX) != attributes.fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is FIFO",
            ));
        }
        if attributes.content_based_deduplication && !attributes.fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        check_dead_letter_pair(changes)?;
        if let Some(existing) = self.queues.get(&name) {
            return if existing.attributes() == &attributes {
                Ok(())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {name:?} already exists with different attributes"),
                ))
            };
        }
        if let Some(config) = &attributes.dead_letter {
            let target = self.queue(&config.queue_name)?;
            if target.is_fifo() != attributes.fifo {
                return Err(mismatched_dead_letter_type());
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues may exist"),
            ));
        }
        self.queues.insert(name, Queue::new(attributes));
        Ok(())
    }

    fn set_queue_attributes(&mut self, name: &str, changes: &AttributeChanges) -> Result<()> {
        let queue = self.queue(name)?;
        if changes.fifo.is_some() {
            return Err(Error::unsupported("the type of a queue never changes"));
        }
        if changes.content_based_deduplication == Some(true) && !queue.is_fifo() {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        check_dead_letter_pair(changes)?;
        if let Some(target_name) = &changes.dead_letter_queue {
            let target = self.queue(target_name)?;
            if target_name == name {
                return Err(Error::invalid_value(
                    "a queue cannot be its own dead-letter queue",
                ));
            }
            if target.is_fifo() != queue.is_fifo() {
                return Err(mismatched_dead_letter_type());
            }
        }
        let now = self.now;
        self.queue_mut(name)?.update_attributes(changes, now);
        Ok(())
    }

    fn delete_queue(&mut self, name: &str) -> Result<()> {
        self.queues
            .remove(name)
            .ok_or_else(|| Error::queue_does_not_exist(name))?;
        for queue in self.queues.values_mut() {
            let points_here = queue
                .attributes()
                .dead_letter
                .as_ref()
                .is_some_and(|config| config.queue_name == name);
            if points_here {
                queue.clear_dead_letter_config();
            }
        }
        Ok(())
    }

    fn send_message(&mut self, send: SendMessage) -> Result<Map<String, Value>> {
        let now = self.now;
        let queue = self
            .queues
            .get_mut(&send.queue_name)
            .ok_or_else(|| Error::queue_does_not_exist(&send.queue_name))?;
        let sent = queue.send(send, now, &mut self.message_ids)?;

        #[derive(Serialize)]
        #[serde(rename_all = "PascalCase")]
        struct SendResult {
            message_id: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            sequence_number: Option<u64>,
        }
        Ok(members(SendResult {
            message_id: format_message_id(sent.message_id),
            sequence_number: sent.sequence_number,
        }))
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u64,
        visibility_timeout: Option<u64>,
    ) -> Result<Map<String, Value>> {
        let now = self.now;
        let queue = self
            .queues
            .get_mut(name)
            .ok_or_else(|| Error::queue_does_not_exist(name))?;
        let max_messages = usize::try_from(max_messages).expect("MaxMessages is at most 10");
        let received = queue.receive(
            max_messages,
            visibility_timeout,
            now,
            &mut self.receipt_handles,
        );

        if !received.dead_letters.is_empty() {
            // Only a queue with a dead-letter configuration hands out dead letters, and deleting a
            // queue clears every configuration that names it, so the target always exists.
            let target_name = queue
                .attributes()
                .dead_letter
                .as_ref()
                .expect("dead letters imply a dead-letter configuration")
                .queue_name
                .clone();
            let target = self
                .queues
                .get_mut(&target_name)
                .expect("a dead-letter queue exists while it is configured");
            for message in received.dead_letters {
                target.accept_dead_letter(message, now);
            }
        }
        Ok(members(json!({ "Messages": received.deliveries })))
    }
}

/// Checks rule 1 of `DeleteMessage` and `ChangeMessageVisibility`: the handle must be one of the
/// `issued` handles, written exactly as it was issued (`r-01`, `R-1` and `r-1 ` never are).
fn parse_receipt_handle(handle: &str, issued: u64) -> Result<u64> {
    handle
        .strip_prefix("r-")
        .filter(|digits| !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|digits| digits.parse::<u64>().ok())
        .filter(|&n| n <= issued)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("receipt handle {handle:?} was never issued"),
            )
        })
}

fn check_dead_letter_pair(changes: &AttributeChanges) -> Result<()> {
    if changes.has_partial_dead_letter_config() {
        return Err(Error::invalid_value(
            "DeadLetterQueue and MaxReceiveCount must be given together",
        ));
    }
    Ok(())
}

fn mismatched_dead_letter_type() -> Error {
    Error::invalid_value("a dead-letter queue must be of the same type (FIFO or standard)")
}

fn no_members(_: ()) -> Map<String, Value> {
    Map::new()
}

/// Turns a serializable result into the members of a success response.
fn members(result: impl Serialize) -> Map<String, Value> {
    match serde_json::to_value(result) {
        Ok(Value::Object(members)) => members,
        other => unreachable!("results serialize to JSON objects, got {other:?}"),
    }
}
