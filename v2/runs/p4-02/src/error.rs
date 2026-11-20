//! Error codes and the error type returned by every failing command.

use std::fmt;

/// The closed set of error codes miniq may report (SPEC section 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "InvalidRequest",
            Self::MissingParameter => "MissingParameter",
            Self::InvalidParameterValue => "InvalidParameterValue",
            Self::QueueNameInvalid => "QueueNameInvalid",
            Self::QueueAlreadyExists => "QueueAlreadyExists",
            Self::QueueDoesNotExist => "QueueDoesNotExist",
            Self::ReceiptHandleInvalid => "ReceiptHandleInvalid",
            Self::ReceiptHandleExpired => "ReceiptHandleExpired",
            Self::UnsupportedOperation => "UnsupportedOperation",
            Self::LimitExceeded => "LimitExceeded",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A failed command: a machine-readable code plus a message for humans.
#[derive(Debug, Clone, PartialEq, Eq)]
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

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRequest, message)
    }

    pub fn missing(parameter: &str) -> Self {
        Self::new(
            ErrorCode::MissingParameter,
            format!("missing required parameter {parameter}"),
        )
    }

    pub fn invalid_value(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidParameterValue, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::UnsupportedOperation, message)
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
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T, E = Error> = std::result::Result<T, E>;
