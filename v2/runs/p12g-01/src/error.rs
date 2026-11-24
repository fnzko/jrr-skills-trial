//! Protocol errors: a closed set of codes plus a human-readable message.

use std::fmt;

use serde::Serialize;

/// Every error code miniq may return; no other code is ever sent.
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

/// A failed command. Serializes as the `error` member of a failure response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}
