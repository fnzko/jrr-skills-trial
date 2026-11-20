//! The state of the whole process and the execution of requests.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::attributes::{AttributePatch, Attributes};
use crate::error::{Error, ErrorCode};
use crate::ids::{MessageId, ReceiptHandle};
use crate::name::QueueName;
use crate::queue::{Delivery, Queue, Submission};
use crate::request::Request;

const QUEUE_LIMIT: usize = 1000;

/// Result members of a successful command.
type Members = Map<String, Value>;

/// All queues of the process and its virtual clock.
#[derive(Debug)]
pub struct Service {
    now: u64,
    /// Sorted by name, which is the byte order `ListQueues` promises.
    queues: BTreeMap<String, Queue>,
    /// The number the next accepted message gets.
    next_message: u64,
    /// The number the next delivery's receipt handle gets.
    next_handle: u64,
}

impl Default for Service {
    fn default() -> Self {
        Self {
            now: 0,
            queues: BTreeMap::new(),
            next_message: 1,
            next_handle: 1,
        }
    }
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }

    /// Executes one input line and returns the response object.
    pub fn handle(&mut self, line: &[u8]) -> Value {
        match Request::parse(line).and_then(|request| self.execute(request)) {
            Ok(mut members) => {
                members.insert("ok".into(), Value::Bool(true));
                Value::Object(members)
            }
            Err(error) => json!({
                "ok": false,
                "error": {"code": error.code().as_str(), "message": error.message()},
            }),
        }
    }

    fn execute(&mut self, request: Request) -> Result<Members, Error> {
        match request {
            Request::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Request::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Request::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref())),
            Request::DeleteQueue { queue } => self.delete_queue(&queue),
            Request::PurgeQueue { queue } => {
                self.queue(&queue)?.purge();
                Ok(Members::new())
            }
            Request::SendMessage { queue, submission } => self.send_message(&queue, submission),
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Request::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let now = self.now;
                let (queue, handle) = self.queue_and_handle(&queue, &receipt_handle)?;
                queue.delete(handle, now)?;
                Ok(Members::new())
            }
            Request::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let now = self.now;
                let (queue, handle) = self.queue_and_handle(&queue, &receipt_handle)?;
                queue.change_visibility(handle, visibility_timeout, now)?;
                Ok(Members::new())
            }
            Request::Tick { seconds } => {
                self.now += seconds;
                Ok(members([("Now", json!(self.now))]))
            }
        }
    }

    /// The named queue, with the messages that expired by now removed.
    fn queue(&mut self, name: &str) -> Result<&mut Queue, Error> {
        let now = self.now;
        let queue = self
            .queues
            .get_mut(name)
            .ok_or_else(|| does_not_exist(name))?;
        queue.expire(now);
        Ok(queue)
    }

    /// Looks up the queue, then checks that `handle` was ever issued.
    fn queue_and_handle(
        &mut self,
        name: &str,
        handle: &str,
    ) -> Result<(&mut Queue, ReceiptHandle), Error> {
        let next_unissued = self.next_handle;
        let queue = self.queue(name)?;
        let handle = ReceiptHandle::parse_issued(handle, next_unissued).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("{handle:?} is not a receipt handle this process issued"),
            )
        })?;
        Ok((queue, handle))
    }

    fn create_queue(&mut self, name: QueueName, patch: &AttributePatch) -> Result<Members, Error> {
        let fifo = patch.fifo.unwrap_or(false);
        if name.is_fifo() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue is FIFO exactly when its name ends in `.fifo`",
            ));
        }
        if patch.content_based_deduplication == Some(true) && !fifo {
            return Err(content_based_deduplication_needs_fifo());
        }
        let dead_letter = patch.dead_letter()?;
        let mut attributes = Attributes::default();
        attributes.apply(patch, dead_letter);

        if let Some(existing) = self.queues.get(name.as_str()) {
            return if *existing.attributes() == attributes {
                Ok(Members::new())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }
        if let Some(dead_letter) = &attributes.dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| does_not_exist(&dead_letter.queue))?;
            if target.attributes().fifo != attributes.fifo {
                return Err(dead_letter_type_mismatch());
            }
        }
        if self.queues.len() >= QUEUE_LIMIT {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {QUEUE_LIMIT} queues can exist"),
            ));
        }
        self.queues
            .insert(name.into_string(), Queue::new(attributes));
        Ok(Members::new())
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        patch: &AttributePatch,
    ) -> Result<Members, Error> {
        let now = self.now;
        let is_fifo = self.queue(name)?.attributes().fifo;
        if patch.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if patch.content_based_deduplication == Some(true) && !is_fifo {
            return Err(content_based_deduplication_needs_fifo());
        }
        let dead_letter = patch.dead_letter()?;
        if let Some(dead_letter) = &dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| does_not_exist(&dead_letter.queue))?;
            if dead_letter.queue == name || target.attributes().fifo != is_fifo {
                return Err(dead_letter_type_mismatch());
            }
        }
        self.queue(name)?.set_attributes(patch, dead_letter, now);
        Ok(Members::new())
    }

    fn send_message(&mut self, name: &str, submission: Submission) -> Result<Members, Error> {
        let (now, id) = (self.now, MessageId::new(self.next_message));
        let sent = self.queue(name)?.send(submission, id, now)?;
        if !sent.duplicate {
            self.next_message += 1;
        }
        let mut result = members([("MessageId", json!(sent.id.to_string()))]);
        if let Some(sequence_number) = sent.sequence_number {
            result.insert("SequenceNumber".into(), json!(sequence_number));
        }
        Ok(result)
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u64,
        visibility_timeout: Option<u64>,
    ) -> Result<Members, Error> {
        let now = self.now;
        let mut next_handle = self.next_handle;
        let received =
            self.queue(name)?
                .receive(max_messages, visibility_timeout, now, &mut next_handle);
        self.next_handle = next_handle;
        if let Some(dead) = received.dead_letters {
            self.queues
                .get_mut(&dead.queue)
                .expect("deleting a queue clears the dead-letter configurations naming it")
                .adopt(dead.messages, now);
        }
        let messages: Vec<Value> = received.deliveries.iter().map(delivery_json).collect();
        Ok(members([("Messages", Value::Array(messages))]))
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Members, Error> {
        let now = self.now;
        let queue = self.queue(name)?;
        let counts = queue.counts(now);
        Ok(members([
            ("Attributes", queue.attributes().to_json()),
            (
                "Counts",
                json!({
                    "Visible": counts.visible,
                    "InFlight": counts.in_flight,
                    "Delayed": counts.delayed,
                }),
            ),
        ]))
    }

    fn list_queues(&self, prefix: Option<&str>) -> Members {
        let prefix = prefix.unwrap_or("");
        let names: Vec<&String> = self
            .queues
            .keys()
            .filter(|name| name.starts_with(prefix))
            .collect();
        members([("QueueNames", json!(names))])
    }

    fn delete_queue(&mut self, name: &str) -> Result<Members, Error> {
        self.queues
            .remove(name)
            .ok_or_else(|| does_not_exist(name))?;
        for queue in self.queues.values_mut() {
            if queue
                .attributes()
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue == name)
            {
                queue.clear_dead_letter();
            }
        }
        Ok(Members::new())
    }
}

fn members<const N: usize>(pairs: [(&str, Value); N]) -> Members {
    pairs
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn does_not_exist(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

fn content_based_deduplication_needs_fifo() -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        "`ContentBasedDeduplication` needs a FIFO queue",
    )
}

fn dead_letter_type_mismatch() -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        "the dead-letter queue must be another queue of the same type",
    )
}

fn delivery_json(delivery: &Delivery) -> Value {
    let message = &delivery.message;
    let mut object = members([
        ("MessageId", json!(message.id.to_string())),
        ("ReceiptHandle", json!(delivery.handle.to_string())),
        ("Body", json!(message.body)),
        ("ReceiveCount", json!(message.receive_count)),
        ("SentAt", json!(message.sent_at)),
        ("FirstReceivedAt", json!(message.first_received_at)),
    ]);
    if let Some(place) = &message.fifo {
        object.insert("GroupId".into(), json!(place.group));
        object.insert("SequenceNumber".into(), json!(place.sequence_number));
    }
    Value::Object(object)
}
