use crate::types::{MessageId, QueueUid, ReceiptHandle};

/// The message a receipt handle was issued for.
#[derive(Debug, Clone, Copy)]
pub struct HandleEntry {
    pub queue: QueueUid,
    pub message: MessageId,
}

/// Every receipt handle the process has issued. Handle `r-<n>` is entry `n - 1`.
#[derive(Default)]
pub struct HandleLog {
    entries: Vec<HandleEntry>,
}

impl HandleLog {
    pub fn issue(&mut self, queue: QueueUid, message: MessageId) -> ReceiptHandle {
        self.entries.push(HandleEntry { queue, message });
        let issued = u64::try_from(self.entries.len()).expect("a usize fits in a u64");
        ReceiptHandle(issued)
    }

    /// Finds what `text` was issued for. Only the exact canonical spelling
    /// matches: no sign, no leading zeros, no surrounding text.
    pub fn resolve(&self, text: &str) -> Option<(ReceiptHandle, HandleEntry)> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number: u64 = digits.parse().ok()?;
        let index = usize::try_from(number.checked_sub(1)?).ok()?;
        let entry = *self.entries.get(index)?;
        Some((ReceiptHandle(number), entry))
    }
}
