//! The service: all queues, the virtual clock and the global counters, and
//! the execution of one command at a time.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::attributes::{
    AttributeRequest, AttributesView, DeadLetter, DeadLetterRequest, QueueKind, Update,
};
use crate::error::{Error, ErrorCode, Result};
use crate::protocol::{self, HandleText, Request, Send};
use crate::queue::{Accept, Counts, Delivery, FifoAccept, Queue, Received};
use crate::types::{
    DedupKey, DelaySeconds, MaxReceiveCount, MessageId, QueueName, Ranged, ReceiptHandle,
    RetentionSeconds, SequenceNumber, Time, VisibilityTimeout,
};

const MAX_QUEUES: usize = 1000;

#[derive(Debug)]
pub struct Broker {
    queues: BTreeMap<QueueName, Queue>,
    clock: Time,
    next_message: MessageId,
    next_handle: ReceiptHandle,
}

impl Default for Broker {
    fn default() -> Self {
        Self {
            queues: BTreeMap::new(),
            clock: Time::START,
            next_message: MessageId::FIRST,
            next_handle: ReceiptHandle::FIRST,
        }
    }
}

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Success<'a> {
    Done {},
    Now {
        #[serde(rename = "Now")]
        now: Time,
    },
    Queues {
        #[serde(rename = "QueueNames")]
        names: Vec<&'a QueueName>,
    },
    Attributes {
        #[serde(rename = "Attributes")]
        attributes: AttributesView<'a>,
        #[serde(rename = "Counts")]
        counts: Counts,
    },
    Sent {
        #[serde(rename = "MessageId")]
        message_id: MessageId,
        #[serde(rename = "SequenceNumber", skip_serializing_if = "Option::is_none")]
        sequence_number: Option<SequenceNumber>,
    },
    Received {
        #[serde(rename = "Messages")]
        messages: Vec<Delivery>,
    },
}

#[derive(Serialize)]
struct SuccessLine<'a, 'b> {
    ok: bool,
    #[serde(flatten)]
    result: &'b Success<'a>,
}

#[derive(Serialize)]
struct FailureLine<'a> {
    ok: bool,
    error: ErrorBody<'a>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: ErrorCode,
    message: &'a str,
}

impl Broker {
    /// Runs one input line and returns the response line, without its
    /// newline. Every line gets exactly one response.
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        let outcome = protocol::parse(line).and_then(|request| self.execute(request));
        let rendered = match &outcome {
            Ok(result) => serde_json::to_string(&SuccessLine { ok: true, result }),
            Err(error) => serde_json::to_string(&FailureLine {
                ok: false,
                error: ErrorBody {
                    code: error.code(),
                    message: error.message(),
                },
            }),
        };
        rendered.expect("responses contain only string keys and serialize infallibly")
    }

    fn execute(&mut self, request: Request) -> Result<Success<'_>> {
        match request {
            Request::CreateQueue { name, attributes } => self.create_queue(name, attributes),
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, attributes)
            }
            Request::GetQueueAttributes { queue } => {
                let (_, found) = lookup(&self.queues, &queue)?;
                Ok(Success::Attributes {
                    attributes: found.attributes().view(),
                    counts: found.counts(self.clock),
                })
            }
            Request::ListQueues { prefix } => {
                let prefix = prefix.as_deref().unwrap_or("");
                let names = self
                    .queues
                    .keys()
                    .filter(|name| name.as_str().starts_with(prefix))
                    .collect();
                Ok(Success::Queues { names })
            }
            Request::DeleteQueue { queue } => self.delete_queue(&queue),
            Request::PurgeQueue { queue } => {
                find(&mut self.queues, &queue)?.purge();
                Ok(Success::Done {})
            }
            Request::Tick { seconds } => {
                self.clock = self.clock.plus(seconds);
                for queue in self.queues.values_mut() {
                    queue.expire(self.clock);
                }
                Ok(Success::Now { now: self.clock })
            }
            Request::SendMessage(send) => self.send_message(send),
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Request::DeleteMessage { queue, handle } => {
                let target = find(&mut self.queues, &queue)?;
                let handle = issued(self.next_handle, &handle)?;
                target.delete(handle, self.clock)?;
                Ok(Success::Done {})
            }
            Request::ChangeMessageVisibility {
                queue,
                handle,
                visibility_timeout,
            } => {
                let target = find(&mut self.queues, &queue)?;
                let handle = issued(self.next_handle, &handle)?;
                target.change_visibility(handle, visibility_timeout, self.clock)?;
                Ok(Success::Done {})
            }
        }
    }

    fn create_queue(&mut self, name: QueueName, request: AttributeRequest) -> Result<Success<'_>> {
        let fifo = request.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "the .fifo suffix must be present exactly when Fifo is true",
            ));
        }
        let content_based_deduplication = request.content_based_deduplication.unwrap_or(false);
        if content_based_deduplication && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                "can only be true on a FIFO queue",
            ));
        }
        let dead_letter = dead_letter_pair(request.dead_letter)?;
        let visibility_timeout = request
            .visibility_timeout
            .unwrap_or(VisibilityTimeout::DEFAULT);
        let delay = request.delay.unwrap_or(DelaySeconds::DEFAULT);
        let retention = request.retention.unwrap_or(RetentionSeconds::DEFAULT);
        let kind = if fifo {
            QueueKind::Fifo {
                content_based_deduplication,
            }
        } else {
            QueueKind::Standard
        };

        if let Some(existing) = self.queues.get(&name) {
            let current = existing.attributes();
            let current_dead_letter = current
                .dead_letter
                .as_ref()
                .map(|d| (d.queue.as_str(), d.max_receive_count));
            let requested_dead_letter = dead_letter.as_ref().map(|(q, c)| (q.as_str(), *c));
            let same = current.visibility_timeout == visibility_timeout
                && current.delay == delay
                && current.retention == retention
                && current.kind == kind
                && current_dead_letter == requested_dead_letter;
            return if same {
                Ok(Success::Done {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {} exists with different attributes", name.as_str()),
                ))
            };
        }

        let dead_letter = dead_letter
            .map(|(queue, max_receive_count)| {
                let (target, found) = lookup(&self.queues, &queue)?;
                if found.attributes().kind.is_fifo() != fifo {
                    return Err(Error::invalid_value(
                        "DeadLetterQueue",
                        "must be of the same queue type",
                    ));
                }
                Ok(DeadLetter {
                    queue: target.clone(),
                    max_receive_count,
                })
            })
            .transpose()?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        let attributes = crate::attributes::QueueAttributes {
            visibility_timeout,
            delay,
            retention,
            kind,
            dead_letter,
        };
        self.queues.insert(name, Queue::new(attributes));
        Ok(Success::Done {})
    }

    fn set_queue_attributes(
        &mut self,
        queue: &str,
        request: AttributeRequest,
    ) -> Result<Success<'_>> {
        let (name, existing) = lookup(&self.queues, queue)?;
        let is_fifo = existing.attributes().kind.is_fifo();
        if request.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot be changed",
            ));
        }
        if request.content_based_deduplication == Some(true) && !is_fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                "can only be true on a FIFO queue",
            ));
        }
        let dead_letter = dead_letter_pair(request.dead_letter)?
            .map(|(target, max_receive_count)| {
                let (target, found) = lookup(&self.queues, &target)?;
                if target == name || found.attributes().kind.is_fifo() != is_fifo {
                    return Err(Error::invalid_value(
                        "DeadLetterQueue",
                        "must be another queue of the same type",
                    ));
                }
                Ok(DeadLetter {
                    queue: target.clone(),
                    max_receive_count,
                })
            })
            .transpose()?;
        let update = Update {
            visibility_timeout: request.visibility_timeout,
            delay: request.delay,
            retention: request.retention,
            content_based_deduplication: request.content_based_deduplication,
            dead_letter,
        };
        find(&mut self.queues, queue)?.update(update, self.clock);
        Ok(Success::Done {})
    }

    fn delete_queue(&mut self, queue: &str) -> Result<Success<'_>> {
        lookup(&self.queues, queue)?;
        if let Some((name, _)) = self.queues.remove_entry(queue) {
            for other in self.queues.values_mut() {
                other.forget_dead_letter_queue(&name);
            }
        }
        Ok(Success::Done {})
    }

    fn send_message(&mut self, send: Send) -> Result<Success<'_>> {
        let now = self.clock;
        let queue = find(&mut self.queues, &send.queue)?;
        let kind = queue.attributes().kind;
        let unsupported = match kind {
            QueueKind::Standard => send.group.is_some() || send.deduplication.is_some(),
            QueueKind::Fifo { .. } => send.delay.is_some(),
        };
        if unsupported {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "this parameter is not supported by this type of queue",
            ));
        }
        let fifo = match kind {
            QueueKind::Standard => None,
            QueueKind::Fifo {
                content_based_deduplication,
            } => {
                let group = send.group.ok_or_else(|| Error::missing("GroupId"))?;
                let deduplication = match send.deduplication {
                    Some(key) => key,
                    None if content_based_deduplication => DedupKey::from(&send.body),
                    None => return Err(Error::missing("DeduplicationId")),
                };
                if let Some(original) = queue.original_of(&deduplication, now) {
                    return Ok(Success::Sent {
                        message_id: original.id,
                        sequence_number: Some(original.sequence),
                    });
                }
                Some(FifoAccept {
                    group,
                    deduplication,
                })
            }
        };
        if queue.is_full() {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {} messages", Queue::MAX_MESSAGES),
            ));
        }
        let id = self.next_message;
        self.next_message = id.next();
        let delay = send.delay.unwrap_or(queue.attributes().delay);
        let sequence_number = queue.accept(
            Accept {
                id,
                body: send.body,
                delay,
                fifo,
            },
            now,
        );
        Ok(Success::Sent {
            message_id: id,
            sequence_number,
        })
    }

    fn receive_message(
        &mut self,
        queue: &str,
        max: crate::types::MaxMessages,
        visibility_timeout: Option<VisibilityTimeout>,
    ) -> Result<Success<'_>> {
        let now = self.clock;
        let source = find(&mut self.queues, queue)?;
        let visibility = visibility_timeout.unwrap_or(source.attributes().visibility_timeout);
        let next_handle = &mut self.next_handle;
        let mut issue = || {
            let handle = *next_handle;
            *next_handle = handle.next();
            handle
        };
        let Received { delivered, dead } = source.receive(now, max, visibility, &mut issue);
        if !dead.is_empty() {
            let target = source
                .dead_letter()
                .map(|d| d.queue.clone())
                .expect("messages are dead-lettered only by a queue with a dead-letter queue");
            let target = self
                .queues
                .get_mut(target.as_str())
                .expect("a dead-letter queue exists while any queue refers to it");
            for message in dead {
                target.admit(message, now);
            }
        }
        Ok(Success::Received {
            messages: delivered,
        })
    }
}

fn lookup<'a>(
    queues: &'a BTreeMap<QueueName, Queue>,
    name: &str,
) -> Result<(&'a QueueName, &'a Queue)> {
    queues
        .get_key_value(name)
        .ok_or_else(|| does_not_exist(name))
}

fn find<'a>(queues: &'a mut BTreeMap<QueueName, Queue>, name: &str) -> Result<&'a mut Queue> {
    queues.get_mut(name).ok_or_else(|| does_not_exist(name))
}

fn does_not_exist(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

/// Both dead-letter members or neither; one alone is a bad request.
fn dead_letter_pair(request: DeadLetterRequest) -> Result<Option<(String, MaxReceiveCount)>> {
    match request {
        DeadLetterRequest::Absent => Ok(None),
        DeadLetterRequest::Both {
            queue,
            max_receive_count,
        } => Ok(Some((queue, max_receive_count))),
        DeadLetterRequest::QueueOnly => Err(Error::invalid_value(
            "MaxReceiveCount",
            format!(
                "is required with DeadLetterQueue (from {} to {})",
                MaxReceiveCount::MIN,
                MaxReceiveCount::MAX
            ),
        )),
        DeadLetterRequest::CountOnly => Err(Error::invalid_value(
            "DeadLetterQueue",
            "is required with MaxReceiveCount",
        )),
    }
}

/// A handle miniq never produced is invalid; one it produced earlier is
/// reported by the queue as expired when it is no longer usable.
fn issued(next_unissued: ReceiptHandle, handle: &HandleText) -> Result<ReceiptHandle> {
    match handle {
        HandleText::Canonical(handle) if *handle < next_unissued => Ok(*handle),
        HandleText::Canonical(_) | HandleText::Malformed => Err(Error::new(
            ErrorCode::ReceiptHandleInvalid,
            "the receipt handle was never issued",
        )),
    }
}
