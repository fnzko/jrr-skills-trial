//! The error codes of SPEC section 2. Nothing else is ever sent to a client,
//! so the enum is closed and each variant maps to exactly one wire string.

use std::fmt;

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
    /// The exact string that appears in the `code` member of a failure response.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidRequest => "InvalidRequest",
            ErrorCode::MissingParameter => "MissingParameter",
            ErrorCode::InvalidParameterValue => "InvalidParameterValue",
            ErrorCode::QueueNameInvalid => "QueueNameInvalid",
            ErrorCode::QueueAlreadyExists => "QueueAlreadyExists",
            ErrorCode::QueueDoesNotExist => "QueueDoesNotExist",
            ErrorCode::ReceiptHandleInvalid => "ReceiptHandleInvalid",
            ErrorCode::ReceiptHandleExpired => "ReceiptHandleExpired",
            ErrorCode::UnsupportedOperation => "UnsupportedOperation",
            ErrorCode::LimitExceeded => "LimitExceeded",
        }
    }
}

/// A command that failed. A failed command changes no state, so callers can
/// simply propagate this with `?` before mutating anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Error {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
