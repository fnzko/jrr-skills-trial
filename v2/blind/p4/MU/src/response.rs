use serde::ser::{Serialize, SerializeMap, Serializer};

use crate::error::{Error, ErrorCode};
use crate::queue::{Counts, Delivery, QueueAttributes, QueueKind, Sent};
use crate::types::{QueueName, Seconds, Timestamp};

/// The result members of a successful command.
#[derive(Debug)]
pub(crate) enum Output {
    Empty,
    QueueAttributes {
        attributes: AttributesView,
        counts: Counts,
    },
    QueueNames(Vec<QueueName>),
    Now(Timestamp),
    Sent(Sent),
    Messages(Vec<Delivery>),
}

/// A queue's attributes as `GetQueueAttributes` reports them.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct AttributesView {
    visibility_timeout: Seconds,
    delay_seconds: Seconds,
    retention_seconds: Seconds,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dead_letter_queue: Option<QueueName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_receive_count: Option<u32>,
}

impl From<&QueueAttributes> for AttributesView {
    fn from(attributes: &QueueAttributes) -> Self {
        let dead_letter = attributes.dead_letter.as_ref();
        AttributesView {
            visibility_timeout: attributes.visibility_timeout,
            delay_seconds: attributes.delay,
            retention_seconds: attributes.retention,
            fifo: attributes.kind.is_fifo(),
            content_based_deduplication: matches!(
                attributes.kind,
                QueueKind::Fifo {
                    content_based_deduplication: true
                }
            ),
            dead_letter_queue: dead_letter.map(|dead_letter| dead_letter.queue.clone()),
            max_receive_count: dead_letter.map(|dead_letter| dead_letter.max_receive_count),
        }
    }
}

/// The reply to one input line; serializes to the single JSON object that
/// SPEC.md section 1 prescribes for success or failure.
#[derive(Debug)]
pub struct Response(pub(crate) Result<Output, Error>);

#[derive(serde::Serialize)]
struct ErrorBody<'a> {
    code: ErrorCode,
    message: &'a str,
}

impl Serialize for Response {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match &self.0 {
            Ok(output) => {
                map.serialize_entry("ok", &true)?;
                match output {
                    Output::Empty => {}
                    Output::QueueAttributes { attributes, counts } => {
                        map.serialize_entry("Attributes", attributes)?;
                        map.serialize_entry("Counts", counts)?;
                    }
                    Output::QueueNames(names) => map.serialize_entry("QueueNames", names)?,
                    Output::Now(now) => map.serialize_entry("Now", now)?,
                    Output::Sent(sent) => {
                        map.serialize_entry("MessageId", &sent.id)?;
                        if let Some(sequence) = &sent.sequence {
                            map.serialize_entry("SequenceNumber", sequence)?;
                        }
                    }
                    Output::Messages(messages) => map.serialize_entry("Messages", messages)?,
                }
            }
            Err(error) => {
                map.serialize_entry("ok", &false)?;
                let message = error.to_string();
                map.serialize_entry(
                    "error",
                    &ErrorBody {
                        code: error.code(),
                        message: &message,
                    },
                )?;
            }
        }
        map.end()
    }
}
