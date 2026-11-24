//! The command dispatcher and the state shared by all queues.

use std::collections::BTreeMap;
use std::ops::Bound;

use serde_json::Value;

use crate::attributes::{AttributeSet, Attributes, DeadLetter, Kind};
use crate::error::{ApiError, ErrorCode};
use crate::queue::{Hold, Message, Queue};
use crate::request::Params;
use crate::response::{self, AttributesView, Response};
use crate::types::{
    Body, DeduplicationId, DelaySeconds, GroupId, MaxMessages, MaxReceiveCount, MessageId,
    QueueName, ReceiptHandleIssuer, TickSeconds, Time, VisibilityTimeout,
};

const MAX_QUEUES: usize = 1000;
const MAX_MESSAGES_PER_QUEUE: usize = 100_000;

type Queues = BTreeMap<QueueName, Queue>;

/// All state of the process: the clock, the queues and the id counters.
#[derive(Debug, Default)]
pub struct Broker {
    now: Time,
    queues: Queues,
    accepted_messages: u64,
    handles: ReceiptHandleIssuer,
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs the command on `line` (without its line ending) and returns the
    /// response line, without its line ending either.
    pub fn handle_line(&mut self, line: &[u8]) -> String {
        response::render(&self.execute(line))
    }

    fn execute(&mut self, line: &[u8]) -> Result<Response, ApiError> {
        let value: Value = serde_json::from_slice(line).map_err(|error| {
            ApiError::new(
                ErrorCode::InvalidRequest,
                format!("the line is not valid JSON: {error}"),
            )
        })?;
        let Value::Object(members) = value else {
            return Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "the line is not a JSON object",
            ));
        };
        let command = members
            .get("Command")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::new(ErrorCode::InvalidRequest, "Command must be a string"))?;
        let params = Params::new(&members);
        match command {
            "CreateQueue" => self.create_queue(&params),
            "SetQueueAttributes" => self.set_queue_attributes(&params),
            "GetQueueAttributes" => self.get_queue_attributes(&params),
            "ListQueues" => self.list_queues(&params),
            "DeleteQueue" => self.delete_queue(&params),
            "PurgeQueue" => self.purge_queue(&params),
            "SendMessage" => self.send_message(&params),
            "ReceiveMessage" => self.receive_message(&params),
            "DeleteMessage" => self.delete_message(&params),
            "ChangeMessageVisibility" => self.change_message_visibility(&params),
            "Tick" => self.tick(&params),
            _ => Err(ApiError::new(
                ErrorCode::InvalidRequest,
                format!("unknown command {command}"),
            )),
        }
    }

    fn create_queue(&mut self, params: &Params) -> Result<Response, ApiError> {
        let name = params.required("QueueName")?;
        let attributes = params.optional("Attributes");

        let name = name.string()?;
        // The name is checked before every other parameter.
        let queue_name = QueueName::new(name).ok_or_else(|| {
            ApiError::new(
                ErrorCode::QueueNameInvalid,
                format!("{name:?} is not a valid queue name"),
            )
        })?;
        let set = attributes
            .map(AttributeSet::parse)
            .transpose()?
            .unwrap_or_default();

        let fifo = set.fifo.unwrap_or(false);
        if QueueName::has_fifo_suffix(name) != fifo {
            return Err(ApiError::new(
                ErrorCode::QueueNameInvalid,
                "the name ends in .fifo exactly when Fifo is true",
            ));
        }
        if set.content_based_deduplication == Some(true) && !fifo {
            return Err(ApiError::new(
                ErrorCode::InvalidParameterValue,
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let requested_dead_letter = set.dead_letter_request()?;
        if let Some(existing) = self.queues.get(name) {
            return if existing
                .attributes
                .matches_request(&set, requested_dead_letter)
            {
                Ok(Response::Empty {})
            } else {
                Err(ApiError::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {name} exists with different attributes"),
                ))
            };
        }
        let dead_letter = requested_dead_letter
            .map(|(queue, max)| resolve_dead_letter(&self.queues, queue, max, fifo, name))
            .transpose()?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(ApiError::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues may exist"),
            ));
        }
        self.queues.insert(
            queue_name,
            Queue::new(Attributes::from_request(&set, dead_letter)),
        );
        Ok(Response::Empty {})
    }

    fn set_queue_attributes(&mut self, params: &Params) -> Result<Response, ApiError> {
        let name = params.required("QueueName")?;
        let attributes = params.required("Attributes")?;

        let name = name.string()?;
        let set = AttributeSet::parse(attributes)?;

        let kind = lookup(&mut self.queues, self.now, name)?.attributes.kind;
        if set.fifo.is_some() {
            return Err(ApiError::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue never changes",
            ));
        }
        if set.content_based_deduplication == Some(true) && !kind.is_fifo() {
            return Err(ApiError::new(
                ErrorCode::InvalidParameterValue,
                "ContentBasedDeduplication needs a FIFO queue",
            ));
        }
        let dead_letter = set
            .dead_letter_request()?
            .map(|(queue, max)| resolve_dead_letter(&self.queues, queue, max, kind.is_fifo(), name))
            .transpose()?;
        lookup(&mut self.queues, self.now, name)?
            .attributes
            .update(&set, dead_letter);
        Ok(Response::Empty {})
    }

    fn get_queue_attributes(&mut self, params: &Params) -> Result<Response, ApiError> {
        let name = params.required("QueueName")?.string()?;
        let queue = lookup(&mut self.queues, self.now, name)?;
        Ok(Response::QueueAttributes {
            attributes: AttributesView::from(&queue.attributes),
            counts: queue.counts(self.now),
        })
    }

    fn list_queues(&self, params: &Params) -> Result<Response, ApiError> {
        let prefix = params
            .optional("Prefix")
            .map(|arg| arg.string())
            .transpose()?
            .unwrap_or("");
        let queue_names = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name.as_str())
            .take_while(|name| name.starts_with(prefix))
            .map(str::to_owned)
            .collect();
        Ok(Response::Queues { queue_names })
    }

    fn delete_queue(&mut self, params: &Params) -> Result<Response, ApiError> {
        let name = params.required("QueueName")?.string()?;
        lookup(&mut self.queues, self.now, name)?;
        self.queues.remove(name);
        for queue in self.queues.values_mut() {
            if queue
                .attributes
                .dead_letter
                .as_ref()
                .is_some_and(|d| d.queue.as_str() == name)
            {
                queue.attributes.dead_letter = None;
            }
        }
        Ok(Response::Empty {})
    }

    fn purge_queue(&mut self, params: &Params) -> Result<Response, ApiError> {
        let name = params.required("QueueName")?.string()?;
        lookup(&mut self.queues, self.now, name)?.purge();
        Ok(Response::Empty {})
    }

    fn send_message(&mut self, params: &Params) -> Result<Response, ApiError> {
        let name = params.required("QueueName")?;
        let body = params.required("Body")?;
        let delay = params.optional("DelaySeconds");
        let group = params.optional("GroupId");
        let deduplication = params.optional("DeduplicationId");

        let name = name.string()?;
        let body = body.text(Body::new)?;
        let delay = delay
            .map(|arg| arg.integer(DelaySeconds::new))
            .transpose()?;
        let group = group.map(|arg| arg.text(GroupId::new)).transpose()?;
        let deduplication = deduplication
            .map(|arg| arg.text(DeduplicationId::new))
            .transpose()?;

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let fifo_send = match queue.attributes.kind {
            Kind::Standard => {
                if group.is_some() || deduplication.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                None
            }
            Kind::Fifo {
                content_based_deduplication,
            } => {
                if delay.is_some() {
                    return Err(unsupported("a FIFO queue has no per-message delay"));
                }
                let group = group.ok_or_else(|| ApiError::missing("GroupId"))?;
                let key = match (&deduplication, content_based_deduplication) {
                    (Some(id), _) => id.as_str(),
                    (None, true) => body.as_str(),
                    (None, false) => return Err(ApiError::missing("DeduplicationId")),
                };
                if let Some((message_id, sequence)) = queue.duplicate_of(key, now) {
                    return Ok(Response::Sent {
                        message_id,
                        sequence_number: Some(sequence),
                    });
                }
                Some((group, key.to_owned()))
            }
        };
        if queue.message_count() >= MAX_MESSAGES_PER_QUEUE {
            return Err(ApiError::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES_PER_QUEUE} messages"),
            ));
        }

        self.accepted_messages += 1;
        let id = MessageId::new(self.accepted_messages);
        let delay = delay.unwrap_or(queue.attributes.delay_seconds);
        let (group, key) = match fifo_send {
            Some((group, key)) => (Some(group), Some(key)),
            None => (None, None),
        };
        let position = queue.insert(Message {
            id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group,
            hold: Hold::Delayed {
                until: now.after(delay.get()),
            },
        });
        let sequence_number = match key {
            Some(key) => {
                queue.record_deduplication(key, now, id, position);
                Some(position)
            }
            None => None,
        };
        Ok(Response::Sent {
            message_id: id,
            sequence_number,
        })
    }

    fn receive_message(&mut self, params: &Params) -> Result<Response, ApiError> {
        let name = params.required("QueueName")?;
        let max = params.optional("MaxMessages");
        let visibility_timeout = params.optional("VisibilityTimeout");

        let name = name.string()?;
        let max = max
            .map(|arg| arg.integer(MaxMessages::new))
            .transpose()?
            .unwrap_or_default();
        let visibility_timeout = visibility_timeout
            .map(|arg| arg.integer(VisibilityTimeout::new))
            .transpose()?;

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.attributes.visibility_timeout);
        let received = queue.receive(now, max, visibility_timeout, &mut self.handles);
        if let Some((target, messages)) = received.dead_lettered {
            let target = self
                .queues
                .get_mut(target.as_str())
                .expect("a dead-letter queue exists: deleting a queue clears the references to it");
            for message in messages {
                target.insert_dead_lettered(message, now);
            }
        }
        Ok(Response::Received {
            messages: received.deliveries,
        })
    }

    fn delete_message(&mut self, params: &Params) -> Result<Response, ApiError> {
        let name = params.required("QueueName")?;
        let handle = params.required("ReceiptHandle")?;

        let name = name.string()?;
        let handle = handle.string()?;

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let handle = self.handles.recognize(handle).ok_or_else(invalid_handle)?;
        queue
            .take_in_flight(handle, now)
            .ok_or_else(expired_handle)?;
        Ok(Response::Empty {})
    }

    fn change_message_visibility(&mut self, params: &Params) -> Result<Response, ApiError> {
        let name = params.required("QueueName")?;
        let handle = params.required("ReceiptHandle")?;
        let timeout = params.required("VisibilityTimeout")?;

        let name = name.string()?;
        let handle = handle.string()?;
        let timeout = timeout.integer(VisibilityTimeout::new)?;

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let handle = self.handles.recognize(handle).ok_or_else(invalid_handle)?;
        queue
            .change_visibility(handle, now, timeout)
            .ok_or_else(expired_handle)?;
        Ok(Response::Empty {})
    }

    fn tick(&mut self, params: &Params) -> Result<Response, ApiError> {
        let seconds = params.required("Seconds")?.integer(TickSeconds::new)?;
        self.now = self.now.after(seconds.get());
        Ok(Response::Tick { now: self.now })
    }
}

fn unsupported(message: &str) -> ApiError {
    ApiError::new(ErrorCode::UnsupportedOperation, message)
}

fn invalid_handle() -> ApiError {
    ApiError::new(
        ErrorCode::ReceiptHandleInvalid,
        "the receipt handle was never issued",
    )
}

fn expired_handle() -> ApiError {
    ApiError::new(
        ErrorCode::ReceiptHandleExpired,
        "the receipt handle is not the latest handle of a message in flight in this queue",
    )
}

/// The queue called `name`, with expired messages already removed.
fn lookup<'a>(queues: &'a mut Queues, now: Time, name: &str) -> Result<&'a mut Queue, ApiError> {
    let queue = queues.get_mut(name).ok_or_else(|| {
        ApiError::new(
            ErrorCode::QueueDoesNotExist,
            format!("queue {name} does not exist"),
        )
    })?;
    queue.expire(now);
    Ok(queue)
}

/// The dead-letter configuration naming `target` for the queue `owner`,
/// whose type is FIFO if `owner_is_fifo`. The target must exist, differ from
/// the owner and be of the same type.
fn resolve_dead_letter(
    queues: &Queues,
    target: &str,
    max_receive_count: MaxReceiveCount,
    owner_is_fifo: bool,
    owner: &str,
) -> Result<DeadLetter, ApiError> {
    let (name, queue) = queues.get_key_value(target).ok_or_else(|| {
        ApiError::new(
            ErrorCode::QueueDoesNotExist,
            format!("dead-letter queue {target} does not exist"),
        )
    })?;
    if name.as_str() == owner || queue.attributes.kind.is_fifo() != owner_is_fifo {
        return Err(ApiError::new(
            ErrorCode::InvalidParameterValue,
            "the dead-letter queue must be another queue of the same type",
        ));
    }
    Ok(DeadLetter {
        queue: name.clone(),
        max_receive_count,
    })
}
