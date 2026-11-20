use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::attributes::{AttributeUpdate, Attributes};
use crate::error::{Error, ErrorCode};
use crate::ids::{MessageId, ReceiptHandle};
use crate::queue::{Accepted, Delivery, NewMessage, Queue};
use crate::request::{Request, has_fifo_suffix};

/// The members of a success response besides `"ok": true`.
type Members = Map<String, Value>;

const MAX_QUEUES: usize = 1000;
const MAX_MESSAGES_PER_QUEUE: usize = 100_000;

/// All state of the service: queues and the virtual clock.
#[derive(Debug, Default)]
pub struct Broker {
    /// Seconds since start; only `Tick` advances it.
    now: u64,
    queues: HashMap<String, Queue>,
    /// Messages accepted so far, over all queues.
    messages_accepted: u64,
    /// Receipt handles issued so far, over all queues.
    deliveries: u64,
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one input line (without its line terminator) and returns the
    /// single-line JSON response.
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        let response = match Request::parse(line).and_then(|request| self.execute(request)) {
            Ok(members) => {
                let mut response = Map::with_capacity(members.len() + 1);
                response.insert("ok".into(), Value::Bool(true));
                response.extend(members);
                Value::Object(response)
            }
            Err(error) => json!({
                "ok": false,
                "error": { "code": error.code.as_str(), "message": error.message },
            }),
        };
        response.to_string()
    }

    fn execute(&mut self, request: Request) -> Result<Members, Error> {
        let now = self.now;
        match request {
            Request::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Request::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref())),
            Request::DeleteQueue { queue } => self.delete_queue(&queue),
            Request::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Request::PurgeQueue { queue } => {
                live_queue(&mut self.queues, self.now, &queue)?.purge();
                Ok(Members::new())
            }
            Request::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let (queue, handle) = self.queue_and_handle(&queue, &receipt_handle)?;
                queue
                    .delete(handle, now)
                    .map_err(|_| handle_expired(&receipt_handle))?;
                Ok(Members::new())
            }
            Request::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let (queue, handle) = self.queue_and_handle(&queue, &receipt_handle)?;
                queue
                    .change_visibility(handle, now, visibility_timeout)
                    .map_err(|_| handle_expired(&receipt_handle))?;
                Ok(Members::new())
            }
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Request::SendMessage {
                queue,
                body,
                delay_seconds,
                group_id,
                deduplication_id,
            } => self.send_message(&queue, body, delay_seconds, group_id, deduplication_id),
            Request::Tick { seconds } => {
                self.now += u64::from(seconds);
                Ok(members(json!({ "Now": self.now })))
            }
        }
    }

    fn queue(&self, name: &str) -> Result<&Queue, Error> {
        self.queues.get(name).ok_or_else(|| no_such_queue(name))
    }

    fn create_queue(&mut self, name: String, update: &AttributeUpdate) -> Result<Members, Error> {
        let fifo = update.fifo.unwrap_or(false);
        if has_fifo_suffix(&name) != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a name ends in `.fifo` exactly when `Fifo` is true",
            ));
        }
        if update.content_based_deduplication == Some(true) && !fifo {
            return Err(content_based_on_standard());
        }
        let dead_letter = update.dead_letter()?;

        let mut attributes = Attributes::default();
        attributes.apply(update, dead_letter);

        if let Some(existing) = self.queues.get(&name) {
            return if existing.attributes == attributes {
                Ok(Members::new())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue `{name}` exists with different attributes"),
                ))
            };
        }
        if let Some(dead_letter) = &attributes.dead_letter {
            let target = self.queue(&dead_letter.queue)?;
            if target.attributes.fifo != attributes.fifo {
                return Err(dead_letter_type_mismatch());
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        self.queues.insert(name, Queue::new(attributes));
        Ok(Members::new())
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        update: &AttributeUpdate,
    ) -> Result<Members, Error> {
        // Retention runs out under the old value before the new one applies.
        live_queue(&mut self.queues, self.now, name)?;
        let queue = self.queue(name)?;
        if update.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if update.content_based_deduplication == Some(true) && !queue.attributes.fifo {
            return Err(content_based_on_standard());
        }
        let dead_letter = update.dead_letter()?;
        if let Some(dead_letter) = &dead_letter {
            let target = self.queue(&dead_letter.queue)?;
            if dead_letter.queue == name || target.attributes.fifo != queue.attributes.fifo {
                return Err(dead_letter_type_mismatch());
            }
        }
        let queue = self
            .queues
            .get_mut(name)
            .expect("the queue was found above");
        queue.attributes.apply(update, dead_letter);
        Ok(Members::new())
    }

    /// The live queue, and the handle if this process has issued it.
    fn queue_and_handle(
        &mut self,
        name: &str,
        receipt_handle: &str,
    ) -> Result<(&mut Queue, ReceiptHandle), Error> {
        let queue = live_queue(&mut self.queues, self.now, name)?;
        let handle = ReceiptHandle::parse(receipt_handle, self.deliveries).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("`{receipt_handle}` was never issued"),
            )
        })?;
        Ok((queue, handle))
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Members, Error> {
        let now = self.now;
        let queue = live_queue(&mut self.queues, now, name)?;
        let counts = queue.counts(now);
        Ok(members(json!({
            "Attributes": queue.attributes.to_json(),
            "Counts": {
                "Visible": counts.visible,
                "InFlight": counts.in_flight,
                "Delayed": counts.delayed,
            },
        })))
    }

    fn send_message(
        &mut self,
        name: &str,
        body: String,
        delay_seconds: Option<u32>,
        group_id: Option<String>,
        deduplication_id: Option<String>,
    ) -> Result<Members, Error> {
        let now = self.now;
        let queue = live_queue(&mut self.queues, now, name)?;
        let fifo = queue.attributes.fifo;

        let unsupported = if fifo {
            delay_seconds
                .is_some()
                .then_some("`DelaySeconds` is not supported on a FIFO queue")
        } else {
            (group_id.is_some() || deduplication_id.is_some())
                .then_some("`GroupId` and `DeduplicationId` need a FIFO queue")
        };
        if let Some(message) = unsupported {
            return Err(Error::new(ErrorCode::UnsupportedOperation, message));
        }

        let fifo_send = if fifo {
            let group = group_id.ok_or_else(|| missing("GroupId"))?;
            let deduplication_id = match deduplication_id {
                Some(id) => id,
                None if queue.attributes.content_based_deduplication => body.clone(),
                None => return Err(missing("DeduplicationId")),
            };
            if let Some(original) = queue.duplicate_of(&deduplication_id, now) {
                return Ok(sent(original.id, Some(original.sequence)));
            }
            Some((group, deduplication_id))
        } else {
            None
        };

        if queue.len() >= MAX_MESSAGES_PER_QUEUE {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES_PER_QUEUE} messages"),
            ));
        }

        self.messages_accepted += 1;
        let id = MessageId(self.messages_accepted);
        let delay = delay_seconds.unwrap_or(queue.attributes.delay_seconds);
        let new = NewMessage {
            id,
            body,
            sent_at: now,
            available_at: now + u64::from(delay),
        };
        match fifo_send {
            Some((group, deduplication_id)) => {
                let sequence = queue.push_fifo(new, group);
                queue.remember(
                    deduplication_id,
                    Accepted {
                        at: now,
                        id,
                        sequence,
                    },
                );
                Ok(sent(id, Some(sequence)))
            }
            None => {
                queue.push_standard(new);
                Ok(sent(id, None))
            }
        }
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u32,
        visibility_timeout: Option<u32>,
    ) -> Result<Members, Error> {
        let now = self.now;
        let queue = live_queue(&mut self.queues, now, name)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.attributes.visibility_timeout);
        let dead_letter_queue = queue
            .attributes
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.queue.clone());
        let received = queue.receive(
            now,
            max_messages as usize,
            visibility_timeout,
            &mut self.deliveries,
        );

        if let Some(target) = dead_letter_queue {
            let target = self
                .queues
                .get_mut(&target)
                .expect("DeleteQueue clears the dead-letter configs that name the queue");
            for message in received.dead_lettered {
                target.push_dead_letter(message);
            }
            // A message that is already too old for this queue is gone on arrival.
            target.expire(now);
        }

        let messages: Vec<Value> = received.delivered.iter().map(delivery_json).collect();
        Ok(members(json!({ "Messages": messages })))
    }

    fn list_queues(&self, prefix: Option<&str>) -> Members {
        let prefix = prefix.unwrap_or("");
        let mut names: Vec<&str> = self
            .queues
            .keys()
            .map(String::as_str)
            .filter(|name| name.starts_with(prefix))
            .collect();
        names.sort_unstable();
        members(json!({ "QueueNames": names }))
    }

    fn delete_queue(&mut self, name: &str) -> Result<Members, Error> {
        self.queues
            .remove(name)
            .ok_or_else(|| no_such_queue(name))?;
        for queue in self.queues.values_mut() {
            if queue
                .attributes
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue == name)
            {
                queue.attributes.dead_letter = None;
            }
        }
        Ok(Members::new())
    }
}

/// The queue called `name`, with the messages that ran out of retention at
/// `now` already removed.
fn live_queue<'a>(
    queues: &'a mut HashMap<String, Queue>,
    now: u64,
    name: &str,
) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| no_such_queue(name))?;
    queue.expire(now);
    Ok(queue)
}

fn delivery_json(delivery: &Delivery) -> Value {
    let mut message = members(json!({
        "MessageId": delivery.id.to_string(),
        "ReceiptHandle": delivery.handle.to_string(),
        "Body": delivery.body,
        "ReceiveCount": delivery.receive_count,
        "SentAt": delivery.sent_at,
        "FirstReceivedAt": delivery.first_received_at,
    }));
    if let Some(fifo) = &delivery.fifo {
        message.insert("GroupId".into(), fifo.group.as_str().into());
        message.insert("SequenceNumber".into(), fifo.sequence.into());
    }
    Value::Object(message)
}

fn handle_expired(receipt_handle: &str) -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        format!("`{receipt_handle}` is not the latest handle of a message in flight in this queue"),
    )
}

fn missing(parameter: &str) -> Error {
    Error::new(
        ErrorCode::MissingParameter,
        format!("`{parameter}` is required"),
    )
}

fn sent(id: MessageId, sequence: Option<u64>) -> Members {
    let mut result = members(json!({ "MessageId": id.to_string() }));
    if let Some(sequence) = sequence {
        result.insert("SequenceNumber".into(), sequence.into());
    }
    result
}

fn no_such_queue(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue `{name}` does not exist"),
    )
}

fn content_based_on_standard() -> Error {
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

/// Unwraps a `json!` object literal into response members.
fn members(value: Value) -> Members {
    match value {
        Value::Object(members) => members,
        other => unreachable!("members() is only called with object literals, got {other}"),
    }
}
