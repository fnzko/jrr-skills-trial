//! The service itself: the clock, the identifier counters and all queues.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::attributes::{AttributeUpdate, Attributes};
use crate::command::{self, Command, SendRequest};
use crate::error::{Error, ErrorCode, Result};
use crate::ids::{Ids, ReceiptHandle};
use crate::queue::{Delivery, Queue};

const MAX_QUEUES: usize = 1000;

#[derive(Debug, Default)]
pub struct Broker {
    now: u64,
    ids: Ids,
    queues: BTreeMap<String, Queue>,
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one input line and returns its response.
    pub fn handle_line(&mut self, line: &[u8]) -> Value {
        match command::parse(line).and_then(|command| self.execute(command)) {
            Ok(mut result) => {
                result["ok"] = json!(true);
                result
            }
            Err(error) => error.to_response(),
        }
    }

    /// Runs a command and returns its result members as a JSON object.
    fn execute(&mut self, command: Command) -> Result<Value> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => {
                let now = self.now;
                let queue = lookup(&mut self.queues, &queue, now)?;
                let counts = queue.counts(now);
                Ok(json!({
                    "Attributes": queue.attributes.to_json(),
                    "Counts": {
                        "Visible": counts.visible,
                        "InFlight": counts.in_flight,
                        "Delayed": counts.delayed,
                    },
                }))
            }
            Command::ListQueues { prefix } => {
                let prefix = prefix.unwrap_or_default();
                let names: Vec<_> = self
                    .queues
                    .keys()
                    .filter(|name| name.starts_with(&prefix))
                    .collect();
                Ok(json!({ "QueueNames": names }))
            }
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                lookup(&mut self.queues, &queue, self.now)?.purge();
                Ok(json!({}))
            }
            Command::Tick { seconds } => {
                self.now += u64::from(seconds);
                Ok(json!({ "Now": self.now }))
            }
            Command::SendMessage(request) => self.send_message(&request),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let queue = lookup(&mut self.queues, &queue, self.now)?;
                let handle = parse_handle(&receipt_handle, &self.ids)?;
                queue.delete_message(handle, self.now)?;
                Ok(json!({}))
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let queue = lookup(&mut self.queues, &queue, self.now)?;
                let handle = parse_handle(&receipt_handle, &self.ids)?;
                queue.change_visibility(handle, visibility_timeout, self.now)?;
                Ok(json!({}))
            }
        }
    }

    fn create_queue(&mut self, name: String, update: &AttributeUpdate) -> Result<Value> {
        let mut attributes = Attributes::default();
        let fifo = update.fifo.unwrap_or(false);
        if name.ends_with(".fifo") != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "the name must end in .fifo exactly when Fifo is true",
            ));
        }
        if update.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                "requires a FIFO queue",
            ));
        }
        let dead_letter = update.dead_letter()?;
        attributes.apply(update, dead_letter);

        if let Some(existing) = self.queues.get(&name) {
            return if existing.attributes == attributes {
                Ok(json!({}))
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
                .ok_or_else(|| Error::queue_missing(&dead_letter.queue))?;
            if target.attributes.fifo != fifo {
                return Err(dead_letter_type_mismatch());
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("{MAX_QUEUES} queues already exist"),
            ));
        }
        self.queues.insert(name, Queue::new(attributes));
        Ok(json!({}))
    }

    fn set_queue_attributes(&mut self, name: &str, update: &AttributeUpdate) -> Result<Value> {
        let now = self.now;
        let fifo = lookup(&mut self.queues, name, now)?.attributes.fifo;
        if update.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot be changed",
            ));
        }
        if update.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                "requires a FIFO queue",
            ));
        }
        let dead_letter = update.dead_letter()?;
        if let Some(dead_letter) = &dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| Error::queue_missing(&dead_letter.queue))?;
            if dead_letter.queue == name {
                return Err(Error::invalid_value(
                    "DeadLetterQueue",
                    "must not be the queue itself",
                ));
            }
            if target.attributes.fifo != fifo {
                return Err(dead_letter_type_mismatch());
            }
        }
        let queue = lookup(&mut self.queues, name, now)?;
        queue.attributes.apply(update, dead_letter);
        // A lower RetentionSeconds removes messages for good, so it must be
        // applied now, before a later change could raise it again.
        queue.expire(now);
        Ok(json!({}))
    }

    fn delete_queue(&mut self, name: &str) -> Result<Value> {
        lookup(&mut self.queues, name, self.now)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            if queue
                .attributes
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue == name)
            {
                queue.attributes.dead_letter = None;
            }
        }
        Ok(json!({}))
    }

    fn send_message(&mut self, request: &SendRequest) -> Result<Value> {
        let queue = lookup(&mut self.queues, &request.queue, self.now)?;
        let accepted = queue.send(self.now, request, &mut self.ids)?;
        let mut result = json!({ "MessageId": accepted.id.to_string() });
        if let Some(sequence) = accepted.sequence {
            result["SequenceNumber"] = json!(sequence);
        }
        Ok(result)
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u32,
        visibility_timeout: Option<u32>,
    ) -> Result<Value> {
        let now = self.now;
        let queue = lookup(&mut self.queues, name, now)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.attributes.visibility_timeout);
        let dead_letter_queue = queue
            .attributes
            .dead_letter
            .as_ref()
            .map(|d| d.queue.clone());
        let (deliveries, moves) = queue.receive(
            now,
            usize::try_from(max_messages).unwrap_or(usize::MAX),
            visibility_timeout,
            &mut self.ids,
        );
        if let Some(target) = dead_letter_queue.and_then(|name| self.queues.get_mut(&name)) {
            for moved in moves {
                target.accept_dead_letter(moved);
            }
        }
        let messages: Vec<_> = deliveries.iter().map(delivery_json).collect();
        Ok(json!({ "Messages": messages }))
    }
}

/// Finds a queue by exact name and applies retention to it.
fn lookup<'a>(
    queues: &'a mut BTreeMap<String, Queue>,
    name: &str,
    now: u64,
) -> Result<&'a mut Queue> {
    let queue = queues
        .get_mut(name)
        .ok_or_else(|| Error::queue_missing(name))?;
    queue.expire(now);
    Ok(queue)
}

fn parse_handle(text: &str, ids: &Ids) -> Result<ReceiptHandle> {
    ReceiptHandle::parse(text, ids).ok_or_else(|| {
        Error::new(
            ErrorCode::ReceiptHandleInvalid,
            format!("receipt handle {text:?} was never issued"),
        )
    })
}

fn dead_letter_type_mismatch() -> Error {
    Error::invalid_value(
        "DeadLetterQueue",
        "must be of the same type (FIFO or standard) as the queue",
    )
}

fn delivery_json(delivery: &Delivery) -> Value {
    let mut message = json!({
        "MessageId": delivery.id.to_string(),
        "ReceiptHandle": delivery.handle.to_string(),
        "Body": delivery.body,
        "ReceiveCount": delivery.receive_count,
        "SentAt": delivery.sent_at,
        "FirstReceivedAt": delivery.first_received_at,
    });
    if let Some(group_id) = &delivery.group_id {
        message["GroupId"] = json!(group_id);
    }
    if let Some(sequence) = delivery.sequence {
        message["SequenceNumber"] = json!(sequence);
    }
    message
}
