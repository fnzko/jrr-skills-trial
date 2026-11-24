//! The whole service state: the clock, the id counters and every queue, plus the rules
//! that involve more than one queue.

use std::collections::BTreeMap;
use std::ops::Bound;

use crate::error::{Error, ErrorCode};
use crate::queue::{DeadLetter, Queue};
use crate::request::{AttributeSet, Request};
use crate::response::Output;
use crate::types::{IdSource, MaxReceiveCount, QueueName, ReceiptHandle, Timestamp};

const QUEUE_LIMIT: usize = 1000;

type Queues = BTreeMap<QueueName, Queue>;

#[derive(Debug, Default)]
pub struct Broker {
    now: Timestamp,
    ids: IdSource,
    queues: Queues,
}

fn does_not_exist(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

fn invalid_attributes(message: &str) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

/// Finds a queue and drops its messages whose retention has run out, so that the caller
/// only ever sees the messages that exist at `now`.
fn lookup<'a>(queues: &'a mut Queues, now: Timestamp, name: &str) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| does_not_exist(name))?;
    queue.expire(now);
    Ok(queue)
}

fn issued_receipt(ids: &IdSource, text: &str) -> Result<ReceiptHandle, Error> {
    ids.issued_receipt(text).ok_or_else(|| {
        Error::new(
            ErrorCode::ReceiptHandleInvalid,
            format!("receipt handle {text:?} was never issued"),
        )
    })
}

/// The dead-letter configuration of a request: both attributes or neither.
fn requested_dead_letter(
    attributes: &AttributeSet,
) -> Result<Option<(&str, MaxReceiveCount)>, Error> {
    match (
        attributes.dead_letter_queue.as_deref(),
        attributes.max_receive_count,
    ) {
        (Some(queue), Some(max)) => Ok(Some((queue, max))),
        (None, None) => Ok(None),
        _ => Err(invalid_attributes(
            "DeadLetterQueue and MaxReceiveCount must be given together",
        )),
    }
}

impl Broker {
    pub fn execute(&mut self, request: Request) -> Result<Output, Error> {
        match request {
            Request::CreateQueue { name, attributes } => {
                self.create_queue(name, &attributes)?;
                Ok(Output::Empty {})
            }
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)?;
                Ok(Output::Empty {})
            }
            Request::GetQueueAttributes { queue } => {
                let queue = lookup(&mut self.queues, self.now, &queue)?;
                Ok(Output::Attributes {
                    attributes: queue.attributes(),
                    counts: queue.counts(self.now),
                })
            }
            Request::ListQueues { prefix } => {
                let prefix = prefix.as_deref().unwrap_or("");
                let queue_names = self
                    .queues
                    .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
                    .map(|(name, _)| name)
                    .take_while(|name| name.as_str().starts_with(prefix))
                    .cloned()
                    .collect();
                Ok(Output::QueueNames { queue_names })
            }
            Request::DeleteQueue { queue } => {
                self.queues
                    .remove(queue.as_str())
                    .ok_or_else(|| does_not_exist(&queue))?;
                for remaining in self.queues.values_mut() {
                    remaining.clear_dead_letter_to(&queue);
                }
                Ok(Output::Empty {})
            }
            Request::PurgeQueue { queue } => {
                lookup(&mut self.queues, self.now, &queue)?.purge();
                Ok(Output::Empty {})
            }
            Request::Tick { seconds } => {
                self.now = self.now.after(seconds.get());
                Ok(Output::Now { now: self.now })
            }
            Request::SendMessage { queue, message } => {
                let queue = lookup(&mut self.queues, self.now, &queue)?;
                queue
                    .send(self.now, &mut self.ids, message)
                    .map(Output::Sent)
            }
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => {
                let queue = lookup(&mut self.queues, self.now, &queue)?;
                let received =
                    queue.receive(self.now, &mut self.ids, max_messages, visibility_timeout);
                if !received.dead_lettered.is_empty() {
                    let target = queue
                        .dead_letter()
                        .map(|dead_letter| dead_letter.queue.clone())
                        .expect("messages are only dead-lettered by a queue with a configuration");
                    let target = self
                        .queues
                        .get_mut(&target)
                        .expect("a configured dead-letter queue exists: deleting it clears the configuration");
                    for message in received.dead_lettered {
                        target.adopt(message);
                    }
                }
                Ok(Output::Messages {
                    messages: received.delivered,
                })
            }
            Request::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let queue = lookup(&mut self.queues, self.now, &queue)?;
                let handle = issued_receipt(&self.ids, &receipt_handle)?;
                queue.delete(self.now, handle)?;
                Ok(Output::Empty {})
            }
            Request::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let queue = lookup(&mut self.queues, self.now, &queue)?;
                let handle = issued_receipt(&self.ids, &receipt_handle)?;
                queue.change_visibility(self.now, handle, visibility_timeout)?;
                Ok(Output::Empty {})
            }
        }
    }

    fn create_queue(&mut self, name: QueueName, attributes: &AttributeSet) -> Result<(), Error> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is a FIFO queue",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid_attributes(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let requested = requested_dead_letter(attributes)?;
        if let Some(existing) = self.queues.get(&name) {
            return if existing.matches(attributes) {
                Ok(())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with other attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = requested
            .map(|(queue, max)| self.resolve_dead_letter(queue, max, fifo, None))
            .transpose()?;
        if self.queues.len() >= QUEUE_LIMIT {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("{QUEUE_LIMIT} queues already exist"),
            ));
        }
        self.queues
            .insert(name, Queue::new(attributes, dead_letter));
        Ok(())
    }

    fn set_queue_attributes(&mut self, name: &str, attributes: &AttributeSet) -> Result<(), Error> {
        let fifo = lookup(&mut self.queues, self.now, name)?.is_fifo();
        if attributes.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid_attributes(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = requested_dead_letter(attributes)?
            .map(|(queue, max)| self.resolve_dead_letter(queue, max, fifo, Some(name)))
            .transpose()?;
        lookup(&mut self.queues, self.now, name)?.apply(attributes, dead_letter);
        Ok(())
    }

    /// Checks that `target` can be the dead-letter queue of a queue of the given type.
    /// `owner` is the name of that queue when it already exists.
    fn resolve_dead_letter(
        &self,
        target: &str,
        max_receive_count: MaxReceiveCount,
        fifo: bool,
        owner: Option<&str>,
    ) -> Result<DeadLetter, Error> {
        let (name, queue) = self
            .queues
            .get_key_value(target)
            .ok_or_else(|| does_not_exist(target))?;
        if owner == Some(target) || queue.is_fifo() != fifo {
            return Err(invalid_attributes(
                "DeadLetterQueue must be another queue of the same type",
            ));
        }
        Ok(DeadLetter {
            queue: name.clone(),
            max_receive_count,
        })
    }
}
