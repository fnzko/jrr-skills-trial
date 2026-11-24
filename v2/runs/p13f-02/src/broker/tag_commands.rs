//! Queue tag commands (SPEC-feature section 3).

use super::{Broker, Members, members};
use crate::error::Result;
use crate::request::Params;
use crate::tags::{TagKey, Tags};

impl Broker {
    pub(super) fn tag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Tags"])?;
        let name = params
            .string("QueueName")?
            .expect("present: require() checked it above");
        let tags = Tags::parse(
            params
                .object("Tags")?
                .expect("present: require() checked it above"),
        )?;
        self.queue_mut(name)?.tags.set(tags)?;
        Ok(Members::new())
    }

    pub(super) fn untag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "TagKeys"])?;
        let name = params.string("QueueName")?.expect("required");
        let keys = TagKey::parse_all(
            params
                .array("TagKeys")?
                .expect("present: require() checked it above"),
        )?;
        self.queue_mut(name)?.tags.remove(&keys);
        Ok(Members::new())
    }

    pub(super) fn list_queue_tags(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName"])?;
        let name = params.string("QueueName")?.expect("required");
        let queue = self.queue_mut(name)?;
        Ok(members([("Tags", queue.tags.to_json())]))
    }
}
