//! The in-memory state of the service and the execution of commands.

use std::collections::BTreeMap;
use std::ops::Bound;

use serde_json::{Map, Value, json};

use crate::attributes::AttributeUpdate;
use crate::error::{Error, ErrorCode};
use crate::queue::{Delivered, MessageId, Queue, QueueId};
use crate::receipts::ReceiptLog;
use crate::request::{Command, FIFO_SUFFIX};

const MAX_QUEUES: usize = 1000;

/// Result members of a successful command.
type Members = Map<String, Value>;

#[derive(Debug)]
pub struct Broker {
    /// Virtual clock in seconds; only `Tick` advances it.
    now: u64,
    /// Ordered by name, which is the order `ListQueues` answers in.
    queues: BTreeMap<String, Queue>,
    next_queue_id: u64,
    next_message_id: u64,
    receipts: ReceiptLog,
}

impl Default for Broker {
    fn default() -> Self {
        Self {
            now: 0,
            queues: BTreeMap::new(),
            next_queue_id: 1,
            next_message_id: 1,
            receipts: ReceiptLog::default(),
        }
    }
}

/// Finds a queue by exact name and drops its expired messages.
fn lookup<'a>(
    queues: &'a mut BTreeMap<String, Queue>,
    name: &str,
    now: u64,
) -> Result<&'a mut Queue, Error> {
    let queue = queues
        .get_mut(name)
        .ok_or_else(|| Error::no_such_queue(name))?;
    queue.expire(now);
    Ok(queue)
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one input line and returns the response line, without its newline.
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        let outcome = Command::parse(line).and_then(|command| self.execute(command));
        render(outcome).to_string()
    }

    fn execute(&mut self, command: Command) -> Result<Members, Error> {
        match command {
            Command::CreateQueue { name, attributes } => {
                self.create_queue(name, attributes).map(|()| Members::new())
            }
            Command::SetQueueAttributes { queue, attributes } => self
                .set_queue_attributes(&queue, attributes)
                .map(|()| Members::new()),
            Command::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Command::ListQueues { prefix } => Ok(self.list_queues(&prefix.unwrap_or_default())),
            Command::DeleteQueue { queue } => self.delete_queue(&queue).map(|()| Members::new()),
            Command::PurgeQueue { queue } => {
                let queue = self
                    .queues
                    .get_mut(&queue)
                    .ok_or_else(|| Error::no_such_queue(&queue))?;
                queue.purge();
                Ok(Members::new())
            }
            Command::Tick { seconds } => {
                self.now += seconds;
                Ok(members([("Now", self.now.into())]))
            }
            Command::SendMessage { queue, request } => {
                let queue = lookup(&mut self.queues, &queue, self.now)?;
                let sent = queue.send(self.now, MessageId(self.next_message_id), request)?;
                if sent.accepted {
                    self.next_message_id += 1;
                }
                let mut result = members([("MessageId", sent.message_id.to_string().into())]);
                if queue.config().fifo {
                    result.insert("SequenceNumber".into(), sent.sequence.into());
                }
                Ok(result)
            }
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => {
                let queue = lookup(&mut self.queues, &queue, self.now)?;
                let visibility_timeout =
                    visibility_timeout.unwrap_or(queue.config().visibility_timeout);
                let fifo = queue.config().fifo;
                let received = queue.receive(
                    self.now,
                    max_messages,
                    visibility_timeout,
                    &mut self.receipts,
                );
                if !received.dead.is_empty() {
                    let target = queue
                        .config()
                        .dead_letter
                        .as_ref()
                        .expect("messages only move when a dead-letter queue is configured")
                        .queue
                        .clone();
                    let target = self
                        .queues
                        .get_mut(&target)
                        .expect("deleting a queue removes the dead-letter configs naming it");
                    for message in received.dead {
                        target.accept_moved(self.now, message);
                    }
                }
                let messages: Vec<Value> = received
                    .delivered
                    .into_iter()
                    .map(|message| delivered_json(message, fifo))
                    .collect();
                Ok(members([("Messages", messages.into())]))
            }
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let queue = lookup(&mut self.queues, &queue, self.now)?;
                let (receipt, issued) = self.receipts.resolve(&receipt_handle)?;
                queue.delete(self.now, receipt, issued)?;
                Ok(Members::new())
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let queue = lookup(&mut self.queues, &queue, self.now)?;
                let (receipt, issued) = self.receipts.resolve(&receipt_handle)?;
                queue.change_visibility(self.now, receipt, issued, visibility_timeout)?;
                Ok(Members::new())
            }
        }
    }

    fn create_queue(&mut self, name: String, update: AttributeUpdate) -> Result<(), Error> {
        let fifo = update.fifo.unwrap_or(false);
        if name.ends_with(FIFO_SUFFIX) != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("a queue is FIFO exactly when its name ends in {FIFO_SUFFIX}"),
            ));
        }
        if update.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = update.dead_letter()?;
        let config = update.into_config(dead_letter);

        if let Some(existing) = self.queues.get(&name) {
            return if *existing.config() == config {
                Ok(())
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
                .ok_or_else(|| Error::no_such_queue(&dead_letter.queue))?;
            if target.config().fifo != fifo {
                return Err(Error::invalid_value(
                    "the dead-letter queue must be of the same type as the queue",
                ));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("{MAX_QUEUES} queues already exist"),
            ));
        }

        let id = QueueId(self.next_queue_id);
        self.next_queue_id += 1;
        self.queues.insert(name, Queue::new(id, config));
        Ok(())
    }

    fn set_queue_attributes(&mut self, name: &str, update: AttributeUpdate) -> Result<(), Error> {
        let fifo = self
            .queues
            .get(name)
            .ok_or_else(|| Error::no_such_queue(name))?
            .config()
            .fifo;
        if update.fifo.is_some() {
            return Err(Error::unsupported("the type of a queue cannot change"));
        }
        if update.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = update.dead_letter()?;
        if let Some(dead_letter) = &dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| Error::no_such_queue(&dead_letter.queue))?;
            if dead_letter.queue == name || target.config().fifo != fifo {
                return Err(Error::invalid_value(
                    "the dead-letter queue must be another queue of the same type",
                ));
            }
        }
        self.queues
            .get_mut(name)
            .expect("the queue was found above and nothing has been removed since")
            .set_attributes(self.now, update, dead_letter);
        Ok(())
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Members, Error> {
        let queue = lookup(&mut self.queues, name, self.now)?;
        let config = queue.config();
        let mut attributes = json!({
            "VisibilityTimeout": config.visibility_timeout,
            "DelaySeconds": config.delay_seconds,
            "RetentionSeconds": config.retention_seconds,
            "Fifo": config.fifo,
            "ContentBasedDeduplication": config.content_based_deduplication,
        });
        if let Some(dead_letter) = &config.dead_letter {
            attributes["DeadLetterQueue"] = dead_letter.queue.as_str().into();
            attributes["MaxReceiveCount"] = dead_letter.max_receive_count.into();
        }
        let counts = queue.counts(self.now);
        Ok(members([
            ("Attributes", attributes),
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

    fn list_queues(&self, prefix: &str) -> Members {
        let names: Vec<&str> = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name.as_str())
            .take_while(|name| name.starts_with(prefix))
            .collect();
        members([("QueueNames", names.into())])
    }

    fn delete_queue(&mut self, name: &str) -> Result<(), Error> {
        self.queues
            .remove(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        for queue in self.queues.values_mut() {
            queue.drop_dead_letter(name);
        }
        Ok(())
    }
}

fn members<const N: usize>(entries: [(&str, Value); N]) -> Members {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn delivered_json(message: Delivered, fifo: bool) -> Value {
    let mut json = json!({
        "MessageId": message.message_id.to_string(),
        "ReceiptHandle": message.receipt.to_string(),
        "Body": message.body,
        "ReceiveCount": message.receive_count,
        "SentAt": message.sent_at,
        "FirstReceivedAt": message.first_received_at,
    });
    if fifo {
        json["GroupId"] = message.group_id.into();
        json["SequenceNumber"] = message.sequence.into();
    }
    json
}

fn render(outcome: Result<Members, Error>) -> Value {
    match outcome {
        Ok(mut members) => {
            members.insert("ok".into(), true.into());
            Value::Object(members)
        }
        Err(error) => json!({
            "ok": false,
            "error": { "code": error.code.as_str(), "message": error.message },
        }),
    }
}
