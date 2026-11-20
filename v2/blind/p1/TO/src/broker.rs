//! All process state and the execution of commands against it.

use std::collections::BTreeMap;
use std::ops::Bound;

use crate::error::{Error, ErrorCode};
use crate::queue::{FifoSend, MessageId, NewMessage, Queue, QueueConfig, QueueId};
use crate::receipt::DeliveryLog;
use crate::request::{self, AttributeUpdate, Command, SendMessage};
use crate::response::{self, AttributesView, Reply};

const MAX_QUEUES: usize = 1000;
const MAX_MESSAGES_PER_QUEUE: usize = 100_000;
const FIFO_SUFFIX: &str = ".fifo";

#[derive(Debug, Default)]
pub struct Broker {
    now: u64,
    /// Ordered by name, which is the byte order `ListQueues` must return.
    queues: BTreeMap<String, Queue>,
    queues_created: u64,
    messages_accepted: u64,
    deliveries: DeliveryLog,
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one input line and returns the response line, without its newline.
    pub fn respond(&mut self, line: &[u8]) -> String {
        response::render(request::parse(line).and_then(|command| self.execute(command)))
    }

    fn execute(&mut self, command: Command) -> Result<Reply, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref())),
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                self.queue_mut(&queue)?.purge();
                Ok(Reply::Done {})
            }
            Command::Tick { seconds } => Ok(self.tick(seconds)),
            Command::SendMessage(send) => self.send_message(send),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let (queue, sequence) = self.locate(&queue, &receipt_handle)?;
                queue.delete(sequence);
                Ok(Reply::Done {})
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let now = self.now;
                let (queue, sequence) = self.locate(&queue, &receipt_handle)?;
                queue.change_visibility(sequence, now, visibility_timeout);
                Ok(Reply::Done {})
            }
        }
    }

    fn queue(&self, name: &str) -> Result<&Queue, Error> {
        self.queues
            .get(name)
            .ok_or_else(|| Error::no_such_queue(name))
    }

    fn queue_mut(&mut self, name: &str) -> Result<&mut Queue, Error> {
        self.queues
            .get_mut(name)
            .ok_or_else(|| Error::no_such_queue(name))
    }

    fn create_queue(&mut self, name: String, update: &AttributeUpdate) -> Result<Reply, Error> {
        let fifo = update.fifo.unwrap_or(false);
        if name.ends_with(FIFO_SUFFIX) != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is FIFO",
            ));
        }
        if update.content_based_deduplication == Some(true) && !fifo {
            return Err(content_based_deduplication_needs_fifo());
        }
        let config = QueueConfig::new(update, update.dead_letter()?);
        if let Some(existing) = self.queues.get(&name) {
            return if *existing.config() == config {
                Ok(Reply::Done {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {name:?} exists with different attributes"),
                ))
            };
        }
        if let Some(dead_letter) = &config.dead_letter {
            self.check_dead_letter_target(&dead_letter.queue, fifo)?;
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        self.queues_created += 1;
        let queue = Queue::new(QueueId::new(self.queues_created), config);
        self.queues.insert(name, queue);
        Ok(Reply::Done {})
    }

    /// The dead-letter queue must exist and be of the same type as the queue using it.
    fn check_dead_letter_target(&self, target: &str, fifo: bool) -> Result<(), Error> {
        if self.queue(target)?.config().fifo == fifo {
            Ok(())
        } else {
            Err(Error::invalid_value(
                "a queue and its dead-letter queue must both be FIFO or both be standard",
            ))
        }
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        update: &AttributeUpdate,
    ) -> Result<Reply, Error> {
        let fifo = self.queue(name)?.config().fifo;
        if update.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if update.content_based_deduplication == Some(true) && !fifo {
            return Err(content_based_deduplication_needs_fifo());
        }
        let dead_letter = update.dead_letter()?;
        if let Some(dead_letter) = &dead_letter {
            if dead_letter.queue == name {
                return Err(Error::invalid_value(
                    "a queue cannot be its own dead-letter queue",
                ));
            }
            self.check_dead_letter_target(&dead_letter.queue, fifo)?;
        }
        let now = self.now;
        self.queue_mut(name)?
            .set_attributes(update, dead_letter, now);
        Ok(Reply::Done {})
    }

    fn get_queue_attributes(&self, name: &str) -> Result<Reply, Error> {
        let queue = self.queue(name)?;
        let config = queue.config();
        let dead_letter = config.dead_letter.as_ref();
        Ok(Reply::Attributes {
            attributes: AttributesView {
                visibility_timeout: config.visibility_timeout,
                delay_seconds: config.delay_seconds,
                retention_seconds: config.retention_seconds,
                fifo: config.fifo,
                content_based_deduplication: config.content_based_deduplication,
                dead_letter_queue: dead_letter.map(|d| d.queue.clone()),
                max_receive_count: dead_letter.map(|d| d.max_receive_count),
            },
            counts: queue.counts(self.now),
        })
    }

    fn list_queues(&self, prefix: Option<&str>) -> Reply {
        let prefix = prefix.unwrap_or("");
        let queue_names = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name)
            .take_while(|name| name.starts_with(prefix))
            .cloned()
            .collect();
        Reply::QueueNames { queue_names }
    }

    fn delete_queue(&mut self, name: &str) -> Result<Reply, Error> {
        self.queue(name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            if queue
                .config()
                .dead_letter
                .as_ref()
                .is_some_and(|d| d.queue == name)
            {
                queue.clear_dead_letter();
            }
        }
        Ok(Reply::Done {})
    }

    fn tick(&mut self, seconds: u64) -> Reply {
        self.now += seconds;
        for queue in self.queues.values_mut() {
            queue.sweep(self.now);
        }
        Reply::Now { now: self.now }
    }

    fn send_message(&mut self, send: SendMessage) -> Result<Reply, Error> {
        let now = self.now;
        let queue = self
            .queues
            .get_mut(&send.queue)
            .ok_or_else(|| Error::no_such_queue(&send.queue))?;
        let config = queue.config();
        let fifo = if config.fifo {
            if send.delay_seconds.is_some() {
                return Err(unsupported("DelaySeconds on a FIFO queue"));
            }
            let group_id = send.group_id.ok_or_else(|| Error::missing("GroupId"))?;
            let deduplication_id = match send.deduplication_id {
                Some(id) => id,
                None if config.content_based_deduplication => send.body.clone(),
                None => return Err(Error::missing("DeduplicationId")),
            };
            if let Some(original) = queue.duplicate_of(&deduplication_id, now) {
                return Ok(sent(original.id, Some(original.sequence)));
            }
            Some(FifoSend {
                group_id,
                deduplication_id,
            })
        } else {
            if send.group_id.is_some() || send.deduplication_id.is_some() {
                return Err(unsupported(
                    "GroupId and DeduplicationId on a standard queue",
                ));
            }
            None
        };
        if queue.len() >= MAX_MESSAGES_PER_QUEUE {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES_PER_QUEUE} messages"),
            ));
        }
        self.messages_accepted += 1;
        let id = MessageId::new(self.messages_accepted);
        let is_fifo = fifo.is_some();
        let new = NewMessage {
            id,
            body: send.body,
            delay_seconds: send.delay_seconds.unwrap_or(queue.config().delay_seconds),
            fifo,
        };
        let sequence = queue.accept(new, now);
        Ok(sent(id, is_fifo.then_some(sequence)))
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u64,
        visibility_timeout: Option<u64>,
    ) -> Result<Reply, Error> {
        let now = self.now;
        let queue = self
            .queues
            .get_mut(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        let received = queue.receive(now, max_messages, visibility_timeout, &mut self.deliveries);
        if !received.dead.is_empty() {
            let target = queue
                .config()
                .dead_letter
                .as_ref()
                .expect("messages are only dead-lettered when a dead-letter queue is configured")
                .queue
                .clone();
            let target = self
                .queues
                .get_mut(&target)
                .expect("deleting a queue clears every reference to it");
            for message in received.dead {
                target.accept_dead_letter(message, now);
            }
        }
        Ok(Reply::Messages {
            messages: received.delivered,
        })
    }

    /// Finds the in-flight message a receipt handle currently refers to.
    fn locate(&mut self, queue: &str, handle: &str) -> Result<(&mut Queue, u64), Error> {
        let now = self.now;
        let queue = self
            .queues
            .get_mut(queue)
            .ok_or_else(|| Error::no_such_queue(queue))?;
        let delivery = self.deliveries.resolve(handle)?;
        if delivery.queue == queue.id()
            && queue.is_in_flight(delivery.sequence, delivery.handle, now)
        {
            Ok((queue, delivery.sequence))
        } else {
            Err(Error::new(
                ErrorCode::ReceiptHandleExpired,
                format!("receipt handle {handle:?} is no longer usable"),
            ))
        }
    }
}

fn sent(id: MessageId, sequence_number: Option<u64>) -> Reply {
    Reply::Sent {
        message_id: id,
        sequence_number,
    }
}

fn unsupported(what: &str) -> Error {
    Error::new(
        ErrorCode::UnsupportedOperation,
        format!("{what} is not supported"),
    )
}

fn content_based_deduplication_needs_fifo() -> Error {
    Error::invalid_value("ContentBasedDeduplication requires a FIFO queue")
}
