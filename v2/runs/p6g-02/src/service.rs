use std::collections::BTreeMap;
use std::ops::Bound;

use crate::config::{AttributeSet, Config, QueueKind};
use crate::error::{Error, ErrorCode};
use crate::ids::{IdGenerator, ReceiptHandle};
use crate::queue::{FifoSend, MAX_MESSAGES, NewMessage, Queue};
use crate::request::Command;
use crate::response::{AttributesView, Outcome};

/// Most queues that may exist at once.
const MAX_QUEUES: usize = 1000;

/// All state of the process: the virtual clock, the id counters and the queues.
#[derive(Debug, Default)]
pub struct Service {
    now: u64,
    ids: IdGenerator,
    /// Ordered by name, which is the order `ListQueues` must answer in.
    queues: BTreeMap<String, Queue>,
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one command. A failed command leaves the state unchanged.
    pub fn execute(&mut self, command: Command) -> Result<Outcome, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => {
                let queue = self.queue(&queue)?;
                Ok(Outcome::Attributes {
                    attributes: AttributesView::of(queue.config()),
                    counts: queue.counts(self.now),
                })
            }
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                self.queue_mut(&queue)?.purge();
                Ok(Outcome::Done {})
            }
            Command::Tick { seconds } => {
                self.now += u64::from(seconds);
                for queue in self.queues.values_mut() {
                    queue.expire(self.now);
                }
                Ok(Outcome::Now { now: self.now })
            }
            Command::SendMessage {
                queue,
                body,
                delay_seconds,
                group_id,
                deduplication_id,
            } => self.send_message(&queue, body, delay_seconds, group_id, deduplication_id),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let handle = self.issued_handle(&queue, &receipt_handle)?;
                let now = self.now;
                if self.queue_mut(&queue)?.delete(now, handle) {
                    Ok(Outcome::Done {})
                } else {
                    Err(expired(&receipt_handle))
                }
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let handle = self.issued_handle(&queue, &receipt_handle)?;
                let now = self.now;
                if self
                    .queue_mut(&queue)?
                    .change_visibility(now, handle, visibility_timeout)
                {
                    Ok(Outcome::Done {})
                } else {
                    Err(expired(&receipt_handle))
                }
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

    /// Lookup of the queue (spec check 4) followed by the handle check that
    /// opens the rules of DeleteMessage and ChangeMessageVisibility.
    fn issued_handle(&self, queue: &str, text: &str) -> Result<ReceiptHandle, Error> {
        self.queue(queue)?;
        ReceiptHandle::parse_issued(text, self.ids.receipts_issued()).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("receipt handle {text:?} was never issued"),
            )
        })
    }

    fn create_queue(&mut self, name: String, attributes: &AttributeSet) -> Result<Outcome, Error> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.ends_with(".fifo") != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "the .fifo suffix and the Fifo attribute must agree",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                "requires a FIFO queue",
            ));
        }
        let config = Config::from_attributes(attributes, attributes.dead_letter()?);
        if let Some(existing) = self.queues.get(&name) {
            return if *existing.config() == config {
                Ok(Outcome::Done {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {name:?} exists with different attributes"),
                ))
            };
        }
        if let Some(dead_letter) = &config.dead_letter
            && self.queue(&dead_letter.queue)?.config().kind.is_fifo() != fifo
        {
            return Err(Error::invalid_value(
                "DeadLetterQueue",
                "must be of the same type as the queue",
            ));
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues may exist"),
            ));
        }
        self.queues.insert(name, Queue::new(config));
        Ok(Outcome::Done {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        attributes: &AttributeSet,
    ) -> Result<Outcome, Error> {
        let queue = self.queue(name)?;
        if attributes.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !queue.config().kind.is_fifo() {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                "requires a FIFO queue",
            ));
        }
        let dead_letter = attributes.dead_letter()?;
        if let Some(dead_letter) = &dead_letter {
            let target = self.queue(&dead_letter.queue)?;
            if dead_letter.queue == name
                || target.config().kind.is_fifo() != queue.config().kind.is_fifo()
            {
                return Err(Error::invalid_value(
                    "DeadLetterQueue",
                    "must be another queue of the same type",
                ));
            }
        }
        let config = queue.config().updated(attributes, dead_letter);
        let now = self.now;
        self.queue_mut(name)?.set_config(config, now);
        Ok(Outcome::Done {})
    }

    fn list_queues(&self, prefix: &str) -> Outcome {
        let queue_names = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name)
            .take_while(|name| name.starts_with(prefix))
            .cloned()
            .collect();
        Outcome::Queues { queue_names }
    }

    fn delete_queue(&mut self, name: &str) -> Result<Outcome, Error> {
        self.queue(name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            if queue
                .config()
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue == name)
            {
                queue.clear_dead_letter();
            }
        }
        Ok(Outcome::Done {})
    }

    fn send_message(
        &mut self,
        name: &str,
        body: String,
        delay_seconds: Option<u32>,
        group_id: Option<String>,
        deduplication_id: Option<String>,
    ) -> Result<Outcome, Error> {
        let now = self.now;
        let queue = self
            .queues
            .get_mut(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        let unsupported = |what: &str| Error::new(ErrorCode::UnsupportedOperation, what.to_owned());
        let fifo = match queue.config().kind {
            QueueKind::Standard => {
                if group_id.is_some() || deduplication_id.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                None
            }
            QueueKind::Fifo {
                content_based_deduplication,
            } => {
                if delay_seconds.is_some() {
                    return Err(unsupported(
                        "DelaySeconds per message needs a standard queue",
                    ));
                }
                let group_id = group_id.ok_or_else(|| Error::missing("GroupId"))?;
                let deduplication_id = match deduplication_id {
                    Some(id) => id,
                    None if content_based_deduplication => body.clone(),
                    None => return Err(Error::missing("DeduplicationId")),
                };
                Some(FifoSend {
                    group_id,
                    deduplication_id,
                })
            }
        };
        if let Some(send) = &fifo
            && let Some((message_id, sequence)) = queue.duplicate_of(now, &send.deduplication_id)
        {
            return Ok(Outcome::Sent {
                message_id,
                sequence_number: Some(sequence),
            });
        }
        if queue.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }
        let message_id = self.ids.next_message();
        let delay = delay_seconds.unwrap_or(queue.config().delay_seconds);
        let sequence_number = queue.enqueue(NewMessage {
            id: message_id,
            body,
            sent_at: now,
            delay,
            fifo,
        });
        Ok(Outcome::Sent {
            message_id,
            sequence_number,
        })
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: usize,
        visibility_timeout: Option<u32>,
    ) -> Result<Outcome, Error> {
        let now = self.now;
        let queue = self
            .queues
            .get_mut(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        let received = queue.receive(now, max_messages, visibility_timeout, &mut self.ids);
        if !received.dead_lettered.is_empty() {
            let target = queue
                .config()
                .dead_letter
                .as_ref()
                .expect("messages are only dead-lettered when a configuration exists")
                .queue
                .clone();
            let target = self.queues.get_mut(&target).expect(
                "a configured dead-letter queue exists: deleting it clears the configuration",
            );
            for message in received.dead_lettered {
                target.accept_dead_letter(message, now);
            }
        }
        Ok(Outcome::Received {
            messages: received.delivered,
        })
    }
}

fn expired(handle: &str) -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        format!("receipt handle {handle:?} is not in flight in this queue"),
    )
}
