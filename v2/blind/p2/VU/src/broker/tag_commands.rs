//! Queue tag commands (feature spec section 3).

use std::collections::BTreeMap;

use serde_json::json;

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming::{self, MAX_TAG_KEY_CHARS};
use crate::request::{Params, invalid_value};

const MAX_TAGS: usize = 50;
const MAX_TAG_VALUE_BYTES: usize = 256;

fn check_key(key: &str) -> Result<()> {
    if naming::is_identifier(key, MAX_TAG_KEY_CHARS) {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::InvalidParameterValue,
            format!(
                "tag key `{key}` must be 1 to {MAX_TAG_KEY_CHARS} characters from A-Z a-z 0-9 _ -"
            ),
        ))
    }
}

/// Validates the `Tags` object of `TagQueue`: every key and every value.
fn parse_tags<'a>(params: Params<'a>) -> Result<BTreeMap<&'a str, &'a str>> {
    let tags = params.object("Tags")?.expect("required");
    let mut parsed = BTreeMap::new();
    for (key, value) in tags.members() {
        check_key(key)?;
        let value = value
            .as_str()
            .filter(|v| v.len() <= MAX_TAG_VALUE_BYTES)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidParameterValue,
                    format!("value of tag `{key}` must be a string of at most {MAX_TAG_VALUE_BYTES} bytes"),
                )
            })?;
        parsed.insert(key.as_str(), value);
    }
    Ok(parsed)
}

impl Broker {
    pub(super) fn tag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Tags"])?;
        let name = params.string("QueueName")?.expect("required");
        let tags = parse_tags(params)?;
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
        for (key, value) in tags {
            queue.tags.insert(key.to_owned(), value.to_owned());
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
                    .ok_or_else(|| invalid_value("a tag key", "a string"))?;
                check_key(key).map(|()| key)
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
