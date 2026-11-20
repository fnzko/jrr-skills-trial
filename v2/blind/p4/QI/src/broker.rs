//! The broker: every queue and the clock, and the execution of each command against them.
//! The rules of each command follow SPEC section 5 and 6 in the order they are written there,
//! after the request and parameter checks that the parser has already done.

use crate::attributes::{AttributeUpdate, DeadLetter, DeadLetterRequest, QueueConfig};
use crate::command::{self, Command};
use crate::error::{ErrorCode, Failure};
use crate::ids::{DeduplicationId, GroupId, MessageId, QueueName, ReceiptHandle};
use crate::params::{invalid, missing};
use crate::queue::{Accepted, Enqueue, Message, Queue};
use crate::reply::{
    AttributesResult, Empty, NowResult, Outcome, QueueNamesResult, ReceiveResult, ReceivedMessage,
    Reply, SentResult,
};
use crate::time::{Instant, Seconds, MAX_CLOCK};
use std::collections::BTreeMap;
use std::ops::Bound;

/// A broker holds at most this many queues.
const MAX_QUEUES: usize = 1000;
/// A queue holds at most this many messages, in any state.
const MAX_MESSAGES: usize = 100_000;

#[derive(Debug, Default)]
pub struct Broker {
    queues: BTreeMap<QueueName, Queue>,
    now: Instant,
    /// Messages accepted so far. The next MessageId is one more than this.
    messages_accepted: u64,
    /// Deliveries made so far. Receipt handles 1 through this number have been issued.
    deliveries: u64,
}

impl Broker {
    /// Runs one command line and returns its response.
    pub fn handle_line(&mut self, line: &[u8]) -> Reply<'_> {
        let result = match command::parse(line) {
            Ok(command) => self.execute(command),
            Err(failure) => Err(failure),
        };
        match result {
            Ok(result) => Reply::Success { ok: true, result },
            Err(error) => Reply::Failure { ok: false, error },
        }
    }

    fn execute(&mut self, command: Command) -> Result<Outcome<'_>, Failure> {
        match command {
            Command::CreateQueue { name, attributes } => {
                self.create_queue(name, &attributes)?;
                Ok(Outcome::Empty(Empty {}))
            }
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)?;
                Ok(Outcome::Empty(Empty {}))
            }
            Command::GetQueueAttributes { queue } => {
                let found = self.queue(&queue)?;
                Ok(Outcome::Attributes(AttributesResult::new(
                    found.config(),
                    found.counts(self.now),
                )))
            }
            Command::ListQueues { prefix } => Ok(Outcome::QueueNames(QueueNamesResult {
                queue_names: self.list(prefix.as_deref()),
            })),
            Command::DeleteQueue { queue } => {
                self.delete_queue(&queue)?;
                Ok(Outcome::Empty(Empty {}))
            }
            Command::PurgeQueue { queue } => {
                lookup_mut(&mut self.queues, &queue)?.purge();
                Ok(Outcome::Empty(Empty {}))
            }
            Command::Tick { seconds } => {
                self.tick(seconds)?;
                Ok(Outcome::Now(NowResult { now: self.now }))
            }
            Command::SendMessage {
                queue,
                body,
                delay,
                group,
                deduplication,
            } => self.send_message(&queue, &body, delay, group, deduplication),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_message(&queue, max_messages, visibility_timeout),
            Command::DeleteMessage { queue, receipt } => {
                self.delete_message(&queue, &receipt)?;
                Ok(Outcome::Empty(Empty {}))
            }
            Command::ChangeMessageVisibility {
                queue,
                receipt,
                visibility_timeout,
            } => {
                self.change_message_visibility(&queue, &receipt, visibility_timeout)?;
                Ok(Outcome::Empty(Empty {}))
            }
        }
    }

    fn queue(&self, name: &str) -> Result<&Queue, Failure> {
        self.queues.get(name).ok_or_else(|| does_not_exist(name))
    }

    fn create_queue(
        &mut self,
        name: QueueName,
        attributes: &AttributeUpdate,
    ) -> Result<(), Failure> {
        let defaults = QueueConfig::defaults();
        let wanted = defaults.updated(attributes, None);
        if name.as_str().ends_with(".fifo") != wanted.fifo() {
            return Err(Failure::new(
                ErrorCode::QueueNameInvalid,
                format!(
                    "queue name {} must end in .fifo exactly when Fifo is true",
                    name.as_str()
                ),
            ));
        }
        if wanted.content_based_deduplication() && !wanted.fifo() {
            return Err(invalid("ContentBasedDeduplication", "needs a FIFO queue"));
        }
        let requested = attributes.dead_letter()?;
        if let Some(existing) = self.queues.get(name.as_str()) {
            let stored = existing.config();
            let same = dead_letter_matches(stored.dead_letter(), requested)
                && stored.without_dead_letter() == wanted;
            return if same {
                Ok(())
            } else {
                Err(Failure::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {} exists with different attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = requested
            .map(|request| dead_letter_target(&self.queues, name.as_str(), wanted.fifo(), request))
            .transpose()?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Failure::new(
                ErrorCode::LimitExceeded,
                format!("the broker already holds {MAX_QUEUES} queues"),
            ));
        }
        self.queues
            .insert(name, Queue::new(defaults.updated(attributes, dead_letter)));
        Ok(())
    }

    fn set_queue_attributes(
        &mut self,
        name: &str,
        attributes: &AttributeUpdate,
    ) -> Result<(), Failure> {
        let fifo = self.queue(name)?.config().fifo();
        if attributes.fifo().is_some() {
            return Err(Failure::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if attributes.content_based_deduplication() == Some(true) && !fifo {
            return Err(invalid("ContentBasedDeduplication", "needs a FIFO queue"));
        }
        let dead_letter = attributes
            .dead_letter()?
            .map(|request| dead_letter_target(&self.queues, name, fifo, request))
            .transpose()?;
        let queue = lookup_mut(&mut self.queues, name)?;
        let config = queue.config().updated(attributes, dead_letter);
        queue.set_config(config);
        // A shorter retention takes effect at once, for messages already queued.
        queue.expire(self.now);
        Ok(())
    }

    fn delete_queue(&mut self, name: &str) -> Result<(), Failure> {
        self.queues
            .remove(name)
            .ok_or_else(|| does_not_exist(name))?;
        for queue in self.queues.values_mut() {
            if queue
                .config()
                .dead_letter()
                .is_some_and(|dead_letter| dead_letter.queue().as_str() == name)
            {
                let config = queue.config().without_dead_letter();
                queue.set_config(config);
            }
        }
        Ok(())
    }

    /// Queue names that start with `prefix`, in byte order. Names with a given prefix are next to
    /// each other in that order, so the walk stops at the first one that does not match.
    fn list(&self, prefix: Option<&str>) -> Vec<&QueueName> {
        let prefix = prefix.unwrap_or("");
        self.queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name)
            .take_while(|name| name.as_str().starts_with(prefix))
            .collect()
    }

    fn tick(&mut self, by: Seconds) -> Result<(), Failure> {
        let now = self
            .now
            .advanced_by(by)
            .ok_or_else(|| invalid("Seconds", format!("would move the clock past {MAX_CLOCK}")))?;
        self.now = now;
        for queue in self.queues.values_mut() {
            queue.expire(now);
            queue.promote(now);
        }
        Ok(())
    }

    fn send_message(
        &mut self,
        name: &str,
        body: &str,
        delay: Option<Seconds>,
        group: Option<GroupId>,
        deduplication: Option<DeduplicationId>,
    ) -> Result<Outcome<'_>, Failure> {
        let now = self.now;
        let number = self
            .messages_accepted
            .checked_add(1)
            .expect("a u64 message count does not run out");
        let queue = lookup_mut(&mut self.queues, name)?;
        let fifo = queue.config().fifo();
        if !fifo && (group.is_some() || deduplication.is_some()) {
            return Err(Failure::new(
                ErrorCode::UnsupportedOperation,
                "GroupId and DeduplicationId need a FIFO queue",
            ));
        }
        if fifo && delay.is_some() {
            return Err(Failure::new(
                ErrorCode::UnsupportedOperation,
                "DelaySeconds is not allowed on a FIFO queue",
            ));
        }
        let group = match (fifo, group) {
            (true, Some(group)) => Some(group),
            (true, None) => return Err(missing("GroupId")),
            (false, _) => None,
        };
        let key = if fifo {
            match (&deduplication, queue.config().content_based_deduplication()) {
                (Some(id), _) => Some(id.as_str()),
                (None, true) => Some(body),
                (None, false) => return Err(missing("DeduplicationId")),
            }
        } else {
            None
        };
        if let Some(key) = key {
            if let Some(accepted) = queue.recent_duplicate(key, now) {
                return Ok(Outcome::Sent(SentResult {
                    message_id: accepted.id(),
                    sequence_number: Some(accepted.sequence()),
                }));
            }
        }
        if queue.message_count() >= MAX_MESSAGES {
            return Err(Failure::new(
                ErrorCode::LimitExceeded,
                format!("the queue already holds {MAX_MESSAGES} messages"),
            ));
        }
        let id = MessageId::issued(number);
        let delay = delay.unwrap_or(queue.config().delay_seconds());
        let entry = Enqueue::new(id, body.to_owned(), now, now + delay, group);
        let sequence = queue.append(entry, now);
        if let (Some(key), Some(sequence)) = (key, sequence) {
            queue.record_accepted(key, Accepted::new(id, sequence, now));
        }
        self.messages_accepted = number;
        Ok(Outcome::Sent(SentResult {
            message_id: id,
            sequence_number: sequence,
        }))
    }

    fn receive_message(
        &mut self,
        name: &str,
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
    ) -> Result<Outcome<'_>, Failure> {
        let now = self.now;
        let queue = lookup_mut(&mut self.queues, name)?;
        let timeout = visibility_timeout.unwrap_or(queue.config().visibility_timeout());
        let deliveries = &mut self.deliveries;
        let received = queue.receive(now, max_messages, timeout, || {
            *deliveries += 1;
            ReceiptHandle::issued(*deliveries)
        });
        let moved = received.dead_lettered;
        if !moved.is_empty() {
            let target = queue
                .config()
                .dead_letter()
                .map(|dead_letter| dead_letter.queue().clone())
                .expect("messages move only from a queue with a dead-letter configuration");
            self.move_to_dead_letter(&target, moved);
        }
        let queue = self.queue(name)?;
        let messages = received
            .delivered
            .iter()
            .map(|&arrival| ReceivedMessage::new(queue.message(arrival)))
            .collect();
        Ok(Outcome::Received(ReceiveResult { messages }))
    }

    /// Adds messages that ran out of delivery attempts to the end of their dead-letter queue.
    fn move_to_dead_letter(&mut self, target: &QueueName, messages: Vec<Message>) {
        let now = self.now;
        let queue = self.queues.get_mut(target.as_str()).expect(
            "deleting a queue clears the dead-letter references to it, so the target exists",
        );
        for message in messages {
            queue.append(Enqueue::dead_lettered(message, now), now);
        }
        // A message already past its retention in the dead-letter queue is removed on arrival.
        queue.expire(now);
    }

    fn delete_message(&mut self, name: &str, receipt: &str) -> Result<(), Failure> {
        let now = self.now;
        let issued = self.deliveries;
        let queue = lookup_mut(&mut self.queues, name)?;
        let handle = issued_receipt(receipt, issued)?;
        let arrival = queue
            .in_flight(handle, now)
            .ok_or_else(|| expired(receipt))?;
        queue.remove(arrival, now);
        Ok(())
    }

    fn change_message_visibility(
        &mut self,
        name: &str,
        receipt: &str,
        visibility_timeout: Seconds,
    ) -> Result<(), Failure> {
        let now = self.now;
        let issued = self.deliveries;
        let queue = lookup_mut(&mut self.queues, name)?;
        let handle = issued_receipt(receipt, issued)?;
        let arrival = queue
            .in_flight(handle, now)
            .ok_or_else(|| expired(receipt))?;
        queue.set_visible_at(arrival, now + visibility_timeout, now);
        Ok(())
    }
}

fn lookup_mut<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    name: &str,
) -> Result<&'a mut Queue, Failure> {
    queues.get_mut(name).ok_or_else(|| does_not_exist(name))
}

fn does_not_exist(name: &str) -> Failure {
    Failure::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name} does not exist"),
    )
}

fn expired(receipt: &str) -> Failure {
    Failure::new(
        ErrorCode::ReceiptHandleExpired,
        format!("receipt handle {receipt} is no longer usable"),
    )
}

/// A receipt handle this process has issued. Anything else, including a handle with a number
/// nothing has been issued for yet, is invalid.
fn issued_receipt(text: &str, issued: u64) -> Result<ReceiptHandle, Failure> {
    ReceiptHandle::parse(text)
        .filter(|handle| handle.number() <= issued)
        .ok_or_else(|| {
            Failure::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("receipt handle {text} was never issued"),
            )
        })
}

/// Whether a request's dead-letter configuration is the one already stored.
fn dead_letter_matches(
    stored: Option<&DeadLetter>,
    requested: Option<DeadLetterRequest<'_>>,
) -> bool {
    match (stored, requested) {
        (None, None) => true,
        (Some(stored), Some(request)) => {
            stored.queue().as_str() == request.queue()
                && stored.max_receive_count() == request.max_receive_count()
        }
        _ => false,
    }
}

/// The dead-letter configuration a request names. The target must exist, must not be the queue
/// being configured, and must have the same Fifo setting as that queue.
fn dead_letter_target(
    queues: &BTreeMap<QueueName, Queue>,
    own_name: &str,
    fifo: bool,
    request: DeadLetterRequest<'_>,
) -> Result<DeadLetter, Failure> {
    let (target_name, target) = queues
        .get_key_value(request.queue())
        .ok_or_else(|| does_not_exist(request.queue()))?;
    if target_name.as_str() == own_name || target.config().fifo() != fifo {
        return Err(invalid(
            "DeadLetterQueue",
            "must name another queue of the same type",
        ));
    }
    Ok(DeadLetter::from_request(target_name.clone(), request))
}
