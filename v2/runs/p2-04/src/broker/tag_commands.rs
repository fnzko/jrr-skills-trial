//! Queue tag commands (SPEC-feature section 3).

use serde_json::{Value, json};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_TAGS: usize = 50;
const MAX_KEY_CHARS: usize = 128;
const MAX_VALUE_BYTES: usize = 256;

fn invalid_value(message: String) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

fn check_key(key: &str) -> Result<()> {
    if naming::is_identifier(key, MAX_KEY_CHARS) {
        Ok(())
    } else {
        Err(invalid_value(format!(
            "tag key `{key}` must be 1 to {MAX_KEY_CHARS} characters from A-Z a-z 0-9 _ -"
        )))
    }
}

fn tag_value(key: &str, value: &Value) -> Result<String> {
    match value.as_str() {
        Some(s) if s.len() <= MAX_VALUE_BYTES => Ok(s.to_owned()),
        _ => Err(invalid_value(format!(
            "value of tag `{key}` must be a string of at most {MAX_VALUE_BYTES} bytes"
        ))),
    }
}

impl Broker {
    pub(super) fn tag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Tags"])?;
        let name = params.string("QueueName")?.expect("required");
        let given = params.object("Tags")?.expect("required");
        let mut tags = Vec::new();
        for (key, value) in given.members() {
            check_key(key)?;
            tags.push((key, tag_value(key, value)?));
        }

        let queue = self.queue_mut(name)?;
        let new_keys = tags
            .iter()
            .filter(|(key, _)| !queue.tags.contains_key(*key))
            .count();
        if queue.tags.len() + new_keys > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        for (key, value) in tags {
            queue.tags.insert(key.to_owned(), value);
        }
        Ok(Members::new())
    }

    pub(super) fn untag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "TagKeys"])?;
        let name = params.string("QueueName")?.expect("required");
        let keys = params.array("TagKeys")?.expect("required");
        let keys = keys
            .iter()
            .map(|key| {
                let key = key
                    .as_str()
                    .ok_or_else(|| invalid_value("tag keys must be strings".into()))?;
                check_key(key)?;
                Ok(key)
            })
            .collect::<Result<Vec<_>>>()?;

        let queue = self.queue_mut(name)?;
        for key in keys {
            queue.tags.remove(key);
        }
        Ok(Members::new())
    }

    pub(super) fn list_queue_tags(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName"])?;
        let name = params.string("QueueName")?.expect("required");
        let queue = self.queue_mut(name)?;
        Ok(members([("Tags", json!(queue.tags))]))
    }
}
