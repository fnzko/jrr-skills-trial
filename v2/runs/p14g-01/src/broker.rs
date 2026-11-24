//! The set of queues and the virtual clock: executes parsed commands.

use std::collections::BTreeMap;

use crate::error::{Error, Limit, NameProblem, Problem, Unsupported};
use crate::queue::{DeadLetter, Kind, Queue, Settings};
use crate::request::{Attributes, DeadLetterRequest, Request, SendParams};
use crate::response::{
    AttributesReply, MessagesReply, NowReply, QueueAttributes, QueueNamesReply, Reply,
};
use crate::types::{Ids, QueueName, ReceiptHandle, Seconds, Time};

const MAX_QUEUES: usize = 1000;

#[derive(Debug, Default)]
pub struct Broker {
    now: Time,
    queues: BTreeMap<QueueName, Queue>,
    ids: Ids,
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn execute(&mut self, request: Request) -> Result<Reply, Error> {
        match request {
            Request::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Request::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Request::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Request::DeleteQueue { queue } => self.delete_queue(&queue),
            Request::PurgeQueue { queue } => {
                queue_mut(&mut self.queues, &queue, self.now)?.purge();
                Ok(Reply::Empty {})
            }
            Request::Tick { seconds } => {
                self.now = self.now.after(seconds);
                Ok(Reply::Now(NowReply { now: self.now }))
            }
            Request::SendMessage { queue, send } => self.send_message(&queue, send),
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Request::DeleteMessage {
                queue,
                receipt_handle,
            } => self.delete_message(&queue, &receipt_handle),
            Request::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => self.change_message_visibility(&queue, &receipt_handle, visibility_timeout),
        }
    }

    fn create_queue(&mut self, name: QueueName, attributes: &Attributes) -> Result<Reply, Error> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::QueueNameInvalid {
                name: name.to_string(),
                problem: NameProblem::FifoMismatch,
            });
        }
        let content_based_deduplication = attributes.content_based_deduplication.unwrap_or(false);
        if content_based_deduplication && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                Problem::NeedsFifoQueue,
            ));
        }
        let requested_dead_letter = attributes.dead_letter()?;
        if let Some(existing) = self.queues.get(&name) {
            return if existing.matches(attributes, requested_dead_letter.as_ref()) {
                Ok(Reply::Empty {})
            } else {
                Err(Error::QueueAlreadyExists {
                    name: name.to_string(),
                })
            };
        }
        let dead_letter = requested_dead_letter
            .map(|request| self.resolve_dead_letter(&request, fifo, None))
            .transpose()?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::LimitExceeded(Limit::Queues));
        }
        let queue = Queue::new(
            Settings::from_request(attributes, dead_letter),
            Kind::new(fifo, content_based_deduplication),
        );
        self.queues.insert(name, queue);
        Ok(Reply::Empty {})
    }

    /// Checks that the dead-letter queue exists and is compatible with the queue it will serve.
    /// `owner` is the name of that queue when it already exists.
    fn resolve_dead_letter(
        &self,
        request: &DeadLetterRequest<'_>,
        fifo: bool,
        owner: Option<&str>,
    ) -> Result<DeadLetter, Error> {
        let (name, target) =
            self.queues
                .get_key_value(request.queue)
                .ok_or_else(|| Error::QueueDoesNotExist {
                    name: request.queue.to_owned(),
                })?;
        let problem = if owner == Some(request.queue) {
            Some(Problem::DeadLetterIsSelf)
        } else if target.is_fifo() != fifo {
            Some(Problem::DeadLetterKindMismatch)
        } else {
            None
        };
        match problem {
            Some(problem) => Err(Error::invalid_value("DeadLetterQueue", problem)),
            None => Ok(DeadLetter {
                queue: name.clone(),
                max_receive_count: request.max_receive_count,
            }),
        }
    }

    fn set_queue_attributes(
        &mut self,
        queue: &str,
        attributes: &Attributes,
    ) -> Result<Reply, Error> {
        let now = self.now;
        let fifo = queue_mut(&mut self.queues, queue, now)?.is_fifo();
        if attributes.fifo.is_some() {
            return Err(Error::UnsupportedOperation(Unsupported::FifoChange));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                Problem::NeedsFifoQueue,
            ));
        }
        let dead_letter = attributes
            .dead_letter()?
            .map(|request| self.resolve_dead_letter(&request, fifo, Some(queue)))
            .transpose()?;
        self.queues
            .get_mut(queue)
            .expect("the queue was found above and nothing removed it since")
            .apply(attributes, dead_letter, now);
        Ok(Reply::Empty {})
    }

    fn get_queue_attributes(&mut self, queue: &str) -> Result<Reply, Error> {
        let queue = queue_mut(&mut self.queues, queue, self.now)?;
        let settings = &queue.settings;
        let attributes = QueueAttributes {
            visibility_timeout: settings.visibility_timeout,
            delay_seconds: settings.delay,
            retention_seconds: settings.retention,
            fifo: queue.is_fifo(),
            content_based_deduplication: queue.content_based_deduplication(),
            dead_letter_queue: settings.dead_letter.as_ref().map(|d| d.queue.clone()),
            max_receive_count: settings.dead_letter.as_ref().map(|d| d.max_receive_count),
        };
        Ok(Reply::Attributes(AttributesReply {
            attributes,
            counts: queue.counts(self.now),
        }))
    }

    fn list_queues(&self, prefix: &str) -> Reply {
        let queue_names = self
            .queues
            .keys()
            .filter(|name| name.as_str().starts_with(prefix))
            .cloned()
            .collect();
        Reply::QueueNames(QueueNamesReply { queue_names })
    }

    fn delete_queue(&mut self, queue: &str) -> Result<Reply, Error> {
        self.queues
            .remove(queue)
            .ok_or_else(|| does_not_exist(queue))?;
        for other in self.queues.values_mut() {
            if other
                .settings
                .dead_letter
                .as_ref()
                .is_some_and(|d| d.queue.as_str() == queue)
            {
                other.settings.dead_letter = None;
            }
        }
        Ok(Reply::Empty {})
    }

    fn send_message(&mut self, queue: &str, send: SendParams) -> Result<Reply, Error> {
        let sent =
            queue_mut(&mut self.queues, queue, self.now)?.send(self.now, send, &mut self.ids)?;
        Ok(Reply::Sent(sent.into()))
    }

    fn receive_message(
        &mut self,
        queue: &str,
        max_messages: usize,
        visibility_timeout: Option<Seconds>,
    ) -> Result<Reply, Error> {
        let now = self.now;
        let received = queue_mut(&mut self.queues, queue, now)?.receive(
            now,
            max_messages,
            visibility_timeout,
            &mut self.ids,
        );
        if let Some(dead) = received.dead_lettered {
            let target = self
                .queues
                .get_mut(&dead.to)
                .expect("DeleteQueue clears every dead-letter configuration that names the queue");
            for message in dead.messages {
                target.accept_dead_letter(message, now);
            }
        }
        Ok(Reply::Messages(MessagesReply {
            messages: received.delivered.into_iter().map(Into::into).collect(),
        }))
    }

    fn delete_message(&mut self, queue: &str, raw_handle: &str) -> Result<Reply, Error> {
        let now = self.now;
        let queue = queue_mut(&mut self.queues, queue, now)?;
        let handle = parse_handle(raw_handle, &self.ids)?;
        queue.delete_message(handle, raw_handle, now)?;
        Ok(Reply::Empty {})
    }

    fn change_message_visibility(
        &mut self,
        queue: &str,
        raw_handle: &str,
        timeout: Seconds,
    ) -> Result<Reply, Error> {
        let now = self.now;
        let queue = queue_mut(&mut self.queues, queue, now)?;
        let handle = parse_handle(raw_handle, &self.ids)?;
        queue.change_visibility(handle, raw_handle, timeout, now)?;
        Ok(Reply::Empty {})
    }
}

fn does_not_exist(name: &str) -> Error {
    Error::QueueDoesNotExist {
        name: name.to_owned(),
    }
}

/// Finds a queue for a command and applies retention first, so the command sees only the
/// messages that still exist at `now`.
fn queue_mut<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    name: &str,
    now: Time,
) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| does_not_exist(name))?;
    queue.expire(now);
    Ok(queue)
}

fn parse_handle(raw: &str, ids: &Ids) -> Result<ReceiptHandle, Error> {
    ReceiptHandle::parse(raw, ids.issued_handles()).ok_or_else(|| Error::ReceiptHandleInvalid {
        handle: raw.to_owned(),
    })
}
