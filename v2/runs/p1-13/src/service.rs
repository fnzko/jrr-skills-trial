//! The state of the whole process and the execution of commands against it.

use std::collections::BTreeMap;
use std::ops::Bound;

use crate::command::{AttributePatch, Command, SendRequest};
use crate::error::{Error, ErrorCode};
use crate::ids::{MessageId, ReceiptHandle};
use crate::output::{AttributesView, DeadLetterView, Output};
use crate::queue::{Arrival, DeadLetterConfig, FifoSend, Queue, QueueKind, QueueSettings};
use crate::scalar::{Counter, MaxMessages, MaxReceiveCount, Timestamp, VisibilityTimeout};
use crate::text::{DedupKey, QueueName};

const MAX_QUEUES: usize = 1000;

#[derive(Debug, Default)]
pub struct Service {
    now: Timestamp,
    queues: BTreeMap<QueueName, Queue>,
    message_ids: Counter,
    receipt_handles: Counter,
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn execute(&mut self, command: Command) -> Result<Output<'_>, Error> {
        match command {
            Command::CreateQueue { name, attributes } => {
                self.create_queue(name, &attributes)?;
                Ok(Output::Empty {})
            }
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)?;
                Ok(Output::Empty {})
            }
            Command::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Command::DeleteQueue { queue } => {
                self.delete_queue(&queue)?;
                Ok(Output::Empty {})
            }
            Command::PurgeQueue { queue } => {
                lookup(&mut self.queues, &queue, self.now)?.purge();
                Ok(Output::Empty {})
            }
            Command::Tick { seconds } => {
                self.now = self.now.plus(seconds.get());
                Ok(Output::Now { now: self.now })
            }
            Command::SendMessage(request) => self.send_message(request),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let now = self.now;
                let issued = self.receipt_handles.issued();
                let queue = lookup(&mut self.queues, &queue, now)?;
                let arrival = find_in_flight(queue, &receipt_handle, issued, now)?;
                queue.delete(arrival);
                Ok(Output::Empty {})
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let now = self.now;
                let issued = self.receipt_handles.issued();
                let queue = lookup(&mut self.queues, &queue, now)?;
                let arrival = find_in_flight(queue, &receipt_handle, issued, now)?;
                queue.change_visibility(arrival, now, visibility_timeout);
                Ok(Output::Empty {})
            }
        }
    }

    fn create_queue(&mut self, name: QueueName, patch: &AttributePatch) -> Result<(), Error> {
        let fifo = patch.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "the .fifo suffix and the Fifo attribute must agree",
            ));
        }
        let content_based_deduplication = patch.content_based_deduplication.unwrap_or(false);
        if content_based_deduplication && !fifo {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = patch.dead_letter()?;
        let kind = if fifo {
            QueueKind::Fifo {
                content_based_deduplication,
            }
        } else {
            QueueKind::Standard
        };
        let settings = override_settings(QueueSettings::with_defaults(kind), patch);

        if let Some(existing) = self.queues.get(name.as_str()) {
            let existing_dead_letter = existing
                .dead_letter()
                .map(|config| (config.queue.as_str(), config.max_receive_count));
            return if *existing.settings() == settings && existing_dead_letter == dead_letter {
                Ok(())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {} exists with other attributes", name.as_str()),
                ))
            };
        }
        let config = dead_letter
            .map(|(target, count)| self.dead_letter_config(target, count, name.as_str(), fifo))
            .transpose()?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues may exist"),
            ));
        }
        self.queues.insert(name, Queue::new(settings, config));
        Ok(())
    }

    fn set_queue_attributes(&mut self, name: &str, patch: &AttributePatch) -> Result<(), Error> {
        let kind = lookup(&mut self.queues, name, self.now)?.settings().kind;
        if patch.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if patch.content_based_deduplication == Some(true) && !kind.is_fifo() {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let config = patch
            .dead_letter()?
            .map(|(target, count)| self.dead_letter_config(target, count, name, kind.is_fifo()))
            .transpose()?;

        let now = self.now;
        let queue = lookup(&mut self.queues, name, now)?;
        let mut settings = override_settings(*queue.settings(), patch);
        if let (QueueKind::Fifo { .. }, Some(content_based_deduplication)) =
            (kind, patch.content_based_deduplication)
        {
            settings.kind = QueueKind::Fifo {
                content_based_deduplication,
            };
        }
        queue.update(settings, config, now);
        Ok(())
    }

    /// Resolves the dead-letter queue named by a request for the queue `owner`
    /// whose type is `fifo`.
    fn dead_letter_config(
        &self,
        target: &str,
        max_receive_count: MaxReceiveCount,
        owner: &str,
        fifo: bool,
    ) -> Result<DeadLetterConfig, Error> {
        let (target_name, target_queue) = self.queues.get_key_value(target).ok_or_else(|| {
            Error::new(
                ErrorCode::QueueDoesNotExist,
                format!("dead-letter queue {target:?} does not exist"),
            )
        })?;
        if target == owner || target_queue.settings().kind.is_fifo() != fifo {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "the dead-letter queue must be another queue of the same type",
            ));
        }
        Ok(DeadLetterConfig {
            queue: target_name.clone(),
            max_receive_count,
        })
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Output<'_>, Error> {
        let now = self.now;
        let queue = lookup(&mut self.queues, name, now)?;
        let settings = queue.settings();
        let attributes = AttributesView {
            visibility_timeout: settings.visibility_timeout,
            delay_seconds: settings.delay,
            retention_seconds: settings.retention,
            fifo: settings.kind.is_fifo(),
            content_based_deduplication: matches!(
                settings.kind,
                QueueKind::Fifo {
                    content_based_deduplication: true
                }
            ),
            dead_letter: queue.dead_letter().map(|config| DeadLetterView {
                queue: &config.queue,
                max_receive_count: config.max_receive_count,
            }),
        };
        Ok(Output::Attributes {
            attributes,
            counts: queue.counts(now),
        })
    }

    fn list_queues(&self, prefix: &str) -> Output<'_> {
        let queue_names = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name)
            .take_while(|name| name.as_str().starts_with(prefix))
            .collect();
        Output::QueueNames { queue_names }
    }

    fn delete_queue(&mut self, name: &str) -> Result<(), Error> {
        self.queues
            .remove(name)
            .ok_or_else(|| no_such_queue(name))?;
        for queue in self.queues.values_mut() {
            if queue
                .dead_letter()
                .is_some_and(|config| config.queue.as_str() == name)
            {
                queue.clear_dead_letter();
            }
        }
        Ok(())
    }

    fn send_message(&mut self, request: SendRequest) -> Result<Output<'_>, Error> {
        let now = self.now;
        let queue = lookup(&mut self.queues, &request.queue, now)?;
        let unsupported = |what: &str| Error::new(ErrorCode::UnsupportedOperation, what.to_owned());
        let missing = |name: &str| {
            Error::new(
                ErrorCode::MissingParameter,
                format!("parameter {name} is required on a FIFO queue"),
            )
        };
        let fifo = match queue.settings().kind {
            QueueKind::Standard => {
                if request.group.is_some() || request.dedup_id.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                None
            }
            QueueKind::Fifo {
                content_based_deduplication,
            } => {
                if request.delay.is_some() {
                    return Err(unsupported(
                        "DelaySeconds is not supported per message on a FIFO queue",
                    ));
                }
                let group = request.group.ok_or_else(|| missing("GroupId"))?;
                let dedup = match (&request.dedup_id, content_based_deduplication) {
                    (Some(id), _) => id.as_str(),
                    (None, true) => request.body.as_str(),
                    (None, false) => return Err(missing("DeduplicationId")),
                };
                if let Some(original) = queue.find_duplicate(dedup, now) {
                    return Ok(Output::Sent {
                        message_id: original.id,
                        sequence_number: Some(original.sequence),
                    });
                }
                Some(FifoSend {
                    group,
                    dedup: DedupKey::new(dedup),
                })
            }
        };
        if queue.is_full() {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "the queue already holds the most messages it may",
            ));
        }
        let id = MessageId::new(self.message_ids.issue());
        let delay = request.delay.unwrap_or(queue.settings().delay);
        let sequence_number = queue.accept(id, request.body, now, delay, fifo);
        Ok(Output::Sent {
            message_id: id,
            sequence_number,
        })
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: MaxMessages,
        visibility_timeout: Option<VisibilityTimeout>,
    ) -> Result<Output<'_>, Error> {
        let now = self.now;
        let queue = lookup(&mut self.queues, name, now)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.settings().visibility_timeout);
        let receipt = queue.receive(
            now,
            max_messages,
            visibility_timeout,
            &mut self.receipt_handles,
        );
        if !receipt.dead.is_empty() {
            let target = queue
                .dead_letter()
                .expect("messages are only dead-lettered by a queue that has a configuration")
                .queue
                .clone();
            let target = self
                .queues
                .get_mut(target.as_str())
                .expect("DeleteQueue clears every reference to a deleted queue");
            for dead in receipt.dead {
                target.accept_dead_letter(dead, now);
            }
        }
        Ok(Output::Messages {
            messages: receipt.delivered,
        })
    }
}

/// Finds a queue and applies retention to it, so that the caller sees only
/// messages that still exist.
fn lookup<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    name: &str,
    now: Timestamp,
) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| no_such_queue(name))?;
    queue.expire(now);
    Ok(queue)
}

fn no_such_queue(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

fn override_settings(settings: QueueSettings, patch: &AttributePatch) -> QueueSettings {
    QueueSettings {
        visibility_timeout: patch
            .visibility_timeout
            .unwrap_or(settings.visibility_timeout),
        delay: patch.delay.unwrap_or(settings.delay),
        retention: patch.retention.unwrap_or(settings.retention),
        kind: settings.kind,
    }
}

fn find_in_flight(
    queue: &Queue,
    handle: &str,
    issued: u64,
    now: Timestamp,
) -> Result<Arrival, Error> {
    let handle = ReceiptHandle::parse(handle, issued).ok_or_else(|| {
        Error::new(
            ErrorCode::ReceiptHandleInvalid,
            format!("receipt handle {handle:?} was never issued"),
        )
    })?;
    queue.find_in_flight(handle, now).ok_or_else(|| {
        Error::new(
            ErrorCode::ReceiptHandleExpired,
            format!("receipt handle {handle} is not usable"),
        )
    })
}
