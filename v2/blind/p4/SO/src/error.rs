//! Error codes of section 2 of the specification.
//!
//! Every failure a command can report is an [`Error`] carrying one of the
//! [`ErrorCode`] variants. The variant set is closed on purpose: the spec
//! says no other code is ever returned, so callers cannot invent one.

use std::fmt;

/// One of the error codes listed in section 2 of the specification.
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
    /// The code as it appears on the wire.
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

/// A failed command: a code from section 2 and a human-readable message.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_use_the_spec_spelling() {
        let all = [
            (ErrorCode::InvalidRequest, "InvalidRequest"),
            (ErrorCode::MissingParameter, "MissingParameter"),
            (ErrorCode::InvalidParameterValue, "InvalidParameterValue"),
            (ErrorCode::QueueNameInvalid, "QueueNameInvalid"),
            (ErrorCode::QueueAlreadyExists, "QueueAlreadyExists"),
            (ErrorCode::QueueDoesNotExist, "QueueDoesNotExist"),
            (ErrorCode::ReceiptHandleInvalid, "ReceiptHandleInvalid"),
            (ErrorCode::ReceiptHandleExpired, "ReceiptHandleExpired"),
            (ErrorCode::UnsupportedOperation, "UnsupportedOperation"),
            (ErrorCode::LimitExceeded, "LimitExceeded"),
        ];
        for (code, text) in all {
            assert_eq!(code.as_str(), text);
        }
    }

    #[test]
    fn display_names_the_code_and_message() {
        let err = Error::new(ErrorCode::QueueDoesNotExist, "no queue named jobs");
        assert_eq!(err.to_string(), "QueueDoesNotExist: no queue named jobs");
        assert_eq!(err.code(), ErrorCode::QueueDoesNotExist);
        assert_eq!(err.message(), "no queue named jobs");
    }
}
