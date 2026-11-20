//! The service state and the execution of parsed commands against it.

use std::collections::BTreeMap;
use std::ops::Bound;

use serde::Serialize;

use crate::error::Error;
use crate::queue::{
    DEFAULT_DELAY, DEFAULT_RETENTION, DEFAULT_VISIBILITY_TIMEOUT, DeadLetter, Queue,
    QueueAttributes, Sent,
};
use crate::request::{AttributeChanges, ReceiveMessage, Request, SendMessage};
use crate::store::{Counts, NotInFlight, ReceivedMessage};
use crate::types::{Counter, DeadLetterRef, QueueName, ReceiptHandle, Timestamp};

const MAX_QUEUES: usize = 1000;

/// The result members of a successful command.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Output<'a> {
    Empty {},
    #[serde(rename_all = "PascalCase")]
    Clock {
        now: Timestamp,
    },
    #[serde(rename_all = "PascalCase")]
    QueueAttributes {
        attributes: QueueAttributes<'a>,
        counts: Counts,
    },
    #[serde(rename_all = "PascalCase")]
    QueueNames {
        queue_names: Vec<&'a QueueName>,
    },
    Sent(Sent),
    #[serde(rename_all = "PascalCase")]
    Received {
        messages: Vec<ReceivedMessage>,
    },
}

/// All queues and counters of one process.
#[derive(Debug, Default)]
pub struct Broker {
    now: Timestamp,
    /// Ordered by name, which for `String` is byte order, as `ListQueues` requires.
    queues: BTreeMap<QueueName, Queue>,
    message_ids: Counter,
    receipt_handles: Counter,
}

impl Broker {
    /// Runs one command. On error the state is unchanged.
    pub fn execute(&mut self, request: Request) -> Result<Output<'_>, Error> {
        match request {
            Request::CreateQueue { queue, attributes } => self
                .create_queue(queue, &attributes)
                .map(|()| Output::Empty {}),
            Request::SetQueueAttributes { queue, attributes } => self
                .set_queue_attributes(&queue, &attributes)
                .map(|()| Output::Empty {}),
            Request::GetQueueAttributes { queue } => {
                let queue = find_queue(&mut self.queues, &queue, self.now)?;
                Ok(Output::QueueAttributes {
                    counts: queue.counts(self.now),
                    attributes: queue.attributes(),
                })
            }
            Request::ListQueues { prefix } => {
                let prefix = prefix.as_deref().unwrap_or("");
                let queue_names = self
                    .queues
                    .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
                    .map(|(name, _)| name)
                    .take_while(|name| name.as_str().starts_with(prefix))
                    .collect();
                Ok(Output::QueueNames { queue_names })
            }
            Request::DeleteQueue { queue } => self.delete_queue(&queue).map(|()| Output::Empty {}),
            Request::PurgeQueue { queue } => {
                find_queue(&mut self.queues, &queue, self.now)?.purge();
                Ok(Output::Empty {})
            }
            Request::Tick { seconds } => {
                self.now = self.now.plus(seconds);
                Ok(Output::Clock { now: self.now })
            }
            Request::SendMessage(send) => self.send_message(send).map(Output::Sent),
            Request::ReceiveMessage(receive) => self
                .receive_message(&receive)
                .map(|messages| Output::Received { messages }),
            Request::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let handle = self.issued_handle(&receipt_handle);
                let queue = find_queue(&mut self.queues, &queue, self.now)?;
                let handle = handle?;
                queue
                    .delete_message(handle, self.now)
                    .map_err(|NotInFlight| Error::ReceiptHandleExpired(handle))?;
                Ok(Output::Empty {})
            }
            Request::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let handle = self.issued_handle(&receipt_handle);
                let queue = find_queue(&mut self.queues, &queue, self.now)?;
                let handle = handle?;
                queue
                    .change_visibility(handle, self.now, visibility_timeout)
                    .map_err(|NotInFlight| Error::ReceiptHandleExpired(handle))?;
                Ok(Output::Empty {})
            }
        }
    }

    /// The handle that `text` names if this process has issued it. Callers
    /// report a failure only after the queue lookup, which the spec checks first.
    fn issued_handle(&self, text: &str) -> Result<ReceiptHandle, Error> {
        ReceiptHandle::parse(text)
            .filter(|&handle| self.receipt_handles.has_issued(handle))
            .ok_or_else(|| Error::ReceiptHandleInvalid(text.to_owned()))
    }

    fn create_queue(&mut self, name: QueueName, changes: &AttributeChanges) -> Result<(), Error> {
        let fifo = changes.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::QueueNameInvalid {
                rule: "must end in .fifo exactly when the Fifo attribute is true",
            });
        }
        let content_based_deduplication = changes.content_based_deduplication.unwrap_or(false);
        if content_based_deduplication && !fifo {
            return Err(Error::InvalidParameterValue {
                parameter: "ContentBasedDeduplication",
                rule: "can only be true for a FIFO queue",
            });
        }
        let requested = QueueAttributes {
            visibility_timeout: changes
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay_seconds: changes.delay.unwrap_or(DEFAULT_DELAY),
            retention_seconds: changes.retention.unwrap_or(DEFAULT_RETENTION),
            fifo,
            content_based_deduplication,
            dead_letter: changes.dead_letter()?,
        };
        if let Some(existing) = self.queues.get(&name) {
            return if existing.attributes() == requested {
                Ok(())
            } else {
                Err(Error::QueueAlreadyExists(name))
            };
        }
        let dead_letter = requested
            .dead_letter
            .map(|requested| self.resolve_dead_letter(requested, fifo))
            .transpose()?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::LimitExceeded("1000 queues already exist"));
        }
        self.queues
            .insert(name, Queue::new(&requested, dead_letter));
        Ok(())
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        changes: &AttributeChanges,
    ) -> Result<(), Error> {
        let fifo = self
            .queues
            .get(name)
            .ok_or_else(|| Error::QueueDoesNotExist(name.to_owned()))?
            .is_fifo();
        if changes.fifo.is_some() {
            return Err(Error::UnsupportedOperation(
                "the type of a queue never changes",
            ));
        }
        if changes.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::InvalidParameterValue {
                parameter: "ContentBasedDeduplication",
                rule: "can only be true for a FIFO queue",
            });
        }
        let dead_letter = changes
            .dead_letter()?
            .map(|requested| {
                // The queue itself exists, so naming it passes the existence
                // check and fails the next one.
                if requested.queue == name {
                    return Err(Error::InvalidParameterValue {
                        parameter: "DeadLetterQueue",
                        rule: "must not be the queue itself",
                    });
                }
                self.resolve_dead_letter(requested, fifo)
            })
            .transpose()?;
        // Expiring with the old retention first keeps a later increase from
        // bringing back messages that were already due for removal.
        let queue = find_queue(&mut self.queues, name, self.now)?;
        if let Some(timeout) = changes.visibility_timeout {
            queue.visibility_timeout = timeout;
        }
        if let Some(delay) = changes.delay {
            queue.delay = delay;
        }
        if let Some(retention) = changes.retention {
            queue.retention = retention;
        }
        if let Some(enabled) = changes.content_based_deduplication {
            queue.set_content_based_deduplication(enabled);
        }
        if let Some(dead_letter) = dead_letter {
            queue.dead_letter = Some(dead_letter);
        }
        Ok(())
    }

    /// Checks that the requested dead-letter queue exists and has the given
    /// type, and returns the configuration that refers to it.
    fn resolve_dead_letter(
        &self,
        requested: DeadLetterRef<'_>,
        fifo: bool,
    ) -> Result<DeadLetter, Error> {
        let (queue, target) = self
            .queues
            .get_key_value(requested.queue)
            .ok_or_else(|| Error::QueueDoesNotExist(requested.queue.to_owned()))?;
        if target.is_fifo() != fifo {
            return Err(Error::InvalidParameterValue {
                parameter: "DeadLetterQueue",
                rule: "must be a queue of the same type",
            });
        }
        Ok(DeadLetter {
            queue: queue.clone(),
            max_receive_count: requested.max_receive_count,
        })
    }

    fn delete_queue(&mut self, name: &str) -> Result<(), Error> {
        self.queues
            .remove(name)
            .ok_or_else(|| Error::QueueDoesNotExist(name.to_owned()))?;
        for queue in self.queues.values_mut() {
            if queue
                .dead_letter
                .as_ref()
                .is_some_and(|d| d.queue.as_str() == name)
            {
                queue.dead_letter = None;
            }
        }
        Ok(())
    }

    fn send_message(&mut self, send: SendMessage) -> Result<Sent, Error> {
        let queue = find_queue(&mut self.queues, &send.queue, self.now)?;
        queue.send(send, self.now, &mut self.message_ids)
    }

    fn receive_message(&mut self, receive: &ReceiveMessage) -> Result<Vec<ReceivedMessage>, Error> {
        let now = self.now;
        // The queue is taken out of the map so that its dead-letter queue can be
        // borrowed mutably at the same time; a queue is never its own
        // dead-letter queue, and the queue goes back before returning.
        let (name, mut queue) = self
            .queues
            .remove_entry(receive.queue.as_str())
            .ok_or_else(|| Error::QueueDoesNotExist(receive.queue.clone()))?;
        queue.expire(now);
        let (delivered, dead) = queue.receive(
            now,
            receive.max_messages,
            receive.visibility_timeout,
            &mut self.receipt_handles,
        );
        // Without a dead-letter configuration nothing is dead-lettered.
        if let Some(dead_letter) = &queue.dead_letter {
            let target = self.queues.get_mut(&dead_letter.queue).expect(
                "a configured dead-letter queue exists; DeleteQueue clears references to it",
            );
            for message in dead {
                target.accept_dead_letter(message, now);
            }
        }
        self.queues.insert(name, queue);
        Ok(delivered)
    }
}

/// Looks a queue up by exact name and brings its messages up to date with
/// retention, which every observation of a queue must do first.
fn find_queue<'q>(
    queues: &'q mut BTreeMap<QueueName, Queue>,
    name: &str,
    now: Timestamp,
) -> Result<&'q mut Queue, Error> {
    let queue = queues
        .get_mut(name)
        .ok_or_else(|| Error::QueueDoesNotExist(name.to_owned()))?;
    queue.expire(now);
    Ok(queue)
}
