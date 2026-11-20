use std::fmt;

use serde::Serialize;

/// The error codes of the protocol. No other code is ever sent.
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

/// A failed command: the protocol code plus a message for people.
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

    #[cfg(test)]
    pub fn code(&self) -> ErrorCode {
        self.code
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_code_and_message_only() {
        let error = Error::new(ErrorCode::QueueDoesNotExist, "no queue named jobs");
        let json = serde_json::to_value(&error).expect("plain strings always serialize");
        assert_eq!(
            json,
            serde_json::json!({"code": "QueueDoesNotExist", "message": "no queue named jobs"})
        );
    }

    #[test]
    fn display_names_code_and_message() {
        let error = Error::new(ErrorCode::LimitExceeded, "queue is full");
        assert_eq!(error.to_string(), "LimitExceeded: queue is full");
    }
}
