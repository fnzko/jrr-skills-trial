//! The broker: all queue state, and execution of parsed commands.
//!
//! This module carries checks 4 and 5 of SPEC section 2: the queue lookup,
//! then each command's own numbered rules, in the order the spec gives. Every
//! rule is checked before the first mutation, so a failed command changes
//! nothing.

use std::collections::{BTreeMap, HashSet};

use serde_json::{json, Map, Value};

use crate::attributes::{AttributeUpdates, Attributes};
use crate::command::{Command, SendMessage};
use crate::error::{Error, ErrorCode, Result};
use crate::names::has_fifo_suffix;
use crate::queue::{DedupEntry, Message, Queue, State, MAX_MESSAGES};

/// Queues a broker can hold at once (SPEC section 2, LimitExceeded).
pub const MAX_QUEUES: usize = 1_000;

/// The result members of a successful command, without the `ok` member.
pub type Output = Map<String, Value>;

#[derive(Debug)]
pub struct Broker {
    now: u64,
    queues: BTreeMap<String, Queue>,
    next_message_id: u64,
    next_receipt: u64,
}

impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}

impl Broker {
    pub fn new() -> Self {
        Broker {
            now: 0,
            queues: BTreeMap::new(),
            next_message_id: 1,
            next_receipt: 1,
        }
    }

    /// The virtual clock, in seconds since the process started.
    pub fn now(&self) -> u64 {
        self.now
    }

    /// Executes a command whose parameters already passed checks 1 to 3.
    pub fn execute(&mut self, command: Command) -> Result<Output> {
        match command {
            Command::CreateQueue { name, attributes } => {
                self.create_queue(&name, &attributes)?;
                Ok(Output::new())
            }
            Command::SetQueueAttributes { name, attributes } => {
                self.set_queue_attributes(&name, &attributes)?;
                Ok(Output::new())
            }
            Command::GetQueueAttributes { name } => self.get_queue_attributes(&name),
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref())),
            Command::DeleteQueue { name } => {
                self.delete_queue(&name)?;
                Ok(Output::new())
            }
            Command::PurgeQueue { name } => {
                self.purge_queue(&name)?;
                Ok(Output::new())
            }
            Command::Tick { seconds } => Ok(self.tick(seconds)),
            Command::SendMessage(send) => self.send_message(send),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                self.delete_message(&queue, &receipt_handle)?;
                Ok(Output::new())
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                self.change_message_visibility(&queue, &receipt_handle, visibility_timeout)?;
                Ok(Output::new())
            }
        }
    }

    fn create_queue(&mut self, name: &str, updates: &AttributeUpdates) -> Result<()> {
        let wants_fifo = updates.fifo == Some(true);
        if has_fifo_suffix(name) != wants_fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("{name:?} must end in .fifo exactly when Fifo is true"),
            ));
        }
        if updates.content_based_deduplication == Some(true) && !wants_fifo {
            return Err(invalid_value(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        let dead_letter = updates.dead_letter()?;
        let mut attributes = Attributes::default();
        updates.apply_to(&mut attributes, dead_letter);

        if let Some(existing) = self.queues.get(name) {
            return if existing.attributes() == &attributes {
                Ok(())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("{name:?} exists with different attributes"),
                ))
            };
        }
        if let Some(policy) = &attributes.dead_letter {
            let target = self
                .queues
                .get(&policy.queue_name)
                .ok_or_else(|| does_not_exist(&policy.queue_name))?;
            if target.attributes().fifo != attributes.fifo {
                return Err(invalid_value(
                    "the dead-letter queue must have the same Fifo setting",
                ));
            }
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a broker holds at most {MAX_QUEUES} queues"),
            ));
        }
        self.queues.insert(name.to_owned(), Queue::new(attributes));
        Ok(())
    }

    fn set_queue_attributes(&mut self, name: &str, updates: &AttributeUpdates) -> Result<()> {
        let queue = self.queues.get(name).ok_or_else(|| does_not_exist(name))?;
        let fifo = queue.attributes().fifo;

        if updates.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if updates.content_based_deduplication == Some(true) && !fifo {
            return Err(invalid_value(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        let dead_letter = updates.dead_letter()?;
        if let Some(policy) = &dead_letter {
            let target = self
                .queues
                .get(&policy.queue_name)
                .ok_or_else(|| does_not_exist(&policy.queue_name))?;
            if policy.queue_name == name || target.attributes().fifo != fifo {
                return Err(invalid_value(
                    "the dead-letter queue must be another queue with the same Fifo setting",
                ));
            }
        }

        let mut attributes = queue.attributes().clone();
        updates.apply_to(&mut attributes, dead_letter);
        let now = self.now;
        let queue = self
            .queues
            .get_mut(name)
            .ok_or_else(|| does_not_exist(name))?;
        queue.replace_attributes(attributes, now);
        Ok(())
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Output> {
        let now = self.now;
        let queue = sweep_queue(&mut self.queues, name, now)?;
        let attributes = queue.attributes();

        let mut shown = Map::new();
        shown.insert(
            "VisibilityTimeout".into(),
            json!(attributes.visibility_timeout),
        );
        shown.insert("DelaySeconds".into(), json!(attributes.delay_seconds));
        shown.insert(
            "RetentionSeconds".into(),
            json!(attributes.retention_seconds),
        );
        shown.insert("Fifo".into(), json!(attributes.fifo));
        shown.insert(
            "ContentBasedDeduplication".into(),
            json!(attributes.content_based_deduplication),
        );
        if let Some(policy) = &attributes.dead_letter {
            shown.insert("DeadLetterQueue".into(), json!(policy.queue_name));
            shown.insert("MaxReceiveCount".into(), json!(policy.max_receive_count));
        }

        let counts = queue.counts(now);
        let mut output = Output::new();
        output.insert("Attributes".into(), Value::Object(shown));
        output.insert(
            "Counts".into(),
            json!({
                "Visible": counts.visible,
                "InFlight": counts.in_flight,
                "Delayed": counts.delayed,
            }),
        );
        Ok(output)
    }

    fn list_queues(&self, prefix: Option<&str>) -> Output {
        let names: Vec<Value> = self
            .queues
            .keys()
            .filter(|name| prefix.is_none_or(|prefix| name.starts_with(prefix)))
            .map(|name| Value::from(name.as_str()))
            .collect();
        single("QueueNames", Value::Array(names))
    }

    fn delete_queue(&mut self, name: &str) -> Result<()> {
        self.queues
            .remove(name)
            .ok_or_else(|| does_not_exist(name))?;
        // A queue that named the deleted one as its dead-letter queue loses
        // that configuration, since the spec makes it unset rather than dangling.
        let now = self.now;
        for queue in self.queues.values_mut() {
            let points_here = queue
                .attributes()
                .dead_letter
                .as_ref()
                .is_some_and(|policy| policy.queue_name == name);
            if points_here {
                let mut attributes = queue.attributes().clone();
                attributes.dead_letter = None;
                queue.replace_attributes(attributes, now);
            }
        }
        Ok(())
    }

    fn purge_queue(&mut self, name: &str) -> Result<()> {
        let queue = self
            .queues
            .get_mut(name)
            .ok_or_else(|| does_not_exist(name))?;
        queue.purge();
        Ok(())
    }

    fn send_message(&mut self, send: SendMessage) -> Result<Output> {
        let now = self.now;
        let queue = sweep_queue(&mut self.queues, &send.queue, now)?;
        let fifo = queue.attributes().fifo;
        let content_based = queue.attributes().content_based_deduplication;
        let default_delay = queue.attributes().delay_seconds;

        // SPEC section 6, checks 1 to 3, in order.
        if fifo && send.delay_seconds.is_some() {
            return Err(unsupported("DelaySeconds cannot be set on a FIFO queue"));
        }
        if !fifo && (send.group_id.is_some() || send.deduplication_id.is_some()) {
            return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
        }
        let (group_id, deduplication_id) = if fifo {
            let group_id = send
                .group_id
                .clone()
                .ok_or_else(|| missing("GroupId is required on a FIFO queue"))?;
            let deduplication_id = match (&send.deduplication_id, content_based) {
                (Some(id), _) => id.clone(),
                (None, true) => send.body.clone(),
                (None, false) => return Err(missing(
                    "DeduplicationId is required on a FIFO queue without ContentBasedDeduplication",
                )),
            };
            (Some(group_id), Some(deduplication_id))
        } else {
            (None, None)
        };

        // Check 4: a duplicate succeeds with the original's numbers and changes nothing.
        if let Some(id) = &deduplication_id {
            if let Some(entry) = queue.open_dedup_window(id, now) {
                return Ok(send_result(entry.message_id, Some(entry.sequence_number)));
            }
        }
        // Check 5.
        if queue.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }

        let id = self.next_message_id;
        self.next_message_id += 1;
        let delay = send.delay_seconds.unwrap_or(default_delay);
        let mut message = Message::new(id, send.body, now, delay, group_id);
        let sequence_number = queue.take_sequence_number();
        message.sequence_number = sequence_number;
        if let (Some(dedup_id), Some(sequence_number)) = (deduplication_id, sequence_number) {
            queue.record_dedup(
                dedup_id,
                DedupEntry {
                    opened_at: now,
                    message_id: id,
                    sequence_number,
                },
            );
        }
        queue.push(message);
        Ok(send_result(id, sequence_number))
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u64,
        visibility_timeout: Option<u64>,
    ) -> Result<Output> {
        let now = self.now;
        let queue = sweep_queue(&mut self.queues, name, now)?;
        let visibility = visibility_timeout.unwrap_or(queue.attributes().visibility_timeout);
        let dead_letter = queue.attributes().dead_letter.clone();

        let mut delivered = Vec::new();
        // Groups with a message still in the queue that the scan has kept. A later
        // message of such a group is not eligible (SPEC section 6).
        let mut blocked_groups: HashSet<String> = HashSet::new();
        let mut cursor = None;

        while (delivered.len() as u64) < max_messages {
            let source = queue_mut(&mut self.queues, name)?;
            let Some((key, message)) = source.next_after(cursor) else {
                break;
            };
            cursor = Some(key);

            let group = message.group_id.clone();
            let blocked = group.as_ref().is_some_and(|g| blocked_groups.contains(g));
            let step = if blocked || message.state(now) != State::Visible {
                Step::Keep
            } else {
                match &dead_letter {
                    Some(policy) if message.receive_count >= policy.max_receive_count => {
                        Step::MoveToDeadLetter(policy.queue_name.clone())
                    }
                    _ => Step::Deliver,
                }
            };

            match step {
                Step::Keep => blocked_groups.extend(group),
                Step::MoveToDeadLetter(target) => {
                    if let Some(moved) = source.remove(key) {
                        queue_mut(&mut self.queues, &target)?.accept_moved(moved, now);
                    }
                }
                Step::Deliver => {
                    let handle = self.next_receipt;
                    self.next_receipt += 1;
                    source.deliver(key, handle, now, visibility);
                    if let Some(message) = source.message(key) {
                        delivered.push(delivery_json(message, handle));
                    }
                    blocked_groups.extend(group);
                }
            }
        }

        Ok(single("Messages", Value::Array(delivered)))
    }

    fn delete_message(&mut self, name: &str, receipt: &str) -> Result<()> {
        let (queue, key) = self.locate_receipt(name, receipt)?;
        queue.remove(key);
        Ok(())
    }

    fn change_message_visibility(
        &mut self,
        name: &str,
        receipt: &str,
        visibility_timeout: u64,
    ) -> Result<()> {
        let now = self.now;
        let (queue, key) = self.locate_receipt(name, receipt)?;
        queue.change_visibility(key, now, visibility_timeout);
        Ok(())
    }

    /// The queue and message a receipt handle names, provided the handle is the
    /// message's latest one and the message is still in flight (SPEC section 6).
    fn locate_receipt(&mut self, name: &str, receipt: &str) -> Result<(&mut Queue, u64)> {
        let now = self.now;
        let issued = self.next_receipt;
        let queue = sweep_queue(&mut self.queues, name, now)?;
        let handle = parse_receipt_handle(receipt)
            .filter(|handle| *handle < issued)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("{receipt:?} was never issued"),
                )
            })?;
        let key = queue.key_for_handle(handle, now).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleExpired,
                format!("{receipt:?} is no longer usable"),
            )
        })?;
        Ok((queue, key))
    }

    fn tick(&mut self, seconds: u64) -> Output {
        self.now = self.now.saturating_add(seconds);
        single("Now", json!(self.now))
    }
}

/// What a receive scan does with one message.
enum Step {
    /// Left in the queue. It also blocks the rest of its FIFO group.
    Keep,
    /// Moved to the named dead-letter queue. Does not count toward MaxMessages.
    MoveToDeadLetter(String),
    /// Delivered. Counts toward MaxMessages.
    Deliver,
}

/// Parses `r-<n>`. Only the canonical decimal form is accepted, so `r-01`,
/// `R-1` and `r-1 ` never name a handle (SPEC section 6).
fn parse_receipt_handle(text: &str) -> Option<u64> {
    let digits = text.strip_prefix("r-")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    (n.to_string() == digits && n >= 1).then_some(n)
}

fn send_result(message_id: u64, sequence_number: Option<u64>) -> Output {
    let mut output = single("MessageId", json!(format!("m-{message_id}")));
    if let Some(sequence_number) = sequence_number {
        output.insert("SequenceNumber".into(), json!(sequence_number));
    }
    output
}

fn delivery_json(message: &Message, handle: u64) -> Value {
    let mut delivery = Map::new();
    delivery.insert("MessageId".into(), json!(format!("m-{}", message.id)));
    delivery.insert("ReceiptHandle".into(), json!(format!("r-{handle}")));
    delivery.insert("Body".into(), json!(message.body));
    delivery.insert("ReceiveCount".into(), json!(message.receive_count));
    delivery.insert("SentAt".into(), json!(message.sent_at));
    delivery.insert("FirstReceivedAt".into(), json!(message.first_received_at));
    if let Some(group_id) = &message.group_id {
        delivery.insert("GroupId".into(), json!(group_id));
    }
    if let Some(sequence_number) = message.sequence_number {
        delivery.insert("SequenceNumber".into(), json!(sequence_number));
    }
    Value::Object(delivery)
}

/// The named queue, without any expiry sweep.
fn queue_mut<'a>(queues: &'a mut BTreeMap<String, Queue>, name: &str) -> Result<&'a mut Queue> {
    queues.get_mut(name).ok_or_else(|| does_not_exist(name))
}

/// The named queue with expired messages already removed.
fn sweep_queue<'a>(
    queues: &'a mut BTreeMap<String, Queue>,
    name: &str,
    now: u64,
) -> Result<&'a mut Queue> {
    let queue = queue_mut(queues, name)?;
    queue.sweep(now);
    Ok(queue)
}

fn does_not_exist(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("no queue named {name:?}"),
    )
}

fn invalid_value(message: &str) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

fn missing(message: &str) -> Error {
    Error::new(ErrorCode::MissingParameter, message)
}

fn single(member: &str, value: Value) -> Output {
    let mut output = Output::new();
    output.insert(member.to_owned(), value);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn updates(value: Value) -> AttributeUpdates {
        AttributeUpdates::parse(&value).unwrap()
    }

    fn code(result: Result<Output>) -> ErrorCode {
        result.unwrap_err().code
    }

    fn create(broker: &mut Broker, name: &str, attributes: Value) -> Result<Output> {
        broker.execute(Command::CreateQueue {
            name: name.into(),
            attributes: updates(attributes),
        })
    }

    fn attributes_of(broker: &mut Broker, name: &str) -> Map<String, Value> {
        let output = broker
            .execute(Command::GetQueueAttributes { name: name.into() })
            .unwrap();
        output["Attributes"].as_object().unwrap().clone()
    }

    #[test]
    fn creating_an_existing_queue_with_the_same_attributes_succeeds() {
        let mut broker = Broker::new();
        create(&mut broker, "jobs", json!({})).unwrap();
        assert!(create(&mut broker, "jobs", json!({"DelaySeconds": 0})).is_ok());
    }

    #[test]
    fn creating_an_existing_queue_with_other_attributes_is_refused() {
        let mut broker = Broker::new();
        create(&mut broker, "jobs", json!({})).unwrap();
        assert_eq!(
            code(create(&mut broker, "jobs", json!({"DelaySeconds": 5}))),
            ErrorCode::QueueAlreadyExists
        );
    }

    #[test]
    fn fifo_suffix_and_fifo_attribute_must_agree() {
        let mut broker = Broker::new();
        assert_eq!(
            code(create(&mut broker, "a.fifo", json!({}))),
            ErrorCode::QueueNameInvalid
        );
        assert_eq!(
            code(create(&mut broker, "b", json!({"Fifo": true}))),
            ErrorCode::QueueNameInvalid
        );
        assert!(create(&mut broker, "c.fifo", json!({"Fifo": true})).is_ok());
    }

    #[test]
    fn content_based_deduplication_needs_fifo() {
        let mut broker = Broker::new();
        assert_eq!(
            code(create(
                &mut broker,
                "q",
                json!({"ContentBasedDeduplication": true})
            )),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn dead_letter_members_are_all_or_nothing_on_create() {
        let mut broker = Broker::new();
        create(&mut broker, "dlq", json!({})).unwrap();
        assert_eq!(
            code(create(&mut broker, "q", json!({"DeadLetterQueue": "dlq"}))),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(create(
                &mut broker,
                "q",
                json!({"DeadLetterQueue": "missing", "MaxReceiveCount": 2})
            )),
            ErrorCode::QueueDoesNotExist
        );
    }

    #[test]
    fn dead_letter_queue_must_match_fifo() {
        let mut broker = Broker::new();
        create(&mut broker, "dlq.fifo", json!({"Fifo": true})).unwrap();
        assert_eq!(
            code(create(
                &mut broker,
                "q",
                json!({"DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 2})
            )),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn queue_count_is_limited() {
        let mut broker = Broker::new();
        for i in 0..MAX_QUEUES {
            create(&mut broker, &format!("q{i}"), json!({})).unwrap();
        }
        assert_eq!(
            code(create(&mut broker, "one-too-many", json!({}))),
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn set_attributes_rejects_changing_fifo_and_unknown_queues() {
        let mut broker = Broker::new();
        create(&mut broker, "q", json!({})).unwrap();
        let set = |broker: &mut Broker, name: &str, attributes: Value| {
            broker.execute(Command::SetQueueAttributes {
                name: name.into(),
                attributes: updates(attributes),
            })
        };
        assert_eq!(
            code(set(&mut broker, "q", json!({"Fifo": false}))),
            ErrorCode::UnsupportedOperation
        );
        assert_eq!(
            code(set(&mut broker, "nope", json!({}))),
            ErrorCode::QueueDoesNotExist
        );
    }

    #[test]
    fn set_attributes_cannot_point_a_queue_at_itself() {
        let mut broker = Broker::new();
        create(&mut broker, "q", json!({})).unwrap();
        let result = broker.execute(Command::SetQueueAttributes {
            name: "q".into(),
            attributes: updates(json!({"DeadLetterQueue": "q", "MaxReceiveCount": 2})),
        });
        assert_eq!(code(result), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn set_attributes_changes_only_what_is_given() {
        let mut broker = Broker::new();
        create(&mut broker, "q", json!({"DelaySeconds": 9})).unwrap();
        broker
            .execute(Command::SetQueueAttributes {
                name: "q".into(),
                attributes: updates(json!({"VisibilityTimeout": 5})),
            })
            .unwrap();
        let shown = attributes_of(&mut broker, "q");
        assert_eq!(shown["VisibilityTimeout"], json!(5));
        assert_eq!(shown["DelaySeconds"], json!(9));
    }

    #[test]
    fn get_attributes_shows_dead_letter_members_only_when_set() {
        let mut broker = Broker::new();
        create(&mut broker, "dlq", json!({})).unwrap();
        create(&mut broker, "q", json!({})).unwrap();
        let shown = attributes_of(&mut broker, "q");
        assert!(!shown.contains_key("DeadLetterQueue"));
        assert!(!shown.contains_key("MaxReceiveCount"));

        broker
            .execute(Command::SetQueueAttributes {
                name: "q".into(),
                attributes: updates(json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 3})),
            })
            .unwrap();
        let shown = attributes_of(&mut broker, "q");
        assert_eq!(shown["DeadLetterQueue"], json!("dlq"));
        assert_eq!(shown["MaxReceiveCount"], json!(3));
    }

    #[test]
    fn list_queues_is_sorted_by_bytes_and_filters_by_prefix() {
        let mut broker = Broker::new();
        for name in ["lower", "Upper", "_under", "9digit", "-dash", "a.fifo"] {
            let attributes = if name.ends_with(".fifo") {
                json!({"Fifo": true})
            } else {
                json!({})
            };
            create(&mut broker, name, attributes).unwrap();
        }
        let all = broker
            .execute(Command::ListQueues { prefix: None })
            .unwrap();
        assert_eq!(
            all["QueueNames"],
            json!(["-dash", "9digit", "Upper", "_under", "a.fifo", "lower"])
        );
        let some = broker
            .execute(Command::ListQueues {
                prefix: Some("l".into()),
            })
            .unwrap();
        assert_eq!(some["QueueNames"], json!(["lower"]));
    }

    #[test]
    fn deleting_a_dead_letter_queue_unsets_the_configuration_pointing_at_it() {
        let mut broker = Broker::new();
        create(&mut broker, "dlq", json!({})).unwrap();
        create(
            &mut broker,
            "q",
            json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
        )
        .unwrap();
        broker
            .execute(Command::DeleteQueue { name: "dlq".into() })
            .unwrap();
        let shown = attributes_of(&mut broker, "q");
        assert!(!shown.contains_key("DeadLetterQueue"));
        assert!(!shown.contains_key("MaxReceiveCount"));
    }

    #[test]
    fn a_recreated_queue_shares_nothing_with_the_deleted_one() {
        let mut broker = Broker::new();
        create(&mut broker, "q", json!({"DelaySeconds": 4})).unwrap();
        broker
            .execute(Command::DeleteQueue { name: "q".into() })
            .unwrap();
        create(&mut broker, "q", json!({})).unwrap();
        assert_eq!(attributes_of(&mut broker, "q")["DelaySeconds"], json!(0));
    }

    #[test]
    fn tick_advances_the_clock_and_reports_it() {
        let mut broker = Broker::new();
        assert_eq!(
            broker.execute(Command::Tick { seconds: 5 }).unwrap()["Now"],
            json!(5)
        );
        assert_eq!(
            broker.execute(Command::Tick { seconds: 0 }).unwrap()["Now"],
            json!(5)
        );
        assert_eq!(broker.now(), 5);
    }

    #[test]
    fn purge_of_a_missing_queue_is_refused() {
        let mut broker = Broker::new();
        assert_eq!(
            code(broker.execute(Command::PurgeQueue { name: "x".into() })),
            ErrorCode::QueueDoesNotExist
        );
    }
}
