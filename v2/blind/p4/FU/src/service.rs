//! The service state: every queue, the virtual clock and the process-wide id counters.
//!
//! [`Service::execute`] runs a parsed [`Request`] and implements SPEC section 2 steps 4 and 5
//! (queue lookup, then each command's numbered rules). A failed command changes no state: every
//! check runs before the first mutation.

use std::collections::BTreeMap;

use crate::error::{Error, ErrorCode, Result};
use crate::queue::{Attributes, Counter, Counts, DeadLetterConfig, Delivery, Message, Queue};
use crate::request::{AttributeChanges, ReceiveMessage, Request, SendMessage};

pub const MAX_QUEUES: usize = 1000;
pub const MAX_MESSAGES_PER_QUEUE: usize = 100_000;

/// The result members of a successful command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// No result members.
    Done,
    Now(u64),
    QueueNames(Vec<String>),
    QueueAttributes {
        attributes: Attributes,
        counts: Counts,
    },
    Sent {
        message_id: u64,
        sequence_number: Option<u64>,
    },
    Messages(Vec<Delivery>),
}

#[derive(Debug, Default)]
pub struct Service {
    now: u64,
    /// Ordered by name, which is the order `ListQueues` reports.
    queues: BTreeMap<String, Queue>,
    message_ids: Counter,
    receipts: Counter,
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn now(&self) -> u64 {
        self.now
    }

    pub fn execute(&mut self, request: Request) -> Result<Response> {
        match request {
            Request::CreateQueue {
                queue_name,
                attributes,
            } => self.create_queue(queue_name, attributes),
            Request::SetQueueAttributes {
                queue_name,
                attributes,
            } => self.set_queue_attributes(&queue_name, attributes),
            Request::GetQueueAttributes { queue_name } => {
                let queue = self.queue(&queue_name)?;
                Ok(Response::QueueAttributes {
                    attributes: queue.attributes().clone(),
                    counts: queue.counts(),
                })
            }
            Request::ListQueues { prefix } => {
                let prefix = prefix.as_deref().unwrap_or("");
                Ok(Response::QueueNames(
                    self.queues
                        .keys()
                        .filter(|name| name.starts_with(prefix))
                        .cloned()
                        .collect(),
                ))
            }
            Request::DeleteQueue { queue_name } => self.delete_queue(&queue_name),
            Request::PurgeQueue { queue_name } => {
                self.queue_mut(&queue_name)?.purge();
                Ok(Response::Done)
            }
            Request::Tick { seconds } => {
                self.now += seconds;
                for queue in self.queues.values_mut() {
                    queue.settle(self.now);
                }
                Ok(Response::Now(self.now))
            }
            Request::SendMessage(send) => self.send_message(send),
            Request::ReceiveMessage(receive) => self.receive_message(receive),
            Request::DeleteMessage {
                queue_name,
                receipt_handle,
            } => {
                let receipt = self.check_receipt(&queue_name, &receipt_handle)?;
                self.queue_mut(&queue_name)?
                    .delete(receipt)
                    .map_err(|_| expired(&receipt_handle))?;
                Ok(Response::Done)
            }
            Request::ChangeMessageVisibility {
                queue_name,
                receipt_handle,
                visibility_timeout,
            } => {
                let receipt = self.check_receipt(&queue_name, &receipt_handle)?;
                let now = self.now;
                self.queue_mut(&queue_name)?
                    .change_visibility(receipt, visibility_timeout, now)
                    .map_err(|_| expired(&receipt_handle))?;
                Ok(Response::Done)
            }
        }
    }

    fn queue(&self, name: &str) -> Result<&Queue> {
        self.queues
            .get(name)
            .ok_or_else(|| Error::no_such_queue(name))
    }

    fn queue_mut(&mut self, name: &str) -> Result<&mut Queue> {
        self.queues
            .get_mut(name)
            .ok_or_else(|| Error::no_such_queue(name))
    }

    fn create_queue(&mut self, name: String, changes: AttributeChanges) -> Result<Response> {
        let fifo = changes.fifo.unwrap_or(false);
        if name.ends_with(".fifo") != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when Fifo is true",
            ));
        }
        if changes.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        let dead_letter = dead_letter_config(&changes)?;
        let attributes = Attributes {
            visibility_timeout: changes
                .visibility_timeout
                .unwrap_or(Attributes::default().visibility_timeout),
            delay_seconds: changes
                .delay_seconds
                .unwrap_or(Attributes::default().delay_seconds),
            retention_seconds: changes
                .retention_seconds
                .unwrap_or(Attributes::default().retention_seconds),
            fifo,
            content_based_deduplication: changes.content_based_deduplication.unwrap_or(false),
            dead_letter,
        };

        if let Some(existing) = self.queues.get(&name) {
            return if *existing.attributes() == attributes {
                Ok(Response::Done)
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {name:?} exists with different attributes"),
                ))
            };
        }
        if let Some(config) = &attributes.dead_letter {
            self.check_dead_letter_target(&config.queue_name, fifo)?;
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues may exist"),
            ));
        }
        self.queues.insert(name, Queue::new(attributes));
        Ok(Response::Done)
    }

    fn set_queue_attributes(&mut self, name: &str, changes: AttributeChanges) -> Result<Response> {
        let queue = self.queue(name)?;
        let fifo = queue.is_fifo();
        if changes.fifo.is_some() {
            return Err(Error::unsupported("the type of a queue never changes"));
        }
        if changes.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        let dead_letter = dead_letter_config(&changes)?;
        if let Some(config) = &dead_letter {
            self.check_dead_letter_target(&config.queue_name, fifo)?;
            if config.queue_name == name {
                return Err(Error::invalid_value(
                    "a queue cannot be its own dead-letter queue",
                ));
            }
        }

        let now = self.now;
        let queue = self.queue_mut(name)?;
        let attributes = queue.attributes_mut();
        if let Some(value) = changes.visibility_timeout {
            attributes.visibility_timeout = value;
        }
        if let Some(value) = changes.delay_seconds {
            attributes.delay_seconds = value;
        }
        if let Some(value) = changes.retention_seconds {
            attributes.retention_seconds = value;
        }
        if let Some(value) = changes.content_based_deduplication {
            attributes.content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            attributes.dead_letter = dead_letter;
        }
        // A lower retention takes effect at once.
        queue.settle(now);
        Ok(Response::Done)
    }

    /// Checks that a dead-letter queue exists and has the same type as its source.
    fn check_dead_letter_target(&self, name: &str, fifo: bool) -> Result<()> {
        if self.queue(name)?.is_fifo() != fifo {
            return Err(Error::invalid_value(
                "a dead-letter queue must have the same type as its source queue",
            ));
        }
        Ok(())
    }

    fn delete_queue(&mut self, name: &str) -> Result<Response> {
        self.queues
            .remove(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        for queue in self.queues.values_mut() {
            let attributes = queue.attributes_mut();
            if attributes
                .dead_letter
                .as_ref()
                .is_some_and(|config| config.queue_name == name)
            {
                attributes.dead_letter = None;
            }
        }
        Ok(Response::Done)
    }

    fn send_message(&mut self, send: SendMessage) -> Result<Response> {
        let now = self.now;
        let queue = self
            .queues
            .get_mut(&send.queue_name)
            .ok_or_else(|| Error::no_such_queue(&send.queue_name))?;
        let &Attributes {
            fifo,
            content_based_deduplication,
            delay_seconds: queue_delay,
            ..
        } = queue.attributes();

        let deduplication_id = if fifo {
            if send.delay_seconds.is_some() {
                return Err(Error::unsupported(
                    "FIFO queues do not support per-message DelaySeconds",
                ));
            }
            if send.group_id.is_none() {
                return Err(Error::missing("GroupId"));
            }
            let id = match send.deduplication_id {
                Some(id) => id,
                None if content_based_deduplication => send.body.clone(),
                None => return Err(Error::missing("DeduplicationId")),
            };
            if let Some(original) = queue.find_duplicate(&id, now) {
                return Ok(Response::Sent {
                    message_id: original.message_id,
                    sequence_number: Some(original.sequence_number),
                });
            }
            Some(id)
        } else {
            if send.group_id.is_some() || send.deduplication_id.is_some() {
                return Err(Error::unsupported(
                    "standard queues do not support GroupId or DeduplicationId",
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
        let delay = send.delay_seconds.unwrap_or(queue_delay);
        let message_id = self.message_ids.issue();
        let message = Message::new(message_id, send.body, now, send.group_id);
        let sequence_number = queue.push(message, delay, deduplication_id, now);
        Ok(Response::Sent {
            message_id,
            sequence_number,
        })
    }

    fn receive_message(&mut self, receive: ReceiveMessage) -> Result<Response> {
        let now = self.now;
        let queue = self
            .queues
            .get_mut(&receive.queue_name)
            .ok_or_else(|| Error::no_such_queue(&receive.queue_name))?;
        let attributes = queue.attributes();
        let timeout = receive
            .visibility_timeout
            .unwrap_or(attributes.visibility_timeout);
        let dead_letter = attributes.dead_letter.clone();
        let max_messages =
            usize::try_from(receive.max_messages).expect("MaxMessages is at most 10");

        let received = queue.receive(
            max_messages,
            timeout,
            dead_letter.as_ref().map(|config| config.max_receive_count),
            &mut self.receipts,
            now,
        );

        if let Some(config) = dead_letter {
            let target = self
                .queues
                .get_mut(&config.queue_name)
                .expect("deleting a queue clears every dead-letter configuration naming it");
            for message in received.dead_letters {
                // Dead-letter moves skip deduplication and the message limit.
                target.push(message, 0, None, now);
            }
            // A message already past the target's retention is removed on arrival.
            target.settle(now);
        }
        Ok(Response::Messages(received.deliveries))
    }

    /// Checks that `handle` was issued by this process and returns its number.
    fn check_receipt(&self, queue_name: &str, handle: &str) -> Result<u64> {
        self.queue(queue_name)?;
        parse_receipt(handle)
            .filter(|&n| n <= self.receipts.last())
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("receipt handle {handle:?} was never issued"),
                )
            })
    }
}

/// Builds the dead-letter configuration of an attribute update; both halves or neither.
fn dead_letter_config(changes: &AttributeChanges) -> Result<Option<DeadLetterConfig>> {
    match (&changes.dead_letter_queue, changes.max_receive_count) {
        (Some(queue_name), Some(max_receive_count)) => Ok(Some(DeadLetterConfig {
            queue_name: queue_name.clone(),
            max_receive_count,
        })),
        (None, None) => Ok(None),
        _ => Err(Error::invalid_value(
            "DeadLetterQueue and MaxReceiveCount must be given together",
        )),
    }
}

/// Parses the canonical form `r-<n>`: `n` in decimal without sign or leading zeros.
fn parse_receipt(handle: &str) -> Option<u64> {
    let digits = handle.strip_prefix("r-")?;
    let canonical = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && !digits.starts_with('0');
    canonical.then(|| digits.parse().ok()).flatten()
}

fn expired(handle: &str) -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        format!("receipt handle {handle:?} is no longer usable"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipts_must_be_canonical() {
        assert_eq!(parse_receipt("r-1"), Some(1));
        assert_eq!(parse_receipt("r-1234"), Some(1234));
        for bad in [
            "r-", "r-01", "R-1", "r-1 ", " r-1", "r-+1", "r--1", "r-0", "m-1",
        ] {
            assert_eq!(parse_receipt(bad), None, "{bad:?}");
        }
        assert_eq!(parse_receipt("r-99999999999999999999999"), None);
    }
}
