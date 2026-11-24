//! The process state: the clock, the id counters and every queue, plus the command rules.

use std::collections::BTreeMap;

use crate::error::{ApiError, ErrorCode};
use crate::queue::{
    DeadLetterConfig, MAX_MESSAGES_PER_QUEUE, NewMessage, Queue, QueueConfig, QueueKind,
};
use crate::request::{AttributeUpdate, DeadLetterRequest, Request};
use crate::response::{AttributesView, Output};
use crate::types::{DeduplicationId, GroupId, Ids, Instant, QueueName, ReceiptHandle, Seconds};

const MAX_QUEUES: usize = 1000;

#[derive(Debug)]
pub struct Broker {
    now: Instant,
    ids: Ids,
    queues: BTreeMap<QueueName, Queue>,
}

fn no_such_queue(name: &str) -> ApiError {
    ApiError::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name} does not exist"),
    )
}

fn invalid_value(message: impl Into<String>) -> ApiError {
    ApiError::new(ErrorCode::InvalidParameterValue, message)
}

fn missing(name: &str) -> ApiError {
    ApiError::new(
        ErrorCode::MissingParameter,
        format!("parameter {name} is required for a FIFO queue"),
    )
}

fn unsupported(message: impl Into<String>) -> ApiError {
    ApiError::new(ErrorCode::UnsupportedOperation, message)
}

/// Finds a queue and drops its expired messages, so the caller sees it as of `now`.
fn queue_at<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    name: &str,
    now: Instant,
) -> Result<&'a mut Queue, ApiError> {
    let queue = queues.get_mut(name).ok_or_else(|| no_such_queue(name))?;
    queue.expire(now);
    Ok(queue)
}

impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}

impl Broker {
    pub fn new() -> Self {
        Self {
            now: Instant::START,
            ids: Ids::new(),
            queues: BTreeMap::new(),
        }
    }

    pub fn execute(&mut self, request: Request) -> Result<Output, ApiError> {
        match request {
            Request::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Request::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Request::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref())),
            Request::DeleteQueue { queue } => self.delete_queue(&queue),
            Request::PurgeQueue { queue } => {
                queue_at(&mut self.queues, &queue, self.now)?.purge();
                Ok(Output::Empty {})
            }
            Request::Tick { seconds } => {
                self.now = self.now.plus(seconds);
                Ok(Output::Now { now: self.now })
            }
            Request::SendMessage {
                queue,
                body,
                delay,
                group,
                deduplication,
            } => self.send_message(&queue, body, delay, group, deduplication),
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Request::DeleteMessage { queue, receipt } => {
                let handle = self.queue_receipt(&queue, &receipt)?;
                queue_at(&mut self.queues, &queue, self.now)?
                    .delete_in_flight(handle, self.now)
                    .map_err(|_| expired_handle(&receipt))?;
                Ok(Output::Empty {})
            }
            Request::ChangeMessageVisibility {
                queue,
                receipt,
                visibility_timeout,
            } => {
                let handle = self.queue_receipt(&queue, &receipt)?;
                queue_at(&mut self.queues, &queue, self.now)?
                    .change_visibility(handle, self.now, visibility_timeout)
                    .map_err(|_| expired_handle(&receipt))?;
                Ok(Output::Empty {})
            }
        }
    }

    /// Checks the queue exists (lookup precedes the handle rules), then that the handle was
    /// issued.
    fn queue_receipt(&mut self, queue: &str, receipt: &str) -> Result<ReceiptHandle, ApiError> {
        if !self.queues.contains_key(queue) {
            return Err(no_such_queue(queue));
        }
        self.ids.issued_receipt(receipt).ok_or_else(|| {
            ApiError::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("receipt handle {receipt:?} was never issued"),
            )
        })
    }

    /// Resolves a requested dead-letter queue for the queue named `this` (`None` while that
    /// queue is being created).
    fn resolve_dead_letter(
        &self,
        request: DeadLetterRequest<'_>,
        this: Option<&str>,
        this_kind: QueueKind,
    ) -> Result<DeadLetterConfig, ApiError> {
        let (name, target) = self
            .queues
            .get_key_value(request.queue)
            .ok_or_else(|| no_such_queue(request.queue))?;
        if this == Some(name.as_str()) {
            return Err(invalid_value("a queue cannot be its own dead-letter queue"));
        }
        if target.config().kind.is_fifo() != this_kind.is_fifo() {
            return Err(invalid_value(
                "a queue and its dead-letter queue must both be FIFO or both be standard",
            ));
        }
        Ok(DeadLetterConfig {
            queue: name.clone(),
            max_receive_count: request.max_receive_count,
        })
    }

    fn create_queue(
        &mut self,
        name: QueueName,
        attributes: &AttributeUpdate,
    ) -> Result<Output, ApiError> {
        let fifo = attributes.fifo.unwrap_or(false);
        if fifo != name.has_fifo_suffix() {
            return Err(ApiError::new(
                ErrorCode::QueueNameInvalid,
                "the name must end in .fifo exactly when Fifo is true",
            ));
        }
        let kind = match (fifo, attributes.content_based_deduplication) {
            (true, flag) => QueueKind::Fifo {
                content_based_deduplication: flag.unwrap_or(false),
            },
            (false, Some(true)) => {
                return Err(invalid_value(
                    "ContentBasedDeduplication requires a FIFO queue",
                ));
            }
            (false, _) => QueueKind::Standard,
        };
        let dead_letter = attributes.dead_letter()?;
        let mut config = QueueConfig::new(kind);
        config.apply_timings(attributes);

        if let Some(existing) = self.queues.get(name.as_str()) {
            let existing_dead_letter = existing
                .config()
                .dead_letter
                .as_ref()
                .map(|existing| (existing.queue.as_str(), existing.max_receive_count));
            let requested_dead_letter =
                dead_letter.map(|requested| (requested.queue, requested.max_receive_count));
            let same = QueueConfig {
                dead_letter: existing.config().dead_letter.clone(),
                ..config
            } == *existing.config()
                && existing_dead_letter == requested_dead_letter;
            return if same {
                Ok(Output::Empty {})
            } else {
                Err(ApiError::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {} exists with different attributes", name.as_str()),
                ))
            };
        }
        config.dead_letter = dead_letter
            .map(|request| self.resolve_dead_letter(request, None, kind))
            .transpose()?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(ApiError::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        self.queues.insert(name, Queue::new(config));
        Ok(Output::Empty {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        attributes: &AttributeUpdate,
    ) -> Result<Output, ApiError> {
        let kind = queue_at(&mut self.queues, name, self.now)?.config().kind;
        if attributes.fifo.is_some() {
            return Err(unsupported("the type of a queue cannot change"));
        }
        if attributes.content_based_deduplication == Some(true) && !kind.is_fifo() {
            return Err(invalid_value(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        let dead_letter = attributes
            .dead_letter()?
            .map(|request| self.resolve_dead_letter(request, Some(name), kind))
            .transpose()?;

        let queue = queue_at(&mut self.queues, name, self.now)?;
        let config = queue.config_mut();
        config.apply_timings(attributes);
        if let (QueueKind::Fifo { .. }, Some(flag)) = (kind, attributes.content_based_deduplication)
        {
            config.kind = QueueKind::Fifo {
                content_based_deduplication: flag,
            };
        }
        if dead_letter.is_some() {
            config.dead_letter = dead_letter;
        }
        // A shorter retention takes effect at once and cannot be undone by a later change.
        queue.expire(self.now);
        Ok(Output::Empty {})
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Output, ApiError> {
        let queue = queue_at(&mut self.queues, name, self.now)?;
        let config = queue.config();
        let (fifo, content_based_deduplication) = match config.kind {
            QueueKind::Standard => (false, false),
            QueueKind::Fifo {
                content_based_deduplication,
            } => (true, content_based_deduplication),
        };
        Ok(Output::Attributes {
            attributes: AttributesView {
                visibility_timeout: config.visibility_timeout,
                delay_seconds: config.delay,
                retention_seconds: config.retention,
                fifo,
                content_based_deduplication,
                dead_letter: config.dead_letter.clone(),
            },
            counts: queue.counts(self.now),
        })
    }

    fn list_queues(&self, prefix: Option<&str>) -> Output {
        Output::QueueNames {
            queue_names: self
                .queues
                .keys()
                .filter(|name| prefix.is_none_or(|prefix| name.as_str().starts_with(prefix)))
                .cloned()
                .collect(),
        }
    }

    fn delete_queue(&mut self, name: &str) -> Result<Output, ApiError> {
        self.queues
            .remove(name)
            .ok_or_else(|| no_such_queue(name))?;
        for queue in self.queues.values_mut() {
            let config = queue.config_mut();
            if config
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue.as_str() == name)
            {
                config.dead_letter = None;
            }
        }
        Ok(Output::Empty {})
    }

    fn send_message(
        &mut self,
        name: &str,
        body: String,
        delay: Option<Seconds>,
        group: Option<GroupId>,
        deduplication: Option<DeduplicationId>,
    ) -> Result<Output, ApiError> {
        let now = self.now;
        let queue = queue_at(&mut self.queues, name, now)?;
        let (group, deduplication) = match queue.config().kind {
            QueueKind::Standard => {
                if group.is_some() || deduplication.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                (None, None)
            }
            QueueKind::Fifo {
                content_based_deduplication,
            } => {
                if delay.is_some() {
                    return Err(unsupported(
                        "DelaySeconds per message needs a standard queue",
                    ));
                }
                let group = group.ok_or_else(|| missing("GroupId"))?;
                let id = match deduplication {
                    Some(id) => id,
                    None if content_based_deduplication => DeduplicationId::from_body(&body),
                    None => return Err(missing("DeduplicationId")),
                };
                (Some(group), Some(id))
            }
        };
        if let Some(original) = deduplication
            .as_ref()
            .and_then(|id| queue.duplicate_of(id, now))
        {
            return Ok(Output::Sent {
                message_id: original.message_id,
                sequence_number: Some(original.sequence),
            });
        }
        if queue.len() >= MAX_MESSAGES_PER_QUEUE {
            return Err(ApiError::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES_PER_QUEUE} messages"),
            ));
        }
        let message_id = self.ids.next_message();
        let delay = delay.unwrap_or(queue.config().delay);
        let sequence_number = queue.add(NewMessage {
            id: message_id,
            body,
            sent_at: now,
            group,
            delayed_until: Some(now.plus(delay)),
        });
        if let Some((id, sequence)) = deduplication.zip(sequence_number) {
            queue.record_accepted(id, message_id, sequence, now);
        }
        Ok(Output::Sent {
            message_id,
            sequence_number,
        })
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
    ) -> Result<Output, ApiError> {
        let now = self.now;
        let queue = queue_at(&mut self.queues, name, now)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.config().visibility_timeout);
        let dead_letter_queue = queue
            .config()
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.queue.clone());
        let received = queue.receive(now, max_messages, visibility_timeout, &mut self.ids);
        if let Some(target) = dead_letter_queue {
            let target = self
                .queues
                .get_mut(target.as_str())
                .expect("DeleteQueue clears every reference to a deleted dead-letter queue");
            for message in received.dead_lettered {
                target.add(message);
            }
        }
        Ok(Output::Received {
            messages: received.delivered,
        })
    }
}

fn expired_handle(receipt: &str) -> ApiError {
    ApiError::new(
        ErrorCode::ReceiptHandleExpired,
        format!("receipt handle {receipt} is not the latest handle of an in-flight message"),
    )
}
