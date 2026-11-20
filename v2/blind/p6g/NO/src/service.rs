use std::collections::BTreeMap;
use std::ops::Bound;
use std::time::Duration;

use crate::error::{Error, ErrorCode};
use crate::queue::{DeadLetter, MAX_MESSAGES, Message, Queue, QueueKind, Settings};
use crate::reply::{Reply, SequenceNumber};
use crate::request::{AttributeUpdate, Request};
use crate::types::{GroupId, Ids, QueueName};

const MAX_QUEUES: usize = 1000;

/// The whole state of a miniq process.
#[derive(Debug, Default)]
pub struct Service {
    now: Duration,
    queues: BTreeMap<QueueName, Queue>,
    ids: Ids,
}

/// The queue called `name`, with the messages whose retention ran out at
/// `now` already removed.
fn find_queue<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    name: &str,
    now: Duration,
) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| does_not_exist(name))?;
    queue.expire(now);
    Ok(queue)
}

/// The message of `queue` that is in flight under the handle spelled
/// `receipt_handle`.
fn in_flight(
    ids: &Ids,
    queue: &Queue,
    receipt_handle: &str,
    now: Duration,
) -> Result<SequenceNumber, Error> {
    let receipt = ids.issued_receipt(receipt_handle).ok_or_else(|| {
        Error::new(
            ErrorCode::ReceiptHandleInvalid,
            format!("receipt handle {receipt_handle:?} was never issued"),
        )
    })?;
    queue.in_flight(receipt, now).ok_or_else(|| {
        Error::new(
            ErrorCode::ReceiptHandleExpired,
            format!("receipt handle {receipt_handle:?} is not in flight in this queue"),
        )
    })
}

fn does_not_exist(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

fn unsupported(what: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, what)
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one input line and returns the response line, without its
    /// newline.
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        let result = Request::parse(line).and_then(|request| self.execute(request));
        render(&result)
    }

    fn execute(&mut self, request: Request) -> Result<Reply, Error> {
        match request {
            Request::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Request::GetQueueAttributes { queue } => {
                let queue = find_queue(&mut self.queues, &queue, self.now)?;
                Ok(Reply::QueueAttributes {
                    attributes: queue.attributes(),
                    counts: queue.counts(self.now),
                })
            }
            Request::ListQueues { prefix } => {
                let prefix = prefix.as_deref().unwrap_or("");
                // Names are ordered by bytes, so those with a prefix are
                // contiguous starting at the prefix itself.
                let queue_names = self
                    .queues
                    .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
                    .map(|(name, _)| name)
                    .take_while(|name| name.as_str().starts_with(prefix))
                    .cloned()
                    .collect();
                Ok(Reply::Queues { queue_names })
            }
            Request::DeleteQueue { queue } => {
                self.queues
                    .remove(queue.as_str())
                    .ok_or_else(|| does_not_exist(&queue))?;
                for other in self.queues.values_mut() {
                    other.forget_dead_letter(&queue);
                }
                Ok(Reply::Done {})
            }
            Request::PurgeQueue { queue } => {
                find_queue(&mut self.queues, &queue, self.now)?.purge();
                Ok(Reply::Done {})
            }
            Request::Tick { seconds } => {
                self.now += seconds;
                Ok(Reply::Clock { now: self.now })
            }
            Request::SendMessage {
                queue,
                body,
                delay,
                group,
                deduplication_id,
            } => self.send_message(&queue, body, delay, group, deduplication_id),
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Request::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let queue = find_queue(&mut self.queues, &queue, self.now)?;
                let sequence = in_flight(&self.ids, queue, &receipt_handle, self.now)?;
                queue.delete(sequence);
                Ok(Reply::Done {})
            }
            Request::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let queue = find_queue(&mut self.queues, &queue, self.now)?;
                let sequence = in_flight(&self.ids, queue, &receipt_handle, self.now)?;
                queue.hide_until(sequence, self.now + visibility_timeout);
                Ok(Reply::Done {})
            }
        }
    }

    /// The dead-letter queue `name` as a configuration, after checking that
    /// it exists and has the same type as a queue with `fifo`.
    fn dead_letter_for(
        &self,
        name: &str,
        max_receive_count: u32,
        fifo: bool,
    ) -> Result<DeadLetter, Error> {
        let (queue, target) = self
            .queues
            .get_key_value(name)
            .ok_or_else(|| does_not_exist(name))?;
        if target.settings().kind.is_fifo() != fifo {
            return Err(Error::invalid(
                "DeadLetterQueue",
                "must be a FIFO queue exactly when this queue is one",
            ));
        }
        Ok(DeadLetter {
            queue: queue.clone(),
            max_receive_count,
        })
    }

    fn create_queue(
        &mut self,
        name: QueueName,
        attributes: &AttributeUpdate,
    ) -> Result<Reply, Error> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.ends_in_fifo() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is FIFO",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid(
                "ContentBasedDeduplication",
                "needs a FIFO queue",
            ));
        }
        let dead_letter = attributes.dead_letter()?;
        let kind = if fifo {
            QueueKind::Fifo {
                content_based_deduplication: false,
            }
        } else {
            QueueKind::Standard
        };
        let settings = attributes.settings_over(Settings {
            kind,
            ..Settings::default()
        });

        if let Some(existing) = self.queues.get(&name) {
            let same_dead_letter = existing
                .dead_letter()
                .map(|d| (d.queue.as_str(), d.max_receive_count))
                == dead_letter;
            return if existing.settings() == settings && same_dead_letter {
                Ok(Reply::Done {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = dead_letter
            .map(|(queue, count)| self.dead_letter_for(queue, count, fifo))
            .transpose()?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        self.queues.insert(name, Queue::new(settings, dead_letter));
        Ok(Reply::Done {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        attributes: &AttributeUpdate,
    ) -> Result<Reply, Error> {
        let queue = find_queue(&mut self.queues, name, self.now)?;
        let current = queue.settings();
        if attributes.fifo.is_some() {
            return Err(unsupported("the type of a queue cannot change"));
        }
        if attributes.content_based_deduplication == Some(true) && !current.kind.is_fifo() {
            return Err(Error::invalid(
                "ContentBasedDeduplication",
                "needs a FIFO queue",
            ));
        }
        let dead_letter = match attributes.dead_letter()? {
            Some((queue, _)) if queue == name => {
                return Err(Error::invalid(
                    "DeadLetterQueue",
                    "must not be the queue itself",
                ));
            }
            Some((queue, count)) => {
                Some(self.dead_letter_for(queue, count, current.kind.is_fifo())?)
            }
            None => None,
        };
        let queue = find_queue(&mut self.queues, name, self.now)?;
        let dead_letter = dead_letter.or_else(|| queue.dead_letter().cloned());
        queue.configure(attributes.settings_over(current), dead_letter);
        // A lower retention takes effect at once.
        queue.expire(self.now);
        Ok(Reply::Done {})
    }

    fn send_message(
        &mut self,
        name: &str,
        body: String,
        delay: Option<Duration>,
        group: Option<GroupId>,
        deduplication_id: Option<String>,
    ) -> Result<Reply, Error> {
        let queue = find_queue(&mut self.queues, name, self.now)?;
        let settings = queue.settings();
        let deduplication = match settings.kind {
            QueueKind::Standard => {
                if group.is_some() || deduplication_id.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                None
            }
            QueueKind::Fifo {
                content_based_deduplication,
            } => {
                if delay.is_some() {
                    return Err(unsupported("DelaySeconds is set per queue on a FIFO queue"));
                }
                if group.is_none() {
                    return Err(Error::missing("GroupId"));
                }
                let id = match deduplication_id {
                    Some(id) => id,
                    None if content_based_deduplication => body.clone(),
                    None => return Err(Error::missing("DeduplicationId")),
                };
                if let Some(original) = queue.duplicate_of(&id, self.now) {
                    return Ok(Reply::Sent {
                        message_id: original.message_id,
                        sequence_number: Some(original.sequence_number),
                    });
                }
                Some(id)
            }
        };
        if queue.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }
        let message_id = self.ids.next_message();
        let delay = delay.unwrap_or(settings.delay);
        let sequence = queue.add(Message::sent(message_id, body, self.now, delay, group));
        let sequence_number = deduplication.map(|id| {
            queue.record_accepted(id, message_id, sequence, self.now);
            sequence
        });
        Ok(Reply::Sent {
            message_id,
            sequence_number,
        })
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: usize,
        visibility_timeout: Option<Duration>,
    ) -> Result<Reply, Error> {
        let queue = find_queue(&mut self.queues, name, self.now)?;
        let received = queue.receive(self.now, max_messages, visibility_timeout, &mut self.ids);
        if let Some(dead_letter) = queue.dead_letter().filter(|_| !received.dead.is_empty()) {
            let target = dead_letter.queue.clone();
            let target = self
                .queues
                .get_mut(&target)
                .expect("deleting a queue clears the dead-letter configurations naming it");
            for message in received.dead {
                target.add(message);
            }
        }
        Ok(Reply::Received {
            messages: received.delivered,
        })
    }
}

fn render(result: &Result<Reply, Error>) -> String {
    #[derive(serde::Serialize)]
    struct Success<'a> {
        ok: bool,
        #[serde(flatten)]
        reply: &'a Reply,
    }
    #[derive(serde::Serialize)]
    struct Failure<'a> {
        ok: bool,
        error: &'a Error,
    }
    let json = match result {
        Ok(reply) => serde_json::to_string(&Success { ok: true, reply }),
        Err(error) => serde_json::to_string(&Failure { ok: false, error }),
    };
    json.expect("responses are plain structs with string keys")
}
