//! The whole state of a miniq process and the commands that act on it.

use std::collections::BTreeMap;

use crate::error::{ApiError, ErrorCode};
use crate::queue::{Attributes, Queue};
use crate::request::{self, AttributeUpdate, Request};
use crate::response::{self, Output};
use crate::types::{MessageIds, QueueName, ReceiptHandle, ReceiptHandles, Time};

const MAX_QUEUES: usize = 1000;

/// An in-memory set of queues on a virtual clock.
#[derive(Debug, Default)]
pub struct Service {
    now: Time,
    /// Ordered by name bytes, the order `ListQueues` reports.
    queues: BTreeMap<QueueName, Queue>,
    message_ids: MessageIds,
    receipt_handles: ReceiptHandles,
}

impl Service {
    pub fn new() -> Service {
        Service::default()
    }

    /// Runs one command line (without its `\n`) and returns the response
    /// line (without its `\n`).
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        let result = request::parse(line).and_then(|request| self.execute(request));
        response::render(&result)
    }

    fn execute(&mut self, request: Request) -> Result<Output, ApiError> {
        let now = self.now;
        match request {
            Request::CreateQueue { queue, attributes } => self.create_queue(queue, &attributes),
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Request::GetQueueAttributes { queue } => {
                let queue = find(&mut self.queues, &queue, now)?;
                Ok(Output::QueueAttributes {
                    attributes: queue.attributes(),
                    counts: queue.counts(now),
                })
            }
            Request::ListQueues { prefix } => {
                let prefix = prefix.as_deref().unwrap_or("");
                let queue_names = self
                    .queues
                    .keys()
                    .filter(|name| name.as_str().starts_with(prefix))
                    .cloned()
                    .collect();
                Ok(Output::QueueNames { queue_names })
            }
            Request::DeleteQueue { queue } => {
                if self.queues.remove(queue.as_str()).is_none() {
                    return Err(ApiError::queue_does_not_exist(&queue));
                }
                for other in self.queues.values_mut() {
                    other.forget_dead_letter_queue(&queue);
                }
                Ok(Output::Empty {})
            }
            Request::PurgeQueue { queue } => {
                find(&mut self.queues, &queue, now)?.purge();
                Ok(Output::Empty {})
            }
            Request::Tick { seconds } => {
                self.now = now.plus(seconds);
                Ok(Output::Now { now: self.now })
            }
            Request::SendMessage(send) => {
                let queue = find(&mut self.queues, &send.queue, now)?;
                let sent = queue.send(send, now, &mut self.message_ids)?;
                Ok(Output::Sent(sent))
            }
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Request::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let queue = find(&mut self.queues, &queue, now)?;
                let handle = issued(&self.receipt_handles, &receipt_handle)?;
                queue.delete_message(handle, now)?;
                Ok(Output::Empty {})
            }
            Request::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let queue = find(&mut self.queues, &queue, now)?;
                let handle = issued(&self.receipt_handles, &receipt_handle)?;
                queue.change_visibility(handle, now, visibility_timeout)?;
                Ok(Output::Empty {})
            }
        }
    }

    fn create_queue(
        &mut self,
        name: QueueName,
        update: &AttributeUpdate,
    ) -> Result<Output, ApiError> {
        let fifo = update.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(ApiError::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when Fifo is true",
            ));
        }
        let content_based_deduplication = update.content_based_deduplication.unwrap_or(false);
        if content_based_deduplication && !fifo {
            return Err(ApiError::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let attributes = Attributes {
            visibility_timeout: update.visibility_timeout.unwrap_or(30),
            delay_seconds: update.delay_seconds.unwrap_or(0),
            retention_seconds: update.retention_seconds.unwrap_or(345_600),
            fifo,
            content_based_deduplication,
            dead_letter: update.dead_letter()?,
        };

        if let Some(existing) = self.queues.get(&name) {
            if existing.attributes() == attributes {
                return Ok(Output::Empty {});
            }
            return Err(ApiError::new(
                ErrorCode::QueueAlreadyExists,
                format!("queue {name} exists with different attributes"),
            ));
        }
        if let Some(dead_letter) = &attributes.dead_letter {
            let target = self
                .queues
                .get(dead_letter.queue.as_str())
                .ok_or_else(|| ApiError::queue_does_not_exist(&dead_letter.queue))?;
            if target.is_fifo() != fifo {
                return Err(ApiError::invalid_value(
                    "a dead-letter queue must have the same type as its source queue",
                ));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(ApiError::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        self.queues.insert(name, Queue::new(attributes));
        Ok(Output::Empty {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        update: &AttributeUpdate,
    ) -> Result<Output, ApiError> {
        let now = self.now;
        // The lookup also expires messages under the old retention period,
        // before a new one can apply.
        let fifo = find(&mut self.queues, name, now)?.is_fifo();
        if update.fifo.is_some() {
            return Err(ApiError::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue never changes",
            ));
        }
        if update.content_based_deduplication == Some(true) && !fifo {
            return Err(ApiError::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = update.dead_letter()?;
        if let Some(dead_letter) = &dead_letter {
            let target = self
                .queues
                .get(dead_letter.queue.as_str())
                .ok_or_else(|| ApiError::queue_does_not_exist(&dead_letter.queue))?;
            if dead_letter.queue == name || target.is_fifo() != fifo {
                return Err(ApiError::invalid_value(
                    "a dead-letter queue must be another queue of the same type",
                ));
            }
        }
        self.queues
            .get_mut(name)
            .expect("the queue was found above")
            .update(update, dead_letter);
        Ok(Output::Empty {})
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u32,
        visibility_timeout: Option<u32>,
    ) -> Result<Output, ApiError> {
        let now = self.now;
        let queue = find(&mut self.queues, name, now)?;
        let max = usize::try_from(max_messages).expect("MaxMessages is at most 10");
        let received = queue.receive(now, max, visibility_timeout, &mut self.receipt_handles);
        if !received.dead_lettered.is_empty() {
            let target_name = queue
                .dead_letter()
                .expect("messages are dead-lettered only when a dead-letter queue is set")
                .queue
                .clone();
            let target = self
                .queues
                .get_mut(target_name.as_str())
                .expect("deleting a queue removes it as dead-letter queue everywhere");
            for message in received.dead_lettered {
                target.accept_dead_letter(message, now);
            }
        }
        Ok(Output::Messages {
            messages: received.deliveries,
        })
    }
}

/// The queue named `name`, with its expired messages removed.
fn find<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    name: &str,
    now: Time,
) -> Result<&'a mut Queue, ApiError> {
    let queue = queues
        .get_mut(name)
        .ok_or_else(|| ApiError::queue_does_not_exist(name))?;
    queue.expire(now);
    Ok(queue)
}

/// The handle named by `text`, if the process has issued it.
fn issued(handles: &ReceiptHandles, text: &str) -> Result<ReceiptHandle, ApiError> {
    text.parse()
        .ok()
        .filter(|handle| handles.has_issued(*handle))
        .ok_or_else(|| {
            ApiError::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("{text:?} was never issued as a receipt handle"),
            )
        })
}
