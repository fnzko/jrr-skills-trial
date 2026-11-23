//! All queues of the process and the commands that act on them.

use std::collections::BTreeMap;
use std::ops::Bound;

use crate::command::{AttributeSet, Command, NewMessage};
use crate::error::{Error, Limit, NameProblem, Problem};
use crate::queue::{Config, DeadLetter, Kind, Queue, Received, content_based_requires_fifo};
use crate::reply::{Attributes, Output};
use crate::types::{
    Counter, MaxMessages, QueueName, ReceiptHandle, TickSeconds, Time, VisibilityTimeout,
};

const MAX_QUEUES: usize = 1000;

#[derive(Debug)]
pub struct Service {
    now: Time,
    queues: BTreeMap<QueueName, Queue>,
    message_ids: Counter,
    receipts: Counter,
}

impl Default for Service {
    fn default() -> Self {
        Service {
            now: Time::START,
            queues: BTreeMap::new(),
            message_ids: Counter::default(),
            receipts: Counter::default(),
        }
    }
}

impl Service {
    pub fn execute(&mut self, command: Command) -> Result<Output, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => {
                let now = self.now;
                let queue = self.queue(&queue)?;
                Ok(Output::QueueState {
                    attributes: Attributes::from(queue.config()),
                    counts: queue.counts(now),
                })
            }
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                self.queue(&queue)?.purge();
                Ok(Output::Done {})
            }
            Command::Tick { seconds } => Ok(self.tick(seconds)),
            Command::SendMessage { queue, message } => self.send(&queue, message),
            Command::ReceiveMessage {
                queue,
                max,
                visibility_timeout,
            } => self.receive(&queue, max, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let now = self.now;
                let handle = self.parse_handle(&queue, &receipt_handle)?;
                self.queue(&queue)?.delete(handle, now)?;
                Ok(Output::Done {})
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let now = self.now;
                let handle = self.parse_handle(&queue, &receipt_handle)?;
                self.queue(&queue)?
                    .change_visibility(handle, now, visibility_timeout)?;
                Ok(Output::Done {})
            }
        }
    }

    /// The named queue with its expired messages already removed.
    fn queue(&mut self, name: &str) -> Result<&mut Queue, Error> {
        live_queue(&mut self.queues, self.now, name)
    }

    fn existing_name(&self, name: &str) -> Result<QueueName, Error> {
        self.queues
            .get_key_value(name)
            .map(|(key, _)| key.clone())
            .ok_or_else(|| Error::QueueDoesNotExist(name.to_owned()))
    }

    fn kind_of(&self, name: &str) -> Result<Kind, Error> {
        self.queues
            .get(name)
            .map(|queue| queue.config().kind)
            .ok_or_else(|| Error::QueueDoesNotExist(name.to_owned()))
    }

    /// Checks `ReceiptHandleInvalid` after the queue lookup, as the spec orders them.
    fn parse_handle(&self, queue: &str, text: &str) -> Result<ReceiptHandle, Error> {
        self.kind_of(queue)?;
        ReceiptHandle::parse(text)
            .filter(|handle| handle.number() <= self.receipts.issued())
            .ok_or(Error::ReceiptHandleInvalid)
    }

    fn create_queue(
        &mut self,
        name: QueueName,
        attributes: &AttributeSet,
    ) -> Result<Output, Error> {
        let fifo = attributes.fifo.unwrap_or(false);
        if fifo != name.has_fifo_suffix() {
            return Err(Error::QueueNameInvalid(NameProblem::FifoMismatch));
        }
        let kind = Kind::from_flags(
            fifo,
            attributes.content_based_deduplication.unwrap_or(false),
        )?;
        let requested_dead_letter = attributes.dead_letter()?;
        if let Some(existing) = self.queues.get(&name) {
            return if existing.config().is_requested_by(attributes, kind) {
                Ok(Output::Done {})
            } else {
                Err(Error::QueueAlreadyExists)
            };
        }
        let dead_letter = match requested_dead_letter {
            Some((target, max_receive_count)) => {
                if self.kind_of(target)?.is_fifo() != fifo {
                    return Err(dead_letter_kind_mismatch());
                }
                Some(DeadLetter {
                    queue: self.existing_name(target)?,
                    max_receive_count,
                })
            }
            None => None,
        };
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::LimitExceeded(Limit::Queues));
        }
        let config = Config::from_attributes(attributes, kind, dead_letter);
        self.queues.insert(name, Queue::new(config));
        Ok(Output::Done {})
    }

    fn set_attributes(&mut self, name: &str, attributes: &AttributeSet) -> Result<Output, Error> {
        let kind = self.queue(name)?.config().kind;
        if attributes.fifo.is_some() {
            return Err(Error::UnsupportedOperation(
                "the type of a queue never changes",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !kind.is_fifo() {
            return Err(content_based_requires_fifo());
        }
        let dead_letter = match attributes.dead_letter()? {
            Some((target, max_receive_count)) => {
                let target_kind = self.kind_of(target)?;
                if target == name || target_kind.is_fifo() != kind.is_fifo() {
                    return Err(dead_letter_kind_mismatch());
                }
                Some(DeadLetter {
                    queue: self.existing_name(target)?,
                    max_receive_count,
                })
            }
            None => None,
        };
        let config = self.queue(name)?.config_mut();
        if let Some(v) = attributes.visibility_timeout {
            config.visibility_timeout = v;
        }
        if let Some(v) = attributes.delay {
            config.delay = v;
        }
        if let Some(v) = attributes.retention {
            config.retention = v;
        }
        if let (Some(v), Kind::Fifo { .. }) = (attributes.content_based_deduplication, config.kind)
        {
            config.kind = Kind::Fifo {
                content_based_deduplication: v,
            };
        }
        if dead_letter.is_some() {
            config.dead_letter = dead_letter;
        }
        // A lowered retention removes messages at once.
        self.queue(name)?;
        Ok(Output::Done {})
    }

    fn list_queues(&self, prefix: &str) -> Output {
        let queue_names = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name)
            .take_while(|name| name.as_str().starts_with(prefix))
            .cloned()
            .collect();
        Output::Queues { queue_names }
    }

    fn delete_queue(&mut self, name: &str) -> Result<Output, Error> {
        self.existing_name(name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            let config = queue.config_mut();
            if config
                .dead_letter
                .as_ref()
                .is_some_and(|d| d.queue.as_str() == name)
            {
                config.dead_letter = None;
            }
        }
        Ok(Output::Done {})
    }

    fn tick(&mut self, seconds: TickSeconds) -> Output {
        self.now = self.now.plus(seconds.get());
        Output::Clock { now: self.now }
    }

    fn send(&mut self, name: &str, message: NewMessage) -> Result<Output, Error> {
        let now = self.now;
        let queue = live_queue(&mut self.queues, now, name)?;
        let accepted = queue.send(now, message, &mut self.message_ids)?;
        Ok(Output::Sent {
            message_id: accepted.id,
            sequence_number: accepted.sequence,
        })
    }

    fn receive(
        &mut self,
        name: &str,
        max: MaxMessages,
        visibility_timeout: Option<VisibilityTimeout>,
    ) -> Result<Output, Error> {
        let now = self.now;
        let queue = live_queue(&mut self.queues, now, name)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.config().visibility_timeout);
        let Received {
            delivered,
            dead_lettered,
        } = queue.receive(now, max, visibility_timeout, &mut self.receipts);
        if let Some(moved) = dead_lettered {
            let target = self
                .queues
                .get_mut(moved.target.as_str())
                .expect("DeleteQueue clears every dead-letter reference to a deleted queue");
            for message in moved.messages {
                target.accept_dead_letter(message, now);
            }
        }
        Ok(Output::Received {
            messages: delivered,
        })
    }
}

/// Looks a queue up and removes its expired messages, so callers only see live ones.
fn live_queue<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    now: Time,
    name: &str,
) -> Result<&'a mut Queue, Error> {
    let queue = queues
        .get_mut(name)
        .ok_or_else(|| Error::QueueDoesNotExist(name.to_owned()))?;
    queue.expire(now);
    Ok(queue)
}

fn dead_letter_kind_mismatch() -> Error {
    Error::invalid(
        "DeadLetterQueue",
        Problem::Conflict("a dead-letter queue must be another queue of the same type"),
    )
}
