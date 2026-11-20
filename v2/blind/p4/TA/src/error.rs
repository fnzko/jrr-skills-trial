use std::fmt;

/// The error codes a failed command can report (SPEC.md section 2).
///
/// Only these codes ever reach a client, so they are an enum rather than
/// free-form strings.
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
    /// The name sent on the wire.
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

/// A failed command: a code from the fixed list plus a human-readable message.
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
