use std::collections::BTreeMap;
use std::ops::Bound;

use crate::attributes::{Attributes, DeadLetter, Kind, RequestedAttributes};
use crate::error::{Error, ErrorCode};
use crate::queue::{FifoSend, NewMessage, Queue, StaleHandle};
use crate::reply::{self, AttributesView, Reply};
use crate::request::Request;
use crate::types::{
    DeduplicationId, DeduplicationKey, DelaySeconds, GroupId, IdGenerator, MaxMessages,
    MaxReceiveCount, QueueName, ReceiptHandle, TickSeconds, Time, VisibilityTimeout,
};

const MAX_QUEUES: usize = 1000;

/// All state of the service: the virtual clock, the queues and the id counters.
#[derive(Debug)]
pub struct Miniq {
    now: Time,
    queues: BTreeMap<QueueName, Queue>,
    ids: IdGenerator,
}

impl Default for Miniq {
    fn default() -> Self {
        Self::new()
    }
}

impl Miniq {
    pub fn new() -> Self {
        Self {
            now: Time::START,
            queues: BTreeMap::new(),
            ids: IdGenerator::default(),
        }
    }

    /// Runs one input line and returns its response line, without the newline.
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        let outcome = Request::parse(line).and_then(|request| self.execute(request));
        reply::render(&outcome)
    }

    fn execute(&mut self, request: Request) -> Result<Reply, Error> {
        match request {
            Request::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Request::GetQueueAttributes { queue } => {
                let queue = self.queue(&queue)?;
                Ok(Reply::Attributes {
                    attributes: AttributesView::from(queue.attributes()),
                    counts: queue.counts(),
                })
            }
            Request::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Request::DeleteQueue { queue } => self.delete_queue(&queue),
            Request::PurgeQueue { queue } => {
                self.queue(&queue)?.purge();
                Ok(Reply::Empty {})
            }
            Request::Tick { seconds } => Ok(self.tick(seconds)),
            Request::SendMessage {
                queue,
                body,
                delay,
                group,
                deduplication_id,
            } => self.send_message(&queue, body, delay, group, deduplication_id),
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Request::DeleteMessage { queue, handle } => self.delete_message(&queue, handle),
            Request::ChangeMessageVisibility {
                queue,
                handle,
                visibility_timeout,
            } => self.change_message_visibility(&queue, handle, visibility_timeout),
        }
    }

    /// Looks up a queue by exactly this name and brings it up to the clock.
    fn queue(&mut self, name: &str) -> Result<&mut Queue, Error> {
        Self::lookup(&mut self.queues, name, self.now)
    }

    fn lookup<'q>(
        queues: &'q mut BTreeMap<QueueName, Queue>,
        name: &str,
        now: Time,
    ) -> Result<&'q mut Queue, Error> {
        let queue = queues
            .get_mut(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        queue.sync(now);
        Ok(queue)
    }

    /// Checks that a dead-letter queue exists and is of the same kind as the
    /// queue that wants it.
    fn dead_letter_for(
        &self,
        target: &str,
        max_receive_count: MaxReceiveCount,
        kind: Kind,
    ) -> Result<DeadLetter, Error> {
        let (name, queue) = self
            .queues
            .get_key_value(target)
            .ok_or_else(|| Error::no_such_queue(target))?;
        if queue.attributes().kind.is_fifo() != kind.is_fifo() {
            return Err(Error::invalid_value(
                "the dead-letter queue must be of the same type as the queue",
            ));
        }
        Ok(DeadLetter {
            queue: name.clone(),
            max_receive_count,
        })
    }

    fn create_queue(
        &mut self,
        name: QueueName,
        requested: &RequestedAttributes,
    ) -> Result<Reply, Error> {
        let fifo = requested.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is FIFO",
            ));
        }
        if requested.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = requested.dead_letter()?;
        if let Some(existing) = self.queues.get(&name) {
            return if existing.attributes().satisfies(requested, dead_letter) {
                Ok(Reply::Empty {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = dead_letter
            .map(|(target, max)| self.dead_letter_for(target, max, requested.kind()))
            .transpose()?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        let attributes = Attributes::from_request(requested, dead_letter);
        self.queues.insert(name, Queue::new(attributes));
        Ok(Reply::Empty {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        requested: &RequestedAttributes,
    ) -> Result<Reply, Error> {
        let kind = self.queue(name)?.attributes().kind;
        if requested.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if requested.content_based_deduplication == Some(true) && !kind.is_fifo() {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = requested
            .dead_letter()?
            .map(|(target, max)| {
                let dead_letter = self.dead_letter_for(target, max, kind)?;
                if dead_letter.queue.as_str() == name {
                    return Err(Error::invalid_value(
                        "a queue cannot be its own dead-letter queue",
                    ));
                }
                Ok(dead_letter)
            })
            .transpose()?;
        let now = self.now;
        self.queue(name)?.update(requested, dead_letter, now);
        Ok(Reply::Empty {})
    }

    fn list_queues(&self, prefix: &str) -> Reply {
        let from = (Bound::Included(prefix), Bound::Unbounded);
        let queue_names = self
            .queues
            .range::<str, _>(from)
            .map(|(name, _)| name)
            .take_while(|name| name.as_str().starts_with(prefix))
            .cloned()
            .collect();
        Reply::Queues { queue_names }
    }

    fn delete_queue(&mut self, name: &str) -> Result<Reply, Error> {
        self.queues
            .remove(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        for queue in self.queues.values_mut() {
            queue.clear_dead_letter_to(name);
        }
        Ok(Reply::Empty {})
    }

    fn tick(&mut self, seconds: TickSeconds) -> Reply {
        self.now = self.now.after(seconds.get());
        Reply::Ticked { now: self.now }
    }

    fn send_message(
        &mut self,
        name: &str,
        body: String,
        delay: Option<DelaySeconds>,
        group: Option<GroupId>,
        deduplication_id: Option<DeduplicationId>,
    ) -> Result<Reply, Error> {
        let now = self.now;
        let queue = Self::lookup(&mut self.queues, name, now)?;
        let unsupported = |message: &str| Error::new(ErrorCode::UnsupportedOperation, message);
        let fifo = match queue.attributes().kind {
            Kind::Standard => {
                if group.is_some() || deduplication_id.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                None
            }
            Kind::Fifo {
                content_based_deduplication,
            } => {
                if delay.is_some() {
                    return Err(unsupported(
                        "DelaySeconds cannot be set per message on a FIFO queue",
                    ));
                }
                let group = group.ok_or_else(|| Error::missing("GroupId"))?;
                let key = match deduplication_id {
                    Some(id) => DeduplicationKey::from_id(id),
                    None if content_based_deduplication => DeduplicationKey::from_body(&body),
                    None => return Err(Error::missing("DeduplicationId")),
                };
                if let Some(original) = queue.duplicate_of(&key, now) {
                    return Ok(Reply::Sent {
                        message_id: original.id,
                        sequence_number: Some(original.sequence),
                    });
                }
                Some(FifoSend { group, key })
            }
        };
        if queue.is_full() {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "a queue holds at most 100000 messages",
            ));
        }
        let id = self.ids.next_message();
        let delay = delay.unwrap_or(queue.attributes().delay);
        let sequence_number = queue.accept(
            NewMessage {
                id,
                body,
                delay,
                fifo,
            },
            now,
        );
        Ok(Reply::Sent {
            message_id: id,
            sequence_number,
        })
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: MaxMessages,
        visibility_timeout: Option<VisibilityTimeout>,
    ) -> Result<Reply, Error> {
        let now = self.now;
        let queue = Self::lookup(&mut self.queues, name, now)?;
        let visibility = visibility_timeout.unwrap_or(queue.attributes().visibility_timeout);
        let dead_letter_queue = queue
            .attributes()
            .dead_letter
            .as_ref()
            .map(|d| d.queue.clone());
        let receipt = queue.receive(now, max_messages, visibility, &mut self.ids);
        if let Some(target) = dead_letter_queue {
            for message in receipt.dead_lettered {
                Self::lookup(&mut self.queues, target.as_str(), now)?.adopt(message, now);
            }
        }
        Ok(Reply::Received {
            messages: receipt.delivered,
        })
    }

    /// Finds the queue and checks that the handle was ever issued; the caller
    /// then learns whether it is still usable.
    fn queue_for_handle(
        &mut self,
        name: &str,
        handle: Option<ReceiptHandle>,
    ) -> Result<(&mut Queue, ReceiptHandle), Error> {
        let queue = Self::lookup(&mut self.queues, name, self.now)?;
        let handle = handle
            .filter(|handle| self.ids.has_issued(*handle))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    "the receipt handle was never issued",
                )
            })?;
        Ok((queue, handle))
    }

    fn delete_message(
        &mut self,
        name: &str,
        handle: Option<ReceiptHandle>,
    ) -> Result<Reply, Error> {
        let (queue, handle) = self.queue_for_handle(name, handle)?;
        queue.delete_in_flight(handle).map_err(expired)?;
        Ok(Reply::Empty {})
    }

    fn change_message_visibility(
        &mut self,
        name: &str,
        handle: Option<ReceiptHandle>,
        visibility: VisibilityTimeout,
    ) -> Result<Reply, Error> {
        let now = self.now;
        let (queue, handle) = self.queue_for_handle(name, handle)?;
        queue
            .change_visibility(handle, visibility, now)
            .map_err(expired)?;
        Ok(Reply::Empty {})
    }
}

fn expired(StaleHandle: StaleHandle) -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        "the receipt handle is not the latest handle of a message in flight in this queue",
    )
}
