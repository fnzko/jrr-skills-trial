use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
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
    EmptyBatchRequest,
    TooManyEntriesInBatchRequest,
    InvalidBatchEntryId,
    BatchEntryIdsNotDistinct,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::InvalidRequest => "InvalidRequest",
            Code::MissingParameter => "MissingParameter",
            Code::InvalidParameterValue => "InvalidParameterValue",
            Code::QueueNameInvalid => "QueueNameInvalid",
            Code::QueueAlreadyExists => "QueueAlreadyExists",
            Code::QueueDoesNotExist => "QueueDoesNotExist",
            Code::ReceiptHandleInvalid => "ReceiptHandleInvalid",
            Code::ReceiptHandleExpired => "ReceiptHandleExpired",
            Code::UnsupportedOperation => "UnsupportedOperation",
            Code::LimitExceeded => "LimitExceeded",
            Code::EmptyBatchRequest => "EmptyBatchRequest",
            Code::TooManyEntriesInBatchRequest => "TooManyEntriesInBatchRequest",
            Code::InvalidBatchEntryId => "InvalidBatchEntryId",
            Code::BatchEntryIdsNotDistinct => "BatchEntryIdsNotDistinct",
        }
    }
}

#[derive(Debug)]
pub struct Failure {
    code: Code,
    message: String,
}

impl Failure {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn invalid_value(message: impl Into<String>) -> Self {
        Self::new(Code::InvalidParameterValue, message)
    }

    pub fn missing(name: &str) -> Self {
        Self::new(Code::MissingParameter, format!("{name} is required"))
    }

    pub fn to_json(&self) -> Value {
        json!({"code": self.code.as_str(), "message": self.message})
    }
}
