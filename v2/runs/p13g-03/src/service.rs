//! The miniq state and the execution of commands against it.

use std::collections::BTreeMap;

use crate::command::Command;
use crate::error::{Error, ErrorCode};
use crate::queue::{Arrival, MAX_MESSAGES, Placement, Queue};
use crate::reply::{AttributeValues, Clock, QueueAttributes, QueueNames, Received, Reply, Sent};
use crate::types::{
    AttributeUpdate, DeadLetter, GroupId, MessageId, QueueKind, QueueName, ReceiptHandle, Seconds,
    Settings,
};

const MAX_QUEUES: usize = 1000;

#[derive(Debug, Default)]
pub struct Service {
    now: Seconds,
    queues: BTreeMap<QueueName, Queue>,
    messages_accepted: u64,
    handles_issued: u64,
}

/// Finds a queue by exact name and brings it up to `now`.
fn lookup<'q>(
    queues: &'q mut BTreeMap<QueueName, Queue>,
    now: Seconds,
    name: &str,
) -> Result<&'q mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| {
        Error::new(
            ErrorCode::QueueDoesNotExist,
            format!("queue {name:?} does not exist"),
        )
    })?;
    queue.advance(now);
    Ok(queue)
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn execute(&mut self, command: Command<'_>) -> Result<Reply, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => self.get_queue_attributes(queue),
            Command::ListQueues { prefix } => Ok(Reply::QueueNames(QueueNames {
                queue_names: self
                    .queues
                    .keys()
                    .filter(|name| name.as_str().starts_with(prefix.unwrap_or("")))
                    .cloned()
                    .collect(),
            })),
            Command::DeleteQueue { queue } => self.delete_queue(queue),
            Command::PurgeQueue { queue } => {
                lookup(&mut self.queues, self.now, queue)?.purge();
                Ok(Reply::Empty {})
            }
            Command::Tick { seconds } => {
                self.now = self.now + seconds;
                Ok(Reply::Clock(Clock { now: self.now }))
            }
            Command::SendMessage {
                queue,
                body,
                delay,
                group,
                deduplication_id,
            } => self.send_message(queue, body, delay, group, deduplication_id),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let (queue, _, arrival) = self.claim(queue, receipt_handle)?;
                queue.remove(arrival);
                Ok(Reply::Empty {})
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let now = self.now;
                let (queue, handle, arrival) = self.claim(queue, receipt_handle)?;
                queue.change_visibility(arrival, handle, visibility_timeout, now);
                Ok(Reply::Empty {})
            }
        }
    }

    /// Resolves a dead-letter queue name given in a request to the queue's
    /// own name, and checks that it can serve `queue_kind`.
    fn dead_letter_target(
        &self,
        request: Option<(&str, u32)>,
        queue_kind: QueueKind,
        queue_name: &str,
    ) -> Result<Option<DeadLetter>, Error> {
        let Some((target, max_receive_count)) = request else {
            return Ok(None);
        };
        let (name, queue) = self.queues.get_key_value(target).ok_or_else(|| {
            Error::new(
                ErrorCode::QueueDoesNotExist,
                format!("dead-letter queue {target:?} does not exist"),
            )
        })?;
        if name.as_str() == queue_name || queue.settings().kind.is_fifo() != queue_kind.is_fifo() {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("{target:?} cannot be the dead-letter queue of {queue_name:?}"),
            ));
        }
        Ok(Some(DeadLetter {
            queue: name.clone(),
            max_receive_count,
        }))
    }

    fn create_queue(
        &mut self,
        name: QueueName,
        update: &AttributeUpdate<'_>,
    ) -> Result<Reply, Error> {
        let fifo = update.fifo.unwrap_or(false);
        let content_based = update.content_based_deduplication.unwrap_or(false);
        if fifo != name.has_fifo_suffix() {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is FIFO",
            ));
        }
        if content_based && !fifo {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let requested_dead_letter = update.dead_letter()?;
        let kind = if fifo {
            QueueKind::Fifo {
                content_based_deduplication: content_based,
            }
        } else {
            QueueKind::Standard
        };
        let mut settings = Settings::defaults(kind);
        settings.apply(update);

        if let Some(existing) = self.queues.get(&name) {
            let current_dead_letter = existing
                .dead_letter()
                .map(|dead_letter| (dead_letter.queue.as_str(), dead_letter.max_receive_count));
            return if *existing.settings() == settings
                && current_dead_letter == requested_dead_letter
            {
                Ok(Reply::Empty {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = self.dead_letter_target(requested_dead_letter, kind, name.as_str())?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        self.queues.insert(name, Queue::new(settings, dead_letter));
        Ok(Reply::Empty {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        update: &AttributeUpdate<'_>,
    ) -> Result<Reply, Error> {
        let kind = lookup(&mut self.queues, self.now, name)?.settings().kind;
        if update.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if update.content_based_deduplication == Some(true) && !kind.is_fifo() {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = self.dead_letter_target(update.dead_letter()?, kind, name)?;
        lookup(&mut self.queues, self.now, name)?.reconfigure(update, dead_letter);
        Ok(Reply::Empty {})
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Reply, Error> {
        let queue = lookup(&mut self.queues, self.now, name)?;
        let settings = queue.settings();
        let (fifo, content_based_deduplication) = match settings.kind {
            QueueKind::Standard => (false, false),
            QueueKind::Fifo {
                content_based_deduplication,
            } => (true, content_based_deduplication),
        };
        Ok(Reply::Attributes(QueueAttributes {
            attributes: AttributeValues {
                visibility_timeout: settings.visibility_timeout,
                delay_seconds: settings.delay,
                retention_seconds: settings.retention,
                fifo,
                content_based_deduplication,
                dead_letter: queue.dead_letter().cloned(),
            },
            counts: queue.counts(),
        }))
    }

    fn delete_queue(&mut self, name: &str) -> Result<Reply, Error> {
        if self.queues.remove(name).is_none() {
            return Err(Error::new(
                ErrorCode::QueueDoesNotExist,
                format!("queue {name:?} does not exist"),
            ));
        }
        for queue in self.queues.values_mut() {
            queue.detach_dead_letter(name);
        }
        Ok(Reply::Empty {})
    }

    fn send_message(
        &mut self,
        name: &str,
        body: &str,
        delay: Option<Seconds>,
        group: Option<GroupId>,
        deduplication_id: Option<&str>,
    ) -> Result<Reply, Error> {
        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let placement = match queue.settings().kind {
            QueueKind::Standard => {
                if group.is_some() || deduplication_id.is_some() {
                    return Err(Error::new(
                        ErrorCode::UnsupportedOperation,
                        "GroupId and DeduplicationId need a FIFO queue",
                    ));
                }
                Placement::Standard
            }
            QueueKind::Fifo {
                content_based_deduplication,
            } => {
                if delay.is_some() {
                    return Err(Error::new(
                        ErrorCode::UnsupportedOperation,
                        "DelaySeconds is not supported on a FIFO queue",
                    ));
                }
                let group = group.ok_or_else(|| {
                    Error::new(ErrorCode::MissingParameter, "GroupId is required")
                })?;
                let deduplication_key = match (deduplication_id, content_based_deduplication) {
                    (Some(id), _) => id,
                    (None, true) => body,
                    (None, false) => {
                        return Err(Error::new(
                            ErrorCode::MissingParameter,
                            "DeduplicationId is required",
                        ));
                    }
                };
                if let Some((message_id, sequence)) = queue.find_duplicate(deduplication_key, now) {
                    return Ok(Reply::Sent(Sent {
                        message_id,
                        sequence_number: Some(sequence),
                    }));
                }
                Placement::Fifo {
                    group,
                    deduplication_key,
                }
            }
        };
        if queue.message_count() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }
        self.messages_accepted += 1;
        let message_id = MessageId(self.messages_accepted);
        let delay = delay.unwrap_or(queue.settings().delay);
        let sequence_number = queue.send(message_id, body, placement, delay, now);
        Ok(Reply::Sent(Sent {
            message_id,
            sequence_number,
        }))
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: usize,
        visibility_timeout: Option<Seconds>,
    ) -> Result<Reply, Error> {
        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.settings().visibility_timeout);
        let plan = queue.plan_receive(max_messages);
        let moved: Vec<_> = plan
            .dead_letter
            .into_iter()
            .map(|arrival| queue.remove(arrival))
            .collect();
        let messages = plan
            .deliver
            .into_iter()
            .map(|arrival| {
                self.handles_issued += 1;
                let handle = ReceiptHandle(self.handles_issued);
                queue.deliver(arrival, visibility_timeout, handle, now)
            })
            .collect();
        if !moved.is_empty() {
            let target = queue
                .dead_letter()
                .expect("messages are only planned for a move while a dead-letter queue is set")
                .queue
                .clone();
            let dead_letter_queue = self
                .queues
                .get_mut(&target)
                .expect("a queue named as dead-letter queue exists until it is deleted");
            for message in moved {
                dead_letter_queue.accept_moved(message, now);
            }
        }
        Ok(Reply::Received(Received { messages }))
    }

    /// Resolves a receipt handle of `queue_name` to the in-flight message it
    /// belongs to.
    fn claim(
        &mut self,
        queue_name: &str,
        receipt_handle: &str,
    ) -> Result<(&mut Queue, ReceiptHandle, Arrival), Error> {
        let queue = lookup(&mut self.queues, self.now, queue_name)?;
        let handle = ReceiptHandle::parse(receipt_handle)
            .filter(|handle| handle.0 <= self.handles_issued)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("receipt handle {receipt_handle:?} was never issued"),
                )
            })?;
        let arrival = queue.in_flight(handle).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleExpired,
                format!("receipt handle {receipt_handle:?} is no longer usable"),
            )
        })?;
        Ok((queue, handle, arrival))
    }
}
