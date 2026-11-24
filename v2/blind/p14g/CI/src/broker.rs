//! The state of the whole process and the execution of commands.

use std::collections::BTreeMap;
use std::ops::Bound;

use crate::attributes::AttributeSet;
use crate::error::{ApiError, ErrorCode};
use crate::queue::{DeadLetter, Queue, QueueKind, SendOutcome, Settings};
use crate::reply::{
    AttributesOutput, Empty, Output, QueueNamesOutput, ReceiveOutput, Reply, TickOutput,
};
use crate::request::Request;
use crate::types::{HandleIssuer, MessageId, QueueName, QueueRef, ReceiptHandle, Seconds, Time};

const MAX_QUEUES: usize = 1000;

#[derive(Debug)]
pub struct Broker {
    now: Time,
    queues: BTreeMap<QueueName, Queue>,
    next_message_id: MessageId,
    handles: HandleIssuer,
}

impl Default for Broker {
    fn default() -> Self {
        Self {
            now: Time::START,
            queues: BTreeMap::new(),
            next_message_id: MessageId::FIRST,
            handles: HandleIssuer::default(),
        }
    }
}

impl Broker {
    /// Runs the command on `line` and returns its response.
    pub fn handle_line(&mut self, line: &[u8]) -> Reply {
        Request::parse(line)
            .and_then(|request| self.execute(request))
            .into()
    }

    fn execute(&mut self, request: Request) -> Result<Output, ApiError> {
        let now = self.now;
        match request {
            Request::CreateQueue { name, attributes } => self.create_queue(name, &attributes),
            Request::SetQueueAttributes { queue, attributes } => {
                self.set_queue_attributes(&queue, &attributes)
            }
            Request::GetQueueAttributes { queue } => {
                let queue = find(&mut self.queues, &queue, now)?;
                Ok(Output::Attributes(AttributesOutput {
                    attributes: queue.attributes(),
                    counts: queue.counts(now),
                }))
            }
            Request::ListQueues { prefix } => {
                let prefix = prefix.as_deref().unwrap_or("");
                let queue_names = self
                    .queues
                    .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
                    .map(|(name, _)| name)
                    .take_while(|name| name.as_str().starts_with(prefix))
                    .cloned()
                    .collect();
                Ok(Output::QueueNames(QueueNamesOutput { queue_names }))
            }
            Request::DeleteQueue { queue } => {
                self.queues
                    .remove(queue.as_str())
                    .ok_or_else(|| queue_missing(&queue))?;
                for other in self.queues.values_mut() {
                    other.drop_dead_letter(queue.as_str());
                }
                Ok(Output::Empty(Empty {}))
            }
            Request::PurgeQueue { queue } => {
                find(&mut self.queues, &queue, now)?.purge();
                Ok(Output::Empty(Empty {}))
            }
            Request::Tick { seconds } => {
                self.now = now.plus(seconds);
                Ok(Output::Tick(TickOutput { now: self.now }))
            }
            Request::SendMessage { queue, message } => {
                let target = find(&mut self.queues, &queue, now)?;
                let id = self.next_message_id;
                match target.send(now, message, id)? {
                    SendOutcome::Accepted(output) => {
                        self.next_message_id.take();
                        Ok(Output::Sent(output))
                    }
                    SendOutcome::Duplicate(output) => Ok(Output::Sent(output)),
                }
            }
            Request::ReceiveMessage {
                queue,
                max_messages,
                visibility_timeout,
            } => self.receive_messages(&queue, max_messages, visibility_timeout),
            Request::DeleteMessage {
                queue,
                receipt_handle,
            } => {
                let target = find(&mut self.queues, &queue, now)?;
                let handle = recognize(&self.handles, &receipt_handle)?;
                target.delete_message(handle, now)?;
                Ok(Output::Empty(Empty {}))
            }
            Request::ChangeMessageVisibility {
                queue,
                receipt_handle,
                visibility_timeout,
            } => {
                let target = find(&mut self.queues, &queue, now)?;
                let handle = recognize(&self.handles, &receipt_handle)?;
                target.change_visibility(handle, visibility_timeout, now)?;
                Ok(Output::Empty(Empty {}))
            }
        }
    }

    fn create_queue(
        &mut self,
        name: QueueName,
        attributes: &AttributeSet,
    ) -> Result<Output, ApiError> {
        let kind = if attributes.fifo == Some(true) {
            QueueKind::Fifo
        } else {
            QueueKind::Standard
        };
        if name.has_fifo_suffix() != (kind == QueueKind::Fifo) {
            return Err(ApiError::new(
                ErrorCode::QueueNameInvalid,
                "a queue name ends in .fifo exactly when Fifo is true",
            ));
        }
        if kind == QueueKind::Standard && attributes.content_based_deduplication == Some(true) {
            return Err(ApiError::invalid_value(
                "ContentBasedDeduplication",
                "false for a standard queue",
            ));
        }
        let dead_letter = attributes.dead_letter()?;
        if let Some(existing) = self.queues.get(name.as_str()) {
            return if existing.settings().matches(attributes) {
                Ok(Output::Empty(Empty {}))
            } else {
                Err(ApiError::new(
                    ErrorCode::QueueAlreadyExists,
                    format!("queue {} exists with different attributes", name.as_str()),
                ))
            };
        }
        let dead_letter = self.resolve_dead_letter(name.as_str(), kind, dead_letter)?;
        if self.queues.len() >= MAX_QUEUES {
            return Err(ApiError::new(
                ErrorCode::LimitExceeded,
                format!("{MAX_QUEUES} queues already exist"),
            ));
        }
        let mut settings = Settings::default();
        settings.apply(attributes, dead_letter);
        self.queues.insert(name, Queue::new(kind, settings));
        Ok(Output::Empty(Empty {}))
    }

    fn set_queue_attributes(
        &mut self,
        queue: &QueueRef,
        attributes: &AttributeSet,
    ) -> Result<Output, ApiError> {
        let now = self.now;
        let kind = find(&mut self.queues, queue, now)?.kind();
        if attributes.fifo.is_some() {
            return Err(ApiError::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        if kind == QueueKind::Standard && attributes.content_based_deduplication == Some(true) {
            return Err(ApiError::invalid_value(
                "ContentBasedDeduplication",
                "false for a standard queue",
            ));
        }
        let dead_letter =
            self.resolve_dead_letter(queue.as_str(), kind, attributes.dead_letter()?)?;
        find(&mut self.queues, queue, now)?.set_attributes(attributes, dead_letter, now);
        Ok(Output::Empty(Empty {}))
    }

    /// Looks up the dead-letter queue named by a complete configuration; it
    /// must exist, differ from `own_name` and have the queue's `kind`.
    fn resolve_dead_letter(
        &self,
        own_name: &str,
        kind: QueueKind,
        configuration: Option<(&QueueRef, u32)>,
    ) -> Result<Option<DeadLetter>, ApiError> {
        let Some((target, max_receive_count)) = configuration else {
            return Ok(None);
        };
        let (name, queue) = self
            .queues
            .get_key_value(target.as_str())
            .ok_or_else(|| queue_missing(target))?;
        if name.as_str() == own_name || queue.kind() != kind {
            return Err(ApiError::invalid_value(
                "DeadLetterQueue",
                "another queue of the same type",
            ));
        }
        Ok(Some(DeadLetter {
            queue: name.clone(),
            max_receive_count,
        }))
    }

    fn receive_messages(
        &mut self,
        queue: &QueueRef,
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
    ) -> Result<Output, ApiError> {
        let now = self.now;
        let source = find(&mut self.queues, queue, now)?;
        let dead_letter_queue = source.dead_letter().map(|dead| dead.queue.clone());
        let received = source.receive(now, max_messages, visibility_timeout, &mut self.handles);
        if let Some(name) = dead_letter_queue {
            let target = self
                .queues
                .get_mut(&name)
                .expect("a queue that is some queue's dead-letter queue is never removed");
            for message in received.dead_letters {
                target.accept_dead_letter(message, now);
            }
        }
        Ok(Output::Received(ReceiveOutput {
            messages: received.delivered,
        }))
    }
}

/// Finds a queue and drops its expired messages, so callers see it as of `now`.
fn find<'a>(
    queues: &'a mut BTreeMap<QueueName, Queue>,
    name: &QueueRef,
    now: Time,
) -> Result<&'a mut Queue, ApiError> {
    let queue = queues
        .get_mut(name.as_str())
        .ok_or_else(|| queue_missing(name))?;
    queue.expire(now);
    Ok(queue)
}

fn queue_missing(name: &QueueRef) -> ApiError {
    ApiError::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {:?} does not exist", name.as_str()),
    )
}

fn recognize(issuer: &HandleIssuer, text: &str) -> Result<ReceiptHandle, ApiError> {
    issuer.recognize(text).ok_or_else(|| {
        ApiError::new(
            ErrorCode::ReceiptHandleInvalid,
            format!("receipt handle {text:?} was never issued"),
        )
    })
}
