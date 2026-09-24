use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::ThingsId;

/// tombstone properties that mark a deleted object
///
/// the deletion time in `dld` is left unread
/// a tombstone purges what it names whatever that field holds
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TombstoneProps {
    /// `dloid`, the id of the deleted object
    #[serde(rename = "dloid")]
    pub deleted_object_id: ThingsId,
}

/// one-shot command properties
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandProps {
    /// `tp`, command type
    #[serde(rename = "tp", default)]
    pub command_type: i32,

    /// `cd`, creation timestamp
    #[serde(rename = "cd", default)]
    pub creation_date: Option<i64>,

    /// `if`, initial field payload for command execution
    #[serde(rename = "if", default)]
    pub initial_fields: Option<BTreeMap<String, Value>>,
}
