//! Error codes and the error value carried by failure responses.

use std::fmt;

use serde::Serialize;

/// The closed set of error codes miniq ever returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ErrorCode {
    /// The line is not a JSON object with a known string `Command`.
    InvalidRequest,
    /// A required parameter is absent.
    MissingParameter,
    /// A parameter or attribute has the wrong type or an unacceptable value.
    InvalidParameterValue,
    /// `CreateQueue` was given a name that breaks the naming rules.
    QueueNameInvalid,
    /// `CreateQueue` named an existing queue with different attributes.
    QueueAlreadyExists,
    /// The named queue, or dead-letter queue, does not exist.
    QueueDoesNotExist,
    /// The receipt handle was never issued.
    ReceiptHandleInvalid,
    /// The receipt handle was issued but cannot be used any more.
    ReceiptHandleExpired,
    /// The queue type does not support the request.
    UnsupportedOperation,
    /// The queue or message limit is reached.
    LimitExceeded,
}

/// A failed command: a code from the specification and a readable message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Error {
    code: ErrorCode,
    message: String,
}

impl Error {
    /// Builds an error with the given code.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// The code reported to the client.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub(crate) fn missing(parameter: &str) -> Self {
        Self::new(
            ErrorCode::MissingParameter,
            format!("parameter {parameter} is required"),
        )
    }

    pub(crate) fn invalid(parameter: &str, reason: impl fmt::Display) -> Self {
        Self::new(
            ErrorCode::InvalidParameterValue,
            format!("parameter {parameter} {reason}"),
        )
    }

    pub(crate) fn no_queue(name: &str) -> Self {
        Self::new(
            ErrorCode::QueueDoesNotExist,
            format!("queue {name:?} does not exist"),
        )
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}
