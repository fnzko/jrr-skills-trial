//! The command engine: queues, the clock and the process-wide counters.

use std::collections::BTreeMap;
use std::ops::Bound;

use crate::error::{Error, ErrorCode};
use crate::queue::{Attributes, DeadLetter, Queue};
use crate::request::{self, AttributeSet, Command, NewMessage};
use crate::response::{self, AttributesView, Output};
use crate::types::{Ids, MaxMessages, QueueName, TickSeconds, VisibilityTimeout};

const MAX_QUEUES: usize = 1000;

#[derive(Debug, Default)]
pub struct Service {
    clock: u64,
    ids: Ids,
    queues: BTreeMap<QueueName, Queue>,
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one input line and returns its response line, without the
    /// trailing newline.
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        let result = request::parse(line).and_then(|command| self.execute(command));
        response::render(&result)
    }

    fn execute(&mut self, command: Command) -> Result<Output, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => {
                let queue = self.queue(&queue)?;
                Ok(Output::Attributes {
                    attributes: AttributesView::from(queue.attributes()),
                    counts: queue.counts(),
                })
            }
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref().unwrap_or(""))),
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                self.queue(&queue)?.purge();
                Ok(Output::Nothing {})
            }
            Command::Tick { seconds } => Ok(self.tick(seconds)),
            Command::SendMessage { queue, message } => self.send_message(&queue, message),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => self.delete_message(&queue, &receipt_handle),
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => self.change_message_visibility(&queue, &receipt_handle, visibility_timeout),
        }
    }

    /// Error-precedence step 4: the named queue, brought up to date.
    fn queue(&mut self, name: &str) -> Result<&mut Queue, Error> {
        queue_at(&mut self.queues, name, self.clock)
    }

    fn create_queue(&mut self, name: QueueName, set: &AttributeSet) -> Result<Output, Error> {
        let fifo = set.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when Fifo is true",
            ));
        }
        if set.content_based_deduplication == Some(true) && !fifo {
            return Err(content_based_deduplication_needs_fifo());
        }
        check_dead_letter_pair(set)?;
        if let Some(existing) = self.queues.get(&name) {
            return if existing.attributes().matches_request(set) {
                Ok(Output::Nothing {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {} exists with different attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = self.resolve_dead_letter(set)?;
        if let Some((_, dead_letter_fifo)) = &dead_letter
            && *dead_letter_fifo != fifo
        {
            return Err(dead_letter_kind_mismatch());
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("{MAX_QUEUES} queues already exist"),
            ));
        }
        let attributes = Attributes::from_request(set, dead_letter.map(|(config, _)| config));
        self.queues.insert(name, Queue::new(attributes));
        Ok(Output::Nothing {})
    }

    fn set_queue_attributes(&mut self, name: &str, set: &AttributeSet) -> Result<Output, Error> {
        let kind = self.queue(name)?.attributes().kind;
        if set.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue never changes",
            ));
        }
        if set.content_based_deduplication == Some(true) && !kind.is_fifo() {
            return Err(content_based_deduplication_needs_fifo());
        }
        check_dead_letter_pair(set)?;
        let dead_letter = self.resolve_dead_letter(set)?;
        if let Some((config, dead_letter_fifo)) = &dead_letter
            && (config.queue.as_str() == name || *dead_letter_fifo != kind.is_fifo())
        {
            return Err(dead_letter_kind_mismatch());
        }
        let now = self.clock;
        self.queue(name)?
            .set_attributes(now, set, dead_letter.map(|(config, _)| config));
        Ok(Output::Nothing {})
    }

    /// The dead-letter configuration of `set`, with whether the named queue
    /// is a FIFO queue. The pair has been checked to be complete or absent.
    fn resolve_dead_letter(&self, set: &AttributeSet) -> Result<Option<(DeadLetter, bool)>, Error> {
        let (Some(name), Some(max_receive_count)) =
            (set.dead_letter_queue.as_deref(), set.max_receive_count)
        else {
            return Ok(None);
        };
        let (key, queue) = self.queues.get_key_value(name).ok_or_else(|| {
            Error::new(
                ErrorCode::QueueDoesNotExist,
                format!("dead-letter queue {name} does not exist"),
            )
        })?;
        let config = DeadLetter {
            queue: key.clone(),
            max_receive_count,
        };
        Ok(Some((config, queue.attributes().kind.is_fifo())))
    }

    fn list_queues(&self, prefix: &str) -> Output {
        let names = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name)
            .take_while(|name| name.as_str().starts_with(prefix))
            .cloned()
            .collect();
        Output::QueueNames { queue_names: names }
    }

    fn delete_queue(&mut self, name: &str) -> Result<Output, Error> {
        self.queue(name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            if queue
                .attributes()
                .dead_letter
                .as_ref()
                .is_some_and(|d| d.queue.as_str() == name)
            {
                queue.clear_dead_letter();
            }
        }
        Ok(Output::Nothing {})
    }

    fn tick(&mut self, seconds: TickSeconds) -> Output {
        self.clock += seconds.get();
        Output::Now { now: self.clock }
    }

    fn send_message(&mut self, name: &str, message: NewMessage) -> Result<Output, Error> {
        let Self { clock, ids, queues } = self;
        let queue = queue_at(queues, name, *clock)?;
        queue.send(*clock, ids, message).map(Output::from)
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: Option<MaxMessages>,
        visibility_timeout: Option<VisibilityTimeout>,
    ) -> Result<Output, Error> {
        let Self { clock, ids, queues } = self;
        let now = *clock;
        let queue = queue_at(queues, name, now)?;
        let visibility_timeout =
            visibility_timeout.unwrap_or(queue.attributes().visibility_timeout);
        let max_messages = max_messages.unwrap_or(MaxMessages::constant::<1>());
        let received = queue.receive(now, ids, max_messages, visibility_timeout);
        if !received.dead.is_empty() {
            let dead_letter_queue = queue
                .attributes()
                .dead_letter
                .as_ref()
                .expect("messages are only given up on under a dead-letter configuration")
                .queue
                .clone();
            let target = queues
                .get_mut(dead_letter_queue.as_str())
                .expect("a dead-letter configuration always names an existing queue");
            for message in received.dead {
                target.accept_dead(now, message);
            }
        }
        Ok(Output::Messages {
            messages: received.messages,
        })
    }

    fn delete_message(&mut self, name: &str, receipt_handle: &str) -> Result<Output, Error> {
        let Self { clock, ids, queues } = self;
        let queue = queue_at(queues, name, *clock)?;
        let sequence = in_flight(ids, queue, receipt_handle)?;
        queue.delete(sequence);
        Ok(Output::Nothing {})
    }

    fn change_message_visibility(
        &mut self,
        name: &str,
        receipt_handle: &str,
        visibility_timeout: VisibilityTimeout,
    ) -> Result<Output, Error> {
        let Self { clock, ids, queues } = self;
        let queue = queue_at(queues, name, *clock)?;
        let sequence = in_flight(ids, queue, receipt_handle)?;
        queue.change_visibility(*clock, sequence, visibility_timeout);
        Ok(Output::Nothing {})
    }
}

fn queue_at<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    name: &str,
    now: u64,
) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| {
        Error::new(
            ErrorCode::QueueDoesNotExist,
            format!("queue {name} does not exist"),
        )
    })?;
    queue.sync(now);
    Ok(queue)
}

/// The message that `receipt_handle` is the latest handle of, if it is in
/// flight in `queue`.
fn in_flight(
    ids: &Ids,
    queue: &Queue,
    receipt_handle: &str,
) -> Result<crate::types::Sequence, Error> {
    let handle = ids.issued_receipt_handle(receipt_handle).ok_or_else(|| {
        Error::new(
            ErrorCode::ReceiptHandleInvalid,
            format!("receipt handle {receipt_handle:?} was never issued"),
        )
    })?;
    queue.in_flight(handle).ok_or_else(|| {
        Error::new(
            ErrorCode::ReceiptHandleExpired,
            format!("receipt handle {receipt_handle} is no longer usable in this queue"),
        )
    })
}

fn check_dead_letter_pair(set: &AttributeSet) -> Result<(), Error> {
    if set.dead_letter_queue.is_some() == set.max_receive_count.is_some() {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::InvalidParameterValue,
            "DeadLetterQueue and MaxReceiveCount must be given together",
        ))
    }
}

fn content_based_deduplication_needs_fifo() -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        "ContentBasedDeduplication needs a FIFO queue",
    )
}

fn dead_letter_kind_mismatch() -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        "the dead-letter queue must be another queue of the same type",
    )
}
