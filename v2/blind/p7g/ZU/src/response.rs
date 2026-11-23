//! The shape of every response line.

use serde::Serialize;

use crate::attributes::Attributes;
use crate::error::{Error, ErrorCode};
use crate::queue::{Counts, Delivery};
use crate::types::{MessageId, QueueName, SequenceNumber, Time};

/// The result members of a successful command.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Reply {
    /// Commands without result members.
    Done {},
    Now {
        now: Time,
    },
    QueueNames {
        queue_names: Vec<QueueName>,
    },
    QueueAttributes {
        attributes: Attributes,
        counts: Counts,
    },
    Messages {
        messages: Vec<Delivery>,
    },
    Sent {
        message_id: MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence_number: Option<SequenceNumber>,
    },
}

#[derive(Serialize)]
pub struct Success<'a> {
    ok: bool,
    #[serde(flatten)]
    reply: &'a Reply,
}

impl<'a> Success<'a> {
    pub fn new(reply: &'a Reply) -> Self {
        Self { ok: true, reply }
    }
}

#[derive(Serialize)]
pub struct Failure<'a> {
    ok: bool,
    error: ErrorBody<'a>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: ErrorCode,
    message: &'a str,
}

impl<'a> Failure<'a> {
    pub fn new(error: &'a Error) -> Self {
        Self {
            ok: false,
            error: ErrorBody {
                code: error.code(),
                message: error.message(),
            },
        }
    }
}
