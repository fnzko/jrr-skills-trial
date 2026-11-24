//! Queue tag commands (SPEC-feature section 3).

use serde_json::{Map, Value};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;
use crate::tags::{MAX_TAGS, parse_tag_keys, parse_tags};

impl Broker {
    pub(super) fn tag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Tags"])?;
        let name = params.required_string("QueueName")?;
        let tags = parse_tags(params)?;
        let queue = self.queue_mut(name)?;

        let new_keys = tags
            .iter()
            .filter(|(key, _)| !queue.tags.contains_key(key))
            .count();
        if queue.tags.len() + new_keys > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        queue.tags.extend(tags);
        Ok(Members::new())
    }

    pub(super) fn untag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "TagKeys"])?;
        let name = params.required_string("QueueName")?;
        let keys = parse_tag_keys(params)?;
        let queue = self.queue_mut(name)?;
        for key in &keys {
            queue.tags.remove(key);
        }
        Ok(Members::new())
    }

    pub(super) fn list_queue_tags(&mut self, params: Params<'_>) -> Result<Members> {
        let name = params.required_string("QueueName")?;
        let queue = self.queue_mut(name)?;
        let tags: Map<String, Value> = queue
            .tags
            .iter()
            .map(|(key, value)| (key.as_str().to_owned(), Value::from(value.as_str())))
            .collect();
        Ok(members([("Tags", Value::Object(tags))]))
    }
}
