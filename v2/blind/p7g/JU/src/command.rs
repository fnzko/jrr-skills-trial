//! Parsed commands. Everything here has already passed the shape and value checks of
//! the protocol; what is left are the rules that need queue state.

use crate::error::{CommandError, ErrorCode};
use crate::ids::{DedupKey, GroupId, QueueName};
use crate::time::Seconds;

/// One request line, borrowing its strings from the parsed JSON.
#[derive(Debug)]
pub enum Command<'a> {
    CreateQueue {
        name: QueueName,
        attributes: Attributes<'a>,
    },
    SetQueueAttributes {
        queue: &'a str,
        attributes: Attributes<'a>,
    },
    GetQueueAttributes {
        queue: &'a str,
    },
    ListQueues {
        prefix: Option<&'a str>,
    },
    DeleteQueue {
        queue: &'a str,
    },
    PurgeQueue {
        queue: &'a str,
    },
    Tick {
        seconds: Seconds,
    },
    SendMessage(SendMessage<'a>),
    ReceiveMessage {
        queue: &'a str,
        max_messages: usize,
        visibility_timeout: Option<Seconds>,
    },
    DeleteMessage {
        queue: &'a str,
        receipt_handle: &'a str,
    },
    ChangeMessageVisibility {
        queue: &'a str,
        receipt_handle: &'a str,
        visibility_timeout: Seconds,
    },
}

/// The attributes named in a `CreateQueue` or `SetQueueAttributes` request; `None` is "left out".
#[derive(Debug, Default)]
pub struct Attributes<'a> {
    pub visibility_timeout: Option<Seconds>,
    pub delay: Option<Seconds>,
    pub retention: Option<Seconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<&'a str>,
    pub max_receive_count: Option<u32>,
}

/// A complete dead-letter configuration as requested. The target may not exist yet.
#[derive(Debug, Clone, Copy)]
pub struct DeadLetterRequest<'a> {
    pub queue: &'a str,
    pub max_receive_count: u32,
}

impl<'a> Attributes<'a> {
    /// The dead-letter configuration is set or unset as a whole.
    pub fn dead_letter(&self) -> Result<Option<DeadLetterRequest<'a>>, CommandError> {
        match (self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetterRequest {
                queue,
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(CommandError::new(
                ErrorCode::InvalidParameterValue,
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }
}

#[derive(Debug)]
pub struct SendMessage<'a> {
    pub queue: &'a str,
    pub body: &'a str,
    pub delay: Option<Seconds>,
    pub group: Option<GroupId>,
    pub deduplication_id: Option<DedupKey>,
}
