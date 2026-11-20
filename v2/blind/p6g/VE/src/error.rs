use serde::Serialize;

/// Every failure the protocol can report; no other code is ever sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum ErrorCode {
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

/// A failed command, serialized as the `error` member of a failure response.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Error {
    pub code: ErrorCode,
    pub message: String,
}

impl Error {
    pub(crate) fn new(code: ErrorCode, message: &str) -> Self {
        Self {
            code,
            message: message.to_owned(),
        }
    }
}
