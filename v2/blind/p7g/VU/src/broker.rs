//! The service state: every queue, the clock and the identifier counters, and the
//! command rules that span more than one queue.

use std::collections::BTreeMap;

use crate::error::{Error, ErrorCode};
use crate::queue::{DeadLetterConfig, FifoSubmission, Queue, QueueConfig, QueueKind, Submission};
use crate::request::{AttributeSet, DeadLetterRequest, ReceiveMessage, Request, SendMessage};
use crate::response::{QueueAttributes, Reply};
use crate::types::{IdAllocator, QueueName, ReceiptHandle, Timestamp};

const MAX_QUEUES: usize = 1000;

#[derive(Debug, Default)]
pub struct Broker {
    now: Timestamp,
    ids: IdAllocator,
    queues: BTreeMap<QueueName, Queue>,
}

/// The queue named `name`, with expired messages already removed.
fn live_queue<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    name: &QueueName,
    now: Timestamp,
) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| does_not_exist(name))?;
    queue.expire(now);
    Ok(queue)
}

fn does_not_exist(name: &QueueName) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {:?} does not exist", name.as_str()),
    )
}

impl DeadLetterRequest {
    /// `None` when no dead-letter attribute was given.
    fn into_config(self) -> Result<Option<DeadLetterConfig>, Error> {
        match self {
            Self::Unset => Ok(None),
            Self::Complete {
                queue,
                max_receive_count,
            } => Ok(Some(DeadLetterConfig {
                queue,
                max_receive_count,
            })),
            Self::Incomplete => Err(Error::invalid_value(
                "Attributes",
                "naming DeadLetterQueue and MaxReceiveCount together or neither",
            )),
        }
    }
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn execute(&mut self, request: Request) -> Result<Reply, Error> {
        match request {
            Request::CreateQueue { name, attributes } => self.create_queue(name, attributes),
            Request::SetQueueAttributes { name, attributes } => {
                self.set_queue_attributes(&name, attributes)
            }
            Request::GetQueueAttributes { name } => self.get_queue_attributes(&name),
            Request::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref())),
            Request::DeleteQueue { name } => self.delete_queue(&name),
            Request::PurgeQueue { name } => {
                live_queue(&mut self.queues, &name, self.now)?.purge();
                Ok(Reply::Empty {})
            }
            Request::Tick { seconds } => {
                self.now = self.now.advance(seconds);
                Ok(Reply::Now { now: self.now })
            }
            Request::SendMessage(send) => self.send_message(send),
            Request::ReceiveMessage(receive) => self.receive_message(receive),
            Request::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let handle = self.issued_handle(&queue, &receipt_handle)?;
                live_queue(&mut self.queues, &queue, self.now)?
                    .delete_in_flight(handle, self.now)
                    .ok_or_else(|| expired(&receipt_handle))?;
                Ok(Reply::Empty {})
            }
            Request::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let handle = self.issued_handle(&queue, &receipt_handle)?;
                live_queue(&mut self.queues, &queue, self.now)?
                    .change_visibility(handle, self.now, visibility_timeout)
                    .ok_or_else(|| expired(&receipt_handle))?;
                Ok(Reply::Empty {})
            }
        }
    }

    fn create_queue(&mut self, name: QueueName, attributes: AttributeSet) -> Result<Reply, Error> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a name ends in .fifo exactly when the queue is FIFO",
            ));
        }
        let content_based_deduplication = attributes.content_based_deduplication.unwrap_or(false);
        if content_based_deduplication && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                "false on a standard queue",
            ));
        }
        let config = QueueConfig {
            visibility_timeout: attributes.visibility_timeout.unwrap_or_default(),
            delay: attributes.delay.unwrap_or_default(),
            retention: attributes.retention.unwrap_or_default(),
            kind: if fifo {
                QueueKind::Fifo {
                    content_based_deduplication,
                }
            } else {
                QueueKind::Standard
            },
            dead_letter: attributes.dead_letter.into_config()?,
        };
        if let Some(existing) = self.queues.get(&name) {
            return if *existing.config() == config {
                Ok(Reply::Empty {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }
        if let Some(dead_letter) = &config.dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| does_not_exist(&dead_letter.queue))?;
            if target.config().kind.is_fifo() != fifo {
                return Err(Error::invalid_value(
                    "DeadLetterQueue",
                    "a queue of the same type",
                ));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "the limit of 1000 queues is reached",
            ));
        }
        self.queues.insert(name, Queue::new(config));
        Ok(Reply::Empty {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &QueueName,
        attributes: AttributeSet,
    ) -> Result<Reply, Error> {
        let mut config = live_queue(&mut self.queues, name, self.now)?
            .config()
            .clone();
        if attributes.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue never changes",
            ));
        }
        let fifo = config.kind.is_fifo();
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                "false on a standard queue",
            ));
        }
        if let Some(dead_letter) = attributes.dead_letter.into_config()? {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| does_not_exist(&dead_letter.queue))?;
            if dead_letter.queue == *name || target.config().kind.is_fifo() != fifo {
                return Err(Error::invalid_value(
                    "DeadLetterQueue",
                    "another queue of the same type",
                ));
            }
            config.dead_letter = Some(dead_letter);
        }
        if let Some(value) = attributes.visibility_timeout {
            config.visibility_timeout = value;
        }
        if let Some(value) = attributes.delay {
            config.delay = value;
        }
        if let Some(value) = attributes.retention {
            config.retention = value;
        }
        if let (
            Some(value),
            QueueKind::Fifo {
                content_based_deduplication,
            },
        ) = (attributes.content_based_deduplication, &mut config.kind)
        {
            *content_based_deduplication = value;
        }
        live_queue(&mut self.queues, name, self.now)?.reconfigure(self.now, config);
        Ok(Reply::Empty {})
    }

    fn get_queue_attributes(&mut self, name: &QueueName) -> Result<Reply, Error> {
        let queue = live_queue(&mut self.queues, name, self.now)?;
        Ok(Reply::Attributes {
            attributes: QueueAttributes::from(queue.config()),
            counts: queue.counts(self.now),
        })
    }

    fn list_queues(&self, prefix: Option<&str>) -> Reply {
        let prefix = prefix.unwrap_or_default();
        Reply::Queues {
            queue_names: self
                .queues
                .keys()
                .filter(|name| name.as_str().starts_with(prefix))
                .cloned()
                .collect(),
        }
    }

    fn delete_queue(&mut self, name: &QueueName) -> Result<Reply, Error> {
        self.queues
            .remove(name)
            .ok_or_else(|| does_not_exist(name))?;
        for queue in self.queues.values_mut() {
            queue.drop_dead_letter_to(name);
        }
        Ok(Reply::Empty {})
    }

    fn send_message(&mut self, send: SendMessage) -> Result<Reply, Error> {
        let now = self.now;
        let queue = live_queue(&mut self.queues, &send.queue, now)?;
        let fifo = match queue.config().kind {
            QueueKind::Standard => {
                if send.group.is_some() || send.deduplication_id.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                None
            }
            QueueKind::Fifo {
                content_based_deduplication,
            } => {
                if send.delay.is_some() {
                    return Err(unsupported("a FIFO queue has no per-message delay"));
                }
                let group = send.group.ok_or_else(|| Error::missing("GroupId"))?;
                let key = match (&send.deduplication_id, content_based_deduplication) {
                    (Some(id), _) => id.as_str(),
                    (None, true) => send.body.as_str(),
                    (None, false) => return Err(Error::missing("DeduplicationId")),
                };
                if let Some((message_id, sequence)) = queue.find_duplicate(key, now) {
                    return Ok(Reply::Sent {
                        message_id,
                        sequence_number: Some(sequence),
                    });
                }
                Some(FifoSubmission {
                    group,
                    deduplication_key: key.to_owned(),
                })
            }
        };
        if queue.is_full() {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "the queue already holds 100000 messages",
            ));
        }
        let message_id = self.ids.next_message();
        let sequence_number = queue.accept(Submission {
            id: message_id,
            delay: send.delay.unwrap_or(queue.config().delay),
            body: send.body,
            now,
            fifo,
        });
        Ok(Reply::Sent {
            message_id,
            sequence_number,
        })
    }

    fn receive_message(&mut self, receive: ReceiveMessage) -> Result<Reply, Error> {
        let now = self.now;
        let queue = live_queue(&mut self.queues, &receive.queue, now)?;
        let visibility = receive
            .visibility_timeout
            .unwrap_or(queue.config().visibility_timeout);
        let dead_letter_queue = queue
            .config()
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.queue.clone());
        let received = queue.receive(now, receive.max_messages, visibility, &mut self.ids);
        if let Some(name) = dead_letter_queue.filter(|_| !received.dead_lettered.is_empty()) {
            let target = self.queues.get_mut(&name).expect(
                "a configured dead-letter queue exists: deleting a queue unsets configurations naming it",
            );
            for message in received.dead_lettered {
                target.accept_dead_letter(message, now);
            }
        }
        Ok(Reply::Received {
            messages: received.delivered,
        })
    }

    /// The handle `text` spells, if it was issued, after checking the queue exists.
    fn issued_handle(&self, queue: &QueueName, text: &str) -> Result<ReceiptHandle, Error> {
        if !self.queues.contains_key(queue) {
            return Err(does_not_exist(queue));
        }
        self.ids.issued_handle(text).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("receipt handle {text:?} was never issued"),
            )
        })
    }
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

fn expired(text: &str) -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        format!("receipt handle {text:?} is not the latest handle of a message in flight here"),
    )
}
