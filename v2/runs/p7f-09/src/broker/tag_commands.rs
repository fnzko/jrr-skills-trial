//! Queue tag commands (SPEC-feature section 3).

use serde_json::json;

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_TAGS: usize = 50;
const MAX_KEY_CHARS: usize = 128;
const MAX_VALUE_BYTES: usize = 256;

fn invalid(message: String) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

fn check_key(key: &str) -> Result<()> {
    if naming::is_identifier(key, MAX_KEY_CHARS) {
        Ok(())
    } else {
        Err(invalid(format!(
            "tag key `{key}` must be 1 to {MAX_KEY_CHARS} characters from A-Z a-z 0-9 _ -"
        )))
    }
}

impl Broker {
    pub(super) fn tag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Tags"])?;
        let name = params.string("QueueName")?.expect("required");
        let tags = params.object("Tags")?.expect("required");
        let mut new_tags = Vec::new();
        for (key, value) in tags.members() {
            check_key(key)?;
            match value.as_str() {
                Some(v) if v.len() <= MAX_VALUE_BYTES => new_tags.push((key.clone(), v.to_owned())),
                _ => {
                    return Err(invalid(format!(
                        "value of tag `{key}` must be a string of at most {MAX_VALUE_BYTES} bytes"
                    )));
                }
            }
        }

        let queue = self.queue_mut(name)?;
        let added = new_tags
            .iter()
            .filter(|(key, _)| !queue.tags.contains_key(key))
            .count();
        if queue.tags.len() + added > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        queue.tags.extend(new_tags);
        Ok(Members::new())
    }

    pub(super) fn untag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "TagKeys"])?;
        let name = params.string("QueueName")?.expect("required");
        let keys = params.array("TagKeys")?.expect("required");
        let mut to_remove = Vec::new();
        for key in keys {
            let key = key
                .as_str()
                .ok_or_else(|| invalid("`TagKeys` must contain only strings".into()))?;
            check_key(key)?;
            to_remove.push(key);
        }

        let queue = self.queue_mut(name)?;
        for key in to_remove {
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
