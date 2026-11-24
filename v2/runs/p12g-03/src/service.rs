use std::collections::BTreeMap;
use std::ops::Bound;

use serde_json::{Map, Value};

use crate::attributes::Attributes;
use crate::command::{Command, SendRequest};
use crate::error::{Error, ErrorCode};
use crate::handles::HandleLog;
use crate::queue::{Config, FifoSend, FlightTicket, Kind, NewMessage, Queue};
use crate::response::{
    AttributesOutput, MessagesOutput, NowOutput, Output, QueueAttributes, QueueNamesOutput,
    Response, SentOutput,
};
use crate::types::{GroupId, MessageId, QueueName, QueueUid, Seconds, Time};

const MAX_QUEUES: usize = 1000;
const MAX_MESSAGES_PER_QUEUE: usize = 100_000;

/// All state of the process: the clock, the queues and the counters behind
/// the identifiers.
pub struct Service {
    now: Time,
    queues: BTreeMap<QueueName, Queue>,
    handles: HandleLog,
    queues_created: u64,
    messages_accepted: u64,
}

impl Default for Service {
    fn default() -> Self {
        Self {
            now: Time::START,
            queues: BTreeMap::new(),
            handles: HandleLog::default(),
            queues_created: 0,
            messages_accepted: 0,
        }
    }
}

/// Finds a queue by exactly this name and drops its expired messages.
fn find_queue<'q>(
    queues: &'q mut BTreeMap<QueueName, Queue>,
    now: Time,
    name: &str,
) -> Result<&'q mut Queue, Error> {
    let queue = queues
        .get_mut(name)
        .ok_or_else(|| Error::no_such_queue(name))?;
    queue.expire(now);
    Ok(queue)
}

impl Service {
    /// Runs one input line and returns its response.
    pub fn handle_line(&mut self, line: &[u8]) -> Response {
        let members = match parse_members(line) {
            Ok(members) => members,
            Err(error) => return Response::failure(error),
        };
        match Command::parse(&members).and_then(|command| self.execute(command)) {
            Ok(output) => Response::success(output),
            Err(error) => Response::failure(error),
        }
    }

    fn execute(&mut self, command: Command) -> Result<Output, Error> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => self.get_queue_attributes(queue),
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.unwrap_or(""))),
            Command::DeleteQueue { queue } => self.delete_queue(queue),
            Command::PurgeQueue { queue } => {
                find_queue(&mut self.queues, self.now, queue)?.purge();
                Ok(Output::Empty {})
            }
            Command::Tick { seconds } => {
                self.now = self.now.plus(seconds);
                Ok(Output::Now(NowOutput { now: self.now }))
            }
            Command::SendMessage(request) => self.send_message(&request),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => self.delete_message(queue, receipt_handle),
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => self.change_message_visibility(queue, receipt_handle, visibility_timeout),
        }
    }

    fn create_queue(&mut self, name: QueueName, attributes: &Attributes) -> Result<Output, Error> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.is_fifo() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo if and only if Fifo is true",
            ));
        }
        reject_deduplication_on_standard(attributes, fifo)?;
        let config = Config::new(attributes, attributes.dead_letter()?);
        if let Some(existing) = self.queues.get(&name) {
            return if existing.config == config {
                Ok(Output::Empty {})
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {:?} exists with different attributes", name.as_str()),
                ))
            };
        }
        if let Some(dead_letter) = &config.dead_letter {
            let target = self.queues.get(dead_letter.queue.as_str());
            check_dead_letter_target(
                &dead_letter.queue,
                target.map(|queue| queue.config.kind),
                config.kind,
            )?;
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues can exist"),
            ));
        }
        self.queues_created += 1;
        let uid = QueueUid(self.queues_created);
        self.queues.insert(name, Queue::new(uid, config));
        Ok(Output::Empty {})
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        attributes: &Attributes,
    ) -> Result<Output, Error> {
        // The dead-letter queue is looked up before the queue to change is
        // borrowed mutably; its error only counts after the rules above it.
        let target_kind = attributes
            .dead_letter_queue()
            .map(|dead| self.queues.get(dead).map(|queue| queue.config.kind));
        let queue = find_queue(&mut self.queues, self.now, name)?;
        if attributes.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        reject_deduplication_on_standard(attributes, queue.config.kind.is_fifo())?;
        let dead_letter = attributes.dead_letter()?;
        if let Some(dead) = &dead_letter {
            check_dead_letter_target(&dead.queue, target_kind.flatten(), queue.config.kind)?;
            if dead.queue == name {
                return Err(Error::new(
                    ErrorCode::InvalidParameterValue,
                    "a queue cannot be its own dead-letter queue",
                ));
            }
        }
        queue.config.apply(attributes, dead_letter);
        Ok(Output::Empty {})
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Output, Error> {
        let queue = find_queue(&mut self.queues, self.now, name)?;
        let config = &queue.config;
        Ok(Output::Attributes(AttributesOutput {
            attributes: QueueAttributes {
                visibility_timeout: config.visibility_timeout,
                delay_seconds: config.delay,
                retention_seconds: config.retention,
                fifo: config.kind.is_fifo(),
                content_based_deduplication: config.kind.content_based_deduplication(),
                dead_letter_queue: config.dead_letter.as_ref().map(|d| d.queue.clone()),
                max_receive_count: config.dead_letter.as_ref().map(|d| d.max_receive_count),
            },
            counts: queue.counts(self.now),
        }))
    }

    fn list_queues(&self, prefix: &str) -> Output {
        let queue_names = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name)
            .take_while(|name| name.as_str().starts_with(prefix))
            .cloned()
            .collect();
        Output::QueueNames(QueueNamesOutput { queue_names })
    }

    fn delete_queue(&mut self, name: &str) -> Result<Output, Error> {
        self.queues
            .remove(name)
            .ok_or_else(|| Error::no_such_queue(name))?;
        for queue in self.queues.values_mut() {
            if queue
                .config
                .dead_letter
                .as_ref()
                .is_some_and(|dead| dead.queue == name)
            {
                queue.config.dead_letter = None;
            }
        }
        Ok(Output::Empty {})
    }

    fn send_message(&mut self, request: &SendRequest) -> Result<Output, Error> {
        let now = self.now;
        let queue = find_queue(&mut self.queues, now, request.queue)?;
        let fifo = match queue.config.kind {
            Kind::Standard => {
                if request.group.is_some() || request.deduplication_id.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                None
            }
            Kind::Fifo {
                content_based_deduplication,
            } => {
                if request.delay.is_some() {
                    return Err(unsupported(
                        "DelaySeconds per message needs a standard queue",
                    ));
                }
                let group = request.group.ok_or_else(|| Error::missing("GroupId"))?;
                let deduplication_id = match request.deduplication_id {
                    Some(id) => id.as_str(),
                    None if content_based_deduplication => request.body,
                    None => return Err(Error::missing("DeduplicationId")),
                };
                if let Some(original) = queue.duplicate_of(deduplication_id, now) {
                    return Ok(Output::Sent(SentOutput {
                        message_id: original.message,
                        sequence_number: Some(original.sequence),
                    }));
                }
                Some(FifoSend {
                    group: GroupId::new(group),
                    deduplication_id: Some(deduplication_id),
                })
            }
        };
        if queue.len() >= MAX_MESSAGES_PER_QUEUE {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES_PER_QUEUE} messages"),
            ));
        }
        self.messages_accepted += 1;
        let message_id = MessageId(self.messages_accepted);
        let sequence_number = queue.push(NewMessage {
            id: message_id,
            body: request.body.to_owned(),
            sent_at: now,
            delay: request.delay.unwrap_or(queue.config.delay),
            fifo,
        });
        Ok(Output::Sent(SentOutput {
            message_id,
            sequence_number,
        }))
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: usize,
        visibility_timeout: Option<Seconds>,
    ) -> Result<Output, Error> {
        let queue = find_queue(&mut self.queues, self.now, name)?;
        let visibility = visibility_timeout.unwrap_or(queue.config.visibility_timeout);
        let delivery = queue.receive(self.now, max_messages, visibility, &mut self.handles);
        if !delivery.dead.is_empty() {
            let target = queue
                .config
                .dead_letter
                .as_ref()
                .expect("only a queue with a dead-letter configuration dead-letters messages")
                .queue
                .clone();
            let dead_letter_queue = self
                .queues
                .get_mut(target.as_str())
                .expect("deleting a queue clears every reference to it");
            for message in delivery.dead {
                dead_letter_queue.push(message);
            }
        }
        Ok(Output::Messages(MessagesOutput {
            messages: delivery.messages,
        }))
    }

    fn delete_message(&mut self, name: &str, receipt_handle: &str) -> Result<Output, Error> {
        let (queue, ticket) = self.find_in_flight(name, receipt_handle)?;
        queue.delete(ticket);
        Ok(Output::Empty {})
    }

    fn change_message_visibility(
        &mut self,
        name: &str,
        receipt_handle: &str,
        visibility_timeout: Seconds,
    ) -> Result<Output, Error> {
        let until = self.now.plus(visibility_timeout);
        let (queue, ticket) = self.find_in_flight(name, receipt_handle)?;
        queue.set_in_flight_until(ticket, until);
        Ok(Output::Empty {})
    }

    /// Applies the queue lookup and the receipt handle rules shared by
    /// `DeleteMessage` and `ChangeMessageVisibility`.
    fn find_in_flight(
        &mut self,
        name: &str,
        receipt_handle: &str,
    ) -> Result<(&mut Queue, FlightTicket), Error> {
        let queue = find_queue(&mut self.queues, self.now, name)?;
        let (handle, issued_for) = self.handles.resolve(receipt_handle).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("{receipt_handle:?} was never issued"),
            )
        })?;
        let ticket = queue
            .find_in_flight(handle, issued_for, self.now)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleExpired,
                    format!("{receipt_handle:?} is not the latest handle of a message in flight"),
                )
            })?;
        Ok((queue, ticket))
    }
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

fn reject_deduplication_on_standard(attributes: &Attributes, fifo: bool) -> Result<(), Error> {
    if attributes.content_based_deduplication == Some(true) && !fifo {
        return Err(Error::new(
            ErrorCode::InvalidParameterValue,
            "ContentBasedDeduplication needs a FIFO queue",
        ));
    }
    Ok(())
}

/// `target` is the kind of the queue named `name`, if it exists.
fn check_dead_letter_target(name: &str, target: Option<Kind>, kind: Kind) -> Result<(), Error> {
    match target {
        None => Err(Error::no_such_queue(name)),
        Some(target) if target.is_fifo() != kind.is_fifo() => Err(Error::new(
            ErrorCode::InvalidParameterValue,
            "a queue and its dead-letter queue must be of the same type",
        )),
        Some(_) => Ok(()),
    }
}

fn parse_members(line: &[u8]) -> Result<Map<String, Value>, Error> {
    let text = std::str::from_utf8(line)
        .map_err(|e| Error::new(ErrorCode::InvalidRequest, format!("line is not UTF-8: {e}")))?;
    serde_json::from_str(text).map_err(|e| {
        Error::new(
            ErrorCode::InvalidRequest,
            format!("line is not a JSON object: {e}"),
        )
    })
}
