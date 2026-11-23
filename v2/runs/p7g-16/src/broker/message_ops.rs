//! Message commands (SPEC section 6).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{Broker, Output, QUEUE_NAME, output};
use crate::attributes::DeadLetter;
use crate::error::{Error, ErrorCode, Result};
use crate::params::{Param, Params, Rule};
use crate::queue::{Accepted, DEDUPLICATION_WINDOW, MAX_MESSAGES, Message, State, Time};

const SEND_MESSAGE: &[Param] = &[
    QUEUE_NAME,
    Param::required("Body", Rule::Bytes { min: 1, max: 1024 }),
    Param::optional("DelaySeconds", Rule::Int { min: 0, max: 900 }),
    Param::optional("GroupId", Rule::Id),
    Param::optional("DeduplicationId", Rule::Id),
];
const RECEIVE_MESSAGE: &[Param] = &[
    QUEUE_NAME,
    Param::optional("MaxMessages", Rule::Int { min: 1, max: 10 }),
    Param::optional(
        "VisibilityTimeout",
        Rule::Int {
            min: 0,
            max: 43_200,
        },
    ),
];
const DELETE_MESSAGE: &[Param] = &[QUEUE_NAME, RECEIPT_HANDLE];
const CHANGE_MESSAGE_VISIBILITY: &[Param] = &[
    QUEUE_NAME,
    RECEIPT_HANDLE,
    Param::required(
        "VisibilityTimeout",
        Rule::Int {
            min: 0,
            max: 43_200,
        },
    ),
];
const RECEIPT_HANDLE: Param = Param::required("ReceiptHandle", Rule::Str);

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

/// What a `ReceiveMessage` scan decided for one message.
enum Action {
    Deliver,
    DeadLetter,
}

impl Broker {
    pub(super) fn send_message(&mut self, object: &Map<String, Value>) -> Result<Output> {
        let params = Params::parse(SEND_MESSAGE, object)?;
        let queue = self.queue(params.required_str("QueueName"))?;
        let fifo = queue.config.fifo;
        let now = self.now;
        let group = params.str("GroupId");
        let explicit_dedup = params.str("DeduplicationId");
        let body = params.required_str("Body");
        let delay = params.int("DelaySeconds");

        if !fifo && (group.is_some() || explicit_dedup.is_some()) {
            return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
        }
        if fifo && delay.is_some() {
            return Err(unsupported("DelaySeconds is not supported on a FIFO queue"));
        }
        let missing =
            |what: &str| Error::new(ErrorCode::MissingParameter, format!("missing {what}"));
        let dedup_key = if fifo {
            if group.is_none() {
                return Err(missing("GroupId"));
            }
            match explicit_dedup {
                Some(id) => Some(id),
                None if queue.config.content_based_deduplication => Some(body),
                None => return Err(missing("DeduplicationId")),
            }
        } else {
            None
        };

        if let Some(original) = dedup_key
            .and_then(|key| queue.deduplication.get(key))
            .filter(|a| now - a.at < DEDUPLICATION_WINDOW)
        {
            return Ok(send_result(fifo, original.message_id, original.sequence));
        }
        if queue.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }

        let delay = delay.map_or(queue.config.delay_seconds, |d| d as u32);
        self.messages_accepted += 1;
        let id = self.messages_accepted;
        let message = Message {
            id,
            body: body.to_owned(),
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group_id: group.map(str::to_owned),
            sequence: 0,
            delayed_until: now + Time::from(delay),
            in_flight_until: 0,
            receipt: None,
        };
        let queue = self.queue_mut(params.required_str("QueueName"))?;
        let sequence = queue.push(message);
        if let Some(key) = dedup_key {
            let accepted = Accepted {
                at: now,
                message_id: id,
                sequence,
            };
            queue.deduplication.insert(key.to_owned(), accepted);
        }
        Ok(send_result(fifo, id, sequence))
    }

    pub(super) fn receive_message(&mut self, object: &Map<String, Value>) -> Result<Output> {
        let params = Params::parse(RECEIVE_MESSAGE, object)?;
        let name = params.required_str("QueueName");
        let max_messages = params.int("MaxMessages").unwrap_or(1) as usize;
        let now = self.now;
        let queue = self.queue(name)?;
        let visibility = params
            .int("VisibilityTimeout")
            .map_or(queue.config.visibility_timeout, |v| v as u32);
        let fifo = queue.config.fifo;
        // A dead-letter queue that no longer exists was already unconfigured by DeleteQueue.
        let dead_letter: Option<DeadLetter> = queue.config.dead_letter.clone();

        // Examine messages in arrival order. In a FIFO queue a group stays blocked as long
        // as one of its earlier messages remains in the queue, whatever its state.
        let mut plan = Vec::new();
        let mut blocked_groups = HashSet::new();
        let mut delivered = 0;
        for (&key, message) in &queue.messages {
            if delivered == max_messages {
                break;
            }
            let group = message.group_id.as_deref().filter(|_| fifo);
            if group.is_some_and(|g| blocked_groups.contains(g)) {
                continue;
            }
            let visible = message.state(now) == State::Visible;
            if visible
                && dead_letter
                    .as_ref()
                    .is_some_and(|d| message.receive_count >= d.max_receive_count)
            {
                plan.push((key, Action::DeadLetter));
                continue;
            }
            blocked_groups.extend(group);
            if visible {
                plan.push((key, Action::Deliver));
                delivered += 1;
            }
        }

        let mut messages = Vec::new();
        let mut moved = Vec::new();
        let queue = self
            .queues
            .get_mut(name)
            .expect("queue was looked up above");
        for (key, action) in plan {
            match action {
                Action::DeadLetter => moved.extend(queue.messages.remove(&key)),
                Action::Deliver => {
                    self.deliveries += 1;
                    let handle = self.deliveries;
                    let message = queue
                        .messages
                        .get_mut(&key)
                        .expect("planned message exists");
                    if let Some(previous) = message.receipt.replace(handle) {
                        queue.handles.remove(&previous);
                    }
                    queue.handles.insert(handle, key);
                    message.receive_count += 1;
                    message.first_received_at.get_or_insert(now);
                    message.in_flight_until = now + Time::from(visibility);
                    messages.push(delivery_json(message, handle, fifo));
                }
            }
        }

        if let Some(dead_letter) = dead_letter {
            self.dead_letter(&dead_letter.queue, moved);
        }
        Ok(output(json!({ "Messages": messages })))
    }

    /// Appends `moved` to the dead-letter queue `target`, as fresh messages that keep their
    /// identity and age. Deduplication ids and the message limit are not involved.
    fn dead_letter(&mut self, target: &str, moved: Vec<Message>) {
        let now = self.now;
        let Some(queue) = self.queues.get_mut(target) else {
            return;
        };
        let retention = Time::from(queue.config.retention_seconds);
        for message in moved {
            let sent_at = message.sent_at;
            let sequence = queue.push(Message {
                receive_count: 0,
                first_received_at: None,
                delayed_until: 0,
                in_flight_until: 0,
                receipt: None,
                ..message
            });
            if now >= sent_at + retention {
                queue.messages.remove(&sequence);
            }
        }
    }

    pub(super) fn delete_message(&mut self, object: &Map<String, Value>) -> Result<Output> {
        let params = Params::parse(DELETE_MESSAGE, object)?;
        let name = params.required_str("QueueName");
        let (handle, key) = self.find_in_flight(name, params.required_str("ReceiptHandle"))?;
        let queue = self.queue_mut(name)?;
        queue.messages.remove(&key);
        queue.handles.remove(&handle);
        Ok(Output::new())
    }

    pub(super) fn change_message_visibility(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<Output> {
        let params = Params::parse(CHANGE_MESSAGE_VISIBILITY, object)?;
        let name = params.required_str("QueueName");
        let (_, key) = self.find_in_flight(name, params.required_str("ReceiptHandle"))?;
        let until = self.now + params.required_int("VisibilityTimeout") as Time;
        let queue = self.queue_mut(name)?;
        if let Some(message) = queue.messages.get_mut(&key) {
            message.in_flight_until = until;
        }
        Ok(Output::new())
    }

    /// Resolves a receipt handle to the in-flight message it is the latest handle of.
    /// Returns the handle number and the message's arrival sequence.
    fn find_in_flight(&self, queue_name: &str, handle: &str) -> Result<(u64, u64)> {
        let queue = self.queue(queue_name)?;
        let number = parse_receipt_handle(handle)
            .filter(|n| (1..=self.deliveries).contains(n))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("receipt handle {handle:?} was never issued"),
                )
            })?;
        queue
            .handles
            .get(&number)
            .and_then(|key| queue.messages.get(key).map(|message| (key, message)))
            .filter(|(_, m)| m.receipt == Some(number) && m.state(self.now) == State::InFlight)
            .map(|(&key, _)| (number, key))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleExpired,
                    format!("receipt handle {handle:?} is no longer usable"),
                )
            })
    }
}

/// `r-<n>` written exactly as miniq writes it: no sign, no leading zeros, no padding.
fn parse_receipt_handle(handle: &str) -> Option<u64> {
    let digits = handle.strip_prefix("r-")?;
    let canonical = digits.bytes().all(|b| b.is_ascii_digit())
        && !digits.is_empty()
        && (digits == "0" || !digits.starts_with('0'));
    canonical.then(|| digits.parse().ok()).flatten()
}

fn send_result(fifo: bool, message_id: u64, sequence: u64) -> Output {
    let mut result = output(json!({ "MessageId": format!("m-{message_id}") }));
    if fifo {
        result.insert("SequenceNumber".to_owned(), json!(sequence));
    }
    result
}

fn delivery_json(message: &Message, handle: u64, fifo: bool) -> Value {
    let mut value = json!({
        "MessageId": format!("m-{}", message.id),
        "ReceiptHandle": format!("r-{handle}"),
        "Body": message.body,
        "ReceiveCount": message.receive_count,
        "SentAt": message.sent_at,
        "FirstReceivedAt": message.first_received_at,
    });
    if fifo {
        value["GroupId"] = json!(message.group_id);
        value["SequenceNumber"] = json!(message.sequence);
    }
    value
}
