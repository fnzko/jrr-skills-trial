//! The broker: every queue, the virtual clock and the identifier counters.
//!
//! Methods take already-validated, typed arguments and apply the numbered
//! rules of the spec in order. A failing method leaves the state unchanged.

use std::collections::HashMap;

use crate::attributes::AttributeSet;
use crate::error::{Error, ErrorCode, Result};
use crate::queue::{
    Arrival, Counts, DeadLetter, FifoInfo, MAX_MESSAGES_PER_QUEUE, Message, Queue, QueueName,
    QueueType, QueueUid, Settings,
};
use crate::types::{MessageId, ReceiptHandle, Seconds, SequenceNumber};

pub const MAX_QUEUES: usize = 1000;

/// Where a delivery's handle points to.
#[derive(Debug, Clone, Copy)]
struct HandleTarget {
    queue: QueueUid,
    arrival: Arrival,
}

#[derive(Debug, Default)]
pub struct Broker {
    now: Seconds,
    queues: HashMap<QueueName, Queue>,
    next_message_number: u64,
    next_queue_number: u64,
    /// Every handle ever issued, in issue order: `r-n` is at index `n - 1`.
    handles: Vec<HandleTarget>,
}

pub struct SendRequest<'a> {
    pub body: &'a str,
    pub delay: Option<Seconds>,
    pub group: Option<&'a str>,
    pub dedup_id: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accepted {
    pub id: MessageId,
    pub sequence: Option<SequenceNumber>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub id: MessageId,
    pub receipt: ReceiptHandle,
    pub body: String,
    pub receive_count: u32,
    pub sent_at: Seconds,
    pub first_received_at: Option<Seconds>,
    pub fifo: Option<FifoInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueView {
    pub settings: Settings,
    pub counts: Counts,
}

fn no_such_queue(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Finds a queue and drops its expired messages first, so every command
    /// sees the queue as it is at `now`.
    fn lookup<'a>(
        queues: &'a mut HashMap<QueueName, Queue>,
        now: Seconds,
        name: &str,
    ) -> Result<&'a mut Queue> {
        let queue = queues.get_mut(name).ok_or_else(|| no_such_queue(name))?;
        queue.expire(now);
        Ok(queue)
    }

    pub fn tick(&mut self, seconds: Seconds) -> Seconds {
        self.now = self.now.plus(seconds);
        self.now
    }

    pub fn create_queue(&mut self, name: QueueName, attributes: &AttributeSet) -> Result<()> {
        if name.has_fifo_suffix() != attributes.fifo.unwrap_or(false) {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo if and only if Fifo is true",
            ));
        }
        if attributes.content_based_dedup == Some(true) && attributes.fifo != Some(true) {
            return Err(invalid_value(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        let dead_letter = attributes.dead_letter_pair()?;

        if let Some(existing) = self.queues.get(name.as_str()) {
            return if attributes.describes(existing.settings()) {
                Ok(())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }

        let is_fifo = attributes.fifo.unwrap_or(false);
        let dead_letter = dead_letter
            .map(|(queue, max_receive_count)| {
                let (key, target) = self
                    .queues
                    .get_key_value(queue)
                    .ok_or_else(|| no_such_queue(queue))?;
                if target.settings().queue_type.is_fifo() != is_fifo {
                    return Err(invalid_value(
                        "the dead-letter queue must be of the same type as the queue",
                    ));
                }
                Ok(DeadLetter {
                    queue: key.clone(),
                    max_receive_count,
                })
            })
            .transpose()?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues may exist"),
            ));
        }

        let settings = attributes.new_settings(dead_letter);
        self.next_queue_number += 1;
        let queue = Queue::new(QueueUid::new(self.next_queue_number), settings);
        self.queues.insert(name, queue);
        Ok(())
    }

    pub fn set_queue_attributes(&mut self, name: &str, attributes: &AttributeSet) -> Result<()> {
        let queue_type = Self::lookup(&mut self.queues, self.now, name)?
            .settings()
            .queue_type;
        if attributes.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot be changed",
            ));
        }
        if attributes.content_based_dedup == Some(true) && !queue_type.is_fifo() {
            return Err(invalid_value(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        let dead_letter = match attributes.dead_letter_pair()? {
            None => None,
            Some((target_name, max_receive_count)) => {
                let (key, target) = self
                    .queues
                    .get_key_value(target_name)
                    .ok_or_else(|| no_such_queue(target_name))?;
                if target_name == name
                    || target.settings().queue_type.is_fifo() != queue_type.is_fifo()
                {
                    return Err(invalid_value(
                        "the dead-letter queue must be another queue of the same type",
                    ));
                }
                Some(DeadLetter {
                    queue: key.clone(),
                    max_receive_count,
                })
            }
        };

        let queue = Self::lookup(&mut self.queues, self.now, name)?;
        let updated = attributes.apply_to(queue.settings(), dead_letter);
        queue.set_settings(updated);
        queue.expire(self.now);
        Ok(())
    }

    pub fn queue_view(&mut self, name: &str) -> Result<QueueView> {
        let queue = Self::lookup(&mut self.queues, self.now, name)?;
        Ok(QueueView {
            settings: queue.settings().clone(),
            counts: queue.counts(self.now),
        })
    }

    /// Names starting with `prefix`, in ascending byte order.
    pub fn list_queues(&self, prefix: &str) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .queues
            .keys()
            .map(QueueName::as_str)
            .filter(|name| name.starts_with(prefix))
            .collect();
        names.sort_unstable();
        names
    }

    pub fn delete_queue(&mut self, name: &str) -> Result<()> {
        self.queues
            .remove(name)
            .ok_or_else(|| no_such_queue(name))?;
        for queue in self.queues.values_mut() {
            if queue
                .settings()
                .dead_letter
                .as_ref()
                .is_some_and(|d| d.queue.as_str() == name)
            {
                let mut settings = queue.settings().clone();
                settings.dead_letter = None;
                queue.set_settings(settings);
            }
        }
        Ok(())
    }

    pub fn purge_queue(&mut self, name: &str) -> Result<()> {
        Self::lookup(&mut self.queues, self.now, name)?.purge();
        Ok(())
    }

    pub fn send_message(&mut self, name: &str, request: &SendRequest<'_>) -> Result<Accepted> {
        let now = self.now;
        let queue = Self::lookup(&mut self.queues, now, name)?;
        let queue_type = queue.settings().queue_type;
        let queue_delay = queue.settings().delay;

        let (delay, fifo_ids) = match queue_type {
            QueueType::Standard => {
                if request.group.is_some() || request.dedup_id.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                (request.delay.unwrap_or(queue_delay), None)
            }
            QueueType::Fifo {
                content_based_dedup,
            } => {
                if request.delay.is_some() {
                    return Err(unsupported("DelaySeconds is not supported on a FIFO queue"));
                }
                let group = request.group.ok_or_else(|| missing("GroupId"))?;
                let dedup_id = match request.dedup_id {
                    Some(id) => id,
                    None if content_based_dedup => request.body,
                    None => return Err(missing("DeduplicationId")),
                };
                if let Some((id, sequence)) = queue.recent_duplicate(dedup_id, now) {
                    return Ok(Accepted {
                        id,
                        sequence: Some(sequence),
                    });
                }
                (queue_delay, Some((group, dedup_id)))
            }
        };
        if queue.len() >= MAX_MESSAGES_PER_QUEUE {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES_PER_QUEUE} messages"),
            ));
        }

        self.next_message_number += 1;
        let id = MessageId::new(self.next_message_number);
        let fifo = fifo_ids.map(|(group, dedup_id)| {
            let sequence = queue.take_sequence();
            queue.record_dedup(dedup_id.to_owned(), now, id, sequence);
            FifoInfo {
                group: group.to_owned(),
                sequence,
            }
        });
        let sequence = fifo.as_ref().map(|f| f.sequence);
        queue.push(Message::new(id, request.body.to_owned(), now, delay, fifo));
        Ok(Accepted { id, sequence })
    }

    pub fn receive_messages(
        &mut self,
        name: &str,
        max: usize,
        visibility: Option<Seconds>,
    ) -> Result<Vec<Delivery>> {
        let now = self.now;
        let queue = Self::lookup(&mut self.queues, now, name)?;
        let visibility = visibility.unwrap_or(queue.settings().visibility_timeout);
        let dead_letter_target = queue.settings().dead_letter.clone();
        let scan = queue.scan(now, max, visibility);

        let mut deliveries = Vec::with_capacity(scan.delivered.len());
        for arrival in scan.delivered {
            let receipt = ReceiptHandle::new(self.handles.len() as u64 + 1);
            self.handles.push(HandleTarget {
                queue: queue.uid(),
                arrival,
            });
            queue.set_receipt(arrival, receipt);
            if let Some(m) = queue.message(arrival) {
                deliveries.push(Delivery {
                    id: m.id,
                    receipt,
                    body: m.body.clone(),
                    receive_count: m.receive_count,
                    sent_at: m.sent_at,
                    first_received_at: m.first_received_at,
                    fifo: m.fifo.clone(),
                });
            }
        }

        // The target exists: deleting a queue clears every reference to it.
        if let Some(target) = dead_letter_target
            && let Some(dead_queue) = self.queues.get_mut(target.queue.as_str())
        {
            for message in scan.dead {
                dead_queue.push_dead_letter(message, now);
            }
        }
        Ok(deliveries)
    }

    pub fn delete_message(&mut self, name: &str, handle: &str) -> Result<()> {
        let (queue, arrival) = self.resolve_handle(name, handle)?;
        queue.remove(arrival);
        Ok(())
    }

    pub fn change_message_visibility(
        &mut self,
        name: &str,
        handle: &str,
        timeout: Seconds,
    ) -> Result<()> {
        let now = self.now;
        let (queue, arrival) = self.resolve_handle(name, handle)?;
        if let Some(message) = queue.message_mut(arrival) {
            message.set_visibility(now, timeout);
        }
        Ok(())
    }

    /// Finds the in-flight message that `handle` is the latest handle of.
    fn resolve_handle(&mut self, name: &str, handle: &str) -> Result<(&mut Queue, Arrival)> {
        let now = self.now;
        let queue = Self::lookup(&mut self.queues, now, name)?;
        let handle = ReceiptHandle::parse(handle)
            .filter(|h| h.index().is_some_and(|i| i < self.handles.len()))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    "the receipt handle was never issued",
                )
            })?;
        let expired = || {
            Error::new(
                ErrorCode::ReceiptHandleExpired,
                "the receipt handle is no longer usable",
            )
        };
        let target = handle
            .index()
            .and_then(|i| self.handles.get(i))
            .filter(|t| t.queue == queue.uid())
            .ok_or_else(expired)?;
        let in_flight = queue
            .message(target.arrival)
            .is_some_and(|m| m.is_in_flight_as(handle, now));
        if in_flight {
            Ok((queue, target.arrival))
        } else {
            Err(expired())
        }
    }
}

fn invalid_value(message: &str) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

fn missing(parameter: &str) -> Error {
    Error::new(
        ErrorCode::MissingParameter,
        format!("{parameter} is required"),
    )
}
