//! Queue tag commands (SPEC-feature section 3).

use std::collections::BTreeMap;

use serde_json::json;

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
            "tag key must be 1 to {MAX_KEY_CHARS} characters from A-Z a-z 0-9 _ -"
        )))
    }
}

/// Reads the `Tags` object of `TagQueue`, checking every key and value.
fn parse_tags(tags: Params<'_>) -> Result<BTreeMap<&str, &str>> {
    tags.members()
        .map(|(key, value)| {
            check_key(key)?;
            let value = value
                .as_str()
                .filter(|v| v.len() <= MAX_VALUE_BYTES)
                .ok_or_else(|| {
                    invalid_value(format!(
                        "value of tag `{key}` must be a string of at most {MAX_VALUE_BYTES} bytes"
                    ))
                })?;
            Ok((key, value))
        })
        .collect()
}

impl Broker {
    pub(super) fn tag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Tags"])?;
        let name = params.string("QueueName")?.expect("required");
        let tags = parse_tags(params.object("Tags")?.expect("required"))?;

        let queue = self.queue_mut(name)?;
        let new_keys = tags
            .keys()
            .filter(|key| !queue.tags.contains_key(**key))
            .count();
        if queue.tags.len() + new_keys > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        queue.tags.extend(
            tags.into_iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned())),
        );
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
                    .ok_or_else(|| invalid_value("`TagKeys` must hold strings".to_owned()))?;
                check_key(key)?;
                Ok(key)
            })
            .collect::<Result<Vec<&str>>>()?;

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
