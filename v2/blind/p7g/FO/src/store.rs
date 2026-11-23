//! All queues of the process together with the clock and the identifier counters.

use std::collections::BTreeMap;

use crate::error::{Error, ErrorCode};
use crate::queue::{ArrivalKey, Config, DeadLetter, Kind, NewMessage, Queue};
use crate::request::{Attributes, Request};
use crate::response::{Messages, Now, QueueAttributes, QueueNames, Reply};
use crate::types::{
    DEFAULT_DELAY, DEFAULT_RETENTION, DEFAULT_VISIBILITY_TIMEOUT, MaxReceiveCount, MessageId,
    QueueName, ReceiptHandle, Time,
};

const MAX_QUEUES: usize = 1000;

/// Every receipt handle issued so far, in order: handle `r-n` is entry `n - 1`.
#[derive(Debug, Default)]
struct Deliveries(Vec<ArrivalKey>);

impl Deliveries {
    fn issue(&mut self, key: ArrivalKey) -> ReceiptHandle {
        self.0.push(key);
        let issued = u64::try_from(self.0.len()).expect("a usize count fits in u64");
        ReceiptHandle::new(issued)
    }

    /// The handle and the arrival key of the message it was issued for.
    fn resolve(&self, text: &str) -> Result<(ReceiptHandle, ArrivalKey), Error> {
        let number = ReceiptHandle::parse_number(text);
        let index = number
            .and_then(|n| n.checked_sub(1))
            .and_then(|n| usize::try_from(n).ok());
        match (number, index.and_then(|i| self.0.get(i))) {
            (Some(number), Some(&key)) => Ok((ReceiptHandle::new(number), key)),
            _ => Err(Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("{text:?} is not a receipt handle that was issued"),
            )),
        }
    }
}

#[derive(Debug)]
pub struct Store {
    now: Time,
    queues: BTreeMap<QueueName, Queue>,
    messages_accepted: u64,
    deliveries: Deliveries,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    pub fn new() -> Self {
        Self {
            now: Time::START,
            queues: BTreeMap::new(),
            messages_accepted: 0,
            deliveries: Deliveries::default(),
        }
    }

    pub fn execute(&mut self, request: Request<'_>) -> Result<Reply, Error> {
        match request {
            Request::CreateQueue { name, attributes } => {
                self.create_queue(name, &attributes)?;
                Ok(Reply::nothing())
            }
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(queue, &attributes)?;
                Ok(Reply::nothing())
            }
            Request::GetQueueAttributes { queue } => {
                let now = self.now;
                let queue = self.queue(queue)?;
                Ok(Reply::QueueAttributes(QueueAttributes::new(
                    queue.config(),
                    queue.counts(now),
                )))
            }
            Request::ListQueues { prefix } => {
                let prefix = prefix.unwrap_or_default();
                let queue_names = self
                    .queues
                    .keys()
                    .filter(|name| name.as_str().starts_with(prefix))
                    .cloned()
                    .collect();
                Ok(Reply::QueueNames(QueueNames { queue_names }))
            }
            Request::DeleteQueue { queue } => {
                self.queues
                    .remove(queue)
                    .ok_or_else(|| Error::no_such_queue(queue))?;
                for other in self.queues.values_mut() {
                    other.detach_dead_letter(queue);
                }
                Ok(Reply::nothing())
            }
            Request::PurgeQueue { queue } => {
                self.queue(queue)?.purge();
                Ok(Reply::nothing())
            }
            Request::Tick { seconds } => {
                self.now = self.now.plus(seconds);
                Ok(Reply::Now(Now { now: self.now }))
            }
            Request::SendMessage {
                queue,
                body,
                delay,
                group,
                deduplication,
            } => {
                let now = self.now;
                let accepted = &mut self.messages_accepted;
                let queue = Self::lookup(&mut self.queues, queue, now)?;
                let new = NewMessage {
                    body: &body,
                    delay,
                    group: group.as_ref(),
                    deduplication: deduplication.as_ref(),
                };
                let sent = queue.send(now, &new, || {
                    *accepted += 1;
                    MessageId::new(*accepted)
                })?;
                Ok(Reply::Sent(sent))
            }
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => {
                let now = self.now;
                let deliveries = &mut self.deliveries;
                let source = Self::lookup(&mut self.queues, queue, now)?;
                let visibility =
                    visibility_timeout.unwrap_or_else(|| source.config().visibility_timeout);
                let received =
                    source.receive(now, max_messages, visibility, |key| deliveries.issue(key));
                if let Some(moved) = received.dead_lettered {
                    self.queues
                        .get_mut(&moved.target)
                        .expect("a dead-letter target exists: DeleteQueue clears references to it")
                        .accept_dead_lettered(moved.messages, now);
                }
                Ok(Reply::Messages(Messages {
                    messages: received.delivered,
                }))
            }
            Request::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let now = self.now;
                let queue = Self::lookup(&mut self.queues, queue, now)?;
                let (handle, key) = self.deliveries.resolve(receipt_handle)?;
                queue.acknowledge(key, handle, now)?;
                Ok(Reply::nothing())
            }
            Request::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let now = self.now;
                let queue = Self::lookup(&mut self.queues, queue, now)?;
                let (handle, key) = self.deliveries.resolve(receipt_handle)?;
                queue.change_visibility(key, handle, visibility_timeout, now)?;
                Ok(Reply::nothing())
            }
        }
    }

    /// The queue called `name`, with its expired messages already removed.
    fn lookup<'q>(
        queues: &'q mut BTreeMap<QueueName, Queue>,
        name: &str,
        now: Time,
    ) -> Result<&'q mut Queue, Error> {
        let queue = queues
            .get_mut(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        queue.expire(now);
        Ok(queue)
    }

    fn queue(&mut self, name: &str) -> Result<&mut Queue, Error> {
        Self::lookup(&mut self.queues, name, self.now)
    }

    fn create_queue(&mut self, name: QueueName, attributes: &Attributes<'_>) -> Result<(), Error> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is FIFO",
            ));
        }
        let content_based_deduplication = attributes.content_based_deduplication.unwrap_or(false);
        if content_based_deduplication && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = attributes.dead_letter()?;
        let kind = if fifo {
            Kind::Fifo {
                content_based_deduplication,
            }
        } else {
            Kind::Standard
        };
        let visibility_timeout = attributes
            .visibility_timeout
            .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT);
        let delay = attributes.delay.unwrap_or(DEFAULT_DELAY);
        let retention = attributes.retention.unwrap_or(DEFAULT_RETENTION);

        if let Some(existing) = self.queues.get(name.as_str()) {
            let existing = existing.config();
            let same = existing.visibility_timeout == visibility_timeout
                && existing.delay == delay
                && existing.retention == retention
                && existing.kind == kind
                && existing
                    .dead_letter
                    .as_ref()
                    .map(|d| (d.queue.as_str(), d.max_receive_count))
                    == dead_letter;
            return if same {
                Ok(())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }

        let dead_letter = self.resolve_dead_letter(dead_letter, name.as_str(), kind)?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("{MAX_QUEUES} queues already exist"),
            ));
        }
        let config = Config {
            visibility_timeout,
            delay,
            retention,
            kind,
            dead_letter,
        };
        self.queues.insert(name, Queue::new(config));
        Ok(())
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        attributes: &Attributes<'_>,
    ) -> Result<(), Error> {
        let current = self.queue(name)?.config().clone();
        if attributes.fifo.is_some() {
            return Err(Error::unsupported("the type of a queue cannot change"));
        }
        let kind = match (current.kind, attributes.content_based_deduplication) {
            (Kind::Standard, Some(true)) => {
                return Err(Error::invalid_value(
                    "ContentBasedDeduplication needs a FIFO queue",
                ));
            }
            (Kind::Fifo { .. }, Some(content_based_deduplication)) => Kind::Fifo {
                content_based_deduplication,
            },
            (kind, _) => kind,
        };
        let dead_letter = attributes.dead_letter()?;
        let dead_letter = self
            .resolve_dead_letter(dead_letter, name, kind)?
            .or(current.dead_letter);
        let config = Config {
            visibility_timeout: attributes
                .visibility_timeout
                .unwrap_or(current.visibility_timeout),
            delay: attributes.delay.unwrap_or(current.delay),
            retention: attributes.retention.unwrap_or(current.retention),
            kind,
            dead_letter,
        };
        let now = self.now;
        self.queue(name)?.set_config(config, now);
        Ok(())
    }

    /// Checks that the dead-letter queue exists, is not `owner` itself and has the same type
    /// as `owner`'s `kind`.
    fn resolve_dead_letter(
        &self,
        requested: Option<(&str, MaxReceiveCount)>,
        owner: &str,
        kind: Kind,
    ) -> Result<Option<DeadLetter>, Error> {
        let Some((target, max_receive_count)) = requested else {
            return Ok(None);
        };
        let (name, queue) = self
            .queues
            .get_key_value(target)
            .ok_or_else(|| Error::no_such_queue(target))?;
        if target == owner || queue.config().kind.is_fifo() != kind.is_fifo() {
            return Err(Error::invalid_value(
                "DeadLetterQueue must be another queue of the same type",
            ));
        }
        Ok(Some(DeadLetter {
            queue: name.clone(),
            max_receive_count,
        }))
    }
}
