//! Queue name syntax (SPEC section 4).
//!
//! A name is 1 to 80 characters from `A-Z a-z 0-9 _ -`, or 1 to 75 such
//! characters followed by `.fifo`. The `.fifo` suffix is the only place a
//! dot may appear, so a name ending in `.fifo` is always a FIFO name.

pub const FIFO_SUFFIX: &str = ".fifo";
const MAX_NAME_LEN: usize = 80;
const MAX_FIFO_BASE_LEN: usize = 75;

pub fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Whether `name` breaks the syntax rules. Only the syntax is checked here:
/// whether the name is already taken is a separate concern.
pub fn is_valid_queue_name(name: &str) -> bool {
    match name.strip_suffix(FIFO_SUFFIX) {
        Some(base) => is_base(base, MAX_FIFO_BASE_LEN),
        None => is_base(name, MAX_NAME_LEN),
    }
}

/// Whether a name ends in the FIFO suffix, which the spec ties to `Fifo`.
pub fn has_fifo_suffix(name: &str) -> bool {
    name.ends_with(FIFO_SUFFIX)
}

fn is_base(base: &str, max_len: usize) -> bool {
    !base.is_empty() && base.len() <= max_len && base.chars().all(is_name_char)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_names() {
        assert!(is_valid_queue_name("jobs"));
        assert!(is_valid_queue_name("A_b-9"));
        assert!(is_valid_queue_name(&"a".repeat(80)));
        assert!(!is_valid_queue_name(&"a".repeat(81)));
        assert!(!is_valid_queue_name(""));
        assert!(!is_valid_queue_name("a.b"));
        assert!(!is_valid_queue_name("a b"));
        assert!(!is_valid_queue_name("café"));
    }

    #[test]
    fn fifo_names() {
        assert!(is_valid_queue_name("jobs.fifo"));
        assert!(is_valid_queue_name(&format!("{}.fifo", "a".repeat(75))));
        assert!(!is_valid_queue_name(&format!("{}.fifo", "a".repeat(76))));
        assert!(!is_valid_queue_name(".fifo"));
        assert!(!is_valid_queue_name("a.b.fifo"));
        assert!(!is_valid_queue_name("a.fifo.fifo"));
        assert!(has_fifo_suffix("x.fifo"));
        assert!(!has_fifo_suffix("x.fifox"));
    }
}
