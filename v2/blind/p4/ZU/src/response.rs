//! The success results of commands and how a response line is written.

use serde::Serialize;

use crate::error::ApiError;
use crate::queue::{Attributes, Counts, Delivery, Sent};
use crate::types::{QueueName, Time};

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Output {
    Empty {},
    #[serde(rename_all = "PascalCase")]
    Now {
        now: Time,
    },
    #[serde(rename_all = "PascalCase")]
    QueueNames {
        queue_names: Vec<QueueName>,
    },
    #[serde(rename_all = "PascalCase")]
    QueueAttributes {
        attributes: Attributes,
        counts: Counts,
    },
    Sent(Sent),
    #[serde(rename_all = "PascalCase")]
    Messages {
        messages: Vec<Delivery>,
    },
}

#[derive(Serialize)]
struct Success<'a> {
    ok: bool,
    #[serde(flatten)]
    output: &'a Output,
}

#[derive(Serialize)]
struct Failure<'a> {
    ok: bool,
    error: &'a ApiError,
}

/// The response line for `result`, without the trailing newline.
pub fn render(result: &Result<Output, ApiError>) -> String {
    let line = match result {
        Ok(output) => serde_json::to_string(&Success { ok: true, output }),
        Err(error) => serde_json::to_string(&Failure { ok: false, error }),
    };
    line.expect("responses hold only strings, integers, booleans and string-keyed objects")
}
