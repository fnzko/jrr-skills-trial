//! The process state and the rules of each command.

use std::collections::BTreeMap;

use crate::error::{Error, ErrorCode};
use crate::ids::{DedupKey, GroupId, MessageId, QueueName, ReceiptHandle, ReceiptHandles};
use crate::queue::{Attributes, DeadLetter, Kind, MAX_MESSAGES, NewMessage, Queue};
use crate::request::{AttributeInput, Command};
use crate::response::Output;

const MAX_QUEUES: usize = 1000;
/// Behavior past this clock value is unspecified; refusing to go beyond it
/// keeps every time sum far from overflow.
const CLOCK_LIMIT: u64 = 4_000_000_000;

type Queues = BTreeMap<QueueName, Queue>;

/// Looks up a queue by exact name and drops the messages that have expired.
fn find<'a>(queues: &'a mut Queues, name: &str, now: u64) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| {
        Error::new(
            ErrorCode::QueueDoesNotExist,
            format!("queue {name:?} does not exist"),
        )
    })?;
    queue.expire(now);
    Ok(queue)
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

fn missing(parameter: &str) -> Error {
    Error::new(
        ErrorCode::MissingParameter,
        format!("{parameter} is required"),
    )
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

fn check_dead_letter_pair(input: &AttributeInput) -> Result<(), Error> {
    if input.dead_letter_queue.is_some() == input.max_receive_count.is_some() {
        Ok(())
    } else {
        Err(invalid(
            "DeadLetterQueue and MaxReceiveCount must be given together",
        ))
    }
}

#[derive(Debug, Default)]
pub struct Service {
    now: u64,
    queues: Queues,
    messages_accepted: u64,
    handles: ReceiptHandles,
}

impl Service {
    pub fn execute(&mut self, command: Command) -> Result<Output, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => {
                let now = self.now;
                let queue = find(&mut self.queues, &queue, now)?;
                Ok(Output::QueueAttributes {
                    attributes: queue.attributes.view(),
                    counts: queue.counts(now),
                })
            }
            Command::ListQueues { prefix } => {
                let prefix = prefix.as_deref().unwrap_or("");
                Ok(Output::Queues {
                    queue_names: self
                        .queues
                        .keys()
                        .filter(|name| name.as_str().starts_with(prefix))
                        .cloned()
                        .collect(),
                })
            }
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                let now = self.now;
                find(&mut self.queues, &queue, now)?.purge();
                Ok(Output::Empty {})
            }
            Command::Tick { seconds } => {
                self.now = self
                    .now
                    .checked_add(u64::from(seconds))
                    .filter(|now| *now <= CLOCK_LIMIT)
                    .ok_or_else(|| {
                        Error::new(
                            ErrorCode::LimitExceeded,
                            "the clock cannot advance that far",
                        )
                    })?;
                Ok(Output::Now { now: self.now })
            }
            Command::SendMessage {
                queue,
                body,
                delay_seconds,
                group,
                deduplication,
            } => self.send_message(&queue, body, delay_seconds, group, deduplication),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => self.with_handle(&queue, &receipt_handle, |queue, handle, now| {
                queue.delete_in_flight(handle, now)
            }),
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => self.with_handle(&queue, &receipt_handle, |queue, handle, now| {
                queue.change_visibility(handle, now, visibility_timeout)
            }),
        }
    }

    /// Resolves the dead-letter configuration in `input`, if it has one.
    /// `own_name` is the queue being configured, which cannot be its own
    /// dead-letter queue; `fifo` is that queue's type, which the dead-letter
    /// queue must share.
    fn dead_letter(
        &self,
        input: &AttributeInput,
        own_name: Option<&str>,
        fifo: bool,
    ) -> Result<Option<DeadLetter>, Error> {
        let (Some(name), Some(max_receive_count)) =
            (&input.dead_letter_queue, input.max_receive_count)
        else {
            return Ok(None);
        };
        let (key, target) = self.queues.get_key_value(name.as_str()).ok_or_else(|| {
            Error::new(
                ErrorCode::QueueDoesNotExist,
                format!("dead-letter queue {name:?} does not exist"),
            )
        })?;
        if own_name == Some(name.as_str()) {
            return Err(invalid("a queue cannot be its own dead-letter queue"));
        }
        if target.attributes.kind.is_fifo() != fifo {
            return Err(invalid(
                "the dead-letter queue must have the same type as the queue",
            ));
        }
        Ok(Some(DeadLetter {
            queue: key.clone(),
            max_receive_count,
        }))
    }

    fn create_queue(&mut self, name: QueueName, input: &AttributeInput) -> Result<Output, Error> {
        let kind = Kind::requested(input);
        if name.is_fifo() != (input.fifo == Some(true)) {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "the .fifo suffix and the Fifo attribute must agree",
            ));
        }
        if input.content_based_deduplication == Some(true) && !kind.is_fifo() {
            return Err(invalid("ContentBasedDeduplication needs a FIFO queue"));
        }
        check_dead_letter_pair(input)?;
        if let Some(existing) = self.queues.get(&name) {
            return if existing.attributes.matches(input) {
                Ok(Output::Empty {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = self.dead_letter(input, None, kind.is_fifo())?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        let queue = Queue::new(Attributes::new(input, dead_letter));
        self.queues.insert(name, queue);
        Ok(Output::Empty {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        input: &AttributeInput,
    ) -> Result<Output, Error> {
        let now = self.now;
        // Looking the queue up also expires messages under the old retention.
        let fifo = find(&mut self.queues, name, now)?.attributes.kind.is_fifo();
        if input.fifo.is_some() {
            return Err(unsupported("the type of a queue cannot change"));
        }
        if input.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid("ContentBasedDeduplication needs a FIFO queue"));
        }
        check_dead_letter_pair(input)?;
        let dead_letter = self.dead_letter(input, Some(name), fifo)?;
        let queue = find(&mut self.queues, name, now)?;
        queue.attributes.apply(input, dead_letter);
        queue.expire(now);
        Ok(Output::Empty {})
    }

    fn delete_queue(&mut self, name: &str) -> Result<Output, Error> {
        let now = self.now;
        find(&mut self.queues, name, now)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            if queue
                .attributes
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue.as_str() == name)
            {
                queue.attributes.dead_letter = None;
            }
        }
        Ok(Output::Empty {})
    }

    fn send_message(
        &mut self,
        name: &str,
        body: String,
        delay_seconds: Option<u32>,
        group: Option<GroupId>,
        deduplication: Option<DedupKey>,
    ) -> Result<Output, Error> {
        let now = self.now;
        let id = MessageId(self.messages_accepted + 1);
        let queue = find(&mut self.queues, name, now)?;
        let (group, deduplication) = match queue.attributes.kind {
            Kind::Standard => {
                if group.is_some() || deduplication.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                (None, None)
            }
            Kind::Fifo {
                content_based_deduplication,
            } => {
                if delay_seconds.is_some() {
                    return Err(unsupported("DelaySeconds is not supported on a FIFO queue"));
                }
                let group = group.ok_or_else(|| missing("GroupId"))?;
                let key = match deduplication {
                    Some(key) => key,
                    None if content_based_deduplication => DedupKey::from_body(&body),
                    None => return Err(missing("DeduplicationId")),
                };
                if let Some(original) = queue.find_duplicate(&key, now) {
                    return Ok(Output::Sent {
                        message_id: original.id,
                        sequence_number: Some(original.sequence),
                    });
                }
                (Some(group), Some(key))
            }
        };
        if queue.message_count() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }
        let delay_seconds = delay_seconds.unwrap_or(queue.attributes.delay_seconds);
        let sequence_number = queue.enqueue(NewMessage {
            id,
            body,
            sent_at: now,
            delay_seconds,
            group,
            deduplication,
        });
        self.messages_accepted += 1;
        Ok(Output::Sent {
            message_id: id,
            sequence_number,
        })
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u32,
        visibility_timeout: Option<u32>,
    ) -> Result<Output, Error> {
        let now = self.now;
        let queue = find(&mut self.queues, name, now)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.attributes.visibility_timeout);
        let dead_letter_queue = queue
            .attributes
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.queue.clone());
        let received = queue.receive(now, max_messages, visibility_timeout, &mut self.handles);
        if let Some(target) = dead_letter_queue {
            // DeleteQueue clears every reference to a deleted queue, so a
            // configured dead-letter queue always exists.
            let target = self
                .queues
                .get_mut(&target)
                .expect("a configured dead-letter queue exists");
            for message in received.dead {
                target.enqueue_dead(message);
            }
            target.expire(now);
        }
        Ok(Output::Received {
            messages: received.delivered,
        })
    }

    /// Runs `action` on the message that the receipt handle refers to, after
    /// the handle checks of `DeleteMessage` and `ChangeMessageVisibility`.
    /// `action` returns false when the handle is no longer usable.
    fn with_handle(
        &mut self,
        name: &str,
        text: &str,
        action: impl FnOnce(&mut Queue, ReceiptHandle, u64) -> bool,
    ) -> Result<Output, Error> {
        let now = self.now;
        let queue = find(&mut self.queues, name, now)?;
        let handle = self.handles.parse_issued(text).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("receipt handle {text:?} was never issued"),
            )
        })?;
        if action(queue, handle, now) {
            Ok(Output::Empty {})
        } else {
            Err(Error::new(
                ErrorCode::ReceiptHandleExpired,
                format!("receipt handle {text:?} is no longer usable"),
            ))
        }
    }
}
