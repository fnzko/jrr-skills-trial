//! All queues, the clock and the identifier counters; executes parsed commands.

use std::collections::BTreeMap;
use std::ops::Bound;

use crate::attributes::{AttributeUpdate, Attributes, DeadLetter, QueueKind};
use crate::command::{Command, SendMessage};
use crate::error::{Code, Error, Result};
use crate::handles::HandleLog;
use crate::queue::Queue;
use crate::response::{Clock, QueueAttributes, QueueNames, ReceivedMessages, Reply};
use crate::types::{DedupId, MessageId, QueueId, QueueName, ReceiptHandle, Seconds, Slot, Time};

const MAX_QUEUES: usize = 1000;

#[derive(Debug)]
pub struct Broker {
    now: Time,
    queues: BTreeMap<QueueName, Queue>,
    next_queue: QueueId,
    next_message: MessageId,
    handles: HandleLog,
}

impl Default for Broker {
    fn default() -> Self {
        Self {
            now: Time::START,
            queues: BTreeMap::new(),
            next_queue: QueueId::FIRST,
            next_message: MessageId::FIRST,
            handles: HandleLog::default(),
        }
    }
}

fn does_not_exist(name: &str) -> Error {
    Error::new(
        Code::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

/// Finds a queue and applies retention, so callers only see live messages.
fn lookup<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    now: Time,
    name: &str,
) -> Result<&'a mut Queue> {
    let queue = queues.get_mut(name).ok_or_else(|| does_not_exist(name))?;
    queue.expire(now);
    Ok(queue)
}

impl Broker {
    pub fn execute(&mut self, command: Command) -> Result<Reply> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                lookup(&mut self.queues, self.now, &queue)?.purge();
                Ok(Reply::Empty {})
            }
            Command::Tick { seconds } => {
                self.now.advance(seconds);
                Ok(Reply::Clock(Clock { now: self.now }))
            }
            Command::SendMessage(request) => self.send_message(request),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => self.delete_message(&queue, &receipt_handle),
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => self.change_visibility(&queue, &receipt_handle, visibility_timeout),
        }
    }

    fn create_queue(&mut self, name: QueueName, update: &AttributeUpdate) -> Result<Reply> {
        let fifo = update.fifo.unwrap_or(false);
        if name.is_fifo() != fifo {
            return Err(Error::new(
                Code::QueueNameInvalid,
                "the .fifo suffix and the Fifo attribute must agree",
            ));
        }
        let kind = self.kind_for(fifo, update.content_based_deduplication)?;
        let dead_letter = update.dead_letter_request()?;
        if let Some(existing) = self.queues.get_mut(name.as_str()) {
            existing.expire(self.now);
            return if existing.attributes().matches(update, kind, dead_letter) {
                Ok(Reply::Empty {})
            } else {
                Err(Error::new(
                    Code::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = self.resolve_dead_letter(dead_letter, fifo)?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(Code::LimitExceeded, "1000 queues exist already"));
        }
        let attributes = Attributes::from_update(update, kind, dead_letter);
        let id = self.next_queue;
        self.next_queue = id.next();
        self.queues
            .insert(name.clone(), Queue::new(id, name, attributes));
        Ok(Reply::Empty {})
    }

    fn kind_for(&self, fifo: bool, content_based: Option<bool>) -> Result<QueueKind> {
        match (fifo, content_based) {
            (true, content_based) => Ok(QueueKind::Fifo {
                content_based_deduplication: content_based.unwrap_or(false),
            }),
            (false, Some(true)) => Err(Error::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            )),
            (false, _) => Ok(QueueKind::Standard),
        }
    }

    /// Checks that the requested dead-letter queue exists and has the given type.
    fn resolve_dead_letter(
        &self,
        request: Option<(&str, u32)>,
        fifo: bool,
    ) -> Result<Option<DeadLetter>> {
        let Some((queue, count)) = request else {
            return Ok(None);
        };
        let target = self
            .queues
            .get(queue)
            .ok_or_else(|| does_not_exist(queue))?;
        if target.attributes().kind.is_fifo() != fifo {
            return Err(Error::invalid_value(
                "the dead-letter queue must be of the same type as the queue",
            ));
        }
        Ok(Some(DeadLetter::new(target.name().clone(), count)))
    }

    fn set_queue_attributes(&mut self, queue: &str, update: &AttributeUpdate) -> Result<Reply> {
        let fifo = lookup(&mut self.queues, self.now, queue)?
            .attributes()
            .kind
            .is_fifo();
        if update.fifo.is_some() {
            return Err(Error::new(
                Code::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        self.kind_for(fifo, update.content_based_deduplication)?;
        let request = update.dead_letter_request()?;
        if let Some((dead_letter_queue, _)) = request {
            if !self.queues.contains_key(dead_letter_queue) {
                return Err(does_not_exist(dead_letter_queue));
            }
            if dead_letter_queue == queue {
                return Err(Error::invalid_value(
                    "a queue cannot be its own dead-letter queue",
                ));
            }
        }
        let dead_letter = self.resolve_dead_letter(request, fifo)?;
        let target = lookup(&mut self.queues, self.now, queue)?;
        target.attributes_mut().apply(update, dead_letter);
        target.expire(self.now);
        Ok(Reply::Empty {})
    }

    fn get_queue_attributes(&mut self, queue: &str) -> Result<Reply> {
        let queue = lookup(&mut self.queues, self.now, queue)?;
        Ok(Reply::QueueAttributes(QueueAttributes::new(
            queue.attributes(),
            queue.counts(self.now),
        )))
    }

    fn list_queues(&self, prefix: &str) -> Reply {
        let names = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name.as_str())
            .take_while(|name| name.starts_with(prefix))
            .map(str::to_owned)
            .collect();
        Reply::Queues(QueueNames { queue_names: names })
    }

    fn delete_queue(&mut self, queue: &str) -> Result<Reply> {
        let removed = self
            .queues
            .remove(queue)
            .ok_or_else(|| does_not_exist(queue))?;
        for other in self.queues.values_mut() {
            other.attributes_mut().clear_dead_letter_to(removed.name());
        }
        Ok(Reply::Empty {})
    }

    fn send_message(&mut self, request: SendMessage) -> Result<Reply> {
        let SendMessage {
            queue,
            body,
            delay,
            group,
            dedup,
        } = request;
        let queue = lookup(&mut self.queues, self.now, &queue)?;
        let now = self.now;
        let fifo = match queue.attributes().kind {
            QueueKind::Standard => {
                if group.is_some() || dedup.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                None
            }
            QueueKind::Fifo {
                content_based_deduplication,
            } => {
                if delay.is_some() {
                    return Err(unsupported("DelaySeconds is not supported on a FIFO queue"));
                }
                let group = group.ok_or_else(|| Error::missing("GroupId"))?;
                let dedup = match dedup {
                    Some(dedup) => dedup,
                    None if content_based_deduplication => DedupId::from_body(&body),
                    None => return Err(Error::missing("DeduplicationId")),
                };
                if let Some(original) = queue.duplicate_of(&dedup, now) {
                    return Ok(Reply::Sent(original.into()));
                }
                Some((group, dedup))
            }
        };
        if queue.is_full() {
            return Err(Error::new(
                Code::LimitExceeded,
                "the queue holds 100000 messages already",
            ));
        }
        let delay = delay.unwrap_or(queue.attributes().delay);
        let id = self.next_message;
        self.next_message = id.next();
        Ok(Reply::Sent(queue.accept(id, body, now, delay, fifo).into()))
    }

    fn receive_message(
        &mut self,
        queue: &str,
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
    ) -> Result<Reply> {
        let now = self.now;
        let source = lookup(&mut self.queues, now, queue)?;
        let visibility_timeout =
            visibility_timeout.unwrap_or(source.attributes().visibility_timeout);
        let dead_letter_queue = source
            .attributes()
            .dead_letter
            .as_ref()
            .map(|d| d.queue().clone());
        let received = source.receive(now, max_messages, visibility_timeout, &mut self.handles);
        if let Some(name) = dead_letter_queue {
            let target = lookup(&mut self.queues, now, name.as_str())
                .expect("a dead-letter queue exists: deleting it clears the configuration");
            for message in received.dead {
                target.admit_dead(message, now);
            }
        }
        Ok(Reply::Received(ReceivedMessages::from(received.deliveries)))
    }

    fn delete_message(&mut self, queue: &str, receipt_handle: &str) -> Result<Reply> {
        let (queue, slot) = self.locate_delivery(queue, receipt_handle)?;
        queue.delete(slot);
        Ok(Reply::Empty {})
    }

    fn change_visibility(
        &mut self,
        queue: &str,
        receipt_handle: &str,
        visibility_timeout: Seconds,
    ) -> Result<Reply> {
        let until = self.now + visibility_timeout;
        let (queue, slot) = self.locate_delivery(queue, receipt_handle)?;
        queue.hold_until(slot, until);
        Ok(Reply::Empty {})
    }

    /// Finds the queue and the slot of the message that `receipt_handle` still refers to.
    fn locate_delivery(&mut self, queue: &str, receipt_handle: &str) -> Result<(&mut Queue, Slot)> {
        let now = self.now;
        let queue = lookup(&mut self.queues, now, queue)?;
        let handle = ReceiptHandle::parse(receipt_handle);
        let (handle, owner) = handle
            .and_then(|handle| Some((handle, self.handles.owner(handle)?)))
            .ok_or_else(|| {
                Error::new(
                    Code::ReceiptHandleInvalid,
                    format!("receipt handle {receipt_handle:?} was never issued"),
                )
            })?;
        if !queue.holds_in_flight(owner, handle, now) {
            return Err(Error::new(
                Code::ReceiptHandleExpired,
                format!("receipt handle {receipt_handle:?} is not in use any more"),
            ));
        }
        Ok((queue, owner.slot))
    }
}

fn unsupported(message: &str) -> Error {
    Error::new(Code::UnsupportedOperation, message)
}
