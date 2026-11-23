//! Receipt handles issued so far and the message each one was issued for.

use crate::types::{QueueId, ReceiptHandle, Slot};

/// The queue incarnation and slot a handle was issued for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandleOwner {
    pub queue: QueueId,
    pub slot: Slot,
}

#[derive(Debug, Default)]
pub struct HandleLog {
    owners: Vec<HandleOwner>,
}

impl HandleLog {
    pub fn issue(&mut self, owner: HandleOwner) -> ReceiptHandle {
        self.owners.push(owner);
        ReceiptHandle::from_issue_number(self.owners.len() as u64)
    }

    /// The owner of `handle`, or `None` if this process never issued it.
    pub fn owner(&self, handle: ReceiptHandle) -> Option<HandleOwner> {
        let index = usize::try_from(handle.issue_number().checked_sub(1)?).ok()?;
        self.owners.get(index).copied()
    }
}
