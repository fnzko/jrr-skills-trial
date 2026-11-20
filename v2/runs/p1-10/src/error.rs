//! Failure responses: the closed set of error codes and the error carried to the wire.

use std::fmt;

use serde::Serialize;

/// Every code a failed command can report. No other code is ever returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ErrorCode {
    InvalidRequest,
    MissingParameter,
    InvalidParameterValue,
    QueueNameInvalid,
    QueueAlreadyExists,
    QueueDoesNotExist,
    ReceiptHandleInvalid,
    ReceiptHandleExpired,
    UnsupportedOperation,
    LimitExceeded,
}

/// A failed command: the code decides the client's reaction, the message is for people.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Error {
    code: ErrorCode,
    message: String,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRequest, message)
    }

    pub fn missing(parameter: &str) -> Self {
        Self::new(
            ErrorCode::MissingParameter,
            format!("required parameter {parameter} is absent"),
        )
    }

    pub fn invalid_value(parameter: &str, reason: impl fmt::Display) -> Self {
        Self::new(
            ErrorCode::InvalidParameterValue,
            format!("parameter {parameter} is not allowed: {reason}"),
        )
    }

    pub fn no_such_queue(name: &str) -> Self {
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
