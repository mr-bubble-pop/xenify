use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable identifier for compositions and layers, unique within a [`crate::Project`].
///
/// Ids are small integers handed out by [`crate::Project::alloc_id`] so they stay
/// readable in saved files and in AI tool calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id(pub u64);

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}
