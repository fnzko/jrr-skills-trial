//! The whole state of the process and the rules that span queues.

use std::collections::BTreeMap;

use crate::command::{Attributes, Command, DeadLetterRequest};
use crate::error::{CommandError, ErrorCode};
use crate::ids::{MessageId, QueueName, ReceiptHandle};
use crate::queue::{DeadLetter, Queue, QueueSettings};
use crate::reply::{AttributeValues, QueueDescription, QueueList, Reply, Time};
use crate::time::{Seconds, Timestamp};

const MAX_QUEUES: usize = 1000;

#[derive(Debug)]
pub struct Broker {
    clock: Timestamp,
    queues: BTreeMap<QueueName, Queue>,
    next_message_id: MessageId,
    next_receipt_handle: ReceiptHandle,
}

impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}

/// Finds a queue and drops its expired messages, so callers see it as of `now`.
fn lookup<'q>(
    queues: &'q mut BTreeMap<QueueName, Queue>,
    name: &str,
    now: Timestamp,
) -> Result<&'q mut Queue, CommandError> {
    let queue = queues
        .get_mut(name)
        .ok_or_else(|| CommandError::no_such_queue(name))?;
    queue.sweep(now);
    Ok(queue)
}

impl Broker {
    pub fn new() -> Self {
        Self {
            clock: Timestamp::START,
            queues: BTreeMap::new(),
            next_message_id: MessageId::FIRST,
            next_receipt_handle: ReceiptHandle::FIRST,
        }
    }

    pub fn execute(&mut self, command: Command<'_>) -> Result<Reply, CommandError> {
        let now = self.clock;
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => {
                let queue = lookup(&mut self.queues, queue, now)?;
                let settings = queue.settings();
                Ok(Reply::Attributes(QueueDescription {
                    attributes: AttributeValues {
                        visibility_timeout: settings.visibility_timeout,
                        delay_seconds: settings.delay,
                        retention_seconds: settings.retention,
                        fifo: settings.queue_type.is_fifo(),
                        content_based_deduplication: settings
                            .queue_type
                            .content_based_deduplication(),
                        dead_letter: settings.dead_letter.clone(),
                    },
                    counts: queue.counts(now),
                }))
            }
            Command::ListQueues { prefix } => {
                let prefix = prefix.unwrap_or("");
                Ok(Reply::Queues(QueueList {
                    queue_names: self
                        .queues
                        .keys()
                        .filter(|name| name.as_str().starts_with(prefix))
                        .cloned()
                        .collect(),
                }))
            }
            Command::DeleteQueue { queue } => {
                self.queues
                    .remove(queue)
                    .ok_or_else(|| CommandError::no_such_queue(queue))?;
                for other in self.queues.values_mut() {
                    other.forget_dead_letter_queue(queue);
                }
                Ok(Reply::Done {})
            }
            Command::PurgeQueue { queue } => {
                lookup(&mut self.queues, queue, now)?.purge();
                Ok(Reply::Done {})
            }
            Command::Tick { seconds } => {
                self.clock = now + seconds;
                Ok(Reply::Time(Time { now: self.clock }))
            }
            Command::SendMessage(request) => {
                let queue = lookup(&mut self.queues, request.queue, now)?;
                queue
                    .send(request, now, &mut self.next_message_id)
                    .map(Reply::Sent)
            }
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let handle = self.issued_handle(queue, receipt_handle)?;
                lookup(&mut self.queues, queue, now)?.delete_message(handle, now)?;
                Ok(Reply::Done {})
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let handle = self.issued_handle(queue, receipt_handle)?;
                lookup(&mut self.queues, queue, now)?.change_visibility(
                    handle,
                    visibility_timeout,
                    now,
                )?;
                Ok(Reply::Done {})
            }
        }
    }

    fn create_queue(
        &mut self,
        name: QueueName,
        attributes: &Attributes<'_>,
    ) -> Result<Reply, CommandError> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(CommandError::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is a FIFO queue",
            ));
        }
        check_deduplication_flag(attributes, fifo)?;
        let requested = attributes.dead_letter()?;
        if let Some(existing) = self.queues.get(name.as_str()) {
            return if existing.matches_request(attributes, requested) {
                Ok(Reply::Done {})
            } else {
                Err(CommandError::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = requested
            .map(|request| self.resolve_dead_letter(request, fifo, None))
            .transpose()?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        let settings = QueueSettings::requested(attributes, dead_letter);
        self.queues.insert(name, Queue::new(settings));
        Ok(Reply::Done {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        attributes: &Attributes<'_>,
    ) -> Result<Reply, CommandError> {
        let now = self.clock;
        let fifo = lookup(&mut self.queues, name, now)?
            .settings()
            .queue_type
            .is_fifo();
        if attributes.fifo.is_some() {
            return Err(CommandError::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        check_deduplication_flag(attributes, fifo)?;
        let dead_letter = attributes
            .dead_letter()?
            .map(|request| self.resolve_dead_letter(request, fifo, Some(name)))
            .transpose()?;
        lookup(&mut self.queues, name, now)?.update(attributes, dead_letter, now);
        Ok(Reply::Done {})
    }

    /// Checks that the dead-letter queue exists and is of the owner's type. `owner` is the
    /// name of the queue being configured when it already exists, as it cannot be its own
    /// dead-letter queue.
    fn resolve_dead_letter(
        &self,
        request: DeadLetterRequest<'_>,
        owner_is_fifo: bool,
        owner: Option<&str>,
    ) -> Result<DeadLetter, CommandError> {
        let (name, target) = self
            .queues
            .get_key_value(request.queue)
            .ok_or_else(|| CommandError::no_such_queue(request.queue))?;
        if owner == Some(request.queue) {
            return Err(CommandError::new(
                ErrorCode::InvalidParameterValue,
                "a queue cannot be its own dead-letter queue",
            ));
        }
        if target.settings().queue_type.is_fifo() != owner_is_fifo {
            return Err(CommandError::new(
                ErrorCode::InvalidParameterValue,
                "a dead-letter queue must be of the same type as the queue",
            ));
        }
        Ok(DeadLetter {
            queue: name.clone(),
            max_receive_count: request.max_receive_count,
        })
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: usize,
        visibility_timeout: Option<Seconds>,
    ) -> Result<Reply, CommandError> {
        let now = self.clock;
        let receipt = lookup(&mut self.queues, name, now)?.receive(
            max_messages,
            visibility_timeout,
            now,
            &mut self.next_receipt_handle,
        );
        if let Some(transfer) = receipt.dead_lettered {
            let target = self.queues.get_mut(transfer.to.as_str()).expect(
                "a configured dead-letter queue exists: DeleteQueue clears every reference to it",
            );
            for message in transfer.messages {
                target.append(message);
            }
            target.sweep(now);
        }
        Ok(Reply::Received(receipt.received))
    }

    /// Parses a receipt handle and checks that this process has issued it. The queue is
    /// looked up first, as the error order of the specification requires.
    fn issued_handle(&mut self, queue: &str, text: &str) -> Result<ReceiptHandle, CommandError> {
        lookup(&mut self.queues, queue, self.clock)?;
        ReceiptHandle::parse(text)
            .filter(|handle| *handle < self.next_receipt_handle)
            .ok_or_else(|| {
                CommandError::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("receipt handle {text:?} was never issued"),
                )
            })
    }
}

fn check_deduplication_flag(attributes: &Attributes<'_>, fifo: bool) -> Result<(), CommandError> {
    if attributes.content_based_deduplication == Some(true) && !fifo {
        return Err(CommandError::new(
            ErrorCode::InvalidParameterValue,
            "ContentBasedDeduplication needs a FIFO queue",
        ));
    }
    Ok(())
}
