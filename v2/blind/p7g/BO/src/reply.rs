//! The JSON a command produces on stdout.

use serde::Serialize;

use crate::error::Error;
use crate::queue::{Config, Counts, DeadLetter, Delivered, Kind};
use crate::types::{
    DelaySeconds, MessageId, QueueName, RetentionSeconds, SequenceNumber, Time, VisibilityTimeout,
};

/// The attributes `GetQueueAttributes` reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Attributes {
    visibility_timeout: VisibilityTimeout,
    delay_seconds: DelaySeconds,
    retention_seconds: RetentionSeconds,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(flatten)]
    dead_letter: Option<DeadLetter>,
}

impl From<&Config> for Attributes {
    fn from(config: &Config) -> Self {
        Attributes {
            visibility_timeout: config.visibility_timeout,
            delay_seconds: config.delay,
            retention_seconds: config.retention,
            fifo: config.kind.is_fifo(),
            content_based_deduplication: matches!(
                config.kind,
                Kind::Fifo {
                    content_based_deduplication: true
                }
            ),
            dead_letter: config.dead_letter.clone(),
        }
    }
}

/// The result members of a successful command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Output {
    Done {},
    Clock {
        #[serde(rename = "Now")]
        now: Time,
    },
    Queues {
        #[serde(rename = "QueueNames")]
        queue_names: Vec<QueueName>,
    },
    QueueState {
        #[serde(rename = "Attributes")]
        attributes: Attributes,
        #[serde(rename = "Counts")]
        counts: Counts,
    },
    Sent {
        #[serde(rename = "MessageId")]
        message_id: MessageId,
        #[serde(rename = "SequenceNumber", skip_serializing_if = "Option::is_none")]
        sequence_number: Option<SequenceNumber>,
    },
    Received {
        #[serde(rename = "Messages")]
        messages: Vec<Delivered>,
    },
}

#[derive(Serialize)]
struct Success {
    ok: bool,
    #[serde(flatten)]
    output: Output,
}

#[derive(Serialize)]
struct Failure<'a> {
    ok: bool,
    error: ErrorBody<'a>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'static str,
    message: &'a str,
}

/// One response line, without its terminator.
pub fn render(result: Result<Output, Error>) -> String {
    let json = match result {
        Ok(output) => serde_json::to_string(&Success { ok: true, output }),
        Err(error) => {
            let message = error.to_string();
            serde_json::to_string(&Failure {
                ok: false,
                error: ErrorBody {
                    code: error.code(),
                    message: &message,
                },
            })
        }
    };
    json.expect("responses contain only strings, integers and booleans, which always serialize")
}
