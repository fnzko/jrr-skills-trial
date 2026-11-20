//! Queue tag commands (feature spec section 3).

use super::queue_commands::queue_name;
use super::{Broker, Members, members};
use crate::error::Result;
use crate::request::Params;
use crate::tags::{TagKey, parse_tag_object};

impl Broker {
    pub(super) fn tag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Tags"])?;
        let name = params.string("QueueName")?.expect("required");
        let tags = parse_tag_object(params.object_members("Tags")?.expect("required"))?;
        self.queue_mut(name)?.tags.set(tags)?;
        Ok(Members::new())
    }

    pub(super) fn untag_queue(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "TagKeys"])?;
        let name = params.string("QueueName")?.expect("required");
        let keys = params
            .array("TagKeys")?
            .expect("required")
            .iter()
            .map(TagKey::from_json)
            .collect::<Result<Vec<_>>>()?;
        let tags = &mut self.queue_mut(name)?.tags;
        for key in &keys {
            tags.remove(key);
        }
        Ok(Members::new())
    }

    pub(super) fn list_queue_tags(&mut self, params: Params<'_>) -> Result<Members> {
        let name = queue_name(params)?;
        let tags = self.queue_mut(name)?.tags.to_json();
        Ok(members([("Tags", tags)]))
    }
}
