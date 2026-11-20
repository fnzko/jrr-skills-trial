//! The whole process state and the execution of commands against it.

use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;

use crate::attributes::{AttributePatch, Attributes};
use crate::error::{Error, ErrorCode};
use crate::ids::{QueueName, ReceiptHandle};
use crate::queue::{Queue, QueueId};
use crate::reply::Reply;
use crate::request::Command;

const MAX_QUEUES: usize = 1000;

type Queues = BTreeMap<QueueName, Queue>;

/// Where a receipt handle was issued.
struct HandleOrigin {
    queue: QueueId,
    sequence_number: u64,
}

#[derive(Default)]
pub struct Service {
    now: u64,
    queues: Queues,
    queue_ids: QueueId,
    message_ids: u64,
    handles: HashMap<ReceiptHandle, HandleOrigin>,
    handles_issued: u64,
}

/// Finds a queue by exact name and drops its expired messages.
fn lookup<'a>(queues: &'a mut Queues, name: &str, now: u64) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| {
        Error::new(
            ErrorCode::QueueDoesNotExist,
            format!("queue {name:?} does not exist"),
        )
    })?;
    queue.expire(now);
    Ok(queue)
}

fn dead_letter_target<'a>(
    queues: &'a Queues,
    patch: &AttributePatch,
) -> Result<Option<&'a Queue>, Error> {
    let Some(name) = patch.dead_letter_queue.as_deref() else {
        return Ok(None);
    };
    queues.get(name).map(Some).ok_or_else(|| {
        Error::new(
            ErrorCode::QueueDoesNotExist,
            format!("dead-letter queue {name:?} does not exist"),
        )
    })
}

fn partial_dead_letter() -> Error {
    Error::invalid(
        "Attributes",
        "either both or neither of DeadLetterQueue and MaxReceiveCount",
    )
}

impl Service {
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
                let queue = lookup(&mut self.queues, &queue, self.now)?;
                Ok(Reply::Attributes {
                    attributes: queue.attributes().clone(),
                    counts: queue.counts(self.now),
                })
            }
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                lookup(&mut self.queues, &queue, self.now)?.purge();
                Ok(Reply::Empty {})
            }
            Command::Tick { seconds } => {
                self.now += u64::from(seconds);
                Ok(Reply::Now { now: self.now })
            }
            Command::SendMessage { queue, message } => {
                let sent = lookup(&mut self.queues, &queue, self.now)?.send(
                    &message,
                    self.now,
                    &mut self.message_ids,
                )?;
                Ok(Reply::Sent {
                    message_id: sent.message_id,
                    sequence_number: sent.sequence_number,
                })
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
                let now = self.now;
                let (queue, handle, sequence_number) =
                    self.resolve_handle(&queue, &receipt_handle)?;
                queue.delete_in_flight(sequence_number, handle, now)?;
                Ok(Reply::Empty {})
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let now = self.now;
                let (queue, handle, sequence_number) =
                    self.resolve_handle(&queue, &receipt_handle)?;
                queue.change_visibility(sequence_number, handle, visibility_timeout, now)?;
                Ok(Reply::Empty {})
            }
        }
    }

    fn create_queue(&mut self, name: QueueName, patch: &AttributePatch) -> Result<Reply, Error> {
        let fifo = patch.fifo.unwrap_or(false);
        if name.is_fifo_name() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue is FIFO exactly when its name ends in .fifo",
            ));
        }
        if patch.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid(
                "ContentBasedDeduplication",
                "false on a standard queue",
            ));
        }
        if patch.dead_letter_is_partial() {
            return Err(partial_dead_letter());
        }
        let attributes = Attributes::from_patch(patch);
        if let Some(existing) = self.queues.get(name.as_str()) {
            return if *existing.attributes() == attributes {
                Ok(Reply::Empty {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    "a queue with this name exists with different attributes",
                ))
            };
        }
        if let Some(target) = dead_letter_target(&self.queues, patch)?
            && target.attributes().fifo != fifo
        {
            return Err(Error::invalid(
                "DeadLetterQueue",
                "a queue of the same type as this one",
            ));
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "1000 queues already exist",
            ));
        }
        self.queue_ids += 1;
        self.queues
            .insert(name, Queue::new(self.queue_ids, attributes));
        Ok(Reply::Empty {})
    }

    fn set_queue_attributes(&mut self, name: &str, patch: &AttributePatch) -> Result<Reply, Error> {
        let queue = lookup(&mut self.queues, name, self.now)?;
        let fifo = queue.attributes().fifo;
        if patch.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if patch.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid(
                "ContentBasedDeduplication",
                "false on a standard queue",
            ));
        }
        if patch.dead_letter_is_partial() {
            return Err(partial_dead_letter());
        }
        if let Some(target) = dead_letter_target(&self.queues, patch)? {
            let own_name = patch.dead_letter_queue.as_deref() == Some(name);
            if own_name || target.attributes().fifo != fifo {
                return Err(Error::invalid(
                    "DeadLetterQueue",
                    "another queue of the same type as this one",
                ));
            }
        }
        lookup(&mut self.queues, name, self.now)?.apply_attributes(patch, self.now);
        Ok(Reply::Empty {})
    }

    fn list_queues(&self, prefix: &str) -> Reply {
        let from = (Bound::Included(prefix), Bound::Unbounded);
        let queue_names = self
            .queues
            .range::<str, _>(from)
            .map(|(name, _)| name.as_str())
            .take_while(|name| name.starts_with(prefix))
            .map(str::to_owned)
            .collect();
        Reply::QueueNames { queue_names }
    }

    fn delete_queue(&mut self, name: &str) -> Result<Reply, Error> {
        lookup(&mut self.queues, name, self.now)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            let points_here = queue
                .attributes()
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue == name);
            if points_here {
                queue.clear_dead_letter();
            }
        }
        Ok(Reply::Empty {})
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: usize,
        visibility_timeout: Option<u32>,
    ) -> Result<Reply, Error> {
        let queue = lookup(&mut self.queues, name, self.now)?;
        let queue_id = queue.id();
        let dead_letter_queue = queue
            .attributes()
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.queue.clone());
        let received = queue.receive(
            self.now,
            max_messages,
            visibility_timeout,
            &mut self.handles_issued,
        );
        if !received.dead.is_empty() {
            let target = dead_letter_queue
                .as_deref()
                .and_then(|name| self.queues.get_mut(name))
                .expect("dead-lettering needs a configuration, and deleting its queue clears it");
            for message in received.dead {
                target.accept_dead(message, self.now);
            }
        }
        let mut messages = Vec::with_capacity(received.deliveries.len());
        for delivery in received.deliveries {
            self.handles.insert(
                delivery.message.receipt_handle,
                HandleOrigin {
                    queue: queue_id,
                    sequence_number: delivery.sequence_number,
                },
            );
            messages.push(delivery.message);
        }
        Ok(Reply::Messages { messages })
    }

    /// Checks a handle string against what this process has issued and finds
    /// the queue and message it was issued for.
    fn resolve_handle(
        &mut self,
        queue: &str,
        text: &str,
    ) -> Result<(&mut Queue, ReceiptHandle, u64), Error> {
        let queue = lookup(&mut self.queues, queue, self.now)?;
        let handle = ReceiptHandle::parse(text, self.handles_issued).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("receipt handle {text:?} was never issued"),
            )
        })?;
        let origin = self
            .handles
            .get(&handle)
            .expect("every handle up to handles_issued was registered when it was issued");
        if origin.queue != queue.id() {
            return Err(Error::new(
                ErrorCode::ReceiptHandleExpired,
                format!("receipt handle {handle} was issued for another queue"),
            ));
        }
        let sequence_number = origin.sequence_number;
        Ok((queue, handle, sequence_number))
    }
}
