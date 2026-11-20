use std::collections::BTreeMap;
use std::ops::Bound;

use crate::error::Error;
use crate::queue::{DeadLetter, Queue, QueueAttributes, QueueKind};
use crate::request::{self, AttributeChanges, Request};
use crate::response::{Output, Response};
use crate::types::{Counter, QueueName, ReceiptHandle, Timestamp};

pub const MAX_QUEUES: usize = 1000;

/// The whole state of a miniq process: the virtual clock and every queue.
#[derive(Debug, Default)]
pub struct Broker {
    now: Timestamp,
    message_ids: Counter,
    receipts: Counter,
    /// Ordered by name bytes, which is the order `ListQueues` reports.
    queues: BTreeMap<QueueName, Queue>,
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one input line (without its trailing `\n`) as a command.
    pub fn handle_line(&mut self, line: &[u8]) -> Response {
        Response(request::parse(line).and_then(|request| self.execute(request)))
    }

    fn execute(&mut self, request: Request) -> Result<Output, Error> {
        match request {
            Request::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Request::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Request::GetQueueAttributes { queue } => {
                let queue = self.queue(&queue)?;
                Ok(Output::QueueAttributes {
                    attributes: queue.attributes().into(),
                    counts: queue.counts(self.now),
                })
            }
            Request::DeleteQueue { queue } => self.delete_queue(&queue),
            Request::PurgeQueue { queue } => {
                self.queue_mut(&queue)?.purge();
                Ok(Output::Empty)
            }
            Request::Tick { seconds } => {
                self.now = self.now + seconds;
                for queue in self.queues.values_mut() {
                    queue.expire(self.now);
                }
                Ok(Output::Now(self.now))
            }
            Request::SendMessage { queue, message } => {
                let queue = lookup(&mut self.queues, &queue)?;
                let sent = queue.send(message, self.now, &mut self.message_ids)?;
                Ok(Output::Sent(sent))
            }
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => {
                let queue = lookup(&mut self.queues, &queue)?;
                let received = queue.receive(
                    max_messages,
                    visibility_timeout,
                    self.now,
                    &mut self.receipts,
                );
                if let Some(dead_letters) = received.dead_letters {
                    self.queues
                        .get_mut(&dead_letters.queue)
                        .expect("a dead-letter queue exists: DeleteQueue clears references to it")
                        .accept_dead_letters(dead_letters.messages, self.now);
                }
                Ok(Output::Messages(received.deliveries))
            }
            Request::DeleteMessage { queue, handle } => {
                let queue = lookup(&mut self.queues, &queue)?;
                queue.delete(issued(&self.receipts, handle)?, self.now)?;
                Ok(Output::Empty)
            }
            Request::ChangeMessageVisibility {
                queue,
                handle,
                visibility_timeout,
            } => {
                let queue = lookup(&mut self.queues, &queue)?;
                let handle = issued(&self.receipts, handle)?;
                queue.change_visibility(handle, visibility_timeout, self.now)?;
                Ok(Output::Empty)
            }
        }
    }

    fn create_queue(
        &mut self,
        name: QueueName,
        changes: &AttributeChanges,
    ) -> Result<Output, Error> {
        let fifo = changes.fifo.unwrap_or(false);
        if fifo != name.has_fifo_suffix() {
            return Err(Error::FifoSuffixMismatch);
        }
        let kind = match (fifo, changes.content_based_deduplication.unwrap_or(false)) {
            (false, true) => return Err(Error::ContentDeduplicationOnStandard),
            (false, false) => QueueKind::Standard,
            (true, content_based_deduplication) => QueueKind::Fifo {
                content_based_deduplication,
            },
        };
        // A name that fails the syntax cannot belong to an existing queue, so
        // it is reported as missing, but only after the existing-queue rule.
        let dead_letter = changes
            .dead_letter()?
            .map(|(queue, max_receive_count)| {
                QueueName::parse(queue.to_owned())
                    .map(|queue| DeadLetter {
                        queue,
                        max_receive_count,
                    })
                    .map_err(Error::QueueDoesNotExist)
            })
            .transpose();
        let attributes = dead_letter.map(|dead_letter| QueueAttributes {
            visibility_timeout: changes
                .visibility_timeout
                .unwrap_or(QueueAttributes::DEFAULT_VISIBILITY_TIMEOUT),
            delay: changes.delay.unwrap_or(QueueAttributes::DEFAULT_DELAY),
            retention: changes
                .retention
                .unwrap_or(QueueAttributes::DEFAULT_RETENTION),
            kind,
            dead_letter,
        });
        if let Some(existing) = self.queues.get(&name) {
            // An existing dead-letter queue always exists, so a request naming
            // a missing one can never match.
            return match attributes {
                Ok(attributes) if attributes == *existing.attributes() => Ok(Output::Empty),
                _ => Err(Error::QueueAlreadyExists(name)),
            };
        }
        let attributes = attributes?;
        if let Some(dead_letter) = &attributes.dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| Error::QueueDoesNotExist(dead_letter.queue.to_string()))?;
            if target.is_fifo() != fifo {
                return Err(Error::DeadLetterTypeMismatch);
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::TooManyQueues);
        }
        self.queues.insert(name, Queue::new(attributes));
        Ok(Output::Empty)
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        changes: &AttributeChanges,
    ) -> Result<Output, Error> {
        let now = self.now;
        // Looked up before the target is borrowed mutably; only consulted in
        // rule order below.
        let target = changes
            .dead_letter_queue
            .as_deref()
            .and_then(|target| self.queues.get_key_value(target))
            .map(|(target, queue)| (target.clone(), queue.is_fifo()));
        let queue = self.queue_mut(name)?;
        if changes.fifo.is_some() {
            return Err(Error::FifoIsFixed);
        }
        let kind = match (queue.attributes().kind, changes.content_based_deduplication) {
            (QueueKind::Standard, Some(true)) => return Err(Error::ContentDeduplicationOnStandard),
            (QueueKind::Fifo { .. }, Some(content_based_deduplication)) => QueueKind::Fifo {
                content_based_deduplication,
            },
            (kind, _) => kind,
        };
        let dead_letter = match changes.dead_letter()? {
            None => None,
            Some((target_name, max_receive_count)) => {
                let (target, target_fifo) =
                    target.ok_or_else(|| Error::QueueDoesNotExist(target_name.to_owned()))?;
                if target.as_str() == name {
                    return Err(Error::DeadLetterIsSelf);
                }
                if target_fifo != queue.is_fifo() {
                    return Err(Error::DeadLetterTypeMismatch);
                }
                Some(DeadLetter {
                    queue: target,
                    max_receive_count,
                })
            }
        };
        let attributes = queue.attributes_mut();
        attributes.kind = kind;
        if let Some(timeout) = changes.visibility_timeout {
            attributes.visibility_timeout = timeout;
        }
        if let Some(delay) = changes.delay {
            attributes.delay = delay;
        }
        if let Some(retention) = changes.retention {
            attributes.retention = retention;
        }
        if dead_letter.is_some() {
            attributes.dead_letter = dead_letter;
        }
        // A lower retention removes old messages at once.
        queue.expire(now);
        Ok(Output::Empty)
    }

    fn list_queues(&self, prefix: &str) -> Output {
        let names = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name)
            .take_while(|name| name.as_str().starts_with(prefix))
            .cloned()
            .collect();
        Output::QueueNames(names)
    }

    fn delete_queue(&mut self, name: &str) -> Result<Output, Error> {
        self.queues
            .remove(name)
            .ok_or_else(|| Error::QueueDoesNotExist(name.to_owned()))?;
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
        Ok(Output::Empty)
    }

    fn queue(&self, name: &str) -> Result<&Queue, Error> {
        self.queues
            .get(name)
            .ok_or_else(|| Error::QueueDoesNotExist(name.to_owned()))
    }

    fn queue_mut(&mut self, name: &str) -> Result<&mut Queue, Error> {
        lookup(&mut self.queues, name)
    }
}

/// Takes the map rather than the broker so callers can still use the
/// broker's other fields while holding the queue.
fn lookup<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    name: &str,
) -> Result<&'a mut Queue, Error> {
    queues
        .get_mut(name)
        .ok_or_else(|| Error::QueueDoesNotExist(name.to_owned()))
}

/// Parses a receipt handle and checks that this process has issued it.
fn issued(receipts: &Counter, text: String) -> Result<ReceiptHandle, Error> {
    ReceiptHandle::parse(&text)
        .filter(|handle| receipts.has_issued(handle.0))
        .ok_or(Error::ReceiptHandleInvalid(text))
}
