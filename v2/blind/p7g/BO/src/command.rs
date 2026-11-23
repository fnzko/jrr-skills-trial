//! Parsed commands. Everything here already passed the shape, required and
//! value checks of the request, so only state-dependent rules remain.

use crate::error::{Error, Problem};
use crate::types::{
    Body, DeduplicationId, DelaySeconds, GroupId, MaxMessages, MaxReceiveCount, QueueName,
    RetentionSeconds, TickSeconds, VisibilityTimeout,
};

/// Attributes named in a `CreateQueue` or `SetQueueAttributes` request.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttributeSet {
    pub visibility_timeout: Option<VisibilityTimeout>,
    pub delay: Option<DelaySeconds>,
    pub retention: Option<RetentionSeconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<MaxReceiveCount>,
}

impl AttributeSet {
    /// The dead-letter configuration, which is both attributes or neither.
    pub fn dead_letter(&self) -> Result<Option<(&str, MaxReceiveCount)>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(count)) => Ok(Some((queue, count))),
            (None, None) => Ok(None),
            (Some(_), None) => Err(Error::invalid(
                "MaxReceiveCount",
                Problem::Conflict("DeadLetterQueue requires MaxReceiveCount"),
            )),
            (None, Some(_)) => Err(Error::invalid(
                "DeadLetterQueue",
                Problem::Conflict("MaxReceiveCount requires DeadLetterQueue"),
            )),
        }
    }
}

/// A message as the client sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMessage {
    pub body: Body,
    pub delay: Option<DelaySeconds>,
    pub group: Option<GroupId>,
    pub deduplication: Option<DeduplicationId>,
}

/// Queue names in commands other than `CreateQueue` are matched as plain
/// strings, so they carry no naming rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: QueueName,
        attributes: AttributeSet,
    },
    SetQueueAttributes {
        queue: String,
        attributes: AttributeSet,
    },
    GetQueueAttributes {
        queue: String,
    },
    ListQueues {
        prefix: Option<String>,
    },
    DeleteQueue {
        queue: String,
    },
    PurgeQueue {
        queue: String,
    },
    Tick {
        seconds: TickSeconds,
    },
    SendMessage {
        queue: String,
        message: NewMessage,
    },
    ReceiveMessage {
        queue: String,
        max: MaxMessages,
        visibility_timeout: Option<VisibilityTimeout>,
    },
    DeleteMessage {
        queue: String,
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt_handle: String,
        visibility_timeout: VisibilityTimeout,
    },
}
