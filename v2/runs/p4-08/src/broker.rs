//! The process-wide state: the clock, the queues and the id counters.

use std::collections::BTreeMap;

use crate::error::{CommandError, ErrorCode};
use crate::queue::{DeadLetter, Queue, QueueConfig};
use crate::request::{Attributes, Request, SendMessage};
use crate::response::Response;
use crate::types::{Counter, QueueName, ReceiptHandle};

/// Most queues that may exist at once.
pub const QUEUE_LIMIT: usize = 1000;

const DEFAULT_VISIBILITY_TIMEOUT: u64 = 30;
const DEFAULT_DELAY_SECONDS: u64 = 0;
const DEFAULT_RETENTION_SECONDS: u64 = 345_600;

#[derive(Debug, Default)]
pub struct Broker {
    now: u64,
    /// A `BTreeMap` so `ListQueues` gets names in byte order for free.
    queues: BTreeMap<QueueName, Queue>,
    message_ids: Counter,
    receipt_handles: Counter,
}

impl Broker {
    /// Runs one parsed command. On error nothing has changed.
    pub fn execute(&mut self, request: Request) -> Result<Response, CommandError> {
        match request {
            Request::CreateQueue { queue, attributes } => self.create_queue(queue, attributes),
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, attributes)
            }
            Request::GetQueueAttributes { queue } => {
                let now = self.now;
                let queue = lookup(&mut self.queues, &queue, now)?;
                Ok(Response::QueueAttributes {
                    attributes: queue.config.clone(),
                    counts: queue.counts(now),
                })
            }
            Request::ListQueues { prefix } => Ok(Response::QueueNames(
                self.queues
                    .keys()
                    .filter(|name| {
                        prefix
                            .as_deref()
                            .is_none_or(|prefix| name.as_str().starts_with(prefix))
                    })
                    .cloned()
                    .collect(),
            )),
            Request::DeleteQueue { queue } => self.delete_queue(&queue),
            Request::PurgeQueue { queue } => {
                lookup(&mut self.queues, &queue, self.now)?.purge();
                Ok(Response::Done)
            }
            Request::Tick { seconds } => {
                self.now += seconds;
                Ok(Response::Tick { now: self.now })
            }
            Request::SendMessage(request) => self.send_message(request),
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Request::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let queue = lookup(&mut self.queues, &queue, self.now)?;
                let receipt = issued(&receipt_handle, &self.receipt_handles)?;
                queue.delete_message(receipt, self.now)?;
                Ok(Response::Done)
            }
            Request::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let queue = lookup(&mut self.queues, &queue, self.now)?;
                let receipt = issued(&receipt_handle, &self.receipt_handles)?;
                queue.change_visibility(receipt, self.now, visibility_timeout)?;
                Ok(Response::Done)
            }
        }
    }

    fn create_queue(
        &mut self,
        name: QueueName,
        attributes: Attributes,
    ) -> Result<Response, CommandError> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.is_fifo_name() != fifo {
            return Err(CommandError::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when Fifo is true",
            ));
        }
        let content_based_deduplication = attributes.content_based_deduplication.unwrap_or(false);
        if content_based_deduplication && !fifo {
            return Err(invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let config = QueueConfig {
            visibility_timeout: attributes
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay_seconds: attributes.delay_seconds.unwrap_or(DEFAULT_DELAY_SECONDS),
            retention_seconds: attributes
                .retention_seconds
                .unwrap_or(DEFAULT_RETENTION_SECONDS),
            fifo,
            content_based_deduplication,
            dead_letter: dead_letter(attributes.dead_letter_queue, attributes.max_receive_count)?,
        };
        if let Some(existing) = self.queues.get(&name) {
            return if existing.config == config {
                Ok(Response::Done)
            } else {
                Err(CommandError::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {name} exists with different attributes"),
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
                    "a dead-letter queue must be of the same type as its queue",
                ));
            }
        }
        if self.queues.len() >= QUEUE_LIMIT {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                format!("{QUEUE_LIMIT} queues already exist"),
            ));
        }
        self.queues.insert(name, Queue::new(config));
        Ok(Response::Done)
    }

    fn set_queue_attributes(
        &mut self,
        name: &QueueName,
        attributes: Attributes,
    ) -> Result<Response, CommandError> {
        // Expiry is lazy, so messages already past the old retention must go
        // before a raised `RetentionSeconds` could bring them back.
        let fifo = lookup(&mut self.queues, name, self.now)?.config.fifo;
        if attributes.fifo.is_some() {
            return Err(CommandError::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue never changes",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = dead_letter(attributes.dead_letter_queue, attributes.max_receive_count)?;
        if let Some(dead_letter) = &dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| does_not_exist(&dead_letter.queue))?;
            if dead_letter.queue == *name || target.config.fifo != fifo {
                return Err(invalid_value(
                    "a dead-letter queue must be another queue of the same type",
                ));
            }
        }

        let config = &mut self
            .queues
            .get_mut(name)
            .expect("the queue was looked up above")
            .config;
        if let Some(v) = attributes.visibility_timeout {
            config.visibility_timeout = v;
        }
        if let Some(v) = attributes.delay_seconds {
            config.delay_seconds = v;
        }
        if let Some(v) = attributes.retention_seconds {
            config.retention_seconds = v;
        }
        if let Some(v) = attributes.content_based_deduplication {
            config.content_based_deduplication = v;
        }
        if dead_letter.is_some() {
            config.dead_letter = dead_letter;
        }
        Ok(Response::Done)
    }

    fn delete_queue(&mut self, name: &QueueName) -> Result<Response, CommandError> {
        self.queues
            .remove(name)
            .ok_or_else(|| does_not_exist(name))?;
        for queue in self.queues.values_mut() {
            if queue
                .config
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue == *name)
            {
                queue.config.dead_letter = None;
            }
        }
        Ok(Response::Done)
    }

    fn send_message(&mut self, request: SendMessage) -> Result<Response, CommandError> {
        let queue = lookup(&mut self.queues, &request.queue, self.now)?;
        let sent = queue.send(request, self.now, &mut self.message_ids)?;
        Ok(Response::Sent(sent))
    }

    fn receive_message(
        &mut self,
        name: &QueueName,
        max_messages: usize,
        visibility_timeout: Option<u64>,
    ) -> Result<Response, CommandError> {
        let now = self.now;
        let queue = lookup(&mut self.queues, name, now)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.config.visibility_timeout);
        let received = queue.receive(
            now,
            max_messages,
            visibility_timeout,
            &mut self.receipt_handles,
        );
        if !received.dead_lettered.is_empty() {
            let target_name = queue
                .config
                .dead_letter
                .as_ref()
                .expect("messages are dead-lettered only by a queue with a dead-letter config")
                .queue
                .clone();
            let target = self
                .queues
                .get_mut(&target_name)
                .expect("deleting a queue clears every dead-letter config that names it");
            for message in received.dead_lettered {
                target.accept_dead_letter(message, now);
            }
        }
        Ok(Response::Messages(received.delivered))
    }
}

/// Finds a queue and drops its expired messages, so that callers only ever
/// see messages that are still retained.
fn lookup<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    name: &QueueName,
    now: u64,
) -> Result<&'a mut Queue, CommandError> {
    let queue = queues.get_mut(name).ok_or_else(|| does_not_exist(name))?;
    queue.expire(now);
    Ok(queue)
}

/// The handle `text` names, if this process has issued it.
fn issued(text: &str, receipt_handles: &Counter) -> Result<ReceiptHandle, CommandError> {
    ReceiptHandle::parse(text)
        .filter(|handle| handle.0 <= receipt_handles.last())
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("{text:?} was never issued"),
            )
        })
}

/// The dead-letter configuration of a request: both attributes or neither.
fn dead_letter(
    queue: Option<QueueName>,
    max_receive_count: Option<u64>,
) -> Result<Option<DeadLetter>, CommandError> {
    match (queue, max_receive_count) {
        (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
            queue,
            max_receive_count,
        })),
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Err(invalid_value(
            "DeadLetterQueue and MaxReceiveCount must be given together",
        )),
    }
}

fn does_not_exist(name: &QueueName) -> CommandError {
    CommandError::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

fn invalid_value(message: &str) -> CommandError {
    CommandError::new(ErrorCode::InvalidParameterValue, message)
}
