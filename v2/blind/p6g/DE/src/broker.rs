//! The state of the whole service and the rules of every command.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::attributes::{AttributeUpdate, Attributes, DeadLetter, UnpairedDeadLetter};
use crate::domain::{
    DeduplicationId, GroupId, MessageId, QueueName, ReceiptHandle, SequenceNumber,
};
use crate::error::{Error, ErrorCode};
use crate::protocol::Command;
use crate::queue::{Counts, Delivery, Message, NotInFlight, Queue};

const MAX_QUEUES: usize = 1000;

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Output {
    Empty {},
    Clock {
        now: u64,
    },
    Queues {
        queue_names: Vec<String>,
    },
    Attributes {
        attributes: Attributes,
        counts: Counts,
    },
    Sent {
        message_id: MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence_number: Option<SequenceNumber>,
    },
    Received {
        messages: Vec<Delivery>,
    },
}

/// All queues, the virtual clock and the counters of issued identifiers.
#[derive(Debug, Default)]
pub struct Broker {
    now: u64,
    queues: BTreeMap<String, Queue>,
    messages_accepted: u64,
    handles_issued: u64,
}

impl Broker {
    pub fn execute(&mut self, command: Command) -> Result<Output, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(&name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => {
                let now = self.now;
                let queue = self.queue(&queue)?;
                Ok(Output::Attributes {
                    attributes: queue.attributes().clone(),
                    counts: queue.counts(now),
                })
            }
            Command::ListQueues { prefix } => {
                let prefix = prefix.unwrap_or_default();
                let queue_names = self
                    .queues
                    .keys()
                    .filter(|name| name.starts_with(&prefix))
                    .cloned()
                    .collect();
                Ok(Output::Queues { queue_names })
            }
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                self.queue(&queue)?.purge();
                Ok(Output::Empty {})
            }
            Command::Tick { seconds } => {
                self.now = self.now.saturating_add(seconds);
                Ok(Output::Clock { now: self.now })
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
            } => self.with_delivery(&queue, &receipt_handle, |queue, handle, now| {
                queue.delete(handle, now)
            }),
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => self.with_delivery(&queue, &receipt_handle, |queue, handle, now| {
                queue.change_visibility(handle, visibility_timeout, now)
            }),
        }
    }

    fn queue(&mut self, name: &str) -> Result<&mut Queue, Error> {
        lookup(&mut self.queues, self.now, name)
    }

    fn create_queue(
        &mut self,
        name: &QueueName,
        update: &AttributeUpdate,
    ) -> Result<Output, Error> {
        let fifo = update.fifo.unwrap_or(false);
        if name.is_fifo() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "the name ends in .fifo exactly when Fifo is true",
            ));
        }
        if update.content_based_deduplication == Some(true) && !fifo {
            return Err(content_based_on_standard());
        }
        let dead_letter = update.dead_letter().map_err(unpaired)?;
        let attributes = Attributes::default().updated(update, dead_letter);

        if let Some(existing) = self.queues.get(name.as_str()) {
            return if *existing.attributes() == attributes {
                Ok(Output::Empty {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {} exists with different attributes", name.as_str()),
                ))
            };
        }
        if let Some(dead_letter) = &attributes.dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| does_not_exist(&dead_letter.queue))?;
            if target.is_fifo() != fifo {
                return Err(dead_letter_type_mismatch());
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        self.queues
            .insert(name.as_str().to_owned(), Queue::new(attributes));
        Ok(Output::Empty {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        update: &AttributeUpdate,
    ) -> Result<Output, Error> {
        let fifo = self.queue(name)?.is_fifo();
        if update.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if update.content_based_deduplication == Some(true) && !fifo {
            return Err(content_based_on_standard());
        }
        let dead_letter = update.dead_letter().map_err(unpaired)?;
        if let Some(DeadLetter { queue: target, .. }) = &dead_letter {
            let target_queue = self
                .queues
                .get(target)
                .ok_or_else(|| does_not_exist(target))?;
            if target == name || target_queue.is_fifo() != fifo {
                return Err(Error::invalid_value(
                    "DeadLetterQueue",
                    "must be another queue of the same type",
                ));
            }
        }
        let queue = self.queue(name)?;
        let attributes = queue.attributes().updated(update, dead_letter);
        queue.set_attributes(attributes);
        Ok(Output::Empty {})
    }

    fn delete_queue(&mut self, name: &str) -> Result<Output, Error> {
        self.queue(name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            let uses_deleted = queue
                .attributes()
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue == name);
            if uses_deleted {
                queue.clear_dead_letter();
            }
        }
        Ok(Output::Empty {})
    }

    fn send_message(
        &mut self,
        name: &str,
        body: String,
        delay_seconds: Option<u64>,
        group_id: Option<GroupId>,
        deduplication_id: Option<DeduplicationId>,
    ) -> Result<Output, Error> {
        let Self {
            now,
            queues,
            messages_accepted,
            ..
        } = self;
        let now = *now;
        let queue = lookup(queues, now, name)?;
        let fifo = queue.is_fifo();
        let unsupported = if fifo {
            delay_seconds
                .is_some()
                .then_some("DelaySeconds on a FIFO queue")
        } else {
            (group_id.is_some() || deduplication_id.is_some())
                .then_some("GroupId and DeduplicationId on a standard queue")
        };
        if let Some(what) = unsupported {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                format!("{what} is not supported"),
            ));
        }

        let mut deduplication_key = None;
        if fifo {
            if group_id.is_none() {
                return Err(Error::missing("GroupId"));
            }
            let key = match deduplication_id {
                Some(id) => id.as_str().to_owned(),
                None if queue.attributes().content_based_deduplication => body.clone(),
                None => return Err(Error::missing("DeduplicationId")),
            };
            if let Some((message_id, sequence)) = queue.duplicate_of(&key, now) {
                return Ok(Output::Sent {
                    message_id,
                    sequence_number: Some(sequence),
                });
            }
            deduplication_key = Some(key);
        }
        if queue.is_full() {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "the queue already holds 100000 messages",
            ));
        }

        let delay = delay_seconds.unwrap_or(queue.attributes().delay_seconds);
        *messages_accepted += 1;
        let message_id = MessageId::new(*messages_accepted);
        let sequence = queue.push(Message::new(message_id, body, group_id, now, delay), now);
        if let Some(key) = deduplication_key {
            queue.record_deduplication(key, message_id, sequence, now);
        }
        Ok(Output::Sent {
            message_id,
            sequence_number: fifo.then_some(sequence),
        })
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: usize,
        visibility_timeout: Option<u64>,
    ) -> Result<Output, Error> {
        let Self {
            now,
            queues,
            handles_issued,
            ..
        } = self;
        let now = *now;
        let queue = lookup(queues, now, name)?;
        let visibility = visibility_timeout.unwrap_or(queue.attributes().visibility_timeout);
        let dead_letter_queue = queue
            .attributes()
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.queue.clone());
        let received = queue.receive(max_messages, visibility, now, || {
            *handles_issued += 1;
            ReceiptHandle::new(*handles_issued)
        });

        if let Some(target) = dead_letter_queue {
            let target = queues
                .get_mut(&target)
                .expect("a queue's dead-letter queue exists until the configuration is cleared");
            for message in received.dead_lettered {
                target.push(message.restarted(now), now);
            }
            // A message older than the target's retention does not outlive its arrival.
            target.expire(now);
        }
        Ok(Output::Received {
            messages: received.delivered,
        })
    }

    /// Applies `act` to the message in flight that `text` was issued for.
    ///
    /// The queue is looked up before the handle is read, as the spec orders
    /// the checks.
    fn with_delivery(
        &mut self,
        name: &str,
        text: &str,
        act: impl FnOnce(&mut Queue, ReceiptHandle, u64) -> Result<(), NotInFlight>,
    ) -> Result<Output, Error> {
        let Self {
            now,
            queues,
            handles_issued,
            ..
        } = self;
        let queue = lookup(queues, *now, name)?;
        let handle = ReceiptHandle::parse(text)
            .filter(|handle| handle.number() <= *handles_issued)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("receipt handle {text:?} was never issued"),
                )
            })?;
        act(queue, handle, *now).map_err(|NotInFlight| expired(text))?;
        Ok(Output::Empty {})
    }
}

/// The queue with this exact name, with its expired messages removed.
fn lookup<'a>(
    queues: &'a mut BTreeMap<String, Queue>,
    now: u64,
    name: &str,
) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| does_not_exist(name))?;
    queue.expire(now);
    Ok(queue)
}

fn does_not_exist(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

fn expired(handle: &str) -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        format!("receipt handle {handle} does not belong to a message in flight"),
    )
}

fn content_based_on_standard() -> Error {
    Error::invalid_value(
        "ContentBasedDeduplication",
        "cannot be true on a standard queue",
    )
}

fn dead_letter_type_mismatch() -> Error {
    Error::invalid_value("DeadLetterQueue", "must be a queue of the same type")
}

fn unpaired(e: UnpairedDeadLetter) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, e.to_string())
}
