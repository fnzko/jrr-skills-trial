//! Queue tag commands (SPEC-feature section 3).

use serde_json::Value;

use super::queue_commands::queue_name;
use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;
use crate::tags::{self, Tags};

impl Broker {
    pub(super) fn tag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Tags"])?;
        let name = params.string("QueueName")?.expect("required");
        let pairs = Tags::parse(params, "Tags")?;
        self.queue_mut(name)?.tags.set(&pairs)?;
        Ok(Members::new())
    }

    pub(super) fn untag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "TagKeys"])?;
        let name = params.string("QueueName")?.expect("required");
        let keys = params
            .array("TagKeys")?
            .expect("required")
            .iter()
            .map(|key| {
                let key = key.as_str().ok_or_else(|| {
                    Error::new(
                        ErrorCode::InvalidParameterValue,
                        "elements of `TagKeys` must be strings",
                    )
                })?;
                tags::check_key(key)?;
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
        let name = queue_name(params)?;
        let tags: Value = self.queue_mut(name)?.tags.to_json();
        Ok(members([("Tags", tags)]))
    }
}
