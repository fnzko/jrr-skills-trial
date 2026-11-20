//! The state of the process and the execution of commands.

use std::collections::HashMap;

use crate::error::{Error, ErrorCode};
use crate::queue::{
    Config, DeadLetter, MAX_MESSAGES_PER_QUEUE, MAX_QUEUES, Message, Queue, QueueId, Receipts, Sent,
};
use crate::request::{Attributes, Request, SendMessage};
use crate::response::{
    self, AttributesView, CountsView, ListQueuesResult, QueueAttributesResult, ReceiveResult,
    ReceivedMessage, Response, SendResult, TickResult,
};

/// All state of one miniq process.
#[derive(Debug, Default)]
pub struct Service {
    /// Virtual clock in seconds; only `Tick` advances it.
    now: u64,
    queues: HashMap<String, Queue>,
    next_queue_id: u64,
    /// The number of messages accepted so far, which is the last `MessageId`.
    messages_accepted: u64,
    receipts: Receipts,
}

/// Finds a queue by its exact name and applies its retention up to `now`.
fn lookup<'a>(
    queues: &'a mut HashMap<String, Queue>,
    name: &str,
    now: u64,
) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| does_not_exist(name))?;
    queue.sweep(now);
    Ok(queue)
}

fn does_not_exist(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

/// `base` with the attributes of the request laid over it.
fn overlay(base: &Config, attributes: &Attributes, dead_letter: Option<DeadLetter>) -> Config {
    Config {
        visibility_timeout: attributes
            .visibility_timeout
            .unwrap_or(base.visibility_timeout),
        delay_seconds: attributes.delay_seconds.unwrap_or(base.delay_seconds),
        retention_seconds: attributes
            .retention_seconds
            .unwrap_or(base.retention_seconds),
        content_based_deduplication: attributes
            .content_based_deduplication
            .unwrap_or(base.content_based_deduplication),
        dead_letter: dead_letter.or_else(|| base.dead_letter.clone()),
    }
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }

    /// Executes one input line and returns its response line, without the newline.
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        response::render(Request::parse(line).and_then(|request| self.execute(request)))
    }

    fn execute(&mut self, request: Request) -> Result<Response, Error> {
        match request {
            Request::CreateQueue { name, attributes } => self
                .create_queue(name, attributes)
                .map(|()| Response::Empty {}),
            Request::SetQueueAttributes { queue, attributes } => self
                .set_queue_attributes(&queue, attributes)
                .map(|()| Response::Empty {}),
            Request::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Request::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref())),
            Request::DeleteQueue { queue } => {
                self.delete_queue(&queue).map(|()| Response::Empty {})
            }
            Request::PurgeQueue { queue } => {
                lookup(&mut self.queues, &queue, self.now)?.purge();
                Ok(Response::Empty {})
            }
            Request::SendMessage(request) => self.send_message(request),
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
            Request::Tick { seconds } => {
                self.now += u64::from(seconds);
                Ok(Response::Tick(TickResult { now: self.now }))
            }
        }
    }

    fn create_queue(&mut self, name: String, mut attributes: Attributes) -> Result<(), Error> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.ends_with(".fifo") != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "the .fifo suffix and the Fifo attribute must agree",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = attributes.dead_letter()?;
        let config = overlay(&Config::default(), &attributes, dead_letter);

        if let Some(existing) = self.queues.get(&name) {
            return if existing.is_fifo() == fifo && existing.config == config {
                Ok(())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {name:?} exists with different attributes"),
                ))
            };
        }
        if let Some(dead_letter) = &config.dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| does_not_exist(&dead_letter.queue))?;
            if target.is_fifo() != fifo {
                return Err(Error::invalid_value(
                    "the dead-letter queue must be of the same type",
                ));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }

        self.next_queue_id += 1;
        let queue = Queue::new(QueueId(self.next_queue_id), fifo, config);
        self.queues.insert(name, queue);
        Ok(())
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        mut attributes: Attributes,
    ) -> Result<(), Error> {
        let fifo = lookup(&mut self.queues, name, self.now)?.is_fifo();
        if attributes.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = attributes.dead_letter()?;
        if let Some(dead_letter) = &dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| does_not_exist(&dead_letter.queue))?;
            if dead_letter.queue == name || target.is_fifo() != fifo {
                return Err(Error::invalid_value(
                    "the dead-letter queue must be another queue of the same type",
                ));
            }
        }

        let queue = lookup(&mut self.queues, name, self.now)?;
        queue.config = overlay(&queue.config, &attributes, dead_letter);
        Ok(())
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Response, Error> {
        let queue = lookup(&mut self.queues, name, self.now)?;
        let config = &queue.config;
        let counts = queue.counts(self.now);
        Ok(Response::Attributes(QueueAttributesResult {
            attributes: AttributesView {
                visibility_timeout: config.visibility_timeout,
                delay_seconds: config.delay_seconds,
                retention_seconds: config.retention_seconds,
                fifo: queue.is_fifo(),
                content_based_deduplication: config.content_based_deduplication,
                dead_letter_queue: config.dead_letter.as_ref().map(|d| d.queue.clone()),
                max_receive_count: config.dead_letter.as_ref().map(|d| d.max_receive_count),
            },
            counts: CountsView {
                visible: counts.visible,
                in_flight: counts.in_flight,
                delayed: counts.delayed,
            },
        }))
    }

    fn list_queues(&self, prefix: Option<&str>) -> Response {
        let mut queue_names: Vec<String> = self
            .queues
            .keys()
            .filter(|name| name.starts_with(prefix.unwrap_or_default()))
            .cloned()
            .collect();
        // String ordering is byte-wise, which is the order the protocol asks for.
        queue_names.sort_unstable();
        Response::Queues(ListQueuesResult { queue_names })
    }

    fn delete_queue(&mut self, name: &str) -> Result<(), Error> {
        self.queues
            .remove(name)
            .ok_or_else(|| does_not_exist(name))?;
        for queue in self.queues.values_mut() {
            if queue
                .config
                .dead_letter
                .as_ref()
                .is_some_and(|d| d.queue == name)
            {
                queue.config.dead_letter = None;
            }
        }
        Ok(())
    }

    fn send_message(&mut self, request: SendMessage) -> Result<Response, Error> {
        let SendMessage {
            queue: name,
            body,
            delay_seconds,
            group_id,
            deduplication_id,
        } = request;
        let queue = lookup(&mut self.queues, &name, self.now)?;
        let unsupported = |what| Error::new(ErrorCode::UnsupportedOperation, what);

        let (group_id, deduplication_id) = if queue.is_fifo() {
            if delay_seconds.is_some() {
                return Err(unsupported("DelaySeconds is not supported on a FIFO queue"));
            }
            let group_id = group_id.ok_or_else(|| Error::missing("GroupId"))?;
            let deduplication_id = match deduplication_id {
                Some(id) => id,
                None if queue.config.content_based_deduplication => body.clone(),
                None => return Err(Error::missing("DeduplicationId")),
            };
            if let Some(original) = queue.find_duplicate(&deduplication_id, self.now) {
                return Ok(sent_response(original));
            }
            (Some(group_id), Some(deduplication_id))
        } else if group_id.is_some() || deduplication_id.is_some() {
            return Err(unsupported(
                "GroupId and DeduplicationId are supported on FIFO queues only",
            ));
        } else {
            (None, None)
        };
        if queue.len() >= MAX_MESSAGES_PER_QUEUE {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES_PER_QUEUE} messages"),
            ));
        }

        self.messages_accepted += 1;
        let delay = delay_seconds.unwrap_or(queue.config.delay_seconds);
        let message = Message {
            id: self.messages_accepted,
            body,
            sent_at: self.now,
            delayed_until: self.now + u64::from(delay),
            receive_count: 0,
            first_received_at: None,
            delivery: None,
            group: group_id,
            sequence: None,
        };
        Ok(sent_response(queue.send(
            message,
            deduplication_id,
            self.now,
        )))
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u32,
        visibility_timeout: Option<u32>,
    ) -> Result<Response, Error> {
        let queue = lookup(&mut self.queues, name, self.now)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.config.visibility_timeout);
        let received = queue.receive(
            self.now,
            max_messages as usize,
            visibility_timeout,
            &mut self.receipts,
        );

        if !received.dead.is_empty() {
            let target = queue
                .config
                .dead_letter
                .as_ref()
                .expect("messages only die in a queue with a dead-letter configuration")
                .queue
                .clone();
            let target = self
                .queues
                .get_mut(&target)
                .expect("a configured dead-letter queue exists");
            for message in received.dead {
                target.add_dead_letter(message, self.now);
            }
        }

        let messages = received
            .delivered
            .into_iter()
            .map(|message| ReceivedMessage {
                message_id: format!("m-{}", message.id),
                receipt_handle: format!(
                    "r-{}",
                    message
                        .delivery
                        .expect("a delivered message has a delivery")
                        .handle
                ),
                body: message.body,
                receive_count: message.receive_count,
                sent_at: message.sent_at,
                first_received_at: message
                    .first_received_at
                    .expect("a delivered message has been received"),
                group_id: message.group,
                sequence_number: message.sequence,
            })
            .collect();
        Ok(Response::Received(ReceiveResult { messages }))
    }

    fn delete_message(&mut self, name: &str, receipt_handle: &str) -> Result<Response, Error> {
        let queue = lookup(&mut self.queues, name, self.now)?;
        let (handle, key) = find_receipt(&self.receipts, queue, receipt_handle)?;
        if queue.delete_in_flight(key, handle, self.now) {
            Ok(Response::Empty {})
        } else {
            Err(expired(receipt_handle))
        }
    }

    fn change_message_visibility(
        &mut self,
        name: &str,
        receipt_handle: &str,
        visibility_timeout: u32,
    ) -> Result<Response, Error> {
        let queue = lookup(&mut self.queues, name, self.now)?;
        let (handle, key) = find_receipt(&self.receipts, queue, receipt_handle)?;
        if queue.change_visibility(key, handle, self.now, visibility_timeout) {
            Ok(Response::Empty {})
        } else {
            Err(expired(receipt_handle))
        }
    }
}

fn sent_response(sent: Sent) -> Response {
    Response::Sent(SendResult {
        message_id: format!("m-{}", sent.message_id),
        sequence_number: sent.sequence,
    })
}

fn expired(receipt_handle: &str) -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        format!("receipt handle {receipt_handle:?} is no longer usable"),
    )
}

/// The handle number and the message key of a receipt handle string, for a
/// handle that was issued for a message of `queue`.
fn find_receipt(
    receipts: &Receipts,
    queue: &Queue,
    receipt_handle: &str,
) -> Result<(u64, u64), Error> {
    let handle = receipts.parse(receipt_handle).ok_or_else(|| {
        Error::new(
            ErrorCode::ReceiptHandleInvalid,
            format!("receipt handle {receipt_handle:?} was never issued"),
        )
    })?;
    match receipts.location(handle) {
        Some((queue_id, key)) if queue_id == queue.id => Ok((handle, key)),
        _ => Err(expired(receipt_handle)),
    }
}
