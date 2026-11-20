//! Receipt handles: one per delivery, numbered over all queues.

use std::fmt;

use crate::error::{Error, ErrorCode};
use crate::queue::QueueId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Receipt(u64);

impl fmt::Display for Receipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

/// Which message of which queue a receipt was handed out for.
#[derive(Debug, Clone, Copy)]
pub struct Issued {
    pub queue: QueueId,
    pub sequence: u64,
}

/// Every receipt ever issued, indexed by its number. Entries are never
/// dropped: a receipt that is no longer usable must still be told apart from
/// one that was never issued.
#[derive(Debug, Default)]
pub struct ReceiptLog {
    issued: Vec<Issued>,
}

impl ReceiptLog {
    pub fn issue(&mut self, queue: QueueId, sequence: u64) -> Receipt {
        self.issued.push(Issued { queue, sequence });
        let number = u64::try_from(self.issued.len()).expect("a Vec length fits in u64");
        Receipt(number)
    }

    /// Finds what `text` was issued for. Only the exact spelling that
    /// `Receipt` prints counts.
    pub fn resolve(&self, text: &str) -> Result<(Receipt, Issued), Error> {
        let number = text
            .strip_prefix("r-")
            .filter(|digits| !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|digits| digits.parse::<u64>().ok());
        number
            .and_then(|number| {
                let index = usize::try_from(number).ok()? - 1;
                Some((Receipt(number), *self.issued.get(index)?))
            })
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("{text:?} was never issued"),
                )
            })
    }
}
