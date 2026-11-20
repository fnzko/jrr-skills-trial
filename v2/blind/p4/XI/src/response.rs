//! Renders the outcome of a command as its one-line JSON response.

use serde_json::{Value, json};

use crate::error::CommandError;
use crate::queue::{Counts, Delivery, QueueConfig, Sent};
use crate::types::QueueName;

/// The result members of a successful command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// A command without result members.
    Done,
    Tick {
        now: u64,
    },
    QueueAttributes {
        attributes: QueueConfig,
        counts: Counts,
    },
    QueueNames(Vec<QueueName>),
    Sent(Sent),
    Messages(Vec<Delivery>),
}

/// The response line for `result`, without its trailing `\n`.
pub fn render(result: &Result<Response, CommandError>) -> String {
    let value = match result {
        Ok(response) => success(response),
        Err(error) => json!({
            "ok": false,
            "error": {"code": error.code, "message": error.message},
        }),
    };
    value.to_string()
}

fn success(response: &Response) -> Value {
    match response {
        Response::Done => json!({"ok": true}),
        Response::Tick { now } => json!({"ok": true, "Now": now}),
        Response::QueueAttributes { attributes, counts } => {
            json!({"ok": true, "Attributes": attributes, "Counts": counts})
        }
        Response::QueueNames(names) => json!({"ok": true, "QueueNames": names}),
        Response::Sent(Sent {
            message_id,
            sequence_number: None,
        }) => json!({"ok": true, "MessageId": message_id}),
        Response::Sent(Sent {
            message_id,
            sequence_number: Some(sequence_number),
        }) => json!({"ok": true, "MessageId": message_id, "SequenceNumber": sequence_number}),
        Response::Messages(messages) => json!({"ok": true, "Messages": messages}),
    }
}
