//! The process-wide state: the clock, the queues by name and the counters
//! that span queues. Each command is one method.

use std::collections::BTreeMap;

use crate::deliveries::Deliveries;
use crate::error::{Error, ErrorCode};
use crate::queue::{
    DeadLetter, DeadLetterRequest, Queue, QueueSpec, QueueType, SendOutcome, Update,
};
use crate::request::{Attributes, Command, NewMessage};
use crate::response::{Clock, Empty, Messages, Outcome, QueueAttributes, QueueNames};
use crate::types::{
    Delay, Instant, MaxMessages, MaxReceiveCount, MessageId, QueueId, QueueName, Retention,
    TickSeconds, VisibilityTimeout,
};

const QUEUE_LIMIT: usize = 1000;

const DEFAULT_VISIBILITY_TIMEOUT: VisibilityTimeout = VisibilityTimeout::constant(30);
const DEFAULT_DELAY: Delay = Delay::constant(0);
const DEFAULT_RETENTION: Retention = Retention::constant(345_600);

#[derive(Debug)]
pub struct Broker {
    now: Instant,
    queues: BTreeMap<QueueName, Queue>,
    next_queue: QueueId,
    next_message: MessageId,
    deliveries: Deliveries,
}

impl Default for Broker {
    fn default() -> Self {
        Self {
            now: Instant::START,
            queues: BTreeMap::new(),
            next_queue: QueueId::FIRST,
            next_message: MessageId::FIRST,
            deliveries: Deliveries::default(),
        }
    }
}

fn no_such_queue(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("no queue is named {name:?}"),
    )
}

fn invalid_attributes(reason: &str) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, reason)
}

/// Finds a queue and applies retention at `now`, so that callers only see
/// messages that are still alive.
fn lookup<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    now: Instant,
    name: &str,
) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| no_such_queue(name))?;
    queue.expire(now);
    Ok(queue)
}

/// `DeadLetterQueue` and `MaxReceiveCount` are both set or both unset.
fn dead_letter_request(
    queue: Option<String>,
    max_receive_count: Option<MaxReceiveCount>,
) -> Result<Option<DeadLetterRequest>, Error> {
    match (queue, max_receive_count) {
        (None, None) => Ok(None),
        (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetterRequest {
            queue,
            max_receive_count,
        })),
        _ => Err(invalid_attributes(
            "DeadLetterQueue and MaxReceiveCount must be given together",
        )),
    }
}

impl Broker {
    pub fn execute(&mut self, command: Command) -> Result<Outcome, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, attributes)
            }
            Command::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                lookup(&mut self.queues, self.now, &queue)?.purge();
                Ok(Outcome::Done(Empty {}))
            }
            Command::Tick { seconds } => Ok(self.tick(seconds)),
            Command::SendMessage { queue, message } => self.send_message(&queue, message),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let queue = lookup(&mut self.queues, self.now, &queue)?;
                let (handle, arrival) = self.deliveries.resolve(queue.id(), &receipt_handle)?;
                queue.delete(self.now, arrival, handle)?;
                Ok(Outcome::Done(Empty {}))
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let queue = lookup(&mut self.queues, self.now, &queue)?;
                let (handle, arrival) = self.deliveries.resolve(queue.id(), &receipt_handle)?;
                queue.change_visibility(self.now, arrival, handle, visibility_timeout)?;
                Ok(Outcome::Done(Empty {}))
            }
        }
    }

    fn create_queue(&mut self, name: QueueName, attributes: Attributes) -> Result<Outcome, Error> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.is_fifo_name() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is a FIFO queue",
            ));
        }
        let queue_type = match (
            fifo,
            attributes.content_based_deduplication.unwrap_or(false),
        ) {
            (false, true) => {
                return Err(invalid_attributes(
                    "ContentBasedDeduplication requires a FIFO queue",
                ));
            }
            (false, false) => QueueType::Standard,
            (true, content_based_deduplication) => QueueType::Fifo {
                content_based_deduplication,
            },
        };
        let spec = QueueSpec {
            queue_type,
            visibility_timeout: attributes
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay: attributes.delay.unwrap_or(DEFAULT_DELAY),
            retention: attributes.retention.unwrap_or(DEFAULT_RETENTION),
            dead_letter: dead_letter_request(
                attributes.dead_letter_queue,
                attributes.max_receive_count,
            )?,
        };
        if let Some(existing) = self.queues.get(&name) {
            return if existing.matches(&spec) {
                Ok(Outcome::Done(Empty {}))
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = spec
            .dead_letter
            .as_ref()
            .map(|request| self.resolve_dead_letter(request, fifo))
            .transpose()?;
        if self.queues.len() >= QUEUE_LIMIT {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("{QUEUE_LIMIT} queues already exist"),
            ));
        }
        let id = self.next_queue;
        self.next_queue = id.succ();
        self.queues.insert(name, Queue::new(id, &spec, dead_letter));
        Ok(Outcome::Done(Empty {}))
    }

    /// Turns a requested dead-letter configuration into one that points at an
    /// existing queue of the same type as the queue that will use it.
    fn resolve_dead_letter(
        &self,
        request: &DeadLetterRequest,
        fifo: bool,
    ) -> Result<DeadLetter, Error> {
        let (name, target) = self
            .queues
            .get_key_value(request.queue.as_str())
            .ok_or_else(|| no_such_queue(&request.queue))?;
        if target.is_fifo() != fifo {
            return Err(invalid_attributes(
                "the dead-letter queue must be of the same type as the queue",
            ));
        }
        Ok(DeadLetter {
            queue: name.clone(),
            max_receive_count: request.max_receive_count,
        })
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        attributes: Attributes,
    ) -> Result<Outcome, Error> {
        let fifo = lookup(&mut self.queues, self.now, name)?.is_fifo();
        if attributes.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid_attributes(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        let request =
            dead_letter_request(attributes.dead_letter_queue, attributes.max_receive_count)?;
        let dead_letter = request
            .map(|request| {
                let dead_letter = self.resolve_dead_letter(&request, fifo)?;
                if dead_letter.queue.as_str() == name {
                    return Err(invalid_attributes(
                        "a queue cannot be its own dead-letter queue",
                    ));
                }
                Ok(dead_letter)
            })
            .transpose()?;
        lookup(&mut self.queues, self.now, name)?.update(Update {
            visibility_timeout: attributes.visibility_timeout,
            delay: attributes.delay,
            retention: attributes.retention,
            content_based_deduplication: attributes.content_based_deduplication,
            dead_letter,
        });
        Ok(Outcome::Done(Empty {}))
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Outcome, Error> {
        let queue = lookup(&mut self.queues, self.now, name)?;
        Ok(Outcome::QueueAttributes(QueueAttributes {
            attributes: queue.attributes(),
            counts: queue.counts(self.now),
        }))
    }

    fn list_queues(&self, prefix: &str) -> Outcome {
        Outcome::QueueNames(QueueNames {
            queue_names: self
                .queues
                .keys()
                .filter(|name| name.as_str().starts_with(prefix))
                .cloned()
                .collect(),
        })
    }

    fn delete_queue(&mut self, name: &str) -> Result<Outcome, Error> {
        self.queues
            .remove(name)
            .ok_or_else(|| no_such_queue(name))?;
        for queue in self.queues.values_mut() {
            queue.forget_dead_letter(name);
        }
        Ok(Outcome::Done(Empty {}))
    }

    fn tick(&mut self, seconds: TickSeconds) -> Outcome {
        self.now = self.now.plus(seconds);
        Outcome::Clock(Clock { now: self.now })
    }

    fn send_message(&mut self, name: &str, message: NewMessage) -> Result<Outcome, Error> {
        let queue = lookup(&mut self.queues, self.now, name)?;
        let sent = match queue.send(self.now, self.next_message, message)? {
            SendOutcome::Accepted(sent) => {
                self.next_message = self.next_message.succ();
                sent
            }
            SendOutcome::Duplicate(original) => original,
        };
        Ok(Outcome::Sent(sent))
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: MaxMessages,
        visibility_timeout: Option<VisibilityTimeout>,
    ) -> Result<Outcome, Error> {
        let queue = lookup(&mut self.queues, self.now, name)?;
        let received = queue.receive(
            self.now,
            max_messages,
            visibility_timeout,
            &mut self.deliveries,
        );
        if let Some(dead) = received.dead_lettered {
            // The target exists: DeleteQueue clears every reference to it.
            let target = lookup(&mut self.queues, self.now, dead.target.as_str())?;
            for message in dead.messages {
                target.accept_dead_letter(self.now, message);
            }
        }
        Ok(Outcome::Messages(Messages {
            messages: received.messages,
        }))
    }
}
