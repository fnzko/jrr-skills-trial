//! The record of every receipt handle ever issued.

use crate::error::{Error, ErrorCode};
use crate::types::{ArrivalKey, QueueId, ReceiptHandle};

/// Where an issued handle was delivered.
#[derive(Debug, Clone, Copy)]
pub struct Issued {
    pub handle: ReceiptHandle,
    pub queue: QueueId,
    pub key: ArrivalKey,
}

/// Handles are numbered from 1 in delivery order, so the `n`th is stored at `n - 1`.
#[derive(Debug, Default)]
pub struct HandleLog {
    issued: Vec<(QueueId, ArrivalKey)>,
}

impl HandleLog {
    pub fn issue(&mut self, queue: QueueId, key: ArrivalKey) -> ReceiptHandle {
        self.issued.push((queue, key));
        let number = u64::try_from(self.issued.len()).expect("a usize count fits in u64");
        ReceiptHandle::new(number)
    }

    /// Finds the delivery a handle string names, comparing it as an exact string.
    pub fn resolve(&self, text: &str) -> Result<Issued, Error> {
        let number = text
            .strip_prefix("r-")
            .filter(|digits| !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|digits| digits.parse::<u64>().ok());
        let found = number.and_then(|number| {
            let index = usize::try_from(number.checked_sub(1)?).ok()?;
            let (queue, key) = *self.issued.get(index)?;
            Some(Issued {
                handle: ReceiptHandle::new(number),
                queue,
                key,
            })
        });
        found.ok_or_else(|| {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("receipt handle {text:?} was never issued"),
            )
        })
    }
}
