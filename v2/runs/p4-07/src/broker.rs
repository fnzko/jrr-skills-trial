//! The queue table, the clock and the id counters. A command is applied all-or-nothing: every
//! check of SPEC.md runs before the first change, so a refused command leaves no trace.

use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;

use crate::error::{ErrorCode, Failure};
use crate::queue::{ArrivalKey, DeadLetter, Queue, SendOutcome};
use crate::request::{parse_line, AttributeRequest, Command, DeadLetterRequest, SendRequest};
use crate::response::Output;
use crate::types::{Clock, MessageId, QueueName, QueueRef, ReceiptHandle, Seconds};

/// Queues one broker may hold (section 5, CreateQueue rule 7).
pub const MAX_QUEUES: usize = 1000;

/// Where a receipt handle's delivery was made. The queue is named, not identified, because a
/// queue deleted and created again under the same name must not inherit its handles. Arrival
/// keys are unique, so a stale location never finds a message in the new queue.
#[derive(Debug)]
struct Location {
    queue: QueueName,
    arrival: ArrivalKey,
}

#[derive(Debug)]
pub struct Broker {
    now: Clock,
    queues: BTreeMap<QueueName, Queue>,
    next_message: MessageId,
    next_arrival: ArrivalKey,
    next_receipt: ReceiptHandle,
    receipts: HashMap<ReceiptHandle, Location>,
}

impl Default for Broker {
    fn default() -> Broker {
        Broker {
            now: Clock::START,
            queues: BTreeMap::new(),
            next_message: MessageId::FIRST,
            next_arrival: ArrivalKey::FIRST,
            next_receipt: ReceiptHandle::FIRST,
            receipts: HashMap::new(),
        }
    }
}

impl Broker {
    /// Parses and applies one input line, without its newline.
    pub fn execute_line(&mut self, line: &[u8]) -> Result<Output, Failure> {
        let command = parse_line(line)?;
        self.apply(command)
    }

    fn apply(&mut self, command: Command) -> Result<Output, Failure> {
        match command {
            Command::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Command::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Command::GetQueueAttributes { queue } => self.get_queue_attributes(&queue),
            Command::ListQueues { prefix } => Ok(self.list_queues(prefix.as_deref())),
            Command::DeleteQueue { queue } => self.delete_queue(&queue),
            Command::PurgeQueue { queue } => self.purge_queue(&queue),
            Command::Tick { seconds } => self.tick(seconds),
            Command::SendMessage { queue, request } => self.send_message(&queue, request),
            Command::ReceiveMessage {
                queue,
                max_messages,
                visibility,
            } => self.receive_message(&queue, max_messages, visibility),
            Command::DeleteMessage { queue, receipt } => self.delete_message(&queue, receipt),
            Command::ChangeMessageVisibility {
                queue,
                receipt,
                visibility,
            } => self.change_message_visibility(&queue, receipt, visibility),
        }
    }

    fn create_queue(
        &mut self,
        name: QueueName,
        attributes: &AttributeRequest,
    ) -> Result<Output, Failure> {
        let fifo = attributes.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Failure::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when the queue is a FIFO queue",
            ));
        }
        if attributes.content_based_deduplication.unwrap_or(false) && !fifo {
            return Err(Failure::new(
                ErrorCode::InvalidParameterValue,
                "ContentBasedDeduplication applies only to FIFO queues",
            ));
        }
        let dead_letter = attributes.dead_letter_request()?;
        if let Some(existing) = self.queues.get(name.as_str()) {
            return if existing.matches(attributes, dead_letter.as_ref()) {
                Ok(Output::Done {})
            } else {
                Err(Failure::new(
                    ErrorCode::QueueAlreadyExists,
                    "a queue with this name exists with other attributes",
                ))
            };
        }
        let dead_letter = dead_letter
            .map(|request| self.resolve_dead_letter(&request, name.as_str(), fifo))
            .transpose()?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(Failure::new(
                ErrorCode::LimitExceeded,
                "the broker already holds 1000 queues",
            ));
        }
        self.queues
            .insert(name, Queue::create(attributes, dead_letter));
        Ok(Output::Done {})
    }

    fn set_queue_attributes(
        &mut self,
        queue: &QueueRef,
        attributes: &AttributeRequest,
    ) -> Result<Output, Failure> {
        let now = self.now;
        let (_, found) = lookup_mut(&mut self.queues, queue, now)?;
        let fifo = found.is_fifo();
        if attributes.fifo.is_some() {
            return Err(Failure::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot be changed",
            ));
        }
        if attributes.content_based_deduplication.unwrap_or(false) && !fifo {
            return Err(Failure::new(
                ErrorCode::InvalidParameterValue,
                "ContentBasedDeduplication applies only to FIFO queues",
            ));
        }
        let dead_letter = match attributes.dead_letter_request()? {
            None => None,
            Some(request) => Some(self.resolve_dead_letter(&request, queue.as_str(), fifo)?),
        };
        let (_, found) = lookup_mut(&mut self.queues, queue, now)?;
        found.update(attributes, dead_letter);
        Ok(Output::Done {})
    }

    fn get_queue_attributes(&mut self, queue: &QueueRef) -> Result<Output, Failure> {
        let now = self.now;
        let (_, found) = lookup_mut(&mut self.queues, queue, now)?;
        Ok(Output::Attributes {
            attributes: found.attributes(),
            counts: found.counts(now),
        })
    }

    fn list_queues(&self, prefix: Option<&str>) -> Output {
        let prefix = prefix.unwrap_or("");
        let queue_names = self
            .queues
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(name, _)| name)
            .take_while(|name| name.as_str().starts_with(prefix))
            .cloned()
            .collect();
        Output::QueueNames { queue_names }
    }

    fn delete_queue(&mut self, queue: &QueueRef) -> Result<Output, Failure> {
        let name = self
            .queues
            .get_key_value(queue.as_str())
            .map(|(name, _)| name.clone())
            .ok_or_else(|| does_not_exist(queue))?;
        self.queues.remove(&name);
        for remaining in self.queues.values_mut() {
            remaining.forget_dead_letter_target(&name);
        }
        Ok(Output::Done {})
    }

    fn purge_queue(&mut self, queue: &QueueRef) -> Result<Output, Failure> {
        let now = self.now;
        let (_, found) = lookup_mut(&mut self.queues, queue, now)?;
        found.purge();
        Ok(Output::Done {})
    }

    fn tick(&mut self, seconds: Seconds) -> Result<Output, Failure> {
        let now = self.now.advanced(seconds).ok_or_else(|| {
            Failure::new(
                ErrorCode::InvalidParameterValue,
                "Tick would move the clock past 4000000000",
            )
        })?;
        self.now = now;
        Ok(Output::Now { now })
    }

    fn send_message(&mut self, queue: &QueueRef, request: SendRequest) -> Result<Output, Failure> {
        let now = self.now;
        let (_, found) = lookup_mut(&mut self.queues, queue, now)?;
        match found.send(now, request, self.next_message, self.next_arrival)? {
            SendOutcome::Accepted(sent) => {
                self.next_message = self.next_message.next();
                self.next_arrival = self.next_arrival.next();
                Ok(Output::Sent(sent))
            }
            SendOutcome::Duplicate(sent) => Ok(Output::Sent(sent)),
        }
    }

    fn receive_message(
        &mut self,
        queue: &QueueRef,
        max_messages: u32,
        visibility: Option<Seconds>,
    ) -> Result<Output, Failure> {
        let now = self.now;
        let (name, source) = lookup_mut(&mut self.queues, queue, now)?;
        let visibility = visibility.unwrap_or(source.settings().visibility_timeout);
        let dead_letter_target = source.dead_letter_target().cloned();
        let received = source.receive(now, max_messages, visibility, &mut self.next_receipt);
        for (arrival, delivered) in &received.deliveries {
            self.receipts.insert(
                delivered.receipt_handle,
                Location {
                    queue: name.clone(),
                    arrival: *arrival,
                },
            );
        }
        if !received.dead_lettered.is_empty() {
            let target = dead_letter_target
                .expect("only a queue with a dead-letter configuration moves messages");
            for message in received.dead_lettered {
                let arrival = self.next_arrival;
                self.next_arrival = arrival.next();
                self.queues
                    .get_mut(&target)
                    .expect("DeleteQueue clears dead-letter references, so the target exists")
                    .accept_dead_letter(arrival, message, now);
            }
        }
        let messages = received
            .deliveries
            .into_iter()
            .map(|(_, delivered)| delivered)
            .collect();
        Ok(Output::Messages { messages })
    }

    fn delete_message(
        &mut self,
        queue: &QueueRef,
        receipt: Option<ReceiptHandle>,
    ) -> Result<Output, Failure> {
        let now = self.now;
        let (name, found) = lookup_mut(&mut self.queues, queue, now)?;
        let arrival = in_flight_arrival(&self.receipts, name, found, now, receipt)?;
        found.remove(arrival);
        Ok(Output::Done {})
    }

    fn change_message_visibility(
        &mut self,
        queue: &QueueRef,
        receipt: Option<ReceiptHandle>,
        visibility: Seconds,
    ) -> Result<Output, Failure> {
        let now = self.now;
        let (name, found) = lookup_mut(&mut self.queues, queue, now)?;
        let arrival = in_flight_arrival(&self.receipts, name, found, now, receipt)?;
        found.change_visibility(arrival, now, visibility);
        Ok(Output::Done {})
    }

    /// Checks a dead-letter configuration against the queues that exist. `own_name` and `own_fifo`
    /// describe the queue being created or changed; a queue is never its own dead-letter queue.
    fn resolve_dead_letter(
        &self,
        request: &DeadLetterRequest,
        own_name: &str,
        own_fifo: bool,
    ) -> Result<DeadLetter, Failure> {
        let (target_name, target) = self
            .queues
            .get_key_value(request.queue.as_str())
            .ok_or_else(|| does_not_exist(&request.queue))?;
        if target_name.as_str() == own_name || target.is_fifo() != own_fifo {
            return Err(Failure::new(
                ErrorCode::InvalidParameterValue,
                "the dead-letter queue must be another queue of the same type",
            ));
        }
        Ok(DeadLetter {
            queue: target_name.clone(),
            max_receive_count: request.max_receive_count,
        })
    }
}

/// Finds the queue a command names and expires its messages, so that everything the command
/// then reads is the state section 4 defines at `now`.
fn lookup_mut<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    queue: &QueueRef,
    now: Clock,
) -> Result<(&'a QueueName, &'a mut Queue), Failure> {
    // A range with equal included bounds holds at most the one queue with this name, and it
    // yields the key as well as a mutable value, which `BTreeMap::get_key_value` cannot.
    let text = queue.as_str();
    let (name, found) = queues
        .range_mut::<str, _>((Bound::Included(text), Bound::Included(text)))
        .next()
        .ok_or_else(|| does_not_exist(queue))?;
    found.expire(now);
    Ok((name, found))
}

/// The arrival key of the message a receipt handle names, provided the handle is the latest one
/// of a delivery that is still in flight in `queue`.
fn in_flight_arrival(
    receipts: &HashMap<ReceiptHandle, Location>,
    name: &QueueName,
    queue: &Queue,
    now: Clock,
    receipt: Option<ReceiptHandle>,
) -> Result<ArrivalKey, Failure> {
    let never_issued = || {
        Failure::new(
            ErrorCode::ReceiptHandleInvalid,
            "ReceiptHandle was never issued",
        )
    };
    let expired = || {
        Failure::new(
            ErrorCode::ReceiptHandleExpired,
            "ReceiptHandle is no longer in flight",
        )
    };
    let receipt = receipt.ok_or_else(never_issued)?;
    let location = receipts.get(&receipt).ok_or_else(never_issued)?;
    if location.queue != *name {
        return Err(expired());
    }
    let message = queue.message(location.arrival).ok_or_else(expired)?;
    if message.in_flight_with(receipt, now) {
        Ok(location.arrival)
    } else {
        Err(expired())
    }
}

fn does_not_exist(queue: &QueueRef) -> Failure {
    Failure::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {:?} does not exist", queue.as_str()),
    )
}
