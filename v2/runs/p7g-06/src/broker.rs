//! The state of the service and the execution of commands against it.

use std::collections::BTreeMap;

use crate::error::{Error, ErrorCode};
use crate::handles::HandleLog;
use crate::queue::{DeadLetter, MAX_QUEUES, Queue, SendOutcome, Settings};
use crate::request::{AttributeSet, Document, Request, SendRequest};
use crate::response::{AttributesView, Reply};
use crate::types::{Instant, MessageId, QueueId, QueueName, Seconds};

/// All service state: the virtual clock, the queues and the message counter.
pub struct Broker {
    now: Instant,
    queues: BTreeMap<QueueName, Queue>,
    next_message_id: MessageId,
    next_queue_id: QueueId,
    handles: HandleLog,
}

impl Broker {
    pub fn new() -> Self {
        Self {
            now: Instant::START,
            queues: BTreeMap::new(),
            next_message_id: MessageId::FIRST,
            next_queue_id: QueueId::FIRST,
            handles: HandleLog::default(),
        }
    }

    /// Runs the command on one input line, which has no trailing newline.
    pub fn handle_line(&mut self, line: &[u8]) -> Result<Reply, Error> {
        let document = Document::parse(line)?;
        match document.request()? {
            Request::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(queue, &attributes)
            }
            Request::GetQueueAttributes { queue } => {
                let now = self.now;
                let queue = self.queue_mut(queue)?;
                Ok(Reply::Attributes {
                    attributes: AttributesView::from(&queue.settings),
                    counts: queue.counts(now),
                })
            }
            Request::ListQueues { prefix } => Ok(Reply::Queues {
                queue_names: self
                    .queues
                    .keys()
                    .filter(|name| name.as_str().starts_with(prefix.unwrap_or_default()))
                    .cloned()
                    .collect(),
            }),
            Request::DeleteQueue { queue } => self.delete_queue(queue),
            Request::PurgeQueue { queue } => {
                self.queue_mut(queue)?.purge();
                Ok(Reply::Empty {})
            }
            Request::Tick { seconds } => {
                self.now = self.now.plus(seconds);
                Ok(Reply::Tick { now: self.now })
            }
            Request::SendMessage(request) => self.send_message(request),
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(queue, max_messages, visibility_timeout),
            Request::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let now = self.now;
                let queue = find_queue(&mut self.queues, now, queue)?;
                let issued = self.handles.resolve(receipt_handle)?;
                queue.delete_message(issued, now)?;
                Ok(Reply::Empty {})
            }
            Request::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let now = self.now;
                let queue = find_queue(&mut self.queues, now, queue)?;
                let issued = self.handles.resolve(receipt_handle)?;
                queue.change_visibility(issued, visibility_timeout, now)?;
                Ok(Reply::Empty {})
            }
        }
    }

    fn queue_mut(&mut self, name: &str) -> Result<&mut Queue, Error> {
        find_queue(&mut self.queues, self.now, name)
    }

    /// Checks a requested dead-letter queue against the queue `owner` that will use it.
    fn dead_letter_target(
        &self,
        queue: &str,
        max_receive_count: u32,
        owner: &str,
        owner_is_fifo: bool,
    ) -> Result<DeadLetter, Error> {
        let (name, target) = self.queues.get_key_value(queue).ok_or_else(|| {
            Error::new(
                ErrorCode::QueueDoesNotExist,
                format!("dead-letter queue {queue:?} does not exist"),
            )
        })?;
        if name.as_str() == owner || target.settings.kind.is_fifo() != owner_is_fifo {
            return Err(Error::invalid(
                "DeadLetterQueue",
                "must be another queue of the same type",
            ));
        }
        Ok(DeadLetter {
            queue: name.clone(),
            max_receive_count,
        })
    }

    fn create_queue(
        &mut self,
        name: QueueName,
        attributes: &AttributeSet<'_>,
    ) -> Result<Reply, Error> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.is_fifo() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is FIFO",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(content_based_deduplication_needs_fifo());
        }
        let dead_letter = attributes.dead_letter()?;
        if let Some(existing) = self.queues.get(&name) {
            return if existing.settings.matches(attributes) {
                Ok(Reply::Empty {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {} exists with other attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = dead_letter
            .map(|(queue, count)| self.dead_letter_target(queue, count, name.as_str(), fifo))
            .transpose()?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("{MAX_QUEUES} queues already exist"),
            ));
        }
        let mut settings = Settings::defaults(fifo);
        settings.update(attributes, dead_letter);
        let id = self.next_queue_id;
        self.next_queue_id = id.next();
        self.queues.insert(name, Queue::new(id, settings));
        Ok(Reply::Empty {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        attributes: &AttributeSet<'_>,
    ) -> Result<Reply, Error> {
        let fifo = self.queue_mut(name)?.settings.kind.is_fifo();
        if attributes.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(content_based_deduplication_needs_fifo());
        }
        let dead_letter = attributes
            .dead_letter()?
            .map(|(queue, count)| self.dead_letter_target(queue, count, name, fifo))
            .transpose()?;
        let now = self.now;
        let queue = self.queue_mut(name)?;
        queue.settings.update(attributes, dead_letter);
        // A shorter retention takes effect on the existing messages at once.
        queue.expire(now);
        Ok(Reply::Empty {})
    }

    fn delete_queue(&mut self, name: &str) -> Result<Reply, Error> {
        if self.queues.remove(name).is_none() {
            return Err(Error::new(
                ErrorCode::QueueDoesNotExist,
                format!("queue {name:?} does not exist"),
            ));
        }
        for queue in self.queues.values_mut() {
            if queue
                .settings
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue.as_str() == name)
            {
                queue.settings.dead_letter = None;
            }
        }
        Ok(Reply::Empty {})
    }

    fn send_message(&mut self, request: SendRequest<'_>) -> Result<Reply, Error> {
        let (id, now) = (self.next_message_id, self.now);
        let queue = self.queue_mut(request.queue)?;
        match queue.send(request, id, now)? {
            SendOutcome::Added(sent) => {
                self.next_message_id = id.next();
                Ok(Reply::Sent(sent))
            }
            SendOutcome::Duplicate(sent) => Ok(Reply::Sent(sent)),
        }
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
    ) -> Result<Reply, Error> {
        let now = self.now;
        let queue = find_queue(&mut self.queues, now, name)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.settings.visibility_timeout);
        let outcome = queue.receive(max_messages, visibility_timeout, now, &mut self.handles);
        if let Some(moved) = outcome.dead_lettered {
            let target = self
                .queues
                .get_mut(moved.queue.as_str())
                .expect("a configured dead-letter queue always exists");
            for message in moved.messages {
                target.accept_dead_letter(message, now);
            }
        }
        Ok(Reply::Messages {
            messages: outcome.delivered,
        })
    }
}

/// Looks a queue up by exactly this name and drops its expired messages.
fn find_queue<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    now: Instant,
    name: &str,
) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| {
        Error::new(
            ErrorCode::QueueDoesNotExist,
            format!("queue {name:?} does not exist"),
        )
    })?;
    queue.expire(now);
    Ok(queue)
}

impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}

fn content_based_deduplication_needs_fifo() -> Error {
    Error::invalid(
        "ContentBasedDeduplication",
        "can only be true on a FIFO queue",
    )
}
