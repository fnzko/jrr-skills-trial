//! The state of the whole process and the execution of commands.

use std::collections::BTreeMap;
use std::ops::Bound;

use serde_json::{json, Map, Value};

use crate::command::{AttributeUpdate, Command, SendMessage};
use crate::error::{Error, ErrorCode, Result};
use crate::params::missing;
use crate::queue::{
    has_fifo_suffix, Attributes, Message, MessageId, Original, Queue, ReceiptHandle,
    MAX_MESSAGES_PER_QUEUE,
};

pub const MAX_QUEUES: usize = 1000;

#[derive(Debug, Default)]
pub struct Service {
    /// Virtual clock, in seconds.
    now: u64,
    /// Ordered by name, which is the byte order `ListQueues` needs.
    queues: BTreeMap<String, Queue>,
    /// Number of the last `MessageId` handed out.
    last_message_number: u64,
    /// Number of the last `ReceiptHandle` handed out.
    last_receipt_number: u64,
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one command and returns its result members as a JSON object.
    pub fn execute(&mut self, command: Command) -> Result<Value> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref())),
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => {
                self.queue_mut(&queue)?.purge();
                Ok(json!({}))
            }
            Command::SendMessage(request) => self.send_message(request),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages.unwrap_or(1), visibility_timeout),
            Command::DeleteMessage {
                queue,
                receipt_handle,
            } => self.with_handle(&queue, &receipt_handle, |queue, handle, now| {
                queue.delete_by_handle(handle, now)
            }),
            Command::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => self.with_handle(&queue, &receipt_handle, |queue, handle, now| {
                queue.change_visibility(handle, now, visibility_timeout)
            }),
            Command::Tick { seconds } => {
                self.now += seconds;
                Ok(json!({ "Now": self.now }))
            }
        }
    }

    fn queue_mut(&mut self, name: &str) -> Result<&mut Queue> {
        lookup(&mut self.queues, self.now, name)
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: usize,
        visibility_timeout: Option<u32>,
    ) -> Result<Value> {
        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let visibility_timeout = visibility_timeout.unwrap_or(queue.attributes.visibility_timeout);
        let received = queue.receive(
            now,
            max_messages,
            visibility_timeout,
            &mut self.last_receipt_number,
        );
        if !received.dead.is_empty() {
            let target = queue
                .attributes
                .dead_letter
                .as_ref()
                .map(|config| config.queue.clone())
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::QueueDoesNotExist,
                        "dead-lettered without a dead-letter queue",
                    )
                })?;
            let target = lookup(&mut self.queues, now, &target)?;
            for message in received.dead {
                target.push(message);
            }
            // Messages already past the target's retention do not stay.
            target.expire(now);
        }
        let messages: Vec<Value> = received.delivered.iter().map(delivery).collect();
        Ok(json!({ "Messages": messages }))
    }

    /// Runs `action` on the queue with the receipt handle of a command,
    /// reporting why the handle is unusable if `action` says it is.
    fn with_handle(
        &mut self,
        name: &str,
        handle: &str,
        action: impl FnOnce(&mut Queue, ReceiptHandle, u64) -> bool,
    ) -> Result<Value> {
        let now = self.now;
        let last_issued = self.last_receipt_number;
        let queue = lookup(&mut self.queues, now, name)?;
        let handle = parse_handle(handle, last_issued).ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("receipt handle {handle:?} was never issued"),
            )
        })?;
        if action(queue, handle, now) {
            Ok(json!({}))
        } else {
            Err(Error::new(
                ErrorCode::ReceiptHandleExpired,
                format!("receipt handle {handle} is no longer usable in queue {name:?}"),
            ))
        }
    }

    fn send_message(&mut self, request: SendMessage) -> Result<Value> {
        let SendMessage {
            queue: name,
            body,
            delay_seconds,
            group_id,
            deduplication_id,
        } = request;
        let now = self.now;
        let queue = lookup(&mut self.queues, now, &name)?;
        let fifo = queue.attributes.fifo;
        if (!fifo && (group_id.is_some() || deduplication_id.is_some()))
            || (fifo && delay_seconds.is_some())
        {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "GroupId and DeduplicationId need a FIFO queue; DelaySeconds needs a standard queue",
            ));
        }
        // Only FIFO sends deduplicate; the id is the DeduplicationId or the body.
        let mut dedup_id = None;
        if fifo {
            if group_id.is_none() {
                return Err(missing("GroupId"));
            }
            let id = match deduplication_id {
                Some(id) => id,
                None if queue.attributes.content_based_deduplication => body.clone(),
                None => return Err(missing("DeduplicationId")),
            };
            if let Some(original) = queue.original_of(&id, now) {
                return Ok(sent(original));
            }
            dedup_id = Some(id);
        }
        if queue.len() >= MAX_MESSAGES_PER_QUEUE {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES_PER_QUEUE} messages"),
            ));
        }
        self.last_message_number += 1;
        let id = MessageId(self.last_message_number);
        let delay = delay_seconds.unwrap_or(queue.attributes.delay_seconds);
        let sequence_number = queue.push(Message {
            id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group_id,
            sequence_number: None,
            delayed_until: now + u64::from(delay),
            in_flight_until: 0,
            receipt_handle: None,
        });
        let original = Original {
            message: id,
            sequence_number,
        };
        if let Some(dedup_id) = dedup_id {
            queue.record_dedup_id(dedup_id, now, original);
        }
        Ok(sent(original))
    }

    fn create_queue(&mut self, name: String, update: &AttributeUpdate) -> Result<Value> {
        let fifo = update.fifo.unwrap_or(false);
        if has_fifo_suffix(&name) != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo if and only if Fifo is true",
            ));
        }
        check_attribute_combination(update, fifo)?;
        let requested = Attributes::from_update(update);
        if let Some(existing) = self.queues.get(&name) {
            return if existing.attributes == requested {
                Ok(json!({}))
            } else {
                Err(Error::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {name:?} exists with different attributes"),
                ))
            };
        }
        if let Some(dead_letter) = &requested.dead_letter {
            let target = self
                .queues
                .get(&dead_letter.queue)
                .ok_or_else(|| no_such_queue(&dead_letter.queue))?;
            check_dead_letter_type(&target.attributes, fifo)?;
        }
        if self.queues.len() >= MAX_QUEUES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("at most {MAX_QUEUES} queues may exist"),
            ));
        }
        self.queues.insert(name, Queue::new(requested));
        Ok(json!({}))
    }

    fn set_queue_attributes(&mut self, name: &str, update: &AttributeUpdate) -> Result<Value> {
        let fifo = self.queue_mut(name)?.attributes.fifo;
        if update.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot be changed",
            ));
        }
        check_attribute_combination(update, fifo)?;
        if let Some(target_name) = &update.dead_letter_queue {
            let target = self
                .queues
                .get(target_name)
                .ok_or_else(|| no_such_queue(target_name))?;
            if target_name == name {
                return Err(Error::new(
                    ErrorCode::InvalidParameterValue,
                    "a queue cannot be its own dead-letter queue",
                ));
            }
            check_dead_letter_type(&target.attributes, fifo)?;
        }
        let now = self.now;
        let queue = self.queue_mut(name)?;
        queue.attributes.apply(update);
        // A shorter retention takes effect on existing messages at once.
        queue.expire(now);
        Ok(json!({}))
    }

    fn get_queue_attributes(&mut self, name: &str) -> Result<Value> {
        let now = self.now;
        let queue = self.queue_mut(name)?;
        let a = &queue.attributes;
        let mut attributes = Map::new();
        attributes.insert("VisibilityTimeout".into(), a.visibility_timeout.into());
        attributes.insert("DelaySeconds".into(), a.delay_seconds.into());
        attributes.insert("RetentionSeconds".into(), a.retention_seconds.into());
        attributes.insert("Fifo".into(), a.fifo.into());
        attributes.insert(
            "ContentBasedDeduplication".into(),
            a.content_based_deduplication.into(),
        );
        if let Some(dead_letter) = &a.dead_letter {
            attributes.insert("DeadLetterQueue".into(), dead_letter.queue.clone().into());
            attributes.insert(
                "MaxReceiveCount".into(),
                dead_letter.max_receive_count.into(),
            );
        }
        let counts = queue.counts(now);
        Ok(json!({
            "Attributes": attributes,
            "Counts": {
                "Visible": counts.visible,
                "InFlight": counts.in_flight,
                "Delayed": counts.delayed,
            },
        }))
    }

    fn list_queues(&self, prefix: Option<&str>) -> Value {
        let prefix = prefix.unwrap_or("");
        let names: Vec<&str> = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name.as_str())
            .take_while(|name| name.starts_with(prefix))
            .collect();
        json!({ "QueueNames": names })
    }

    fn delete_queue(&mut self, name: &str) -> Result<Value> {
        self.queue_mut(name)?;
        self.queues.remove(name);
        // Queues that dead-lettered into it lose that configuration.
        for queue in self.queues.values_mut() {
            if queue
                .attributes
                .dead_letter
                .as_ref()
                .is_some_and(|d| d.queue == name)
            {
                queue.attributes.dead_letter = None;
            }
        }
        Ok(json!({}))
    }
}

/// Parses `r-<n>` in its canonical spelling, for a handle already issued.
fn parse_handle(text: &str, last_issued: u64) -> Option<ReceiptHandle> {
    let digits = text.strip_prefix("r-")?;
    let number: u64 = digits.parse().ok()?;
    // `parse` accepts `+1` and leading zeros; only the canonical form was issued.
    let issued = (1..=last_issued).contains(&number) && number.to_string() == digits;
    issued.then_some(ReceiptHandle(number))
}

fn delivery(message: &Message) -> Value {
    let mut result = json!({
        "MessageId": message.id.to_string(),
        "Body": message.body,
        "ReceiveCount": message.receive_count,
        "SentAt": message.sent_at,
        "FirstReceivedAt": message.first_received_at,
    });
    if let Some(handle) = message.receipt_handle {
        result["ReceiptHandle"] = handle.to_string().into();
    }
    if let Some(group) = &message.group_id {
        result["GroupId"] = group.as_str().into();
    }
    if let Some(sequence_number) = message.sequence_number {
        result["SequenceNumber"] = sequence_number.into();
    }
    result
}

fn sent(original: Original) -> Value {
    let mut result = json!({ "MessageId": original.message.to_string() });
    if let Some(sequence_number) = original.sequence_number {
        result["SequenceNumber"] = sequence_number.into();
    }
    result
}

/// Looks a queue up for use, first dropping the messages that have expired.
fn lookup<'a>(
    queues: &'a mut BTreeMap<String, Queue>,
    now: u64,
    name: &str,
) -> Result<&'a mut Queue> {
    let queue = queues.get_mut(name).ok_or_else(|| no_such_queue(name))?;
    queue.expire(now);
    Ok(queue)
}

fn no_such_queue(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}

/// Rules shared by `CreateQueue` and `SetQueueAttributes` (the dead-letter
/// pairing and content-based deduplication on a standard queue).
fn check_attribute_combination(update: &AttributeUpdate, fifo: bool) -> Result<()> {
    if update.content_based_deduplication == Some(true) && !fifo {
        return Err(Error::new(
            ErrorCode::InvalidParameterValue,
            "ContentBasedDeduplication requires a FIFO queue",
        ));
    }
    if update.dead_letter_queue.is_some() != update.max_receive_count.is_some() {
        return Err(Error::new(
            ErrorCode::InvalidParameterValue,
            "DeadLetterQueue and MaxReceiveCount must be given together",
        ));
    }
    Ok(())
}

fn check_dead_letter_type(target: &Attributes, fifo: bool) -> Result<()> {
    if target.fifo == fifo {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::InvalidParameterValue,
            "a dead-letter queue must be of the same type as its source queue",
        ))
    }
}
