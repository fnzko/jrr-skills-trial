//! The process-wide state and command execution.

use std::collections::BTreeMap;
use std::ops::Bound;

use crate::attributes::{AttributeUpdates, Attributes, DeadLetter, DeadLetterSpec};
use crate::command::Command;
use crate::error::{Error, ErrorCode};
use crate::ids::{Counters, QueueName};
use crate::queue::{Delivery, Queue};
use crate::reply::{Clock, Messages, QueueNames, QueueState, Reply, Response};
use crate::time::{Seconds, Time};

const MAX_QUEUES: usize = 1000;

type Queues = BTreeMap<QueueName, Queue>;

fn find<'q>(queues: &'q Queues, name: &str) -> Result<&'q Queue, Error> {
    queues.get(name).ok_or_else(|| Error::no_queue(name))
}

fn find_mut<'q>(queues: &'q mut Queues, name: &str) -> Result<&'q mut Queue, Error> {
    queues.get_mut(name).ok_or_else(|| Error::no_queue(name))
}

/// All state of one miniq process.
#[derive(Debug)]
pub struct Service {
    now: Time,
    queues: Queues,
    counters: Counters,
}

impl Default for Service {
    fn default() -> Self {
        Self {
            now: Time::START,
            queues: Queues::new(),
            counters: Counters::default(),
        }
    }
}

impl Service {
    /// Runs one input line and returns the response to print.
    pub fn handle_line(&mut self, line: &[u8]) -> Response<'_> {
        let result = match Command::parse(line) {
            Ok(command) => self.execute(command),
            Err(error) => Err(error),
        };
        match result {
            Ok(reply) => Response::success(reply),
            Err(error) => Response::failure(error),
        }
    }

    fn execute(&mut self, command: Command) -> Result<Reply<'_>, Error> {
        match command {
            Command::CreateQueue { name, attributes } => {
                self.create_queue(name, &attributes)?;
                Ok(Reply::Empty {})
            }
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)?;
                Ok(Reply::Empty {})
            }
            Command::GetQueueAttributes { queue } => {
                let queue = find(&self.queues, &queue)?;
                Ok(Reply::QueueState(QueueState::new(
                    queue.attributes(),
                    queue.counts(self.now),
                )))
            }
            Command::ListQueues { prefix } => Ok(Reply::QueueNames(QueueNames::new(
                self.queues
                    .range::<str, _>((Bound::Included(prefix.as_str()), Bound::Unbounded))
                    .map(|(name, _)| name)
                    .take_while(|name| name.as_str().starts_with(&prefix))
                    .collect(),
            ))),
            Command::DeleteQueue { queue } => {
                self.delete_queue(&queue)?;
                Ok(Reply::Empty {})
            }
            Command::PurgeQueue { queue } => {
                find_mut(&mut self.queues, &queue)?.purge();
                Ok(Reply::Empty {})
            }
            Command::Tick { seconds } => Ok(Reply::Clock(Clock::new(self.tick(seconds)))),
            Command::SendMessage { queue, draft } => {
                let queue = find_mut(&mut self.queues, &queue)?;
                Ok(Reply::Sent(queue.send(
                    draft,
                    self.now,
                    &mut self.counters,
                )?))
            }
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => {
                let messages = self.receive(&queue, max_messages, visibility_timeout)?;
                Ok(Reply::Messages(Messages::new(messages)))
            }
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let queue = find_mut(&mut self.queues, &queue)?;
                let handle = self.counters.issued_handle(&receipt_handle)?;
                queue.delete_message(handle, self.now)?;
                Ok(Reply::Empty {})
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let queue = find_mut(&mut self.queues, &queue)?;
                let handle = self.counters.issued_handle(&receipt_handle)?;
                queue.change_visibility(handle, visibility_timeout, self.now)?;
                Ok(Reply::Empty {})
            }
        }
    }

    /// Finds the dead-letter queue a request names and checks that it can
    /// serve `owner`, a queue of the given type.
    fn resolve_dead_letter(
        &self,
        spec: Option<DeadLetterSpec<'_>>,
        owner: &str,
        owner_is_fifo: bool,
    ) -> Result<Option<DeadLetter>, Error> {
        let Some(spec) = spec else {
            return Ok(None);
        };
        let (name, queue) = self
            .queues
            .get_key_value(spec.queue)
            .ok_or_else(|| Error::no_queue(spec.queue))?;
        if name.as_str() == owner || queue.attributes().is_fifo() != owner_is_fifo {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!(
                    "queue {:?} cannot be the dead-letter queue of {owner:?}",
                    spec.queue
                ),
            ));
        }
        Ok(Some(DeadLetter {
            queue: name.clone(),
            max_receive_count: spec.max_receive_count,
        }))
    }

    fn create_queue(&mut self, name: QueueName, updates: &AttributeUpdates) -> Result<(), Error> {
        let fifo = updates.fifo().unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!(
                    "queue {:?} must end in .fifo exactly when Fifo is true",
                    name.as_str()
                ),
            ));
        }
        if updates.content_based_deduplication() == Some(true) && !fifo {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "ContentBasedDeduplication requires Fifo",
            ));
        }
        let spec = updates.dead_letter()?;
        if let Some(existing) = self.queues.get(&name) {
            return if existing.attributes().matches_create(updates, spec) {
                Ok(())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = self.resolve_dead_letter(spec, name.as_str(), fifo)?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        let attributes = Attributes::for_new_queue(updates, dead_letter);
        self.queues.insert(name, Queue::new(attributes));
        Ok(())
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        updates: &AttributeUpdates,
    ) -> Result<(), Error> {
        let is_fifo = find(&self.queues, name)?.attributes().is_fifo();
        if updates.fifo().is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if updates.content_based_deduplication() == Some(true) && !is_fifo {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        let spec = updates.dead_letter()?;
        let dead_letter = self.resolve_dead_letter(spec, name, is_fifo)?;
        find_mut(&mut self.queues, name)?.set_attributes(updates, dead_letter, self.now);
        Ok(())
    }

    fn delete_queue(&mut self, name: &str) -> Result<(), Error> {
        find(&self.queues, name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            queue.forget_dead_letter_queue(name);
        }
        Ok(())
    }

    fn receive(
        &mut self,
        name: &str,
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
    ) -> Result<Vec<Delivery>, Error> {
        let queue = find_mut(&mut self.queues, name)?;
        let received = queue.receive(
            max_messages,
            visibility_timeout,
            self.now,
            &mut self.counters,
        );
        if let Some(dead_letters) = received.dead_letters {
            self.queues
                .get_mut(&dead_letters.queue)
                .expect("DeleteQueue clears every dead-letter reference to the deleted queue")
                .accept_dead_letters(dead_letters.messages, self.now);
        }
        Ok(received.delivered)
    }

    fn tick(&mut self, seconds: Seconds) -> Time {
        self.now = self.now.plus(seconds);
        for queue in self.queues.values_mut() {
            queue.expire(self.now);
        }
        self.now
    }
}
