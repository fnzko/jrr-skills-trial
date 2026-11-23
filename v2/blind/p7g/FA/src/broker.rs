//! The whole service state and the numbered rules of each command.

use std::collections::BTreeMap;

use crate::attributes::{AttributeSet, Attributes, DeadLetter};
use crate::error::{Error, ErrorCode};
use crate::queue::{Queue, SendRequest};
use crate::request::{self, Command};
use crate::response::{self, Output};
use crate::types::{IdGenerator, QueueName, Seconds, Time};

const MAX_QUEUES: usize = 1000;

#[derive(Debug, Default)]
pub struct Broker {
    now: Time,
    ids: IdGenerator,
    queues: BTreeMap<QueueName, Queue>,
}

impl Broker {
    /// Runs one input line and returns its response line, without the newline.
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        response::render(request::parse(line).and_then(|command| self.execute(command)))
    }

    fn execute(&mut self, command: Command) -> Result<Output, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => {
                let now = self.now;
                let queue = self.queue_mut(&queue)?;
                Ok(Output::Attributes {
                    attributes: queue.attributes().into(),
                    counts: queue.counts(now),
                })
            }
            Command::ListQueues { prefix } => {
                let prefix = prefix.unwrap_or_default();
                Ok(Output::QueueNames {
                    queue_names: self
                        .queues
                        .keys()
                        .filter(|name| name.as_str().starts_with(&prefix))
                        .cloned()
                        .collect(),
                })
            }
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                self.queue_mut(&queue)?.purge();
                Ok(Output::Empty {})
            }
            Command::Tick { seconds } => {
                self.now = self.now + seconds;
                Ok(Output::Now { now: self.now })
            }
            Command::SendMessage {
                queue,
                body,
                delay,
                group,
                deduplication_id,
            } => {
                let now = self.now;
                let request = SendRequest {
                    body,
                    delay,
                    group,
                    deduplication_id,
                };
                let ids = &mut self.ids;
                let sent = Self::lookup(&mut self.queues, now, &queue)?.send(now, ids, request)?;
                Ok(Output::Sent {
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
                let queue = Self::lookup(&mut self.queues, now, &queue)?;
                queue.delete(self.ids.parse_handle(&receipt_handle)?, now)?;
                Ok(Output::Empty {})
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let now = self.now;
                let queue = Self::lookup(&mut self.queues, now, &queue)?;
                let handle = self.ids.parse_handle(&receipt_handle)?;
                queue.change_visibility(handle, now, visibility_timeout)?;
                Ok(Output::Empty {})
            }
        }
    }

    /// Finds a queue by exact name and applies retention up to `now`.
    fn lookup<'a>(
        queues: &'a mut BTreeMap<QueueName, Queue>,
        now: Time,
        name: &str,
    ) -> Result<&'a mut Queue, Error> {
        let queue = queues
            .get_mut(name)
            .ok_or_else(|| Error::queue_missing(name))?;
        queue.expire(now);
        Ok(queue)
    }

    fn queue_mut(&mut self, name: &str) -> Result<&mut Queue, Error> {
        Self::lookup(&mut self.queues, self.now, name)
    }

    /// Resolves a requested dead-letter configuration against the existing queues.
    ///
    /// `owner` is the queue being configured, if it already exists; it may not be its own target.
    fn resolve_dead_letter(
        &self,
        request: &AttributeSet,
        owner: Option<&str>,
        fifo: bool,
    ) -> Result<Option<DeadLetter>, Error> {
        let (Some(target), Some(max_receive_count)) =
            (&request.dead_letter_queue, request.max_receive_count)
        else {
            return Ok(None);
        };
        let (name, queue) = self
            .queues
            .get_key_value(target.as_str())
            .ok_or_else(|| Error::queue_missing(target))?;
        if queue.is_fifo() != fifo || owner == Some(name.as_str()) {
            return Err(Error::invalid_value(
                "DeadLetterQueue",
                "must be another queue of the same type",
            ));
        }
        Ok(Some(DeadLetter {
            queue: name.clone(),
            max_receive_count,
        }))
    }

    fn create_queue(&mut self, name: QueueName, request: &AttributeSet) -> Result<Output, Error> {
        let fifo = request.fifo.unwrap_or(false);
        if name.is_fifo() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is FIFO",
            ));
        }
        if request.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                "needs a FIFO queue",
            ));
        }
        if request.has_exactly_one_dead_letter_part() {
            return Err(dead_letter_pair_error());
        }
        if let Some(existing) = self.queues.get(&name) {
            return if existing.attributes().matches_request(request) {
                Ok(Output::Empty {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = self.resolve_dead_letter(request, None, fifo)?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("{MAX_QUEUES} queues already exist"),
            ));
        }
        let attributes = Attributes::from_request(request, dead_letter);
        self.queues.insert(name, Queue::new(attributes));
        Ok(Output::Empty {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        request: &AttributeSet,
    ) -> Result<Output, Error> {
        let fifo = self.queue_mut(name)?.is_fifo();
        if request.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if request.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                "needs a FIFO queue",
            ));
        }
        if request.has_exactly_one_dead_letter_part() {
            return Err(dead_letter_pair_error());
        }
        let dead_letter = self.resolve_dead_letter(request, Some(name), fifo)?;
        self.queue_mut(name)?
            .attributes_mut()
            .apply(request, dead_letter);
        Ok(Output::Empty {})
    }

    fn delete_queue(&mut self, name: &str) -> Result<Output, Error> {
        self.queue_mut(name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            let attributes = queue.attributes_mut();
            if attributes
                .dead_letter
                .as_ref()
                .is_some_and(|config| config.queue.as_str() == name)
            {
                attributes.dead_letter = None;
            }
        }
        Ok(Output::Empty {})
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: usize,
        visibility_timeout: Option<Seconds>,
    ) -> Result<Output, Error> {
        let now = self.now;
        let queue = Self::lookup(&mut self.queues, now, name)?;
        let visibility_timeout =
            visibility_timeout.unwrap_or(queue.attributes().visibility_timeout);
        let received = queue.receive(now, max_messages, visibility_timeout, &mut self.ids);
        if let Some(target) = queue
            .attributes()
            .dead_letter
            .as_ref()
            .map(|d| d.queue.clone())
        {
            let target = self
                .queues
                .get_mut(&target)
                .expect("deleting a queue clears the dead-letter configurations that name it");
            for message in received.dead {
                target.accept_dead_letter(message, now);
            }
        }
        Ok(Output::Messages {
            messages: received.deliveries,
        })
    }
}

fn dead_letter_pair_error() -> Error {
    Error::invalid_value(
        "DeadLetterQueue",
        "and MaxReceiveCount must be given together",
    )
}
