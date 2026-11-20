//! The state of the whole process and the execution of commands.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::error::{Error, ErrorCode};
use crate::ids::{Ids, ReceiptHandle};
use crate::queue::{DeadLetter, MAX_MESSAGES, Message, Queue, QueueName, Settings};
use crate::request::{Attributes, Command, Send};
use crate::response::Outcome;

const MAX_QUEUES: usize = 1000;

/// All queues of the process together with the virtual clock.
#[derive(Debug, Default)]
pub struct Broker {
    now: u64,
    queues: BTreeMap<QueueName, Queue>,
    ids: Ids,
}

#[derive(Serialize)]
struct Success {
    ok: bool,
    #[serde(flatten)]
    outcome: Outcome,
}

#[derive(Serialize)]
struct Failure {
    ok: bool,
    error: Error,
}

fn no_such_queue(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

fn invalid_value(message: &str) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one input line (without its line ending) and returns the
    /// response line, without a line ending.
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        let response = match Command::parse(line).and_then(|command| self.execute(command)) {
            Ok(outcome) => serde_json::to_string(&Success { ok: true, outcome }),
            Err(error) => serde_json::to_string(&Failure { ok: false, error }),
        };
        response.expect("responses hold only strings, integers and booleans")
    }

    /// The queue called `name`, with its expired messages removed.
    fn queue<'a>(
        queues: &'a mut BTreeMap<QueueName, Queue>,
        now: u64,
        name: &str,
    ) -> Result<&'a mut Queue, Error> {
        let queue = queues.get_mut(name).ok_or_else(|| no_such_queue(name))?;
        queue.expire(now);
        Ok(queue)
    }

    fn execute(&mut self, command: Command) -> Result<Outcome, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => {
                let queue = Self::queue(&mut self.queues, self.now, &queue)?;
                Ok(Outcome::QueueAttributes {
                    attributes: queue.settings.view(),
                    counts: queue.counts(self.now),
                })
            }
            Command::ListQueues { prefix } => {
                let prefix = prefix.unwrap_or_default();
                let queue_names = self
                    .queues
                    .keys()
                    .map(QueueName::as_str)
                    .filter(|name| name.starts_with(&prefix))
                    .map(str::to_owned)
                    .collect();
                Ok(Outcome::QueueNames { queue_names })
            }
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                Self::queue(&mut self.queues, self.now, &queue)?.purge();
                Ok(Outcome::Empty {})
            }
            Command::Tick { seconds } => {
                self.now += u64::from(seconds);
                Ok(Outcome::Tick { now: self.now })
            }
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
                let (queue, key) = self.find_in_flight(&queue, &receipt_handle)?;
                queue.delete(key);
                Ok(Outcome::Empty {})
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let now = self.now;
                let (queue, key) = self.find_in_flight(&queue, &receipt_handle)?;
                queue.change_visibility(key, now, visibility_timeout);
                Ok(Outcome::Empty {})
            }
        }
    }

    /// Looks up the dead-letter queue named in a request and whether it is
    /// a FIFO queue.
    fn dead_letter(&self, name: &str, max_receive_count: u32) -> Result<(DeadLetter, bool), Error> {
        let (key, queue) = self
            .queues
            .get_key_value(name)
            .ok_or_else(|| no_such_queue(name))?;
        let dead_letter = DeadLetter {
            queue: key.clone(),
            max_receive_count,
        };
        Ok((dead_letter, queue.settings.fifo))
    }

    fn create_queue(&mut self, name: QueueName, attributes: &Attributes) -> Result<Outcome, Error> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when Fifo is true",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter_config = dead_letter_config(attributes)?;
        if let Some(existing) = self.queues.get(&name) {
            return if existing.settings.matches(attributes) {
                Ok(Outcome::Empty {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with other attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = match dead_letter_config {
            Some((queue, max_receive_count)) => {
                let (dead_letter, dead_letter_fifo) = self.dead_letter(queue, max_receive_count)?;
                if dead_letter_fifo != fifo {
                    return Err(invalid_value(
                        "DeadLetterQueue must be of the same type as the queue",
                    ));
                }
                Some(dead_letter)
            }
            None => None,
        };
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "the limit of 1000 queues is reached",
            ));
        }
        self.queues
            .insert(name, Queue::new(Settings::new(attributes, dead_letter)));
        Ok(Outcome::Empty {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        attributes: &Attributes,
    ) -> Result<Outcome, Error> {
        let fifo = Self::queue(&mut self.queues, self.now, name)?.settings.fifo;
        if attributes.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = match dead_letter_config(attributes)? {
            Some((queue, max_receive_count)) => {
                let (dead_letter, dead_letter_fifo) = self.dead_letter(queue, max_receive_count)?;
                if queue == name || dead_letter_fifo != fifo {
                    return Err(invalid_value(
                        "DeadLetterQueue must be another queue of the same type",
                    ));
                }
                Some(dead_letter)
            }
            None => None,
        };
        Self::queue(&mut self.queues, self.now, name)?
            .settings
            .update(attributes, dead_letter);
        Ok(Outcome::Empty {})
    }

    fn delete_queue(&mut self, name: &str) -> Result<Outcome, Error> {
        self.queues
            .remove(name)
            .ok_or_else(|| no_such_queue(name))?;
        for queue in self.queues.values_mut() {
            if queue
                .settings
                .dead_letter
                .as_ref()
                .is_some_and(|d| d.queue.as_str() == name)
            {
                queue.settings.dead_letter = None;
            }
        }
        Ok(Outcome::Empty {})
    }

    fn send_message(&mut self, send: Send) -> Result<Outcome, Error> {
        let now = self.now;
        let queue = Self::queue(&mut self.queues, now, &send.queue)?;
        let fifo = queue.settings.fifo;
        let unsupported = if fifo {
            send.delay_seconds
                .map(|_| "DelaySeconds is not supported on a FIFO queue")
        } else if send.group_id.is_some() || send.deduplication_id.is_some() {
            Some("GroupId and DeduplicationId need a FIFO queue")
        } else {
            None
        };
        if let Some(message) = unsupported {
            return Err(Error::new(ErrorCode::UnsupportedOperation, message));
        }
        let missing =
            |name: &str| Error::new(ErrorCode::MissingParameter, format!("{name} is required"));
        let deduplication_id = if fifo {
            if send.group_id.is_none() {
                return Err(missing("GroupId"));
            }
            let id = match send.deduplication_id {
                Some(id) => id,
                None if queue.settings.content_based_deduplication => send.body.clone(),
                None => return Err(missing("DeduplicationId")),
            };
            if let Some(original) = queue.duplicate_of(&id, now) {
                return Ok(Outcome::Sent {
                    message_id: original.message_id,
                    sequence_number: original.sequence,
                });
            }
            Some(id)
        } else {
            None
        };
        if queue.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "the queue holds 100000 messages",
            ));
        }
        let message_id = self.ids.next_message();
        let delay = send.delay_seconds.unwrap_or(queue.settings.delay_seconds);
        let sequence = queue.push(Message::new(
            message_id,
            send.body,
            send.group_id,
            now,
            delay,
        ));
        if let Some(id) = deduplication_id {
            queue.record_accepted(id, message_id, sequence, now);
        }
        Ok(Outcome::Sent {
            message_id,
            sequence_number: sequence,
        })
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u32,
        visibility_timeout: Option<u32>,
    ) -> Result<Outcome, Error> {
        let now = self.now;
        let queue = Self::queue(&mut self.queues, now, name)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.settings.visibility_timeout);
        let max = usize::try_from(max_messages).expect("MaxMessages is at most 10");
        let received = queue.receive(now, max, visibility_timeout, &mut self.ids);
        if !received.dead.is_empty() {
            let target = queue
                .settings
                .dead_letter
                .as_ref()
                .map(|d| d.queue.clone())
                .expect("messages are only dead-lettered when a dead-letter queue is set");
            let target = self
                .queues
                .get_mut(&target)
                .expect("DeleteQueue clears every reference to a deleted dead-letter queue");
            for message in received.dead {
                target.push_dead_letter(message);
            }
        }
        Ok(Outcome::Received {
            messages: received.delivered,
        })
    }

    /// Checks a receipt handle for `DeleteMessage` and
    /// `ChangeMessageVisibility` and returns the queue and message it names.
    fn find_in_flight(&mut self, name: &str, handle: &str) -> Result<(&mut Queue, u64), Error> {
        let queue = Self::queue(&mut self.queues, self.now, name)?;
        let handle = ReceiptHandle::parse(handle, self.ids.receipts_issued()).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("receipt handle {handle:?} was never issued"),
            )
        })?;
        let key = queue.in_flight(handle, self.now).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleExpired,
                "the receipt handle is not the latest handle of an in-flight message",
            )
        })?;
        Ok((queue, key))
    }
}

/// The dead-letter configuration of a request: both attributes or none.
fn dead_letter_config(attributes: &Attributes) -> Result<Option<(&str, u32)>, Error> {
    match (&attributes.dead_letter_queue, attributes.max_receive_count) {
        (Some(queue), Some(max)) => Ok(Some((queue, max))),
        (None, None) => Ok(None),
        _ => Err(invalid_value(
            "DeadLetterQueue and MaxReceiveCount go together",
        )),
    }
}
