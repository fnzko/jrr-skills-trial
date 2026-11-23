//! Character and name rules shared by queue names and message identifiers.

const FIFO_SUFFIX: &str = ".fifo";
const MAX_QUEUE_NAME_LEN: usize = 80;
const MAX_TAG_KEY_LEN: usize = 128;

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// True if `s` is made of 1 to `max_len` characters from `A-Z a-z 0-9 _ -`.
pub fn is_identifier(s: &str, max_len: usize) -> bool {
    (1..=max_len).contains(&s.chars().count()) && s.chars().all(is_name_char)
}

/// True if `s` is a valid queue tag key.
pub fn is_tag_key(s: &str) -> bool {
    is_identifier(s, MAX_TAG_KEY_LEN)
}

/// True if `name` follows the queue name syntax (SPEC section 4).
pub fn is_valid_queue_name(name: &str) -> bool {
    match name.strip_suffix(FIFO_SUFFIX) {
        Some(stem) => is_identifier(stem, MAX_QUEUE_NAME_LEN - FIFO_SUFFIX.len()),
        None => is_identifier(name, MAX_QUEUE_NAME_LEN),
    }
}

/// True if the name denotes a FIFO queue by its suffix.
pub fn has_fifo_suffix(name: &str) -> bool {
    name.ends_with(FIFO_SUFFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names() {
        assert!(is_valid_queue_name("jobs"));
        assert!(is_valid_queue_name("a-b_C9"));
        assert!(is_valid_queue_name("jobs.fifo"));
        assert!(is_valid_queue_name(&"a".repeat(80)));
        assert!(is_valid_queue_name(&format!("{}.fifo", "a".repeat(75))));
        assert!(!is_valid_queue_name(""));
        assert!(!is_valid_queue_name(".fifo"));
        assert!(!is_valid_queue_name("a.b"));
        assert!(!is_valid_queue_name("a.fifo.fifo"));
        assert!(!is_valid_queue_name(&"a".repeat(81)));
        assert!(!is_valid_queue_name(&format!("{}.fifo", "a".repeat(76))));
        assert!(!is_valid_queue_name("jöbs"));
        assert!(!is_valid_queue_name("a b"));
    }

    #[test]
    fn identifiers_count_characters_not_bytes() {
        assert!(is_identifier(&"x".repeat(128), 128));
        assert!(!is_identifier(&"x".repeat(129), 128));
        assert!(!is_identifier("é", 128));
    }
}
