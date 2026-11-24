//! The state of the whole process: the clock, every queue and the identifier counters.

use std::collections::BTreeMap;
use std::ops::Bound;

use serde::Serialize;

use crate::attributes::{AttributePatch, Attributes, DeadLetter};
use crate::error::{Error, ErrorCode};
use crate::queue::{Admission, Counts, Delivered, Outgoing, Queue, Sent};
use crate::request::Command;
use crate::types::{Handles, MessageId, QueueName, ReceiptHandle, Seconds, SequenceNumber};

const MAX_QUEUES: usize = 1000;

/// The result members of a successful command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Reply {
    Empty {},
    Now {
        now: Seconds,
    },
    Attributes {
        attributes: Attributes,
        counts: Counts,
    },
    QueueNames {
        queue_names: Vec<String>,
    },
    Sent {
        message_id: MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence_number: Option<SequenceNumber>,
    },
    Messages {
        messages: Vec<Delivered>,
    },
}

#[derive(Debug, Default)]
pub struct Broker {
    now: Seconds,
    queues: BTreeMap<QueueName, Queue>,
    messages_accepted: u64,
    handles: Handles,
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn execute(&mut self, command: Command) -> Result<Reply, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => {
                let now = self.now;
                let queue = self.queue(&queue)?;
                Ok(Reply::Attributes {
                    attributes: queue.attributes().clone(),
                    counts: queue.counts(now),
                })
            }
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                self.queue(&queue)?.purge();
                Ok(Reply::Empty {})
            }
            Command::Tick { seconds } => {
                self.now = self.now + seconds;
                Ok(Reply::Now { now: self.now })
            }
            Command::SendMessage {
                queue,
                body,
                delay,
                group,
                deduplication_id,
            } => self.send_message(
                &queue,
                Outgoing {
                    body,
                    delay,
                    group,
                    deduplication_id,
                },
            ),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let now = self.now;
                let handle = self.issued_handle(&queue, &receipt_handle)?;
                self.queue(&queue)?.delete(handle, now)?;
                Ok(Reply::Empty {})
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let now = self.now;
                let handle = self.issued_handle(&queue, &receipt_handle)?;
                self.queue(&queue)?
                    .change_visibility(handle, now, visibility_timeout)?;
                Ok(Reply::Empty {})
            }
        }
    }

    /// Finds a queue and removes its expired messages, so the caller sees the
    /// queue as it is at `now`.
    fn queue(&mut self, name: &str) -> Result<&mut Queue, Error> {
        let now = self.now;
        let queue = self
            .queues
            .get_mut(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        queue.refresh(now);
        Ok(queue)
    }

    /// Looks up the queue first, as the spec orders it, then the handle.
    fn issued_handle(&mut self, queue: &str, text: &str) -> Result<ReceiptHandle, Error> {
        self.queue(queue)?;
        ReceiptHandle::parse(text)
            .filter(|handle| self.handles.is_issued(*handle))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("receipt handle {text:?} was never issued"),
                )
            })
    }

    /// Resolves a requested dead-letter configuration to the queue it names.
    fn dead_letter_target(
        &self,
        requested: Option<(&str, u32)>,
    ) -> Result<Option<(DeadLetter, &Queue)>, Error> {
        requested
            .map(|(name, max_receive_count)| {
                let (key, queue) = self
                    .queues
                    .get_key_value(name)
                    .ok_or_else(|| Error::no_such_queue(name))?;
                let dead_letter = DeadLetter {
                    queue: key.clone(),
                    max_receive_count,
                };
                Ok((dead_letter, queue))
            })
            .transpose()
    }

    fn create_queue(&mut self, name: QueueName, patch: &AttributePatch) -> Result<Reply, Error> {
        let fifo = patch.fifo.unwrap_or(false);
        if name.is_fifo() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is FIFO",
            ));
        }
        if patch.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        let requested = patch.dead_letter()?;
        if let Some(existing) = self.queues.get(&name) {
            return if existing.attributes().matches(patch) {
                Ok(Reply::Empty {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {} exists with different attributes", name.as_str()),
                ))
            };
        }
        let target = self.dead_letter_target(requested)?;
        if let Some((_, dead_letter_queue)) = &target
            && dead_letter_queue.attributes().fifo != fifo
        {
            return Err(Error::invalid_value(
                "the dead-letter queue must be of the same type as the queue",
            ));
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        let dead_letter = target.map(|(dead_letter, _)| dead_letter);
        let attributes = Attributes::create(patch, dead_letter);
        self.queues.insert(name, Queue::new(attributes, self.now));
        Ok(Reply::Empty {})
    }

    fn set_queue_attributes(&mut self, name: &str, patch: &AttributePatch) -> Result<Reply, Error> {
        let fifo = self.queue(name)?.attributes().fifo;
        if patch.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot be changed",
            ));
        }
        if patch.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        let requested = patch.dead_letter()?;
        let target = self.dead_letter_target(requested)?;
        if let Some((dead_letter, dead_letter_queue)) = &target
            && (dead_letter.queue.as_str() == name || dead_letter_queue.attributes().fifo != fifo)
        {
            return Err(Error::invalid_value(
                "the dead-letter queue must be another queue of the same type",
            ));
        }
        let dead_letter = target.map(|(dead_letter, _)| dead_letter);
        let now = self.now;
        let queue = self.queue(name)?;
        queue.attributes_mut().update(patch, dead_letter);
        queue.expire(now);
        Ok(Reply::Empty {})
    }

    fn list_queues(&self, prefix: &str) -> Reply {
        let queue_names = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name.as_str())
            .take_while(|name| name.starts_with(prefix))
            .map(str::to_owned)
            .collect();
        Reply::QueueNames { queue_names }
    }

    fn delete_queue(&mut self, name: &str) -> Result<Reply, Error> {
        self.queue(name)?;
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
        Ok(Reply::Empty {})
    }

    fn send_message(&mut self, name: &str, outgoing: Outgoing) -> Result<Reply, Error> {
        let now = self.now;
        let id = MessageId(self.messages_accepted + 1);
        let sent = match self.queue(name)?.send(now, id, outgoing)? {
            Admission::Accepted(sent) => {
                self.messages_accepted += 1;
                sent
            }
            Admission::Duplicate(sent) => sent,
        };
        let Sent { id, sequence } = sent;
        Ok(Reply::Sent {
            message_id: id,
            sequence_number: sequence,
        })
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
    ) -> Result<Reply, Error> {
        let now = self.now;
        // Split borrow: the queue lives in `self.queues`, the handles beside it.
        let queue = self
            .queues
            .get_mut(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        queue.refresh(now);
        let dead_letter_queue = queue
            .attributes()
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.queue.clone());
        let received = queue.receive(now, max_messages, visibility_timeout, &mut self.handles);
        if let Some(dead_letter_queue) = dead_letter_queue {
            let destination = self.queues.get_mut(&dead_letter_queue).expect(
                "a configured dead-letter queue exists because DeleteQueue clears references",
            );
            for message in received.dead_letters {
                destination.accept_dead_letter(message, now);
            }
        }
        Ok(Reply::Messages {
            messages: received.delivered,
        })
    }
}
