//! The command layer: validates a request in the order the specification
//! prescribes and applies it to the queues.

use std::collections::BTreeMap;
use std::ops::Bound;

use serde_json::{Map, Value, json};

use crate::config::{AttributeSet, QueueKind, QueueName, attributes_json};
use crate::error::{Error, ErrorCode, Result};
use crate::ids::{Counter, ReceiptHandle};
use crate::message::{Delivery, GroupId, Message, validate_token};
use crate::params::{Params, invalid, missing};
use crate::queue::{Accepted, Queue};
use crate::time::{Instant, Seconds};

const MAX_QUEUES: usize = 1000;
const MAX_BODY_BYTES: usize = 1024;

type Members = Map<String, Value>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    CreateQueue,
    SetQueueAttributes,
    GetQueueAttributes,
    ListQueues,
    DeleteQueue,
    PurgeQueue,
    Tick,
    SendMessage,
    ReceiveMessage,
    DeleteMessage,
    ChangeMessageVisibility,
}

impl Command {
    fn parse(name: &str) -> Option<Command> {
        Some(match name {
            "CreateQueue" => Command::CreateQueue,
            "SetQueueAttributes" => Command::SetQueueAttributes,
            "GetQueueAttributes" => Command::GetQueueAttributes,
            "ListQueues" => Command::ListQueues,
            "DeleteQueue" => Command::DeleteQueue,
            "PurgeQueue" => Command::PurgeQueue,
            "Tick" => Command::Tick,
            "SendMessage" => Command::SendMessage,
            "ReceiveMessage" => Command::ReceiveMessage,
            "DeleteMessage" => Command::DeleteMessage,
            "ChangeMessageVisibility" => Command::ChangeMessageVisibility,
            _ => return None,
        })
    }
}

/// All state of one miniq process.
#[derive(Debug)]
pub struct Broker {
    now: Instant,
    queues: BTreeMap<QueueName, Queue>,
    message_ids: Counter,
    receipt_handles: Counter,
}

impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}

impl Broker {
    pub fn new() -> Broker {
        Broker {
            now: Instant::START,
            queues: BTreeMap::new(),
            message_ids: Counter::new(),
            receipt_handles: Counter::new(),
        }
    }

    /// Runs one input line and returns the response line, without its newline.
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        let response = match self.execute(line) {
            Ok(result) => {
                let mut response = Map::new();
                response.insert("ok".into(), true.into());
                response.extend(result);
                Value::Object(response)
            }
            Err(error) => json!({
                "ok": false,
                "error": {"code": error.code.as_str(), "message": error.message},
            }),
        };
        response.to_string()
    }

    fn execute(&mut self, line: &[u8]) -> Result<Members> {
        let request = parse_request(line)?;
        let (command, params) = request.command()?;
        match command {
            Command::CreateQueue => self.create_queue(params),
            Command::SetQueueAttributes => self.set_queue_attributes(params),
            Command::GetQueueAttributes => self.get_queue_attributes(params),
            Command::ListQueues => self.list_queues(params),
            Command::DeleteQueue => self.delete_queue(params),
            Command::PurgeQueue => self.purge_queue(params),
            Command::Tick => self.tick(params),
            Command::SendMessage => self.send_message(params),
            Command::ReceiveMessage => self.receive_message(params),
            Command::DeleteMessage => self.delete_message(params),
            Command::ChangeMessageVisibility => self.change_message_visibility(params),
        }
    }

    fn create_queue(&mut self, params: Params) -> Result<Members> {
        params.require(&["QueueName"])?;
        let name = QueueName::parse(params.required_string("QueueName")?)?;
        let attributes = match params.get("Attributes") {
            Some(value) => AttributeSet::parse(value)?,
            None => AttributeSet::default(),
        };
        let fifo = attributes.fifo() == Some(true);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is a FIFO queue",
            ));
        }
        if attributes.content_based_deduplication() == Some(true) && !fifo {
            return Err(invalid("ContentBasedDeduplication needs a FIFO queue"));
        }
        let dead_letter = attributes.dead_letter()?;
        let config = attributes.into_new_config(dead_letter);
        if let Some(existing) = self.queues.get(name.as_str()) {
            return if *existing.config() == config {
                Ok(Members::new())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {} exists with different attributes", name.as_str()),
                ))
            };
        }
        if let Some(dead_letter) = &config.dead_letter {
            let target = self.existing_queue(&dead_letter.queue)?;
            if target.config().kind.is_fifo() != fifo {
                return Err(invalid("the dead-letter queue must be of the same type"));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "the limit of 1000 queues is reached",
            ));
        }
        self.queues.insert(name, Queue::new(config));
        Ok(Members::new())
    }

    fn set_queue_attributes(&mut self, params: Params) -> Result<Members> {
        params.require(&["QueueName", "Attributes"])?;
        let name = params.required_string("QueueName")?;
        let attributes = match params.get("Attributes") {
            Some(value) => AttributeSet::parse(value)?,
            None => AttributeSet::default(),
        };
        let kind = self.existing_queue(name)?.config().kind;
        if attributes.fifo().is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if attributes.content_based_deduplication() == Some(true) && !kind.is_fifo() {
            return Err(invalid("ContentBasedDeduplication needs a FIFO queue"));
        }
        let dead_letter = attributes.dead_letter()?;
        if let Some(dead_letter) = &dead_letter {
            let target = self.existing_queue(&dead_letter.queue)?;
            if dead_letter.queue == name || target.config().kind.is_fifo() != kind.is_fifo() {
                return Err(invalid(
                    "the dead-letter queue must be another queue of the same type",
                ));
            }
        }
        let now = self.now;
        let queue = self.existing_queue_mut(name)?;
        attributes.apply_to(queue.config_mut(), dead_letter);
        queue.expire(now);
        Ok(Members::new())
    }

    fn get_queue_attributes(&self, params: Params) -> Result<Members> {
        params.require(&["QueueName"])?;
        let queue = self.existing_queue(params.required_string("QueueName")?)?;
        let counts = queue.counts(self.now);
        let mut result = Members::new();
        result.insert("Attributes".into(), attributes_json(queue.config()).into());
        result.insert(
            "Counts".into(),
            json!({
                "Visible": counts.visible,
                "InFlight": counts.in_flight,
                "Delayed": counts.delayed,
            }),
        );
        Ok(result)
    }

    fn list_queues(&self, params: Params) -> Result<Members> {
        let prefix = params.string("Prefix")?.unwrap_or("");
        let names: Vec<&str> = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name.as_str())
            .take_while(|name| name.starts_with(prefix))
            .collect();
        Ok(single("QueueNames", json!(names)))
    }

    fn delete_queue(&mut self, params: Params) -> Result<Members> {
        params.require(&["QueueName"])?;
        let name = params.required_string("QueueName")?;
        self.existing_queue(name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            let config = queue.config_mut();
            if config
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue == name)
            {
                config.dead_letter = None;
            }
        }
        Ok(Members::new())
    }

    fn purge_queue(&mut self, params: Params) -> Result<Members> {
        params.require(&["QueueName"])?;
        self.existing_queue_mut(params.required_string("QueueName")?)?
            .purge();
        Ok(Members::new())
    }

    fn tick(&mut self, params: Params) -> Result<Members> {
        params.require(&["Seconds"])?;
        let span = Seconds::new(params.required_integer("Seconds", 0..=1_209_600)?);
        self.now = self.now.after(span);
        for queue in self.queues.values_mut() {
            queue.expire(self.now);
            queue.forget_closed_dedup_windows(self.now);
        }
        Ok(single("Now", self.now.as_secs().into()))
    }

    fn send_message(&mut self, params: Params) -> Result<Members> {
        params.require(&["QueueName", "Body"])?;
        let name = params.required_string("QueueName")?;
        let body = params.required_string("Body")?;
        if body.is_empty() || body.len() > MAX_BODY_BYTES {
            return Err(invalid("Body must be 1 to 1024 bytes"));
        }
        let delay = params.integer("DelaySeconds", 0..=900)?.map(Seconds::new);
        let group = params.string("GroupId")?.map(GroupId::parse).transpose()?;
        let dedup = params
            .string("DeduplicationId")?
            .map(|id| validate_token("DeduplicationId", id))
            .transpose()?;

        let now = self.now;
        let queue = self
            .queues
            .get_mut(name)
            .ok_or_else(|| does_not_exist(name))?;
        let dedup_id = match queue.config().kind {
            QueueKind::Standard => {
                if group.is_some() || dedup.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                None
            }
            QueueKind::Fifo {
                content_based_deduplication,
            } => {
                if delay.is_some() {
                    return Err(unsupported("DelaySeconds is not supported on FIFO queues"));
                }
                if group.is_none() {
                    return Err(missing("GroupId"));
                }
                let dedup_id = match dedup {
                    Some(id) => id,
                    None if content_based_deduplication => body,
                    None => return Err(missing("DeduplicationId")),
                };
                if let Some(original) = queue.find_duplicate(dedup_id, now) {
                    return Ok(accepted_members(original));
                }
                Some(dedup_id.to_owned())
            }
        };
        if queue.is_full() {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "the queue holds 100000 messages",
            ));
        }
        let delay = delay.unwrap_or(queue.config().delay);
        let id = self.message_ids.next_message_id();
        let message = Message::new(id, body.to_owned(), now, delay, group);
        Ok(accepted_members(queue.add(message, dedup_id)))
    }

    fn receive_message(&mut self, params: Params) -> Result<Members> {
        params.require(&["QueueName"])?;
        let name = params.required_string("QueueName")?;
        let max = params.integer("MaxMessages", 1..=10)?.unwrap_or(1);
        let visibility = params
            .integer("VisibilityTimeout", 0..=43_200)?
            .map(Seconds::new);
        let max = usize::try_from(max).map_err(|_| invalid("MaxMessages is too large"))?;

        let now = self.now;
        let queue = self
            .queues
            .get_mut(name)
            .ok_or_else(|| does_not_exist(name))?;
        let visibility = visibility.unwrap_or(queue.config().visibility_timeout);
        let dead_letter_queue = queue
            .config()
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.queue.clone());
        let received = queue.receive(now, max, visibility, &mut self.receipt_handles);
        if let Some(target) = dead_letter_queue.filter(|_| !received.dead.is_empty()) {
            // The dead-letter queue exists: deleting it clears this configuration.
            if let Some(target) = self.queues.get_mut(target.as_str()) {
                for message in received.dead {
                    target.add(message.into_dead_letter(), None);
                }
                target.expire(now);
            }
        }
        let messages: Vec<Value> = received.delivered.iter().map(delivery_json).collect();
        Ok(single("Messages", messages.into()))
    }

    fn delete_message(&mut self, params: Params) -> Result<Members> {
        params.require(&["QueueName", "ReceiptHandle"])?;
        let name = params.required_string("QueueName")?;
        let handle_text = params.required_string("ReceiptHandle")?;
        self.existing_queue(name)?;
        let handle = self.issued_handle(handle_text)?;
        let now = self.now;
        if self.existing_queue_mut(name)?.delete_in_flight(handle, now) {
            Ok(Members::new())
        } else {
            Err(expired(handle))
        }
    }

    fn change_message_visibility(&mut self, params: Params) -> Result<Members> {
        params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
        let name = params.required_string("QueueName")?;
        let handle_text = params.required_string("ReceiptHandle")?;
        let visibility = Seconds::new(params.required_integer("VisibilityTimeout", 0..=43_200)?);
        self.existing_queue(name)?;
        let handle = self.issued_handle(handle_text)?;
        let now = self.now;
        if self
            .existing_queue_mut(name)?
            .change_visibility(handle, now, visibility)
        {
            Ok(Members::new())
        } else {
            Err(expired(handle))
        }
    }

    fn issued_handle(&self, text: &str) -> Result<ReceiptHandle> {
        ReceiptHandle::parse(text)
            .filter(|handle| self.receipt_handles.has_issued(*handle))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("{text:?} was never issued"),
                )
            })
    }

    fn existing_queue(&self, name: &str) -> Result<&Queue> {
        self.queues.get(name).ok_or_else(|| does_not_exist(name))
    }

    fn existing_queue_mut(&mut self, name: &str) -> Result<&mut Queue> {
        self.queues
            .get_mut(name)
            .ok_or_else(|| does_not_exist(name))
    }
}

/// A request line that parsed as a JSON object with a string `Command`.
struct Request(Map<String, Value>);

fn parse_request(line: &[u8]) -> Result<Request> {
    let bad = |what: &str| Error::new(ErrorCode::InvalidRequest, what.to_owned());
    let text = std::str::from_utf8(line).map_err(|_| bad("the line is not valid UTF-8"))?;
    match serde_json::from_str(text) {
        Ok(Value::Object(members)) => Ok(Request(members)),
        Ok(_) => Err(bad("the line is not a JSON object")),
        Err(error) => Err(Error::new(
            ErrorCode::InvalidRequest,
            format!("the line is not valid JSON: {error}"),
        )),
    }
}

impl Request {
    fn command(&self) -> Result<(Command, Params<'_>)> {
        let name = self
            .0
            .get("Command")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "Command must be a string"))?;
        let command = Command::parse(name).ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidRequest,
                format!("unknown command {name:?}"),
            )
        })?;
        Ok((command, Params::new(&self.0)))
    }
}

fn single(name: &str, value: Value) -> Members {
    let mut members = Members::new();
    members.insert(name.into(), value);
    members
}

fn accepted_members(accepted: Accepted) -> Members {
    let mut members = single("MessageId", accepted.id.to_string().into());
    if let Some(sequence) = accepted.sequence {
        members.insert("SequenceNumber".into(), sequence.as_u64().into());
    }
    members
}

fn delivery_json(delivery: &Delivery) -> Value {
    let mut message = Members::new();
    message.insert("MessageId".into(), delivery.id.to_string().into());
    message.insert("ReceiptHandle".into(), delivery.handle.to_string().into());
    message.insert("Body".into(), delivery.body.clone().into());
    message.insert("ReceiveCount".into(), delivery.receive_count.into());
    message.insert("SentAt".into(), delivery.sent_at.as_secs().into());
    message.insert(
        "FirstReceivedAt".into(),
        delivery.first_received_at.as_secs().into(),
    );
    if let Some(group) = &delivery.group {
        message.insert("GroupId".into(), group.as_str().into());
    }
    if let Some(sequence) = delivery.sequence {
        message.insert("SequenceNumber".into(), sequence.as_u64().into());
    }
    Value::Object(message)
}

fn does_not_exist(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message.to_owned())
}

fn expired(handle: ReceiptHandle) -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        format!("{handle} is not the handle of a message in flight in this queue"),
    )
}
