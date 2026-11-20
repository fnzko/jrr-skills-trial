//! The state of the service and the meaning of each command (SPEC.md sections 3 to 6).

use std::collections::BTreeMap;

use crate::attributes::{AttributeChanges, Attributes};
use crate::command::{Command, ReceiveRequest, SendRequest};
use crate::queue::{Counts, Delivery, Queue};
use crate::{Error, ErrorCode};

const MAX_QUEUES: usize = 1000;

/// What a successful command returns, before it is rendered as JSON.
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    Empty,
    QueueAttributes {
        attributes: Attributes,
        counts: Counts,
    },
    QueueNames(Vec<String>),
    Now(u64),
    Sent {
        message_id: u64,
        sequence_number: Option<u64>,
    },
    Received(Vec<Delivery>),
}

#[derive(Debug)]
pub struct Broker {
    now: u64,
    /// Ordered by name, so listings come out in byte order.
    queues: BTreeMap<String, Queue>,
    /// The next `MessageId` number to hand out; numbers are not reused.
    next_message_id: u64,
    /// The next `ReceiptHandle` number to hand out; numbers are not reused.
    next_receipt: u64,
}

impl Broker {
    pub fn new() -> Self {
        Broker {
            now: 0,
            queues: BTreeMap::new(),
            next_message_id: 1,
            next_receipt: 1,
        }
    }

    pub fn execute(&mut self, command: Command) -> Result<Reply, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(&name, &attributes),
            Command::SetQueueAttributes { name, changes } => {
                self.set_queue_attributes(&name, &changes)
            }
            Command::GetQueueAttributes { name } => self.get_queue_attributes(&name),
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Command::DeleteQueue { name } => self.delete_queue(&name),
            Command::PurgeQueue { name } => self.purge_queue(&name),
            Command::Tick { seconds } => {
                self.now = self.now.saturating_add(u64::from(seconds));
                Ok(Reply::Now(self.now))
            }
            Command::SendMessage(request) => self.send_message(request),
            Command::ReceiveMessage(request) => self.receive_message(&request),
            Command::DeleteMessage { queue, receipt } => self.delete_message(&queue, &receipt),
            Command::ChangeMessageVisibility {
                queue,
                receipt,
                visibility_timeout,
            } => self.change_message_visibility(&queue, &receipt, visibility_timeout),
        }
    }

    fn create_queue(&mut self, name: &str, changes: &AttributeChanges) -> Result<Reply, Error> {
        let fifo = changes.fifo.unwrap_or(false);
        let content_based = changes.content_based_deduplication.unwrap_or(false);
        if is_fifo_name(name) != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is FIFO",
            ));
        }
        if content_based && !fifo {
            return Err(invalid_value(
                "ContentBasedDeduplication is only allowed on a FIFO queue",
            ));
        }
        let dead_letter = changes.dead_letter()?;
        let mut wanted = Attributes::default();
        wanted.apply(changes, dead_letter);

        if let Some(existing) = self.queues.get(name) {
            return if existing.attributes() == &wanted {
                Ok(Reply::Empty)
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {name} exists with different attributes"),
                ))
            };
        }
        if let Some(dead_letter) = &wanted.dead_letter {
            let target = self.existing(&dead_letter.queue)?;
            if target.attributes().fifo != fifo {
                return Err(invalid_value(
                    "the dead-letter queue must have the same FIFO type as this queue",
                ));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        self.queues.insert(name.to_owned(), Queue::new(wanted));
        Ok(Reply::Empty)
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        changes: &AttributeChanges,
    ) -> Result<Reply, Error> {
        let fifo = self.existing(name)?.attributes().fifo;
        if changes.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the FIFO type of a queue cannot be changed",
            ));
        }
        if changes.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid_value(
                "ContentBasedDeduplication is only allowed on a FIFO queue",
            ));
        }
        let dead_letter = changes.dead_letter()?;
        if let Some(dead_letter) = &dead_letter {
            if dead_letter.queue == name {
                return Err(invalid_value("a queue cannot be its own dead-letter queue"));
            }
            let target = self.existing(&dead_letter.queue)?;
            if target.attributes().fifo != fifo {
                return Err(invalid_value(
                    "the dead-letter queue must have the same FIFO type as this queue",
                ));
            }
        }
        // Expire under the old retention first: a retention change applies to
        // messages from this moment on, and cannot revive removed ones.
        live_queue(&mut self.queues, name, self.now)?.reconfigure(changes, dead_letter);
        Ok(Reply::Empty)
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Reply, Error> {
        let queue = live_queue(&mut self.queues, name, self.now)?;
        Ok(Reply::QueueAttributes {
            attributes: queue.attributes().clone(),
            counts: queue.counts(self.now),
        })
    }

    fn list_queues(&self, prefix: &str) -> Reply {
        Reply::QueueNames(
            self.queues
                .keys()
                .filter(|name| name.starts_with(prefix))
                .cloned()
                .collect(),
        )
    }

    fn delete_queue(&mut self, name: &str) -> Result<Reply, Error> {
        self.existing(name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            queue.detach_dead_letter(name);
        }
        Ok(Reply::Empty)
    }

    fn purge_queue(&mut self, name: &str) -> Result<Reply, Error> {
        live_queue(&mut self.queues, name, self.now)?.purge();
        Ok(Reply::Empty)
    }

    fn send_message(&mut self, request: SendRequest) -> Result<Reply, Error> {
        let now = self.now;
        let queue = live_queue(&mut self.queues, &request.queue, now)?;
        queue.check_send(&request)?;
        if let Some((message_id, sequence_number)) = queue.duplicate_of(now, &request) {
            return Ok(Reply::Sent {
                message_id,
                sequence_number: Some(sequence_number),
            });
        }
        queue.check_capacity()?;
        let message_id = self.next_message_id;
        self.next_message_id += 1;
        let sequence_number = queue.push(now, message_id, request);
        Ok(Reply::Sent {
            message_id,
            sequence_number,
        })
    }

    fn receive_message(&mut self, request: &ReceiveRequest) -> Result<Reply, Error> {
        let now = self.now;
        let queue = live_queue(&mut self.queues, &request.queue, now)?;
        let visibility_timeout = request
            .visibility_timeout
            .unwrap_or(queue.attributes().visibility_timeout);
        let dead_letter_queue = queue
            .attributes()
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.queue.clone());
        let outcome = queue.receive(
            now,
            request.max_messages,
            visibility_timeout,
            &mut self.next_receipt,
        );
        if !outcome.dead_letters.is_empty() {
            // Only a queue with a dead-letter queue moves messages, and DeleteQueue
            // clears every reference to a queue before it goes.
            let target = dead_letter_queue
                .expect("messages move only from a queue with a dead-letter queue");
            let dead_letters = self
                .queues
                .get_mut(&target)
                .expect("a dead-letter reference always names an existing queue");
            for message in outcome.dead_letters {
                dead_letters.accept_dead_letter(now, message);
            }
        }
        Ok(Reply::Received(outcome.deliveries))
    }

    fn delete_message(&mut self, name: &str, receipt: &str) -> Result<Reply, Error> {
        let now = self.now;
        let queue = live_queue(&mut self.queues, name, now)?;
        let receipt = parse_issued_receipt(receipt, self.next_receipt)?;
        queue.delete_message(now, receipt)?;
        Ok(Reply::Empty)
    }

    fn change_message_visibility(
        &mut self,
        name: &str,
        receipt: &str,
        visibility_timeout: u32,
    ) -> Result<Reply, Error> {
        let now = self.now;
        let queue = live_queue(&mut self.queues, name, now)?;
        let receipt = parse_issued_receipt(receipt, self.next_receipt)?;
        queue.change_visibility(now, receipt, visibility_timeout)?;
        Ok(Reply::Empty)
    }

    fn existing(&self, name: &str) -> Result<&Queue, Error> {
        self.queues.get(name).ok_or_else(|| does_not_exist(name))
    }
}

/// The named queue, with messages that have expired by `now` removed.
fn live_queue<'a>(
    queues: &'a mut BTreeMap<String, Queue>,
    name: &str,
    now: u64,
) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| does_not_exist(name))?;
    queue.advance(now);
    Ok(queue)
}

fn does_not_exist(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name} does not exist"),
    )
}

/// The number of a receipt handle this process has issued by `next_receipt`.
/// Handles are exact strings: `r-01`, `R-1` and `r-1 ` were never issued.
fn parse_issued_receipt(text: &str, next_receipt: u64) -> Result<u64, Error> {
    let number = text
        .strip_prefix("r-")
        .and_then(|digits| digits.parse::<u64>().ok())
        .filter(|number| format!("r-{number}") == text);
    match number {
        Some(number) if (1..next_receipt).contains(&number) => Ok(number),
        _ => Err(Error::new(
            ErrorCode::ReceiptHandleInvalid,
            "the receipt handle was never issued",
        )),
    }
}

fn is_fifo_name(name: &str) -> bool {
    name.ends_with(".fifo")
}

fn invalid_value(message: &str) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}
