use crate::error::{Error, ErrorCode};
use crate::types::{Arrival, QueueId, ReceiptHandle};

#[derive(Debug, Clone, Copy)]
struct Delivery {
    queue: QueueId,
    arrival: Arrival,
}

/// Every delivery the process has made, in order. The receipt handle `r-<n>`
/// is the position of its delivery plus one, so the log is also the set of
/// handles ever issued.
#[derive(Debug, Default)]
pub struct Deliveries {
    issued: Vec<Delivery>,
}

impl Deliveries {
    pub fn issue(&mut self, queue: QueueId, arrival: Arrival) -> ReceiptHandle {
        let handle = ReceiptHandle::at_index(self.issued.len());
        self.issued.push(Delivery { queue, arrival });
        handle
    }

    /// Finds the message a handle was issued for. Fails with
    /// `ReceiptHandleInvalid` if the handle was never issued and with
    /// `ReceiptHandleExpired` if it was issued for a message of another
    /// queue. Whether the message still honours the handle is for the queue
    /// to decide.
    pub fn resolve(&self, queue: QueueId, text: &str) -> Result<(ReceiptHandle, Arrival), Error> {
        let never_issued = || {
            Error::new(
                ErrorCode::ReceiptHandleInvalid,
                format!("receipt handle {text:?} was never issued"),
            )
        };
        let handle = ReceiptHandle::parse(text).ok_or_else(never_issued)?;
        let delivery = handle
            .index()
            .and_then(|index| self.issued.get(index))
            .ok_or_else(never_issued)?;
        if delivery.queue != queue {
            return Err(Error::new(
                ErrorCode::ReceiptHandleExpired,
                format!("receipt handle {text:?} belongs to another queue"),
            ));
        }
        Ok((handle, delivery.arrival))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_resolve_to_their_queue_and_arrival() {
        let mut deliveries = Deliveries::default();
        let queue = QueueId::FIRST;
        let other = queue.succ();
        let first = deliveries.issue(queue, Arrival::FIRST);
        let second = deliveries.issue(other, Arrival::FIRST.succ());

        assert_eq!(first.to_string(), "r-1");
        assert_eq!(
            deliveries.resolve(queue, "r-1").unwrap(),
            (first, Arrival::FIRST)
        );
        assert_eq!(
            deliveries.resolve(other, "r-2").unwrap(),
            (second, Arrival::FIRST.succ())
        );
    }

    #[test]
    fn unissued_handles_are_invalid_and_foreign_ones_expired() {
        let mut deliveries = Deliveries::default();
        deliveries.issue(QueueId::FIRST, Arrival::FIRST);

        let code = |text| {
            deliveries
                .resolve(QueueId::FIRST, text)
                .map_err(|e| e.code())
        };
        assert_eq!(code("r-2"), Err(ErrorCode::ReceiptHandleInvalid));
        assert_eq!(code("r-01"), Err(ErrorCode::ReceiptHandleInvalid));
        assert_eq!(code("R-1"), Err(ErrorCode::ReceiptHandleInvalid));
        assert_eq!(code("r-1 "), Err(ErrorCode::ReceiptHandleInvalid));
        assert_eq!(
            deliveries
                .resolve(QueueId::FIRST.succ(), "r-1")
                .map_err(|e| e.code()),
            Err(ErrorCode::ReceiptHandleExpired)
        );
    }
}
