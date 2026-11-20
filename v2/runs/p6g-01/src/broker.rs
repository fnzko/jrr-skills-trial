use std::collections::BTreeMap;
use std::ops::Bound;

use crate::error::{Error, ErrorCode};
use crate::ids::IdSource;
use crate::names::QueueName;
use crate::queue::{Attributes, DeadLetter, Queue};
use crate::request::{self, AttributePatch, Request};
use crate::response::{self, Outcome};

const MAX_QUEUES: usize = 1000;

type Queues = BTreeMap<QueueName, Queue>;

/// All state of a miniq process: the virtual clock, the queues and the id
/// counters.
#[derive(Debug, Default)]
pub struct Broker {
    now: u64,
    queues: Queues,
    ids: IdSource,
}

impl Broker {
    /// Runs one input line, without its line terminator, and returns the
    /// response line, without a terminator. Never fails: a bad line yields an
    /// error response and leaves the state alone.
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        response::render(request::parse(line).and_then(|request| self.execute(request)))
    }

    fn execute(&mut self, request: Request) -> Result<Outcome, Error> {
        match request {
            Request::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Request::GetQueueAttributes { queue } => {
                let queue = find(&self.queues, &queue)?;
                Ok(Outcome::Queue {
                    attributes: queue.attributes().clone(),
                    counts: queue.counts(self.now),
                })
            }
            Request::ListQueues { prefix } => {
                let prefix = prefix.as_deref().unwrap_or("");
                let queue_names = self
                    .queues
                    .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
                    .map(|(name, _)| name.as_str())
                    .take_while(|name| name.starts_with(prefix))
                    .map(str::to_owned)
                    .collect();
                Ok(Outcome::QueueNames { queue_names })
            }
            Request::DeleteQueue { queue } => self.delete_queue(&queue),
            Request::PurgeQueue { queue } => {
                find_mut(&mut self.queues, &queue)?.purge();
                Ok(Outcome::Done {})
            }
            Request::Tick { seconds } => {
                self.now += u64::from(seconds);
                for queue in self.queues.values_mut() {
                    queue.expire(self.now);
                }
                Ok(Outcome::Now { now: self.now })
            }
            Request::SendMessage { queue, message } => {
                let sent =
                    find_mut(&mut self.queues, &queue)?.send(message, self.now, &mut self.ids)?;
                Ok(Outcome::Sent(sent))
            }
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Request::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let queue = find_mut(&mut self.queues, &queue)?;
                let handle = self
                    .ids
                    .issued_handle(&receipt_handle)
                    .ok_or_else(invalid_handle)?;
                queue.delete_message(handle, self.now)?;
                Ok(Outcome::Done {})
            }
            Request::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let queue = find_mut(&mut self.queues, &queue)?;
                let handle = self
                    .ids
                    .issued_handle(&receipt_handle)
                    .ok_or_else(invalid_handle)?;
                queue.change_visibility(handle, visibility_timeout, self.now)?;
                Ok(Outcome::Done {})
            }
        }
    }

    fn create_queue(&mut self, name: QueueName, patch: &AttributePatch) -> Result<Outcome, Error> {
        let fifo = patch.fifo.unwrap_or(false);
        if name.is_fifo() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when Fifo is true",
            ));
        }
        if patch.content_based_deduplication == Some(true) && !fifo {
            return Err(content_based_on_standard_queue());
        }
        let dead_letter = dead_letter_of(patch)?;
        let mut attributes = Attributes::default();
        attributes.apply(patch, dead_letter);

        if let Some(existing) = self.queues.get(&name) {
            return if *existing.attributes() == attributes {
                Ok(Outcome::Done {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    "a queue with this name exists with different attributes",
                ))
            };
        }
        if let Some(dead_letter) = &attributes.dead_letter {
            let target = find(&self.queues, &dead_letter.queue)?;
            if target.attributes().fifo != fifo {
                return Err(dead_letter_type_mismatch());
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "the maximum number of queues exists",
            ));
        }
        self.queues.insert(name, Queue::new(attributes));
        Ok(Outcome::Done {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        patch: &AttributePatch,
    ) -> Result<Outcome, Error> {
        let fifo = find(&self.queues, name)?.attributes().fifo;
        if patch.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue never changes",
            ));
        }
        if patch.content_based_deduplication == Some(true) && !fifo {
            return Err(content_based_on_standard_queue());
        }
        let dead_letter = dead_letter_of(patch)?;
        if let Some(dead_letter) = &dead_letter {
            let target = find(&self.queues, &dead_letter.queue)?;
            if dead_letter.queue == name || target.attributes().fifo != fifo {
                return Err(dead_letter_type_mismatch());
            }
        }
        find_mut(&mut self.queues, name)
            .expect("the queue was found above and nothing was removed since")
            .set_attributes(patch, dead_letter, self.now);
        Ok(Outcome::Done {})
    }

    fn delete_queue(&mut self, name: &str) -> Result<Outcome, Error> {
        if self.queues.remove(name).is_none() {
            return Err(no_such_queue(name));
        }
        for queue in self.queues.values_mut() {
            queue.clear_dead_letter_to(name);
        }
        Ok(Outcome::Done {})
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u32,
        visibility_timeout: Option<u32>,
    ) -> Result<Outcome, Error> {
        let queue = find_mut(&mut self.queues, name)?;
        let visibility_timeout =
            visibility_timeout.unwrap_or(queue.attributes().visibility_timeout);
        let received = queue.receive(max_messages, visibility_timeout, self.now, &mut self.ids);

        if !received.dead_lettered.is_empty() {
            let target = queue
                .attributes()
                .dead_letter
                .as_ref()
                .map(|dead_letter| dead_letter.queue.clone())
                .expect("a message is only dead-lettered when a configuration exists");
            let target = find_mut(&mut self.queues, &target)
                .expect("deleting a queue clears every configuration that names it");
            for message in received.dead_lettered {
                target.accept_dead_letter(message, self.now);
            }
        }
        Ok(Outcome::Messages {
            messages: received.deliveries,
        })
    }
}

/// Pairs the dead-letter members of a patch: both are present or both absent.
fn dead_letter_of(patch: &AttributePatch) -> Result<Option<DeadLetter>, Error> {
    match (&patch.dead_letter_queue, patch.max_receive_count) {
        (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
            queue: queue.clone(),
            max_receive_count,
        })),
        (None, None) => Ok(None),
        _ => Err(Error::new(
            ErrorCode::InvalidParameterValue,
            "DeadLetterQueue and MaxReceiveCount must be given together",
        )),
    }
}

fn find<'q>(queues: &'q Queues, name: &str) -> Result<&'q Queue, Error> {
    queues.get(name).ok_or_else(|| no_such_queue(name))
}

fn find_mut<'q>(queues: &'q mut Queues, name: &str) -> Result<&'q mut Queue, Error> {
    queues.get_mut(name).ok_or_else(|| no_such_queue(name))
}

fn no_such_queue(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        &format!("queue {name:?} does not exist"),
    )
}

fn invalid_handle() -> Error {
    Error::new(
        ErrorCode::ReceiptHandleInvalid,
        "the receipt handle was never issued",
    )
}

fn content_based_on_standard_queue() -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        "ContentBasedDeduplication needs a FIFO queue",
    )
}

fn dead_letter_type_mismatch() -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        "the dead-letter queue must be another queue of the same type",
    )
}
