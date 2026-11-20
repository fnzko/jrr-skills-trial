//! The set of queues, the clock and the id counters, and the execution of
//! requests against them (the numbered rules of spec sections 5 and 6).

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::config::{AttributePatch, Config, DeadLetter};
use crate::error::{Error, ErrorCode};
use crate::ids::IdSource;
use crate::queue::{Queue, Time};
use crate::request::{self, Request, SendMessage};

const MAX_QUEUES: usize = 1000;

/// The members of a success response besides `"ok"`.
type Members = Map<String, Value>;

#[derive(Debug, Default)]
pub struct Broker {
    now: Time,
    queues: HashMap<String, Queue>,
    ids: IdSource,
}

impl Broker {
    /// Runs one input line (without its terminator) and returns the response
    /// object. Every line gets exactly one response, whatever it contains.
    pub fn handle_line(&mut self, line: &[u8]) -> Value {
        match request::parse(line).and_then(|request| self.execute(request)) {
            Ok(members) => {
                let mut response = Map::with_capacity(members.len() + 1);
                response.insert("ok".to_owned(), Value::Bool(true));
                response.extend(members);
                Value::Object(response)
            }
            Err(error) => json!({
                "ok": false,
                "error": { "code": error.code().as_str(), "message": error.message() },
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
                lookup(&mut self.queues, self.now, &queue)?.purge();
                Ok(Members::new())
            }
            Request::Tick { seconds } => {
                self.now += seconds;
                Ok(members(json!({ "Now": self.now })))
            }
            Request::SendMessage(request) => self.send_message(&request),
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Request::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let queue = lookup(&mut self.queues, self.now, &queue)?;
                queue.delete(self.ids.parse_handle(&receipt_handle)?)?;
                Ok(Members::new())
            }
            Request::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let queue = lookup(&mut self.queues, self.now, &queue)?;
                let handle = self.ids.parse_handle(&receipt_handle)?;
                queue.change_visibility(self.now, handle, visibility_timeout)?;
                Ok(Members::new())
            }
        }
    }

    fn create_queue(
        &mut self,
        name: String,
        attributes: &AttributePatch,
    ) -> Result<Members, Error> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.ends_with(".fifo") != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue is a FIFO queue exactly when its name ends in .fifo and Fifo is true",
            ));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = attributes.dead_letter()?;
        let config = Config::new_from(attributes, dead_letter);

        if let Some(existing) = self.queues.get(&name) {
            return if *existing.config() == config {
                Ok(Members::new())
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
                .ok_or_else(|| Error::no_such_queue(&dead_letter.queue))?;
            if target.config().fifo != fifo {
                return Err(Error::invalid_value(
                    "the dead-letter queue must be of the same type as the queue",
                ));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues may exist"),
            ));
        }
        self.queues.insert(name, Queue::new(config));
        Ok(Members::new())
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        attributes: &AttributePatch,
    ) -> Result<Members, Error> {
        let fifo = lookup(&mut self.queues, self.now, name)?.config().fifo;
        if attributes.fifo.is_some() {
            return Err(Error::unsupported("the type of a queue cannot change"));
        }
        if attributes.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_value(
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = attributes.dead_letter()?;
        if let Some(DeadLetter { queue: target, .. }) = &dead_letter {
            let target_fifo = self
                .queues
                .get(target)
                .ok_or_else(|| Error::no_such_queue(target))?
                .config()
                .fifo;
            if target == name || target_fifo != fifo {
                return Err(Error::invalid_value(
                    "the dead-letter queue must be another queue of the same type",
                ));
            }
        }

        let queue = lookup(&mut self.queues, self.now, name)?;
        let mut config = queue.config().clone();
        config.apply(attributes, dead_letter);
        queue.set_config(self.now, config);
        Ok(Members::new())
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Members, Error> {
        let queue = lookup(&mut self.queues, self.now, name)?;
        let config = queue.config();
        let mut attributes = members(json!({
            "VisibilityTimeout": config.visibility_timeout,
            "DelaySeconds": config.delay_seconds,
            "RetentionSeconds": config.retention_seconds,
            "Fifo": config.fifo,
            "ContentBasedDeduplication": config.content_based_deduplication,
        }));
        if let Some(dead_letter) = &config.dead_letter {
            attributes.insert(
                "DeadLetterQueue".to_owned(),
                dead_letter.queue.clone().into(),
            );
            attributes.insert(
                "MaxReceiveCount".to_owned(),
                dead_letter.max_receive_count.into(),
            );
        }
        let counts = queue.counts();
        Ok(members(json!({
            "Attributes": attributes,
            "Counts": {
                "Visible": counts.visible,
                "InFlight": counts.in_flight,
                "Delayed": counts.delayed,
            },
        })))
    }

    fn list_queues(&self, prefix: Option<&str>) -> Members {
        let mut names: Vec<&str> = self
            .queues
            .keys()
            .map(String::as_str)
            .filter(|name| name.starts_with(prefix.unwrap_or_default()))
            .collect();
        // `str` orders by byte value, which is the order the spec asks for.
        names.sort_unstable();
        members(json!({ "QueueNames": names }))
    }

    fn delete_queue(&mut self, name: &str) -> Result<Members, Error> {
        if self.queues.remove(name).is_none() {
            return Err(Error::no_such_queue(name));
        }
        for queue in self.queues.values_mut() {
            if queue
                .config()
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue == name)
            {
                queue.clear_dead_letter();
            }
        }
        Ok(Members::new())
    }

    fn send_message(&mut self, request: &SendMessage) -> Result<Members, Error> {
        let queue = lookup(&mut self.queues, self.now, &request.queue)?;
        let accepted = queue.send(self.now, request, &mut self.ids)?;
        let mut result = members(json!({ "MessageId": accepted.id.to_string() }));
        if let Some(sequence) = accepted.sequence {
            result.insert("SequenceNumber".to_owned(), sequence.into());
        }
        Ok(result)
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u64,
        visibility_timeout: Option<u64>,
    ) -> Result<Members, Error> {
        let queue = lookup(&mut self.queues, self.now, name)?;
        let dead_letter_queue = queue.config().dead_letter.as_ref().map(|d| d.queue.clone());
        let received = queue.receive(self.now, max_messages, visibility_timeout, &mut self.ids);

        if !received.dead_lettered.is_empty() {
            let target = dead_letter_queue
                .as_deref()
                .and_then(|name| self.queues.get_mut(name))
                .expect("messages are dead-lettered only to an existing queue: DeleteQueue clears references to it");
            for message in received.dead_lettered {
                target.accept_dead_letter(self.now, message);
            }
        }

        let messages: Vec<Value> = received
            .delivered
            .into_iter()
            .map(|delivery| {
                let mut message = members(json!({
                    "MessageId": delivery.id.to_string(),
                    "ReceiptHandle": delivery.receipt_handle.to_string(),
                    "Body": delivery.body,
                    "ReceiveCount": delivery.receive_count,
                    "SentAt": delivery.sent_at,
                    "FirstReceivedAt": delivery.first_received_at,
                }));
                if let Some(group_id) = delivery.group_id {
                    message.insert("GroupId".to_owned(), group_id.as_str().into());
                }
                if let Some(sequence) = delivery.sequence {
                    message.insert("SequenceNumber".to_owned(), sequence.into());
                }
                Value::Object(message)
            })
            .collect();
        Ok(members(json!({ "Messages": messages })))
    }
}

/// Finds a queue by exact name and brings it up to date with the clock.
///
/// Takes the fields rather than the broker so callers can keep borrowing
/// the id counters while holding the queue.
fn lookup<'a>(
    queues: &'a mut HashMap<String, Queue>,
    now: Time,
    name: &str,
) -> Result<&'a mut Queue, Error> {
    let queue = queues
        .get_mut(name)
        .ok_or_else(|| Error::no_such_queue(name))?;
    queue.sync(now);
    Ok(queue)
}

/// The members of a `json!` object literal.
fn members(value: Value) -> Members {
    let Value::Object(members) = value else {
        panic!("members() is only called with object literals, got {value}");
    };
    members
}
