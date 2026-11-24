//! The service state: every queue, the virtual clock and the id counters, plus
//! the command rules that involve more than one queue.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::command::{self, Command};
use crate::error::{Error, ErrorCode};
use crate::queue::{Attributes, Counts, DeadLetter, Delivery, Queue, QueueKind, Sent};
use crate::types::{MessageId, QueueName, ReceiptHandle, Seconds};

const QUEUE_LIMIT: usize = 1000;

type Queues = BTreeMap<QueueName, Queue>;

/// Attribute members of a `GetQueueAttributes` result.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesView {
    visibility_timeout: Seconds,
    delay_seconds: Seconds,
    retention_seconds: Seconds,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(flatten)]
    dead_letter: Option<DeadLetter>,
}

impl From<&Attributes> for AttributesView {
    fn from(attributes: &Attributes) -> Self {
        let (fifo, content_based_deduplication) = match attributes.kind {
            QueueKind::Standard => (false, false),
            QueueKind::Fifo {
                content_based_deduplication,
            } => (true, content_based_deduplication),
        };
        Self {
            visibility_timeout: attributes.visibility_timeout,
            delay_seconds: attributes.delay,
            retention_seconds: attributes.retention,
            fifo,
            content_based_deduplication,
            dead_letter: attributes.dead_letter.clone(),
        }
    }
}

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Reply {
    Done {},
    Attributes {
        #[serde(rename = "Attributes")]
        attributes: AttributesView,
        #[serde(rename = "Counts")]
        counts: Counts,
    },
    Queues {
        #[serde(rename = "QueueNames")]
        names: Vec<QueueName>,
    },
    Clock {
        #[serde(rename = "Now")]
        now: Seconds,
    },
    Sent(Sent),
    Received {
        #[serde(rename = "Messages")]
        messages: Vec<Delivery>,
    },
}

/// Counts how many ids of one kind have been handed out.
#[derive(Debug, Default)]
struct Counter(u64);

impl Counter {
    fn issue(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }

    fn issued(&self) -> u64 {
        self.0
    }
}

#[derive(Debug, Default)]
pub struct Broker {
    queues: Queues,
    now: Seconds,
    messages: Counter,
    handles: Counter,
}

#[derive(Serialize)]
struct Success<'a> {
    ok: bool,
    #[serde(flatten)]
    reply: &'a Reply,
}

#[derive(Serialize)]
struct Failure<'a> {
    ok: bool,
    error: &'a Error,
}

impl Broker {
    /// Runs one input line and returns the response line, without its newline.
    pub fn respond(&mut self, line: &[u8]) -> String {
        let outcome = command::parse(line).and_then(|command| self.execute(command));
        let json = match &outcome {
            Ok(reply) => serde_json::to_string(&Success { ok: true, reply }),
            Err(error) => serde_json::to_string(&Failure { ok: false, error }),
        };
        json.expect("responses hold only string keys and finite integers")
    }

    pub fn execute(&mut self, command: Command) -> Result<Reply, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => {
                let now = self.now;
                let queue = lookup(&mut self.queues, &queue, now)?;
                Ok(Reply::Attributes {
                    attributes: queue.attributes().into(),
                    counts: queue.counts(now),
                })
            }
            Command::ListQueues { prefix } => {
                let prefix = prefix.as_deref().unwrap_or_default();
                let names = self
                    .queues
                    .keys()
                    .filter(|name| name.as_str().starts_with(prefix))
                    .cloned()
                    .collect();
                Ok(Reply::Queues { names })
            }
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                lookup(&mut self.queues, &queue, self.now)?.purge();
                Ok(Reply::Done {})
            }
            Command::Tick { seconds } => {
                self.now = self.now.saturating_add(seconds);
                Ok(Reply::Clock { now: self.now })
            }
            Command::SendMessage { queue, message } => {
                let queue = lookup(&mut self.queues, &queue, self.now)?;
                let messages = &mut self.messages;
                queue
                    .send(self.now, message, || MessageId::new(messages.issue()))
                    .map(Reply::Sent)
            }
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
                let handle = issued_handle(&receipt_handle, &self.handles)?;
                queue.delete(handle, self.now)?;
                Ok(Reply::Done {})
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let queue = lookup(&mut self.queues, &queue, self.now)?;
                let handle = issued_handle(&receipt_handle, &self.handles)?;
                queue.change_visibility(handle, self.now, visibility_timeout)?;
                Ok(Reply::Done {})
            }
        }
    }

    fn create_queue(
        &mut self,
        name: QueueName,
        values: &command::AttributeValues,
    ) -> Result<Reply, Error> {
        if name.has_fifo_suffix() != values.fifo.unwrap_or(false) {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is FIFO",
            ));
        }
        if values.content_based_deduplication == Some(true) && values.fifo != Some(true) {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let requested_dead_letter = values.dead_letter()?;
        if let Some(existing) = self.queues.get(&name) {
            return if existing
                .attributes()
                .match_create(values, requested_dead_letter)
            {
                Ok(Reply::Done {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {} exists with different attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = requested_dead_letter
            .map(|(target, max_receive_count)| {
                let (queue, target_queue) = existing_queue(&self.queues, target)?;
                if target_queue.is_fifo() != name.has_fifo_suffix() {
                    return Err(Error::invalid_value(
                        "the dead-letter queue must be of the same type as the queue",
                    ));
                }
                Ok(DeadLetter {
                    queue: queue.clone(),
                    max_receive_count,
                })
            })
            .transpose()?;
        if self.queues.len() >= QUEUE_LIMIT {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {QUEUE_LIMIT} queues can exist"),
            ));
        }
        let attributes = Attributes::create(values, dead_letter);
        self.queues.insert(name, Queue::new(attributes));
        Ok(Reply::Done {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        values: &command::AttributeValues,
    ) -> Result<Reply, Error> {
        let now = self.now;
        let queue = lookup(&mut self.queues, name, now)?;
        if values.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue never changes",
            ));
        }
        if values.content_based_deduplication == Some(true) && !queue.is_fifo() {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let requested = values.dead_letter()?;
        let is_fifo = queue.is_fifo();
        let dead_letter = requested
            .map(|(target, max_receive_count)| {
                let (target_name, target_queue) = existing_queue(&self.queues, target)?;
                if target == name || target_queue.is_fifo() != is_fifo {
                    return Err(Error::invalid_value(
                        "the dead-letter queue must be another queue of the same type",
                    ));
                }
                Ok(DeadLetter {
                    queue: target_name.clone(),
                    max_receive_count,
                })
            })
            .transpose()?;
        lookup(&mut self.queues, name, now)?
            .attributes_mut()
            .update(values, dead_letter);
        Ok(Reply::Done {})
    }

    fn delete_queue(&mut self, name: &str) -> Result<Reply, Error> {
        lookup(&mut self.queues, name, self.now)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            let attributes = queue.attributes_mut();
            if attributes
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue.as_str() == name)
            {
                attributes.dead_letter = None;
            }
        }
        Ok(Reply::Done {})
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
    ) -> Result<Reply, Error> {
        let now = self.now;
        let queue = lookup(&mut self.queues, name, now)?;
        let visibility_timeout =
            visibility_timeout.unwrap_or(queue.attributes().visibility_timeout);
        let handles = &mut self.handles;
        let received = queue.receive(now, max_messages, visibility_timeout, || {
            ReceiptHandle::new(handles.issue())
        });
        if let Some(moved) = received.dead_lettered {
            let target = self
                .queues
                .get_mut(moved.queue.as_str())
                .expect("a queue that is named as a dead-letter queue exists");
            for entry in moved.entries {
                target.add(entry);
            }
            target.expire(now);
        }
        Ok(Reply::Received {
            messages: received.delivered,
        })
    }
}

/// Finds a queue by exact name and drops the messages it can no longer hold.
fn lookup<'a>(queues: &'a mut Queues, name: &str, now: Seconds) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| queue_missing(name))?;
    queue.expire(now);
    Ok(queue)
}

/// Finds a queue that another queue names as its dead-letter queue.
fn existing_queue<'a>(queues: &'a Queues, name: &str) -> Result<(&'a QueueName, &'a Queue), Error> {
    queues
        .get_key_value(name)
        .ok_or_else(|| queue_missing(name))
}

fn queue_missing(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

fn issued_handle(text: &str, handles: &Counter) -> Result<ReceiptHandle, Error> {
    ReceiptHandle::parse_issued(text, handles.issued()).ok_or_else(|| {
        Error::new(
            ErrorCode::ReceiptHandleInvalid,
            format!("receipt handle {text:?} was never issued"),
        )
    })
}
