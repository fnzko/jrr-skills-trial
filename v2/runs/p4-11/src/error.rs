//! Failure responses. The codes are the closed set of section 2 of SPEC.md.

use std::fmt;

/// The error code sent to the client. Every failure is one of these, so the
/// wire vocabulary cannot drift from the specification.
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
    /// The name written to the `code` member of a failure response.
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

/// A command that failed. Carries the code the client sees and a message
/// for people reading the log; the message text is not part of the contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    code: ErrorCode,
    message: String,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Error {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for Error {}

/// Shorthand for the result of executing or parsing one command.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_spelled_as_in_the_specification() {
        let spelled: Vec<&str> = [
            ErrorCode::InvalidRequest,
            ErrorCode::MissingParameter,
            ErrorCode::InvalidParameterValue,
            ErrorCode::QueueNameInvalid,
            ErrorCode::QueueAlreadyExists,
            ErrorCode::QueueDoesNotExist,
            ErrorCode::ReceiptHandleInvalid,
            ErrorCode::ReceiptHandleExpired,
            ErrorCode::UnsupportedOperation,
            ErrorCode::LimitExceeded,
        ]
        .into_iter()
        .map(ErrorCode::as_str)
        .collect();
        assert_eq!(
            spelled,
            [
                "InvalidRequest",
                "MissingParameter",
                "InvalidParameterValue",
                "QueueNameInvalid",
                "QueueAlreadyExists",
                "QueueDoesNotExist",
                "ReceiptHandleInvalid",
                "ReceiptHandleExpired",
                "UnsupportedOperation",
                "LimitExceeded",
            ]
        );
    }
}
