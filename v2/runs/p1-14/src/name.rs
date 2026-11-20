use crate::error::{Error, ErrorCode};

const FIFO_SUFFIX: &str = ".fifo";

/// A queue name that follows the naming rules, as required when creating one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueName(String);

impl QueueName {
    /// Checks the name syntax: 1 to 80 characters of `A-Z a-z 0-9 _ -`, or
    /// 1 to 75 of them followed by `.fifo`.
    pub fn parse(name: String) -> Result<Self, Error> {
        let (stem, max_len) = match name.strip_suffix(FIFO_SUFFIX) {
            Some(stem) => (stem, 75),
            None => (name.as_str(), 80),
        };
        let is_name_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
        // Every accepted character is ASCII, so the byte length is the
        // character count.
        if stem.is_empty() || stem.len() > max_len || !stem.chars().all(is_name_char) {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                format!("queue name {name:?} breaks the naming rules"),
            ));
        }
        Ok(Self(name))
    }

    pub fn is_fifo(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}
