//! The whole service state and the execution of every command against it.

use std::collections::BTreeMap;
use std::ops::Bound;

use serde_json::{Value, json};

use crate::attributes::{AttributePatch, Config};
use crate::error::{Error, ErrorCode, Result};
use crate::queue::{Delivered, HandleLog, Queue};
use crate::request::{Command, NewMessage};

const MAX_QUEUES: usize = 1000;

#[derive(Debug, Default)]
pub struct Broker {
    now: u64,
    /// Ordered by name, which is the order `ListQueues` reports.
    queues: BTreeMap<String, Queue>,
    next_message_id: u64,
    next_queue_uid: u64,
    handles: HandleLog,
}

impl Broker {
    pub fn new() -> Self {
        Self {
            next_message_id: 1,
            ..Self::default()
        }
    }

    pub fn execute(&mut self, command: Command) -> Result<Value> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                find(&mut self.queues, &queue, self.now)?.purge();
                Ok(json!({}))
            }
            Command::Tick { seconds } => {
                self.now += u64::from(seconds);
                Ok(json!({ "Now": self.now }))
            }
            Command::SendMessage { queue, message } => self.send_message(&queue, message),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                find(&mut self.queues, &queue, self.now)?.delete_message(
                    self.now,
                    &self.handles,
                    &receipt_handle,
                )?;
                Ok(json!({}))
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                find(&mut self.queues, &queue, self.now)?.change_visibility(
                    self.now,
                    &self.handles,
                    &receipt_handle,
                    visibility_timeout,
                )?;
                Ok(json!({}))
            }
        }
    }

    fn create_queue(&mut self, name: String, patch: &AttributePatch) -> Result<Value> {
        let fifo = patch.fifo.unwrap_or(false);
        if name.ends_with(".fifo") != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is FIFO",
            ));
        }
        if patch.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = patch.dead_letter()?;
        let mut config = Config {
            fifo,
            ..Config::default()
        };
        config.apply(patch, dead_letter);

        if let Some(existing) = self.queues.get(&name) {
            return if existing.config == config {
                Ok(json!({}))
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {name:?} exists with different attributes"),
                ))
            };
        }
        if let Some(dead_letter) = &config.dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| does_not_exist(&dead_letter.queue))?;
            if target.config.fifo != fifo {
                return Err(invalid_value(
                    "the dead-letter queue must be of the same type",
                ));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(ErrorCode::LimitExceeded, "1000 queues exist"));
        }
        let uid = self.next_queue_uid;
        self.next_queue_uid += 1;
        self.queues.insert(name, Queue::new(uid, config));
        Ok(json!({}))
    }

    fn set_queue_attributes(&mut self, name: &str, patch: &AttributePatch) -> Result<Value> {
        // Looking the queue up also applies retention under the old value, so lowering
        // and later raising it cannot bring messages back.
        let fifo = find(&mut self.queues, name, self.now)?.config.fifo;
        if patch.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if patch.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = patch.dead_letter()?;
        if let Some(dead_letter) = &dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| does_not_exist(&dead_letter.queue))?;
            if dead_letter.queue == name || target.config.fifo != fifo {
                return Err(invalid_value(
                    "the dead-letter queue must be another queue of the same type",
                ));
            }
        }
        find(&mut self.queues, name, self.now)?
            .config
            .apply(patch, dead_letter);
        Ok(json!({}))
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Value> {
        let now = self.now;
        let queue = find(&mut self.queues, name, now)?;
        let config = &queue.config;
        let mut attributes = json!({
            "VisibilityTimeout": config.visibility_timeout,
            "DelaySeconds": config.delay_seconds,
            "RetentionSeconds": config.retention_seconds,
            "Fifo": config.fifo,
            "ContentBasedDeduplication": config.content_based_deduplication,
        });
        if let Some(dead_letter) = &config.dead_letter {
            attributes["DeadLetterQueue"] = json!(dead_letter.queue);
            attributes["MaxReceiveCount"] = json!(dead_letter.max_receive_count);
        }
        let counts = queue.counts(now);
        Ok(json!({
            "Attributes": attributes,
            "Counts": {
                "Visible": counts.visible,
                "InFlight": counts.in_flight,
                "Delayed": counts.delayed,
            },
        }))
    }

    fn list_queues(&self, prefix: &str) -> Value {
        let names: Vec<&str> = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name.as_str())
            .take_while(|name| name.starts_with(prefix))
            .collect();
        json!({ "QueueNames": names })
    }

    fn delete_queue(&mut self, name: &str) -> Result<Value> {
        if self.queues.remove(name).is_none() {
            return Err(does_not_exist(name));
        }
        for queue in self.queues.values_mut() {
            if queue.dead_letter_queue() == Some(name) {
                queue.config.dead_letter = None;
            }
        }
        Ok(json!({}))
    }

    fn send_message(&mut self, name: &str, message: NewMessage) -> Result<Value> {
        let sent = find(&mut self.queues, name, self.now)?.send(
            self.now,
            message,
            &mut self.next_message_id,
        )?;
        let mut result = json!({ "MessageId": message_id(sent.id) });
        if let Some(sequence) = sent.sequence {
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
        let queue = find(&mut self.queues, name, now)?;
        let (delivered, dead_lettered) =
            queue.receive(now, max_messages, visibility_timeout, &mut self.handles);
        if !dead_lettered.is_empty() {
            let target = queue
                .dead_letter_queue()
                .expect("messages are only dead-lettered by a queue with a configuration")
                .to_owned();
            let target = self
                .queues
                .get_mut(&target)
                .expect("a dead-letter queue exists while a queue refers to it");
            for message in dead_lettered {
                target.add_dead_letter(now, message);
            }
        }
        let messages: Vec<Value> = delivered.into_iter().map(delivery_json).collect();
        Ok(json!({ "Messages": messages }))
    }
}

/// The queue called `name`, with its expired messages already removed.
fn find<'a>(
    queues: &'a mut BTreeMap<String, Queue>,
    name: &str,
    now: u64,
) -> Result<&'a mut Queue> {
    let queue = queues.get_mut(name).ok_or_else(|| does_not_exist(name))?;
    queue.expire(now);
    Ok(queue)
}

fn delivery_json(d: Delivered) -> Value {
    let mut message = json!({
        "MessageId": message_id(d.id),
        "ReceiptHandle": format!("r-{}", d.receipt_handle),
        "Body": d.body,
        "ReceiveCount": d.receive_count,
        "SentAt": d.sent_at,
        "FirstReceivedAt": d.first_received_at,
    });
    if let (Some(group), Some(sequence)) = (d.group_id, d.sequence) {
        message["GroupId"] = json!(&*group);
        message["SequenceNumber"] = json!(sequence);
    }
    message
}

fn message_id(n: u64) -> String {
    format!("m-{n}")
}

fn does_not_exist(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

fn invalid_value(message: &str) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}
