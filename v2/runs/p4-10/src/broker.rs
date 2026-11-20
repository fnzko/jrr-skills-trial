//! The queue service itself: all state, and the execution of commands
//! against it.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::attributes::{AttributeInput, QueueAttributes, MAX_VISIBILITY_TIMEOUT};
use crate::params::{self, Params};
use crate::protocol::{self, Request};
use crate::queue::{Examined, Message, Queue, Sent, State};
use crate::syntax;
use crate::{Error, ErrorCode};

/// Section 2: at most this many queues may exist.
const MAX_QUEUES: usize = 1_000;
/// Section 5 `Tick`: the largest amount of time one command may advance.
const MAX_TICK_SECONDS: u64 = 1_209_600;

/// Section 6: a queue holds at most this many messages.
const MAX_MESSAGES_PER_QUEUE: usize = 100_000;
/// Section 6: the largest body, in UTF-8 bytes.
const MAX_BODY_BYTES: usize = 1_024;
/// Section 6: the largest `DelaySeconds` of a single send.
const MAX_SEND_DELAY_SECONDS: u64 = 900;

/// What a receipt handle `r-<n>` was issued for. Receipt `n` is at index
/// `n - 1` of [`Broker::receipts`].
#[derive(Debug, Clone, Copy)]
struct IssuedReceipt {
    queue_uid: u64,
    arrival: u64,
}

/// The whole state of one miniq process.
#[derive(Debug)]
pub struct Broker {
    /// The virtual clock, in seconds. Only `Tick` advances it.
    clock: u64,
    queues: BTreeMap<String, Queue>,
    /// The number the next accepted message gets in its `m-<n>` id.
    next_message_id: u64,
    /// The uid the next created queue gets.
    next_queue_uid: u64,
    receipts: Vec<IssuedReceipt>,
}

impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}

impl Broker {
    pub fn new() -> Self {
        Broker {
            clock: 0,
            queues: BTreeMap::new(),
            next_message_id: 1,
            next_queue_uid: 1,
            receipts: Vec::new(),
        }
    }

    /// Handles one input line and returns the response object for it.
    pub fn handle_line(&mut self, line: &[u8]) -> Value {
        match protocol::parse_request(line).and_then(|request| self.execute(&request)) {
            Ok(members) => protocol::ok_response(members),
            Err(error) => protocol::error_response(&error),
        }
    }

    /// Runs one parsed command and returns its result members.
    fn execute(&mut self, request: &Request) -> Result<Map<String, Value>, Error> {
        let params = Params::new(&request.params);
        match request.command.as_str() {
            "CreateQueue" => self.create_queue(params),
            "ListQueues" => self.list_queues(params),
            "DeleteQueue" => self.delete_queue(params),
            "GetQueueAttributes" => self.get_queue_attributes(params),
            "SetQueueAttributes" => self.set_queue_attributes(params),
            "PurgeQueue" => self.purge_queue(params),
            "SendMessage" => self.send_message(params),
            "ReceiveMessage" => self.receive_message(params),
            "DeleteMessage" => self.delete_message(params),
            "ChangeMessageVisibility" => self.change_message_visibility(params),
            "Tick" => self.tick(params),
            other => Err(Error::new(
                ErrorCode::InvalidRequest,
                format!("unknown command {other}"),
            )),
        }
    }

    fn create_queue(&mut self, params: Params<'_>) -> Result<Map<String, Value>, Error> {
        params.require(&["QueueName"])?;
        let name = params.required_str("QueueName")?;
        if !syntax::is_valid_queue_name(name) {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("{name:?} is not a valid queue name"),
            ));
        }
        let input = AttributeInput::parse(params.optional_object("Attributes")?)?;
        let fifo = input.fifo.unwrap_or(false);

        // Section 5, CreateQueue, rules 1 to 7, in order.
        if syntax::has_fifo_suffix(name) != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when Fifo is true",
            ));
        }
        if input.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid("ContentBasedDeduplication needs a FIFO queue"));
        }
        let dead_letter = input.dead_letter()?;
        let wanted = QueueAttributes::create(&input, dead_letter.clone());
        if let Some(existing) = self.queues.get(name) {
            return if existing.attributes == wanted {
                Ok(Map::new())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {name} exists with different attributes"),
                ))
            };
        }
        if let Some(policy) = &dead_letter {
            let target = self
                .queues
                .get(&policy.queue)
                .ok_or_else(|| no_such_queue(&policy.queue))?;
            if target.attributes.fifo != fifo {
                return Err(invalid(
                    "a dead-letter queue must have the same Fifo setting",
                ));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "1000 queues already exist",
            ));
        }

        let uid = self.next_queue_uid;
        self.next_queue_uid += 1;
        self.queues.insert(name.to_owned(), Queue::new(uid, wanted));
        Ok(Map::new())
    }

    fn list_queues(&mut self, params: Params<'_>) -> Result<Map<String, Value>, Error> {
        let prefix = params.optional_str("Prefix")?.unwrap_or("");
        let names: Vec<Value> = self
            .queues
            .range(prefix.to_owned()..)
            .map(|(name, _)| name)
            .take_while(|name| name.starts_with(prefix))
            .map(|name| Value::from(name.as_str()))
            .collect();
        Ok(object([("QueueNames", Value::Array(names))]))
    }

    fn delete_queue(&mut self, params: Params<'_>) -> Result<Map<String, Value>, Error> {
        params.require(&["QueueName"])?;
        let name = params.required_str("QueueName")?;
        self.queues
            .remove(name)
            .ok_or_else(|| no_such_queue(name))?;
        for queue in self.queues.values_mut() {
            if queue
                .attributes
                .dead_letter
                .as_ref()
                .is_some_and(|policy| policy.queue == name)
            {
                queue.attributes.dead_letter = None;
            }
        }
        Ok(Map::new())
    }

    fn get_queue_attributes(&mut self, params: Params<'_>) -> Result<Map<String, Value>, Error> {
        params.require(&["QueueName"])?;
        let name = params.required_str("QueueName")?;
        let now = self.clock;
        let queue = self.queue_mut(name)?;
        let attributes = &queue.attributes;

        let mut members = Map::new();
        members.insert(
            "VisibilityTimeout".to_owned(),
            attributes.visibility_timeout.into(),
        );
        members.insert("DelaySeconds".to_owned(), attributes.delay_seconds.into());
        members.insert(
            "RetentionSeconds".to_owned(),
            attributes.retention_seconds.into(),
        );
        members.insert("Fifo".to_owned(), attributes.fifo.into());
        members.insert(
            "ContentBasedDeduplication".to_owned(),
            attributes.content_based_deduplication.into(),
        );
        if let Some(policy) = &attributes.dead_letter {
            members.insert("DeadLetterQueue".to_owned(), policy.queue.as_str().into());
            members.insert(
                "MaxReceiveCount".to_owned(),
                policy.max_receive_count.into(),
            );
        }

        let counts = queue.counts(now);
        let counts = object([
            ("Visible", counts.visible.into()),
            ("InFlight", counts.in_flight.into()),
            ("Delayed", counts.delayed.into()),
        ]);
        Ok(object([
            ("Attributes", Value::Object(members)),
            ("Counts", Value::Object(counts)),
        ]))
    }

    fn set_queue_attributes(&mut self, params: Params<'_>) -> Result<Map<String, Value>, Error> {
        params.require(&["QueueName", "Attributes"])?;
        let name = params.required_str("QueueName")?;
        let input = AttributeInput::parse(params.optional_object("Attributes")?)?;

        let queue = self.queues.get(name).ok_or_else(|| no_such_queue(name))?;
        let fifo = queue.attributes.fifo;
        // Section 5, SetQueueAttributes, rules 1 to 5, in order.
        if input.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the queue type cannot be changed",
            ));
        }
        if input.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid("ContentBasedDeduplication needs a FIFO queue"));
        }
        let dead_letter = input.dead_letter()?;
        if let Some(policy) = &dead_letter {
            let target = self
                .queues
                .get(&policy.queue)
                .ok_or_else(|| no_such_queue(&policy.queue))?;
            if policy.queue == name || target.attributes.fifo != fifo {
                return Err(invalid(
                    "the dead-letter queue must be another queue of the same type",
                ));
            }
        }

        let now = self.clock;
        let queue = self.queue_mut(name)?;
        queue.attributes.update(&input, dead_letter);
        queue.expire(now);
        Ok(Map::new())
    }

    fn purge_queue(&mut self, params: Params<'_>) -> Result<Map<String, Value>, Error> {
        params.require(&["QueueName"])?;
        let name = params.required_str("QueueName")?;
        self.queue_mut(name)?.purge();
        Ok(Map::new())
    }

    fn send_message(&mut self, params: Params<'_>) -> Result<Map<String, Value>, Error> {
        params.require(&["QueueName", "Body"])?;
        let name = params.required_str("QueueName")?;
        let body = params.required_str("Body")?;
        if body.is_empty() || body.len() > MAX_BODY_BYTES {
            return Err(invalid("Body must be 1 to 1024 bytes"));
        }
        let delay = params.optional_int("DelaySeconds", 0..=MAX_SEND_DELAY_SECONDS)?;
        let group_id = optional_id(&params, "GroupId")?;
        let dedup_id = optional_id(&params, "DeduplicationId")?;

        let now = self.clock;
        let message_id = self.next_message_id;
        let queue = self.queue_mut(name)?;
        let fifo = queue.attributes.fifo;

        // Section 6, SendMessage, rules 1 to 5, in order.
        if !fifo && (group_id.is_some() || dedup_id.is_some()) {
            return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
        }
        if fifo && delay.is_some() {
            return Err(unsupported("DelaySeconds is not allowed on a FIFO queue"));
        }
        let mut dedup_key = None;
        if fifo {
            if group_id.is_none() {
                return Err(params::missing("GroupId"));
            }
            let content_based = queue.attributes.content_based_deduplication;
            dedup_key = match (dedup_id, content_based) {
                (Some(id), _) => Some(id.to_owned()),
                (None, true) => Some(body.to_owned()),
                (None, false) => return Err(params::missing("DeduplicationId")),
            };
            if let Some(original) = dedup_key
                .as_deref()
                .and_then(|key| queue.duplicate_of(key, now))
            {
                return Ok(sent_members(original));
            }
        }
        if queue.message_count() >= MAX_MESSAGES_PER_QUEUE {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "the queue holds 100000 messages",
            ));
        }

        let delay = delay.unwrap_or(queue.attributes.delay_seconds);
        let message = Message::new(
            message_id,
            body.to_owned(),
            group_id.map(str::to_owned),
            now,
            delay,
        );
        let sent = queue.send(message, dedup_key);
        self.next_message_id += 1;
        Ok(sent_members(sent))
    }

    fn receive_message(&mut self, params: Params<'_>) -> Result<Map<String, Value>, Error> {
        params.require(&["QueueName"])?;
        let name = params.required_str("QueueName")?;
        let max_messages = params.optional_int("MaxMessages", 1..=10)?.unwrap_or(1);
        let visibility = params.optional_int("VisibilityTimeout", 0..=MAX_VISIBILITY_TIMEOUT)?;

        let now = self.clock;
        let queue = lookup_queue(&mut self.queues, now, name)?;
        let visibility = visibility.unwrap_or(queue.attributes.visibility_timeout);
        let fifo = queue.attributes.fifo;
        let dead_letter = queue.attributes.dead_letter.clone();
        let plan = queue.plan_receive(
            now,
            max_messages as usize,
            dead_letter.as_ref().map(|policy| policy.max_receive_count),
        );

        let mut delivered = Vec::new();
        let mut moved = Vec::new();
        for (arrival, examined) in plan {
            match examined {
                Examined::Deliver => {
                    let receipt = self.receipts.len() as u64 + 1;
                    let message = queue.message_mut(arrival).expect("planned messages exist");
                    message.receive_count += 1;
                    let first_received_at = *message.first_received_at.get_or_insert(now);
                    message.invisible_until = now + visibility;
                    message.receipt = Some(receipt);
                    delivered.push(delivery_members(message, receipt, first_received_at, fifo));
                    self.receipts.push(IssuedReceipt {
                        queue_uid: queue.uid,
                        arrival,
                    });
                }
                Examined::MoveToDeadLetter => {
                    moved.push(queue.take(arrival).expect("planned messages exist"));
                }
            }
        }

        if !moved.is_empty() {
            let policy =
                dead_letter.expect("only a queue with a dead-letter policy moves messages");
            let target = self.queues.get_mut(&policy.queue).expect(
                "DeleteQueue clears the dead-letter setting of queues that name a deleted queue",
            );
            for message in moved {
                target.push(message.into_dead_letter());
            }
            target.expire(now);
        }
        Ok(object([("Messages", Value::Array(delivered))]))
    }

    fn delete_message(&mut self, params: Params<'_>) -> Result<Map<String, Value>, Error> {
        params.require(&["QueueName", "ReceiptHandle"])?;
        let name = params.required_str("QueueName")?;
        let handle = params.required_str("ReceiptHandle")?;

        let now = self.clock;
        let queue = lookup_queue(&mut self.queues, now, name)?;
        let arrival = resolve_receipt(&self.receipts, queue, handle, now)?;
        queue.take(arrival);
        Ok(Map::new())
    }

    fn change_message_visibility(
        &mut self,
        params: Params<'_>,
    ) -> Result<Map<String, Value>, Error> {
        params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
        let name = params.required_str("QueueName")?;
        let handle = params.required_str("ReceiptHandle")?;
        let visibility = params.required_int("VisibilityTimeout", 0..=MAX_VISIBILITY_TIMEOUT)?;

        let now = self.clock;
        let queue = lookup_queue(&mut self.queues, now, name)?;
        let arrival = resolve_receipt(&self.receipts, queue, handle, now)?;
        let message = queue
            .message_mut(arrival)
            .expect("resolve_receipt found the message");
        message.invisible_until = now + visibility;
        Ok(Map::new())
    }

    fn tick(&mut self, params: Params<'_>) -> Result<Map<String, Value>, Error> {
        params.require(&["Seconds"])?;
        let seconds = params.required_int("Seconds", 0..=MAX_TICK_SECONDS)?;
        self.clock += seconds;
        let now = self.clock;
        for queue in self.queues.values_mut() {
            queue.expire(now);
        }
        Ok(object([("Now", now.into())]))
    }

    /// Looks up a queue by name for a command that needs an existing one,
    /// after removing the messages whose retention has run out.
    fn queue_mut(&mut self, name: &str) -> Result<&mut Queue, Error> {
        lookup_queue(&mut self.queues, self.clock, name)
    }
}

/// Finds an existing queue and brings it up to date with the clock.
fn lookup_queue<'a>(
    queues: &'a mut BTreeMap<String, Queue>,
    now: u64,
    name: &str,
) -> Result<&'a mut Queue, Error> {
    let queue = queues.get_mut(name).ok_or_else(|| no_such_queue(name))?;
    queue.expire(now);
    Ok(queue)
}

/// Section 5, DeleteMessage and ChangeMessageVisibility: finds the message
/// that `handle` names, if the handle was issued for this queue and is the
/// latest handle of a message that is in flight right now.
fn resolve_receipt(
    receipts: &[IssuedReceipt],
    queue: &Queue,
    handle: &str,
    now: u64,
) -> Result<u64, Error> {
    let number = parse_receipt_handle(handle).ok_or_else(never_issued)?;
    let issued = usize::try_from(number - 1)
        .ok()
        .and_then(|index| receipts.get(index))
        .ok_or_else(never_issued)?;
    let expired = || {
        Error::new(
            ErrorCode::ReceiptHandleExpired,
            "the receipt handle has expired",
        )
    };
    if issued.queue_uid != queue.uid {
        return Err(expired());
    }
    match queue.message(issued.arrival) {
        Some(message)
            if message.receipt == Some(number) && message.state(now) == State::InFlight =>
        {
            Ok(issued.arrival)
        }
        _ => Err(expired()),
    }
}

/// The number `n` of a handle `r-<n>`. Handles are compared as exact
/// strings, so `r-01`, `R-1` and `r-+1` are not handles at all.
fn parse_receipt_handle(handle: &str) -> Option<u64> {
    let digits = handle.strip_prefix("r-")?;
    let number: u64 = digits.parse().ok()?;
    (number >= 1 && number.to_string() == digits).then_some(number)
}

fn never_issued() -> Error {
    Error::new(
        ErrorCode::ReceiptHandleInvalid,
        "the receipt handle was never issued",
    )
}

fn no_such_queue(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("no queue named {name}"),
    )
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

/// An optional `GroupId` or `DeduplicationId`, checked for its syntax.
fn optional_id<'a>(params: &Params<'a>, name: &str) -> Result<Option<&'a str>, Error> {
    match params.optional_str(name)? {
        Some(id) if !syntax::is_valid_id(id) => Err(invalid(&format!(
            "{name} must be 1 to 128 characters from A-Z a-z 0-9 _ -"
        ))),
        other => Ok(other),
    }
}

/// The members of one message in a ReceiveMessage result.
fn delivery_members(message: &Message, receipt: u64, first_received_at: u64, fifo: bool) -> Value {
    let mut members = object([
        ("MessageId", format!("m-{}", message.id).into()),
        ("ReceiptHandle", format!("r-{receipt}").into()),
        ("Body", message.body.as_str().into()),
        ("ReceiveCount", message.receive_count.into()),
        ("SentAt", message.sent_at.into()),
        ("FirstReceivedAt", first_received_at.into()),
    ]);
    if fifo {
        if let Some(group_id) = &message.group_id {
            members.insert("GroupId".to_owned(), group_id.as_str().into());
        }
        members.insert("SequenceNumber".to_owned(), message.sequence_number.into());
    }
    Value::Object(members)
}

fn sent_members(sent: Sent) -> Map<String, Value> {
    let mut members = object([("MessageId", format!("m-{}", sent.message_id).into())]);
    if let Some(sequence_number) = sent.sequence_number {
        members.insert("SequenceNumber".to_owned(), sequence_number.into());
    }
    members
}

/// Builds a result object from name and value pairs.
fn object<const N: usize>(pairs: [(&str, Value); N]) -> Map<String, Value> {
    pairs
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}
