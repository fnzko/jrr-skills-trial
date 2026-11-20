//! Receipt handles and the log of every delivery made so far.

use std::fmt;

use serde::{Serialize, Serializer};

use crate::error::{Error, ErrorCode};
use crate::queue::QueueId;

/// `r-<n>`, where `n` counts deliveries made by the process from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiptHandle(u64);

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

impl Serialize for ReceiptHandle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Where each issued handle was delivered, indexed by handle number.
#[derive(Debug, Default)]
pub struct DeliveryLog {
    deliveries: Vec<Delivery>,
}

#[derive(Debug, Clone, Copy)]
pub struct Delivery {
    pub queue: QueueId,
    /// Sequence number of the message within its queue.
    pub sequence: u64,
    pub handle: ReceiptHandle,
}

impl DeliveryLog {
    pub fn issue(&mut self, queue: QueueId, sequence: u64) -> ReceiptHandle {
        let handle = ReceiptHandle(self.deliveries.len() as u64 + 1);
        self.deliveries.push(Delivery {
            queue,
            sequence,
            handle,
        });
        handle
    }

    /// Finds the delivery a handle string names.
    ///
    /// Only the exact text a delivery produced is accepted, so `r-01` and `r-1 ` are
    /// reported as never issued.
    pub fn resolve(&self, text: &str) -> Result<Delivery, Error> {
        text.strip_prefix("r-")
            .filter(|digits| !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|digits| digits.parse::<usize>().ok())
            .and_then(|number| self.deliveries.get(number - 1))
            .copied()
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleInvalid,
                    format!("receipt handle {text:?} was never issued"),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_only_handles_in_their_exact_issued_form() {
        let mut log = DeliveryLog::default();
        let first = log.issue(QueueId::new(7), 3);
        log.issue(QueueId::new(7), 4);

        let delivery = log.resolve("r-1").unwrap();
        assert_eq!((delivery.handle, delivery.sequence), (first, 3));
        assert_eq!(log.resolve("r-2").unwrap().sequence, 4);

        for text in [
            "",
            "r-",
            "r-0",
            "r-3",
            "r-01",
            "R-1",
            "r-1 ",
            " r-1",
            "r-+1",
            "r--1",
            "1",
            "r-99999999999999999999999",
        ] {
            assert_eq!(
                log.resolve(text).unwrap_err().code,
                ErrorCode::ReceiptHandleInvalid,
                "handle {text:?}"
            );
        }
    }

    #[test]
    fn handles_display_as_r_and_the_delivery_number() {
        let mut log = DeliveryLog::default();
        assert_eq!(log.issue(QueueId::new(1), 1).to_string(), "r-1");
        assert_eq!(log.issue(QueueId::new(1), 1).to_string(), "r-2");
    }
}
