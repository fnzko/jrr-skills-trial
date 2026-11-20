//! The broker holds every queue, the clock and the delivery log, and runs
//! commands against them. Checks that need queue state run here, in the order
//! sections 5 and 6 of SPEC.md give.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::attributes::{AttributeUpdates, Attributes, DeadLetter, QueueKind};
use crate::command::{Command, SendMessage};
use crate::error::{Error, ErrorCode, Result};
use crate::queue::{Arrival, Counts, Ids, Message, Queue, State};
use crate::types::{QueueName, ReceiptHandle};
use crate::wire;

const MAX_QUEUES: usize = 1000;

/// The members of a successful response, beyond `"ok": true`.
pub type ResultMembers = Map<String, Value>;

/// One delivery, kept so that a receipt handle can be resolved for as long as
/// the process runs.
#[derive(Debug)]
struct Delivery {
    queue: QueueName,
    arrival: Arrival,
}

#[derive(Debug)]
pub struct Broker {
    now: u64,
    queues: BTreeMap<QueueName, Queue>,
    ids: Ids,
    /// Indexed by receipt number minus one, which is how `Ids::receipt` counts.
    deliveries: Vec<Delivery>,
}

impl Broker {
    pub fn new() -> Self {
        Broker {
            now: 0,
            queues: BTreeMap::new(),
            ids: Ids::new(),
            deliveries: Vec::new(),
        }
    }

    pub fn execute(&mut self, command: Command) -> Result<ResultMembers> {
        match command {
            Command::CreateQueue { name, attributes } => self
                .create_queue(name, &attributes)
                .map(|()| ResultMembers::new()),
            Command::SetQueueAttributes { queue, attributes } => self
                .set_queue_attributes(&queue, &attributes)
                .map(|()| ResultMembers::new()),
            Command::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref())),
            Command::DeleteQueue { queue } => {
                self.delete_queue(&queue).map(|()| ResultMembers::new())
            }
            Command::PurgeQueue { queue } => {
                self.purge_queue(&queue).map(|()| ResultMembers::new())
            }
            Command::Tick { seconds } => Ok(self.tick(seconds)),
            Command::SendMessage(request) => self.send(&request),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage { queue, receipt } => self
                .delete_message(&queue, &receipt)
                .map(|()| ResultMembers::new()),
            Command::ChangeMessageVisibility {
                queue,
                receipt,
                visibility_timeout,
            } => self
                .change_message_visibility(&queue, &receipt, visibility_timeout)
                .map(|()| ResultMembers::new()),
        }
    }

    /// Runs the numbered rules of `CreateQueue` in section 5. The parser has
    /// already checked the parameter values of section 2.
    fn create_queue(&mut self, name: QueueName, updates: &AttributeUpdates) -> Result<()> {
        let fifo = updates.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("queue {name} must end in .fifo exactly when Fifo is true"),
            ));
        }
        if updates.content_based_deduplication == Some(true) && !fifo {
            return Err(wire::invalid(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        let dead_letter = updates.dead_letter_request()?;
        if let Some(existing) = self.queues.get(&name) {
            return if updates.matches_existing(existing.attributes(), dead_letter) {
                Ok(())
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {name} exists with different attributes"),
                ))
            };
        }
        let dead_letter = self.resolve_dead_letter(dead_letter, fifo, None)?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("there are already {MAX_QUEUES} queues"),
            ));
        }
        let attributes = updates.apply_to(&Attributes::default(), dead_letter);
        self.queues.insert(name, Queue::new(attributes));
        Ok(())
    }

    fn set_queue_attributes(&mut self, name: &str, updates: &AttributeUpdates) -> Result<()> {
        let queue = self.queue(name)?;
        let current = queue.attributes();
        if updates.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot be changed",
            ));
        }
        if updates.content_based_deduplication == Some(true) && current.kind == QueueKind::Standard
        {
            return Err(wire::invalid(
                "ContentBasedDeduplication requires a FIFO queue",
            ));
        }
        let request = updates.dead_letter_request()?;
        let dead_letter = match request {
            Some(_) => self.resolve_dead_letter(request, queue.is_fifo(), Some(name))?,
            None => current.dead_letter.clone(),
        };
        let attributes = updates.apply_to(current, dead_letter);
        let now = self.now;
        self.queue_mut(name)?.set_attributes(now, attributes);
        Ok(())
    }

    /// Finds the queue a dead-letter pair names. The name must exist, and the
    /// queue must have the same Fifo setting as the queue being configured.
    /// `owner` is that queue's name, if it already exists: a queue cannot be
    /// its own dead-letter queue.
    fn resolve_dead_letter(
        &self,
        request: Option<(&str, u32)>,
        fifo: bool,
        owner: Option<&str>,
    ) -> Result<Option<DeadLetter>> {
        let Some((name, max_receive_count)) = request else {
            return Ok(None);
        };
        let (queue_name, target) = self
            .queues
            .get_key_value(name)
            .ok_or_else(|| queue_missing(name))?;
        if owner == Some(name) {
            return Err(wire::invalid("a queue cannot be its own dead-letter queue"));
        }
        if target.is_fifo() != fifo {
            return Err(wire::invalid(
                "a dead-letter queue must have the same Fifo setting as its source",
            ));
        }
        Ok(Some(DeadLetter {
            queue: queue_name.clone(),
            max_receive_count,
        }))
    }

    fn get_queue_attributes(&self, name: &str) -> Result<ResultMembers> {
        let queue = self.queue(name)?;
        let mut members = ResultMembers::new();
        members.insert(
            "Attributes".into(),
            Value::Object(queue.attributes().to_json()),
        );
        members.insert("Counts".into(), counts_json(queue.counts(self.now)));
        Ok(members)
    }

    fn list_queues(&self, prefix: Option<&str>) -> ResultMembers {
        let prefix = prefix.unwrap_or("");
        let names: Vec<Value> = self
            .queues
            .keys()
            .filter(|name| name.as_str().starts_with(prefix))
            .map(|name| Value::from(name.as_str()))
            .collect();
        let mut members = ResultMembers::new();
        members.insert("QueueNames".into(), Value::Array(names));
        members
    }

    /// Removes the queue. A queue that named it as dead-letter queue loses its
    /// dead-letter configuration.
    fn delete_queue(&mut self, name: &str) -> Result<()> {
        if self.queues.remove(name).is_none() {
            return Err(queue_missing(name));
        }
        let now = self.now;
        for queue in self.queues.values_mut() {
            let names_deleted_queue = queue
                .attributes()
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| dead_letter.queue.as_str() == name);
            if names_deleted_queue {
                let attributes = Attributes {
                    dead_letter: None,
                    ..queue.attributes().clone()
                };
                queue.set_attributes(now, attributes);
            }
        }
        Ok(())
    }

    fn purge_queue(&mut self, name: &str) -> Result<()> {
        self.queue_mut(name)?.purge();
        Ok(())
    }

    fn tick(&mut self, seconds: u64) -> ResultMembers {
        self.now = self.now.saturating_add(seconds);
        let now = self.now;
        for queue in self.queues.values_mut() {
            queue.expire(now);
        }
        let mut members = ResultMembers::new();
        members.insert("Now".into(), now.into());
        members
    }

    fn send(&mut self, request: &SendMessage) -> Result<ResultMembers> {
        let queue = self
            .queues
            .get_mut(request.queue.as_str())
            .ok_or_else(|| queue_missing(&request.queue))?;
        let sent = queue.send(self.now, request, &mut self.ids)?;
        let mut members = ResultMembers::new();
        members.insert("MessageId".into(), sent.message_id.to_string().into());
        if let Some(sequence_number) = sent.sequence_number {
            members.insert("SequenceNumber".into(), sequence_number.into());
        }
        Ok(members)
    }

    /// Delivers up to `max_messages` messages, after moving the ones that have
    /// been received too often to the dead-letter queue.
    fn receive(
        &mut self,
        name: &str,
        max_messages: u64,
        visibility_override: Option<u64>,
    ) -> Result<ResultMembers> {
        let now = self.now;
        let (queue_name, queue) = self
            .queues
            .get_key_value(name)
            .ok_or_else(|| queue_missing(name))?;
        let visibility = visibility_override.unwrap_or(queue.attributes().visibility_timeout);
        let dead_letter_target = queue
            .attributes()
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.queue.clone());
        let examination = queue.examine(now, max_messages);
        let queue_name = queue_name.clone();

        let moved: Vec<Message> = {
            let source = self
                .queues
                .get_mut(name)
                .ok_or_else(|| queue_missing(name))?;
            examination
                .dead_letter
                .iter()
                .filter_map(|&arrival| source.remove(arrival))
                .collect()
        };
        if !moved.is_empty() {
            let target = dead_letter_target
                .expect("only a queue with a dead-letter configuration examines moves");
            let destination = self
                .queues
                .get_mut(&target)
                .expect("DeleteQueue clears dead-letter configurations that name the queue");
            for message in moved {
                let arrival = self.ids.arrival();
                destination.accept_dead_letter(now, arrival, message);
            }
        }

        let source = self
            .queues
            .get_mut(name)
            .ok_or_else(|| queue_missing(name))?;
        let mut messages = Vec::with_capacity(examination.deliver.len());
        for arrival in examination.deliver {
            let receipt = self.ids.receipt();
            let message = source
                .message_mut(arrival)
                .expect("an examined message is still in its queue");
            message.deliver(now, visibility, receipt);
            messages.push(message_json(message, receipt));
            self.deliveries.push(Delivery {
                queue: queue_name.clone(),
                arrival,
            });
        }
        let mut members = ResultMembers::new();
        members.insert("Messages".into(), Value::Array(messages));
        Ok(members)
    }

    fn delete_message(&mut self, name: &str, receipt: &str) -> Result<()> {
        let arrival = self.in_flight(name, receipt)?;
        self.queue_mut(name)?.remove(arrival);
        Ok(())
    }

    fn change_message_visibility(
        &mut self,
        name: &str,
        receipt: &str,
        visibility_timeout: u64,
    ) -> Result<()> {
        let arrival = self.in_flight(name, receipt)?;
        let now = self.now;
        let message = self
            .queue_mut(name)?
            .message_mut(arrival)
            .expect("in_flight found this message in the queue");
        message.change_visibility(now, visibility_timeout);
        Ok(())
    }

    /// The arrival of the message that `receipt` is the latest handle of,
    /// provided the message is in flight in `queue_name` now. Rules 1 and 2 of
    /// section 6 decide the error.
    fn in_flight(&self, queue_name: &str, receipt: &str) -> Result<Arrival> {
        let queue = self.queue(queue_name)?;
        let invalid = || {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("receipt handle {receipt:?} was never issued"),
            )
        };
        let handle = ReceiptHandle::parse(receipt).ok_or_else(invalid)?;
        let delivery = self.delivery(handle).ok_or_else(invalid)?;
        let expired = || {
            Error::new(
                ErrorCode::ReceiptHandleExpired,
                format!("receipt handle {receipt:?} is no longer usable"),
            )
        };
        if delivery.queue.as_str() != queue_name {
            return Err(expired());
        }
        let message = queue.message(delivery.arrival).ok_or_else(expired)?;
        if message.receipt != Some(handle) || message.state(self.now) != State::InFlight {
            return Err(expired());
        }
        Ok(delivery.arrival)
    }

    fn delivery(&self, handle: ReceiptHandle) -> Option<&Delivery> {
        let index = usize::try_from(handle.number().checked_sub(1)?).ok()?;
        self.deliveries.get(index)
    }

    fn queue(&self, name: &str) -> Result<&Queue> {
        self.queues.get(name).ok_or_else(|| queue_missing(name))
    }

    fn queue_mut(&mut self, name: &str) -> Result<&mut Queue> {
        self.queues.get_mut(name).ok_or_else(|| queue_missing(name))
    }
}

impl Default for Broker {
    fn default() -> Self {
        Broker::new()
    }
}

fn queue_missing(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

fn counts_json(counts: Counts) -> Value {
    let mut object = Map::new();
    object.insert("Visible".into(), counts.visible.into());
    object.insert("InFlight".into(), counts.in_flight.into());
    object.insert("Delayed".into(), counts.delayed.into());
    Value::Object(object)
}

/// A message as `ReceiveMessage` reports it. Group and sequence members appear
/// only for a FIFO queue, which is the only kind that has them.
fn message_json(message: &Message, receipt: ReceiptHandle) -> Value {
    let mut object = Map::new();
    object.insert("MessageId".into(), message.id.to_string().into());
    object.insert("ReceiptHandle".into(), receipt.to_string().into());
    object.insert("Body".into(), message.body.as_str().into());
    object.insert("ReceiveCount".into(), message.receive_count.into());
    object.insert("SentAt".into(), message.sent_at.into());
    object.insert("FirstReceivedAt".into(), message.first_received_at.into());
    if let Some(group_id) = &message.group_id {
        object.insert("GroupId".into(), group_id.as_str().into());
    }
    if let Some(sequence_number) = message.sequence_number {
        object.insert("SequenceNumber".into(), sequence_number.into());
    }
    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn exec(broker: &mut Broker, line: &str) -> Result<ResultMembers> {
        let command = Command::parse_line(line.as_bytes()).expect("test commands parse");
        broker.execute(command)
    }

    fn code(broker: &mut Broker, line: &str) -> ErrorCode {
        exec(broker, line).unwrap_err().code()
    }

    fn create(broker: &mut Broker, name: &str) {
        exec(
            broker,
            &format!(r#"{{"Command": "CreateQueue", "QueueName": "{name}"}}"#),
        )
        .unwrap();
    }

    fn send(broker: &mut Broker, queue: &str, body: &str) -> ResultMembers {
        exec(
            broker,
            &format!(r#"{{"Command": "SendMessage", "QueueName": "{queue}", "Body": "{body}"}}"#),
        )
        .unwrap()
    }

    fn receive(broker: &mut Broker, queue: &str) -> Vec<Value> {
        let members = exec(
            broker,
            &format!(
                r#"{{"Command": "ReceiveMessage", "QueueName": "{queue}", "MaxMessages": 10}}"#
            ),
        )
        .unwrap();
        members["Messages"].as_array().unwrap().clone()
    }

    #[test]
    fn list_queues_is_sorted_and_filtered_by_prefix() {
        let mut broker = Broker::new();
        create(&mut broker, "b");
        create(&mut broker, "a2");
        create(&mut broker, "a1");
        let all = exec(&mut broker, r#"{"Command": "ListQueues"}"#).unwrap();
        assert_eq!(all["QueueNames"], json!(["a1", "a2", "b"]));
        let some = exec(&mut broker, r#"{"Command": "ListQueues", "Prefix": "a"}"#).unwrap();
        assert_eq!(some["QueueNames"], json!(["a1", "a2"]));
    }

    #[test]
    fn creating_an_existing_queue_succeeds_only_with_the_same_attributes() {
        let mut broker = Broker::new();
        create(&mut broker, "q");
        create(&mut broker, "q");
        let line =
            r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 1}}"#;
        assert_eq!(code(&mut broker, line), ErrorCode::QueueAlreadyExists);
    }

    #[test]
    fn a_fifo_name_needs_the_fifo_flag_and_the_reverse_too() {
        let mut broker = Broker::new();
        let plain_name_fifo_flag =
            r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Fifo": true}}"#;
        assert_eq!(
            code(&mut broker, plain_name_fifo_flag),
            ErrorCode::QueueNameInvalid
        );
        let fifo_name_no_flag = r#"{"Command": "CreateQueue", "QueueName": "q.fifo"}"#;
        assert_eq!(
            code(&mut broker, fifo_name_no_flag),
            ErrorCode::QueueNameInvalid
        );
    }

    #[test]
    fn a_dead_letter_queue_must_exist_and_match_the_source_type() {
        let mut broker = Broker::new();
        create(&mut broker, "dead");
        let missing = r#"{"Command": "CreateQueue", "QueueName": "src", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 2}}"#;
        assert_eq!(code(&mut broker, missing), ErrorCode::QueueDoesNotExist);
        let type_mismatch = r#"{"Command": "CreateQueue", "QueueName": "src.fifo", "Attributes": {"Fifo": true, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}}"#;
        assert_eq!(
            code(&mut broker, type_mismatch),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn deleting_a_dead_letter_queue_unsets_the_reference() {
        let mut broker = Broker::new();
        create(&mut broker, "dead");
        exec(&mut broker, r#"{"Command": "CreateQueue", "QueueName": "src", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 2}}"#).unwrap();
        exec(
            &mut broker,
            r#"{"Command": "DeleteQueue", "QueueName": "dead"}"#,
        )
        .unwrap();
        let attributes = exec(
            &mut broker,
            r#"{"Command": "GetQueueAttributes", "QueueName": "src"}"#,
        )
        .unwrap();
        assert!(!attributes["Attributes"]
            .as_object()
            .unwrap()
            .contains_key("DeadLetterQueue"));
    }

    #[test]
    fn a_received_message_is_hidden_until_its_visibility_timeout_ends() {
        let mut broker = Broker::new();
        create(&mut broker, "q");
        send(&mut broker, "q", "hello");
        let first = receive(&mut broker, "q");
        assert_eq!(first.len(), 1);
        assert_eq!(
            first[0],
            json!({
                "MessageId": "m-1",
                "ReceiptHandle": "r-1",
                "Body": "hello",
                "ReceiveCount": 1,
                "SentAt": 0,
                "FirstReceivedAt": 0
            })
        );
        assert!(receive(&mut broker, "q").is_empty());
        exec(&mut broker, r#"{"Command": "Tick", "Seconds": 30}"#).unwrap();
        let again = receive(&mut broker, "q");
        assert_eq!(again[0]["ReceiptHandle"], "r-2");
        assert_eq!(again[0]["ReceiveCount"], 2);
        assert_eq!(again[0]["FirstReceivedAt"], 0);
    }

    #[test]
    fn a_stale_receipt_handle_is_expired_and_an_unissued_one_is_invalid() {
        let mut broker = Broker::new();
        create(&mut broker, "q");
        send(&mut broker, "q", "x");
        receive(&mut broker, "q");
        exec(&mut broker, r#"{"Command": "Tick", "Seconds": 30}"#).unwrap();
        receive(&mut broker, "q");
        let stale = r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#;
        assert_eq!(code(&mut broker, stale), ErrorCode::ReceiptHandleExpired);
        let never = r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-9"}"#;
        assert_eq!(code(&mut broker, never), ErrorCode::ReceiptHandleInvalid);
        let current = r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-2"}"#;
        exec(&mut broker, current).unwrap();
    }

    #[test]
    fn a_handle_from_another_queue_is_expired() {
        let mut broker = Broker::new();
        create(&mut broker, "a");
        create(&mut broker, "b");
        send(&mut broker, "a", "x");
        receive(&mut broker, "a");
        let wrong = r#"{"Command": "ChangeMessageVisibility", "QueueName": "b", "ReceiptHandle": "r-1", "VisibilityTimeout": 0}"#;
        assert_eq!(code(&mut broker, wrong), ErrorCode::ReceiptHandleExpired);
    }

    #[test]
    fn a_zero_visibility_change_makes_the_message_visible_and_its_handle_expired() {
        let mut broker = Broker::new();
        create(&mut broker, "q");
        send(&mut broker, "q", "x");
        receive(&mut broker, "q");
        let change = r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 0}"#;
        exec(&mut broker, change).unwrap();
        assert_eq!(receive(&mut broker, "q")[0]["ReceiptHandle"], "r-2");
        assert_eq!(code(&mut broker, change), ErrorCode::ReceiptHandleExpired);
    }

    #[test]
    fn a_message_moves_to_the_dead_letter_queue_after_too_many_receives() {
        let mut broker = Broker::new();
        create(&mut broker, "dead");
        exec(&mut broker, r#"{"Command": "CreateQueue", "QueueName": "src", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 2, "VisibilityTimeout": 5}}"#).unwrap();
        send(&mut broker, "src", "poison");
        receive(&mut broker, "src");
        exec(&mut broker, r#"{"Command": "Tick", "Seconds": 5}"#).unwrap();
        receive(&mut broker, "src");
        exec(&mut broker, r#"{"Command": "Tick", "Seconds": 5}"#).unwrap();
        assert!(receive(&mut broker, "src").is_empty());
        let moved = receive(&mut broker, "dead");
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0]["MessageId"], "m-1");
        assert_eq!(moved[0]["ReceiveCount"], 1);
        assert_eq!(moved[0]["FirstReceivedAt"], 10);
    }

    #[test]
    fn retention_removes_a_message_when_the_clock_passes_it() {
        let mut broker = Broker::new();
        create(&mut broker, "q");
        send(&mut broker, "q", "x");
        exec(&mut broker, r#"{"Command": "Tick", "Seconds": 345599}"#).unwrap();
        assert_eq!(receive(&mut broker, "q").len(), 1);
        exec(&mut broker, r#"{"Command": "Tick", "Seconds": 1}"#).unwrap();
        assert!(receive(&mut broker, "q").is_empty());
    }

    #[test]
    fn set_queue_attributes_refuses_to_change_the_queue_type() {
        let mut broker = Broker::new();
        create(&mut broker, "q");
        let line =
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"Fifo": true}}"#;
        assert_eq!(code(&mut broker, line), ErrorCode::UnsupportedOperation);
    }

    #[test]
    fn purge_removes_every_message_and_keeps_the_queue() {
        let mut broker = Broker::new();
        create(&mut broker, "q");
        send(&mut broker, "q", "x");
        exec(
            &mut broker,
            r#"{"Command": "PurgeQueue", "QueueName": "q"}"#,
        )
        .unwrap();
        let attributes = exec(
            &mut broker,
            r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
        )
        .unwrap();
        assert_eq!(
            attributes["Counts"],
            json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
        );
    }
}
