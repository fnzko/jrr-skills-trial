//! The whole service state: the clock, the queues and the id counters.

use std::collections::BTreeMap;
use std::ops::Bound;

use crate::attributes::{AttributePatch, Attributes};
use crate::command::{Command, SendRequest};
use crate::error::{Error, ErrorCode};
use crate::names::{MessageId, QueueName, ReceiptHandle};
use crate::queue::Queue;
use crate::reply::{AttributesView, Clock, Description, Done, QueueList, Received, Reply, Sent};

const QUEUE_LIMIT: usize = 1000;

#[derive(Debug)]
pub struct Broker {
    now: u64,
    queues: BTreeMap<QueueName, Queue>,
    next_message: MessageId,
    next_handle: ReceiptHandle,
}

impl Default for Broker {
    fn default() -> Self {
        Self {
            now: 0,
            queues: BTreeMap::new(),
            next_message: MessageId::FIRST,
            next_handle: ReceiptHandle::FIRST,
        }
    }
}

/// Finds a queue and applies retention, so callers see only live messages.
fn live_queue<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    name: &str,
    now: u64,
) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| does_not_exist(name))?;
    queue.expire(now);
    Ok(queue)
}

fn does_not_exist(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

/// Parses a handle and checks that this process has issued it, that is, that
/// it is older than `next`.
fn issued_handle(text: &str, next: ReceiptHandle) -> Result<ReceiptHandle, Error> {
    ReceiptHandle::parse(text)
        .filter(|handle| *handle < next)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("{text:?} was never issued"),
            )
        })
}

fn invalid_dead_letter(message: &str) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

impl Broker {
    pub fn execute(&mut self, command: Command) -> Result<Reply, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                live_queue(&mut self.queues, &queue, self.now)?.purge();
                Ok(Reply::Done(Done {}))
            }
            Command::Tick { seconds } => {
                self.now = self.now.saturating_add(seconds);
                Ok(Reply::Clock(Clock { now: self.now }))
            }
            Command::SendMessage { queue, request } => self.send_message(&queue, request),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage { queue, handle } => {
                let queue = live_queue(&mut self.queues, &queue, self.now)?;
                queue.delete(self.now, issued_handle(&handle, self.next_handle)?)?;
                Ok(Reply::Done(Done {}))
            }
            Command::ChangeMessageVisibility {
                queue,
                handle,
                visibility_timeout,
            } => {
                let queue = live_queue(&mut self.queues, &queue, self.now)?;
                let handle = issued_handle(&handle, self.next_handle)?;
                queue.change_visibility(self.now, handle, visibility_timeout)?;
                Ok(Reply::Done(Done {}))
            }
        }
    }

    fn create_queue(&mut self, name: QueueName, patch: &AttributePatch) -> Result<Reply, Error> {
        let requested = Attributes::for_new_queue(&name, patch)?;
        if let Some(existing) = self.queues.get(name.as_str()) {
            return if *existing.attributes() == requested {
                Ok(Reply::Done(Done {}))
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with other attributes", name.as_str()),
                ))
            };
        }
        if let Some(dead_letter) = &requested.dead_letter {
            let target = self
                .queues
                .get(dead_letter.queue.as_str())
                .ok_or_else(|| does_not_exist(&dead_letter.queue))?;
            if target.attributes().is_fifo() != requested.is_fifo() {
                return Err(invalid_dead_letter(
                    "the dead-letter queue must be of the same type",
                ));
            }
        }
        if self.queues.len() >= QUEUE_LIMIT {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "the maximum number of queues exists",
            ));
        }
        self.queues.insert(name, Queue::new(requested));
        Ok(Reply::Done(Done {}))
    }

    fn set_queue_attributes(&mut self, name: &str, patch: &AttributePatch) -> Result<Reply, Error> {
        let updated = live_queue(&mut self.queues, name, self.now)?
            .attributes()
            .updated(patch)?;
        if let Some(target_name) = &patch.dead_letter_queue {
            let target = self
                .queues
                .get(target_name.as_str())
                .ok_or_else(|| does_not_exist(target_name))?;
            if target_name == name || target.attributes().is_fifo() != updated.is_fifo() {
                return Err(invalid_dead_letter(
                    "the dead-letter queue must be another queue of the same type",
                ));
            }
        }
        live_queue(&mut self.queues, name, self.now)?.set_attributes(updated, self.now);
        Ok(Reply::Done(Done {}))
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Reply, Error> {
        let queue = live_queue(&mut self.queues, name, self.now)?;
        Ok(Reply::Description(Description {
            attributes: AttributesView::from(queue.attributes()),
            counts: queue.counts(self.now),
        }))
    }

    fn list_queues(&self, prefix: &str) -> Reply {
        let queue_names = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name.as_str())
            .take_while(|name| name.starts_with(prefix))
            .map(str::to_owned)
            .collect();
        Reply::Queues(QueueList { queue_names })
    }

    fn delete_queue(&mut self, name: &str) -> Result<Reply, Error> {
        self.queues
            .remove(name)
            .ok_or_else(|| does_not_exist(name))?;
        for queue in self.queues.values_mut() {
            queue.forget_dead_letter(name);
        }
        Ok(Reply::Done(Done {}))
    }

    fn send_message(&mut self, name: &str, request: SendRequest) -> Result<Reply, Error> {
        let queue = live_queue(&mut self.queues, name, self.now)?;
        let sent = queue.send(self.now, self.next_message, request)?;
        if !sent.duplicate {
            self.next_message.take();
        }
        Ok(Reply::Sent(Sent {
            message_id: sent.id,
            sequence_number: sent.sequence,
        }))
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u64,
        visibility_timeout: Option<u64>,
    ) -> Result<Reply, Error> {
        let queue = live_queue(&mut self.queues, name, self.now)?;
        let received = queue.receive(
            self.now,
            max_messages,
            visibility_timeout,
            &mut self.next_handle,
        );
        if let Some(batch) = received.dead {
            self.queues
                .get_mut(batch.queue.as_str())
                .expect("deleting a queue clears the dead-letter references to it")
                .accept_dead_letters(batch.messages, self.now);
        }
        Ok(Reply::Received(Received {
            messages: received.delivered,
        }))
    }
}
