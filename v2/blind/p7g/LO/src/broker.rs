//! The service state and the numbered rules of each command that need to look at queues.

use std::collections::BTreeMap;
use std::ops::Bound;

use crate::error::{Error, ErrorCode};
use crate::queue::{
    DEFAULT_DELAY, DEFAULT_RETENTION, DEFAULT_VISIBILITY_TIMEOUT, DeadLetter, DeadLetterRequest,
    Queue, QueueType, Requested, Update,
};
use crate::request::{Attributes, Command, Request};
use crate::response::{AttributesView, ReceivedMessage, Success, render};
use crate::time::Instant;
use crate::values::{MaxMessages, MessageId, QueueName, ReceiptHandle, VisibilityTimeout};

const MAX_QUEUES: usize = 1000;

#[derive(Debug)]
pub struct Broker {
    now: Instant,
    queues: BTreeMap<QueueName, Queue>,
    next_message_id: MessageId,
    next_receipt_handle: ReceiptHandle,
}

impl Default for Broker {
    fn default() -> Self {
        Self {
            now: Instant::START,
            queues: BTreeMap::new(),
            next_message_id: MessageId::FIRST,
            next_receipt_handle: ReceiptHandle::FIRST,
        }
    }
}

/// The two dead-letter attributes, which are set together or not at all.
fn dead_letter_of<'a>(attributes: &Attributes<'a>) -> Result<Option<DeadLetterRequest<'a>>, Error> {
    match (attributes.dead_letter_queue, attributes.max_receive_count) {
        (None, None) => Ok(None),
        (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetterRequest {
            queue,
            max_receive_count,
        })),
        (Some(_), None) | (None, Some(_)) => Err(Error::invalid_value(
            "Attributes",
            "DeadLetterQueue and MaxReceiveCount must be given together",
        )),
    }
}

impl Broker {
    /// Runs one input line and returns the response line, without its newline.
    pub fn respond(&mut self, line: &[u8]) -> Result<String, serde_json::Error> {
        let outcome = Request::from_line(line)
            .and_then(|request| request.command().and_then(|command| self.execute(command)));
        render(&outcome)
    }

    fn execute(&mut self, command: Command<'_>) -> Result<Success<'_>, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => {
                let queue = self.queue(queue)?;
                Ok(Success::QueueAttributes {
                    attributes: AttributesView {
                        visibility_timeout: queue.visibility_timeout(),
                        delay: queue.delay(),
                        retention: queue.retention(),
                        fifo: queue.is_fifo(),
                        content_based_deduplication: matches!(
                            queue.queue_type(),
                            QueueType::Fifo {
                                content_based_deduplication: true
                            }
                        ),
                        dead_letter: queue.dead_letter(),
                    },
                    counts: queue.counts(self.now),
                })
            }
            Command::ListQueues { prefix } => {
                let prefix = prefix.unwrap_or_default();
                let queue_names = self
                    .queues
                    .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
                    .map(|(name, _)| name)
                    .take_while(|name| name.as_str().starts_with(prefix))
                    .collect();
                Ok(Success::QueueNames { queue_names })
            }
            Command::DeleteQueue { queue } => {
                self.queue(queue)?;
                self.queues.remove(queue);
                for other in self.queues.values_mut() {
                    if other
                        .dead_letter()
                        .is_some_and(|config| config.queue.as_str() == queue)
                    {
                        other.clear_dead_letter();
                    }
                }
                Ok(Success::Done {})
            }
            Command::PurgeQueue { queue } => {
                self.queue_mut(queue)?.purge();
                Ok(Success::Done {})
            }
            Command::Tick { seconds } => {
                self.now = self.now.after(seconds.seconds());
                for queue in self.queues.values_mut() {
                    queue.advance_to(self.now);
                }
                Ok(Success::Now { now: self.now })
            }
            Command::SendMessage {
                queue,
                body,
                delay,
                group,
                deduplication,
            } => {
                let queue = self
                    .queues
                    .get_mut(queue)
                    .ok_or_else(|| Error::no_such_queue(queue))?;
                let sent = queue.send(
                    self.next_message_id,
                    self.now,
                    body,
                    delay,
                    group,
                    deduplication,
                )?;
                if sent.accepted {
                    self.next_message_id.take_next();
                }
                Ok(Success::Sent {
                    message_id: sent.message_id,
                    sequence_number: sent.sequence,
                })
            }
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(queue, max_messages, visibility_timeout),
            Command::DeleteMessage { queue, receipt } => {
                let handle = self.issued_handle(queue, receipt)?;
                let now = self.now;
                self.queue_mut(queue)?.delete(handle, now)?;
                Ok(Success::Done {})
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt,
                visibility_timeout,
            } => {
                let handle = self.issued_handle(queue, receipt)?;
                let now = self.now;
                self.queue_mut(queue)?
                    .change_visibility(handle, now, visibility_timeout)?;
                Ok(Success::Done {})
            }
        }
    }

    fn queue(&self, name: &str) -> Result<&Queue, Error> {
        self.queues
            .get(name)
            .ok_or_else(|| Error::no_such_queue(name))
    }

    fn queue_mut(&mut self, name: &str) -> Result<&mut Queue, Error> {
        self.queues
            .get_mut(name)
            .ok_or_else(|| Error::no_such_queue(name))
    }

    /// Looks the queue up, then checks that `receipt` is a handle this process has issued.
    fn issued_handle(&self, queue: &str, receipt: &str) -> Result<ReceiptHandle, Error> {
        self.queue(queue)?;
        ReceiptHandle::parse(receipt)
            .filter(|handle| *handle < self.next_receipt_handle)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("{receipt:?} is not a receipt handle that was issued"),
                )
            })
    }

    /// The queue named as dead-letter queue, which must exist and be of the given type.
    fn dead_letter_target(&self, name: &str, fifo: bool) -> Result<&QueueName, Error> {
        let (key, target) = self
            .queues
            .get_key_value(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        if target.is_fifo() != fifo {
            return Err(Error::invalid_value(
                "DeadLetterQueue",
                "the dead-letter queue must be of the same type as the queue",
            ));
        }
        Ok(key)
    }

    fn create_queue(
        &mut self,
        name: QueueName,
        attributes: &Attributes<'_>,
    ) -> Result<Success<'_>, Error> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "the name must end in .fifo exactly when Fifo is true",
            ));
        }
        let content_based_deduplication = attributes.content_based_deduplication.unwrap_or(false);
        if content_based_deduplication && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                "only FIFO queues support content-based deduplication",
            ));
        }
        let dead_letter = dead_letter_of(attributes)?;
        let requested = Requested {
            visibility_timeout: attributes
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay: attributes.delay.unwrap_or(DEFAULT_DELAY),
            retention: attributes.retention.unwrap_or(DEFAULT_RETENTION),
            queue_type: if fifo {
                QueueType::Fifo {
                    content_based_deduplication,
                }
            } else {
                QueueType::Standard
            },
            dead_letter,
        };
        if let Some(existing) = self.queues.get(name.as_str()) {
            return if existing.matches(&requested) {
                Ok(Success::Done {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = dead_letter
            .map(|request| {
                let target = self.dead_letter_target(request.queue, fifo)?;
                Ok(DeadLetter {
                    queue: target.clone(),
                    max_receive_count: request.max_receive_count,
                })
            })
            .transpose()?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("{MAX_QUEUES} queues already exist"),
            ));
        }
        self.queues
            .insert(name, Queue::new(&requested, dead_letter));
        Ok(Success::Done {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        attributes: &Attributes<'_>,
    ) -> Result<Success<'_>, Error> {
        let fifo = self.queue(name)?.is_fifo();
        if attributes.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                "only FIFO queues support content-based deduplication",
            ));
        }
        let dead_letter = dead_letter_of(attributes)?
            .map(|request| {
                let target = self.dead_letter_target(request.queue, fifo)?;
                if target.as_str() == name {
                    return Err(Error::invalid_value(
                        "DeadLetterQueue",
                        "a queue cannot be its own dead-letter queue",
                    ));
                }
                Ok(DeadLetter {
                    queue: target.clone(),
                    max_receive_count: request.max_receive_count,
                })
            })
            .transpose()?;
        let now = self.now;
        self.queue_mut(name)?.update(
            Update {
                visibility_timeout: attributes.visibility_timeout,
                delay: attributes.delay,
                retention: attributes.retention,
                content_based_deduplication: attributes.content_based_deduplication,
                dead_letter,
            },
            now,
        );
        Ok(Success::Done {})
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: Option<MaxMessages>,
        visibility_timeout: Option<VisibilityTimeout>,
    ) -> Result<Success<'_>, Error> {
        let now = self.now;
        let queue = self
            .queues
            .get_mut(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        let timeout = visibility_timeout.unwrap_or_else(|| queue.visibility_timeout());
        let received = queue.receive(
            now,
            max_messages.unwrap_or(MaxMessages::constant(1)),
            timeout,
            &mut self.next_receipt_handle,
        );
        if !received.dead.is_empty() {
            let target = queue
                .dead_letter()
                .expect("messages are only given up on when a dead-letter configuration exists")
                .queue
                .clone();
            let dead_letter_queue = self.queues.get_mut(target.as_str()).expect(
                "a dead-letter configuration names an existing queue: DeleteQueue clears it",
            );
            for message in received.dead {
                dead_letter_queue.add_dead(message, now);
            }
        }
        let queue = self
            .queues
            .get(name)
            .expect("the queue was found above and nothing since has removed a queue");
        let messages = received
            .delivered
            .iter()
            .map(|&index| &queue.messages()[index])
            .map(|message| ReceivedMessage {
                message_id: message.id(),
                receipt_handle: message
                    .receipt_handle()
                    .expect("a delivered message holds the handle of its delivery"),
                body: message.body(),
                receive_count: message.receive_count(),
                sent_at: message.sent_at(),
                first_received_at: message
                    .first_received_at()
                    .expect("a delivered message has been received"),
                fifo: message.fifo(),
            })
            .collect();
        Ok(Success::Messages { messages })
    }
}

#[cfg(test)]
mod tests;
