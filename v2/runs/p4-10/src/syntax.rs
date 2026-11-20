//! Lexical rules of section 4 (queue names) and section 6 (`GroupId` and
//! `DeduplicationId`).
//!
//! Both kinds of name are made of ASCII letters, digits, `_` and `-`. They
//! differ only in length and in the `.fifo` suffix that a FIFO queue name
//! carries.

const FIFO_SUFFIX: &str = ".fifo";
const MAX_QUEUE_NAME_LEN: usize = 80;
const MAX_FIFO_QUEUE_NAME_BASE_LEN: usize = 75;

/// Maximum length of a `GroupId` or `DeduplicationId`.
pub const MAX_ID_LEN: usize = 128;

/// Whether `name` follows the queue naming rules of section 4: 1 to 80
/// characters from the name alphabet, or 1 to 75 of them followed by
/// `.fifo`.
pub fn is_valid_queue_name(name: &str) -> bool {
    let (base, max_base) = match name.strip_suffix(FIFO_SUFFIX) {
        Some(base) => (base, MAX_FIFO_QUEUE_NAME_BASE_LEN),
        None => (name, MAX_QUEUE_NAME_LEN),
    };
    !base.is_empty() && base.len() <= max_base && base.bytes().all(is_name_byte)
}

/// Whether `name` ends in the `.fifo` suffix.
pub fn has_fifo_suffix(name: &str) -> bool {
    name.ends_with(FIFO_SUFFIX)
}

/// Whether `id` is a valid `GroupId` or `DeduplicationId`: 1 to 128
/// characters from the name alphabet.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_LEN && id.bytes().all(is_name_byte)
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names_accept_the_alphabet_within_length() {
        assert!(is_valid_queue_name("jobs"));
        assert!(is_valid_queue_name("A-z_09"));
        assert!(is_valid_queue_name(&"a".repeat(80)));
        assert!(!is_valid_queue_name(&"a".repeat(81)));
    }

    #[test]
    fn queue_names_reject_empty_and_foreign_characters() {
        assert!(!is_valid_queue_name(""));
        assert!(!is_valid_queue_name("a b"));
        assert!(!is_valid_queue_name("a.b"));
        assert!(!is_valid_queue_name("café"));
        assert!(!is_valid_queue_name("a/b"));
    }

    #[test]
    fn fifo_queue_names_have_a_shorter_base() {
        let base75 = "a".repeat(75);
        assert!(is_valid_queue_name(&format!("{base75}.fifo")));
        assert!(!is_valid_queue_name(&format!("{}.fifo", "a".repeat(76))));
        assert!(is_valid_queue_name("x.fifo"));
    }

    #[test]
    fn fifo_suffix_alone_is_not_a_name() {
        assert!(!is_valid_queue_name(".fifo"));
        assert!(!is_valid_queue_name("a.fifo.fifo.x"));
    }

    #[test]
    fn fifo_suffix_is_detected_on_the_whole_name() {
        assert!(has_fifo_suffix("orders.fifo"));
        assert!(!has_fifo_suffix("orders"));
        assert!(!has_fifo_suffix("orders.fifo2"));
    }

    #[test]
    fn ids_are_one_to_128_name_characters() {
        assert!(is_valid_id("g-1"));
        assert!(is_valid_id(&"x".repeat(128)));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id(&"x".repeat(129)));
        assert!(!is_valid_id("a.b"));
        assert!(!is_valid_id("a b"));
    }
}
