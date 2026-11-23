//! The broker: the virtual clock and every queue.

use std::collections::BTreeMap;
use std::ops::Bound;

use crate::attributes::{AttributeUpdate, Attributes, DeadLetter, content_based_on_standard};
use crate::error::{Error, ErrorCode, Result};
use crate::queue::{NewMessage, Queue};
use crate::request::Request;
use crate::response::Reply;
use crate::types::{
    MaxMessages, MaxReceiveCount, MessageId, QueueName, ReceiptHandle, Time, VisibilityTimeout,
};

const MAX_QUEUES: usize = 1000;

#[derive(Debug)]
pub struct Broker {
    now: Time,
    queues: BTreeMap<QueueName, Queue>,
    next_message_id: MessageId,
    next_handle: ReceiptHandle,
}

fn no_such_queue(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue `{name}` does not exist"),
    )
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}

impl Broker {
    pub fn new() -> Self {
        Self {
            now: Time::default(),
            queues: BTreeMap::new(),
            next_message_id: MessageId::FIRST,
            next_handle: ReceiptHandle::FIRST,
        }
    }

    /// A failed request leaves the broker unchanged.
    pub fn execute(&mut self, request: Request) -> Result<Reply> {
        match request {
            Request::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Request::GetQueueAttributes { queue } => {
                let queue = self.queue(&queue)?;
                Ok(Reply::QueueAttributes {
                    attributes: queue.attributes().clone(),
                    counts: queue.counts(),
                })
            }
            Request::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Request::DeleteQueue { queue } => self.delete_queue(&queue),
            Request::PurgeQueue { queue } => {
                self.queue_mut(&queue)?.purge();
                Ok(Reply::Done {})
            }
            Request::SendMessage { queue, message } => self.send_message(&queue, message),
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Request::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let handle = self.issued_handle(&queue, &receipt_handle)?;
                self.queue_mut(&queue)?.delete(handle)?;
                Ok(Reply::Done {})
            }
            Request::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let handle = self.issued_handle(&queue, &receipt_handle)?;
                let now = self.now;
                self.queue_mut(&queue)?
                    .change_visibility(now, handle, visibility_timeout)?;
                Ok(Reply::Done {})
            }
            Request::Tick { by } => {
                self.now = self.now.after(by);
                for queue in self.queues.values_mut() {
                    queue.advance(self.now);
                }
                Ok(Reply::Now { now: self.now })
            }
        }
    }

    fn queue(&self, name: &str) -> Result<&Queue> {
        self.queues.get(name).ok_or_else(|| no_such_queue(name))
    }

    fn queue_mut(&mut self, name: &str) -> Result<&mut Queue> {
        self.queues.get_mut(name).ok_or_else(|| no_such_queue(name))
    }

    /// The dead-letter configuration naming `queue`, if that queue exists.
    fn dead_letter(&self, queue: &str, max_receive_count: MaxReceiveCount) -> Option<DeadLetter> {
        self.queues
            .get_key_value(queue)
            .map(|(name, _)| DeadLetter {
                queue: name.clone(),
                max_receive_count,
            })
    }

    fn create_queue(&mut self, name: QueueName, update: &AttributeUpdate) -> Result<Reply> {
        if name.is_fifo() != update.is_fifo() {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in `.fifo` exactly when `Fifo` is true",
            ));
        }
        let kind = update.kind_for_new_queue()?;
        let requested = update.dead_letter_request()?;
        let dead_letter = requested.and_then(|(queue, max)| self.dead_letter(queue, max));
        if let Some(existing) = self.queues.get(&name) {
            // A dead-letter queue that does not exist cannot match an existing configuration.
            let same = requested.is_some() == dead_letter.is_some()
                && *existing.attributes() == Attributes::resolve(update, kind, dead_letter);
            return if same {
                Ok(Reply::Done {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue `{}` exists with different attributes", name.as_str()),
                ))
            };
        }
        if let Some((queue, _)) = requested {
            let Some(dead_letter) = &dead_letter else {
                return Err(no_such_queue(queue));
            };
            if dead_letter.queue.is_fifo() != name.is_fifo() {
                return Err(invalid(
                    "the dead-letter queue must have the same `Fifo` value",
                ));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        let attributes = Attributes::resolve(update, kind, dead_letter);
        self.queues.insert(name, Queue::new(attributes));
        Ok(Reply::Done {})
    }

    fn set_queue_attributes(&mut self, queue: &str, update: &AttributeUpdate) -> Result<Reply> {
        let fifo = self.queue(queue)?.attributes().kind.is_fifo();
        if update.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if update.content_based_deduplication == Some(true) && !fifo {
            return Err(content_based_on_standard());
        }
        let requested = update.dead_letter_request()?;
        let dead_letter = requested
            .map(|(name, max)| {
                self.dead_letter(name, max)
                    .ok_or_else(|| no_such_queue(name))
            })
            .transpose()?;
        if let Some(dead_letter) = &dead_letter
            && (dead_letter.queue.as_str() == queue || dead_letter.queue.is_fifo() != fifo)
        {
            return Err(invalid(
                "the dead-letter queue must be another queue with the same `Fifo` value",
            ));
        }
        let now = self.now;
        self.queue_mut(queue)?
            .set_attributes(update, dead_letter, now);
        Ok(Reply::Done {})
    }

    fn send_message(&mut self, queue: &str, message: NewMessage) -> Result<Reply> {
        let (now, id) = (self.now, self.next_message_id);
        let sent = self.queue_mut(queue)?.send(now, id, message)?;
        if !sent.duplicate {
            self.next_message_id = id.next();
        }
        Ok(Reply::Sent {
            message_id: sent.message_id,
            sequence_number: sent.sequence,
        })
    }

    /// Checks the queue first, then that `text` is a handle this process has issued so far.
    fn issued_handle(&self, queue: &str, text: &str) -> Result<ReceiptHandle> {
        self.queue(queue)?;
        ReceiptHandle::parse(text)
            .filter(|handle| *handle < self.next_handle)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("`{text}` was never issued as a receipt handle"),
                )
            })
    }

    fn receive_message(
        &mut self,
        queue: &str,
        max_messages: MaxMessages,
        visibility_timeout: Option<VisibilityTimeout>,
    ) -> Result<Reply> {
        let now = self.now;
        let source = self
            .queues
            .get_mut(queue)
            .ok_or_else(|| no_such_queue(queue))?;
        let visibility_timeout =
            visibility_timeout.unwrap_or(source.attributes().visibility_timeout);
        let received = source.receive(now, max_messages, visibility_timeout, &mut self.next_handle);
        if let Some(dead_lettered) = received.dead_lettered {
            let target = self
                .queues
                .get_mut(&dead_lettered.target)
                .expect("a dead-letter configuration always names an existing queue");
            for message in dead_lettered.messages {
                target.accept_dead_letter(now, message);
            }
        }
        Ok(Reply::Messages {
            messages: received.deliveries,
        })
    }

    fn list_queues(&self, prefix: &str) -> Reply {
        let queue_names = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name)
            .take_while(|name| name.as_str().starts_with(prefix))
            .cloned()
            .collect();
        Reply::QueueNames { queue_names }
    }

    fn delete_queue(&mut self, queue: &str) -> Result<Reply> {
        self.queues
            .remove(queue)
            .ok_or_else(|| no_such_queue(queue))?;
        for other in self.queues.values_mut() {
            other.forget_dead_letter(queue);
        }
        Ok(Reply::Done {})
    }
}
