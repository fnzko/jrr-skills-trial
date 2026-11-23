//! The state of the whole process (clock, ids, queues) and the semantics of every command.

use std::collections::BTreeMap;
use std::ops::Bound;

use crate::attributes::{AttributePatch, QueueAttributes, QueueKind};
use crate::error::{Error, ErrorCode};
use crate::queue::{Incoming, Queue};
use crate::reply::{AttributesView, Reply};
use crate::request::{self, Command};
use crate::types::{
    DeduplicationId, GroupId, IdGenerator, QueueName, ReceiptHandle, Seconds, Time,
};

const MAX_QUEUES: usize = 1000;

#[derive(Debug, Default)]
pub struct Service {
    now: Time,
    ids: IdGenerator,
    queues: BTreeMap<QueueName, Queue>,
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses and runs one input line.
    pub fn handle_line(&mut self, line: &[u8]) -> Result<Reply, Error> {
        self.execute(request::parse(line)?)
    }

    pub fn execute(&mut self, command: Command) -> Result<Reply, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                Self::live_queue(&mut self.queues, &queue, self.now)?.purge();
                Ok(Reply::Done {})
            }
            Command::Tick { seconds } => {
                self.now = self.now.plus(seconds);
                Ok(Reply::Now { now: self.now })
            }
            Command::SendMessage {
                queue,
                body,
                delay,
                group,
                deduplication,
            } => self.send_message(&queue, body, delay, group, deduplication),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let target = Self::live_queue(&mut self.queues, &queue, self.now)?;
                let handle = Self::issued_handle(&self.ids, &receipt_handle)?;
                target
                    .delete(handle, self.now)
                    .map_err(|_| expired_handle())?;
                Ok(Reply::Done {})
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let target = Self::live_queue(&mut self.queues, &queue, self.now)?;
                let handle = Self::issued_handle(&self.ids, &receipt_handle)?;
                target
                    .change_visibility(handle, self.now, visibility_timeout)
                    .map_err(|_| expired_handle())?;
                Ok(Reply::Done {})
            }
        }
    }

    /// Looks a queue up by name and drops the messages whose retention has run out.
    fn live_queue<'q>(
        queues: &'q mut BTreeMap<QueueName, Queue>,
        name: &str,
        now: Time,
    ) -> Result<&'q mut Queue, Error> {
        let queue = queues
            .get_mut(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        queue.expire(now);
        Ok(queue)
    }

    fn issued_handle(ids: &IdGenerator, text: &str) -> Result<ReceiptHandle, Error> {
        ReceiptHandle::parse(text)
            .filter(|handle| ids.has_issued(*handle))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("{text:?} is not a receipt handle that was issued"),
                )
            })
    }

    fn create_queue(&mut self, name: QueueName, patch: &AttributePatch) -> Result<Reply, Error> {
        let fifo = patch.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is a FIFO queue",
            ));
        }
        if patch.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                "false on a standard queue",
            ));
        }
        let attributes = QueueAttributes::from_patch(patch, patch.dead_letter()?);
        if let Some(existing) = self.queues.get(name.as_str()) {
            return if *existing.attributes() == attributes {
                Ok(Reply::Done {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }
        if let Some(dead_letter) = &attributes.dead_letter {
            self.check_dead_letter_queue(&dead_letter.queue, fifo)?;
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        self.queues.insert(name, Queue::new(attributes));
        Ok(Reply::Done {})
    }

    /// The dead-letter queue must exist and be of the same type as the queue that uses it.
    fn check_dead_letter_queue(&self, name: &str, fifo: bool) -> Result<(), Error> {
        let target = self
            .queues
            .get(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        if target.attributes().is_fifo() == fifo {
            Ok(())
        } else {
            Err(Error::invalid_value(
                "DeadLetterQueue",
                "a queue of the same type as this queue",
            ))
        }
    }

    fn set_queue_attributes(&mut self, name: &str, patch: &AttributePatch) -> Result<Reply, Error> {
        let fifo = Self::live_queue(&mut self.queues, name, self.now)?
            .attributes()
            .is_fifo();
        if patch.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if patch.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication",
                "false on a standard queue",
            ));
        }
        let dead_letter = patch.dead_letter()?;
        if let Some(dead_letter) = &dead_letter {
            self.check_dead_letter_queue(&dead_letter.queue, fifo)?;
            if dead_letter.queue == name {
                return Err(Error::invalid_value(
                    "DeadLetterQueue",
                    "a queue other than this one",
                ));
            }
        }
        Self::live_queue(&mut self.queues, name, self.now)?
            .attributes_mut()
            .apply(patch, dead_letter);
        Ok(Reply::Done {})
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Reply, Error> {
        let queue = Self::live_queue(&mut self.queues, name, self.now)?;
        Ok(Reply::Attributes {
            attributes: AttributesView::from(queue.attributes()),
            counts: queue.counts(self.now),
        })
    }

    fn list_queues(&self, prefix: &str) -> Reply {
        let names = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name)
            .take_while(|name| name.as_str().starts_with(prefix))
            .cloned()
            .collect();
        Reply::QueueNames { queue_names: names }
    }

    fn delete_queue(&mut self, name: &str) -> Result<Reply, Error> {
        self.queues
            .remove(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        for queue in self.queues.values_mut() {
            queue.attributes_mut().forget_dead_letter(name);
        }
        Ok(Reply::Done {})
    }

    fn send_message(
        &mut self,
        name: &str,
        body: String,
        delay: Option<Seconds>,
        group: Option<GroupId>,
        deduplication: Option<DeduplicationId>,
    ) -> Result<Reply, Error> {
        let now = self.now;
        let queue = Self::live_queue(&mut self.queues, name, now)?;
        let (group, dedup_key) = match queue.attributes().kind {
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
                    return Err(unsupported("DelaySeconds is not supported on a FIFO queue"));
                }
                let group = group.ok_or_else(|| Error::missing("GroupId"))?;
                let key = match deduplication {
                    Some(id) => id.into_string(),
                    None if content_based_deduplication => body.clone(),
                    None => return Err(Error::missing("DeduplicationId")),
                };
                (Some(group), Some(key))
            }
        };
        if let Some(original) = dedup_key
            .as_deref()
            .and_then(|key| queue.find_duplicate(key, now))
        {
            return Ok(original.into());
        }
        if queue.is_full() {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "a queue holds at most 100000 messages",
            ));
        }
        let delay = delay.unwrap_or_else(|| queue.attributes().delay);
        let accepted = queue.append(Incoming {
            id: self.ids.next_message(),
            body,
            sent_at: now,
            delayed_until: now.plus(delay),
            group,
        });
        if let Some(key) = dedup_key {
            queue.remember(key, now, accepted);
        }
        Ok(accepted.into())
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: usize,
        visibility_timeout: Option<Seconds>,
    ) -> Result<Reply, Error> {
        let now = self.now;
        let queue = Self::live_queue(&mut self.queues, name, now)?;
        let visibility =
            visibility_timeout.unwrap_or_else(|| queue.attributes().visibility_timeout);
        let received = queue.receive(now, max_messages, visibility, &mut self.ids);
        if let Some(moved) = received.dead_lettered {
            let target = self.queues.get_mut(moved.queue.as_str()).expect(
                "a configured dead-letter queue exists: DeleteQueue clears every reference to it",
            );
            for message in moved.messages {
                target.append(message.into_incoming());
            }
            // The messages may already be older than the dead-letter queue's retention.
            target.expire(now);
        }
        Ok(Reply::Messages {
            messages: received.delivered,
        })
    }
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

fn expired_handle() -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        "the receipt handle is not the latest handle of a message that is in flight in this queue",
    )
}
