use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ids::ThingsId,
    wire::{
        deserialize_default_on_null, deserialize_optional_field, deserialize_vec_or_single,
        task::TaskStatus,
    },
};

fn is_false(v: &bool) -> bool {
    !*v
}

/// checklist item wire properties
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ChecklistItemProps {
    /// `tt`, checklist item title
    #[serde(rename = "tt", default)]
    pub title: String,

    /// `ss`, checklist item status
    #[serde(rename = "ss", default)]
    pub status: TaskStatus,

    /// `sp`, completion/cancellation timestamp
    #[serde(rename = "sp", default, skip_serializing_if = "Option::is_none")]
    pub stop_date: Option<f64>,

    /// `ts`, parent task ids (normally a single task UUID)
    #[serde(rename = "ts", default, deserialize_with = "deserialize_vec_or_single")]
    pub task_ids: Vec<ThingsId>,

    /// `ix`, sort index within checklist
    #[serde(rename = "ix", default)]
    pub sort_index: i32,

    /// `cd`, creation timestamp
    #[serde(rename = "cd", default, skip_serializing_if = "Option::is_none")]
    pub creation_date: Option<f64>,

    /// `md`, modification timestamp
    #[serde(rename = "md", default, skip_serializing_if = "Option::is_none")]
    pub modification_date: Option<f64>,

    /// `lt`, leaves tombstone on delete
    #[serde(
        rename = "lt",
        default,
        deserialize_with = "deserialize_default_on_null",
        skip_serializing_if = "is_false"
    )]
    pub leaves_tombstone: bool,

    /// `xx`, conflict override metadata
    #[serde(rename = "xx", default, skip_serializing_if = "Option::is_none")]
    pub conflict_overrides: Option<Value>,
}

/// sparse patch fields for checklist item `t=1` updates
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ChecklistItemPatch {
    /// `tt`, title
    #[serde(rename = "tt", skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// `ss`, status
    #[serde(rename = "ss", skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,

    /// `sp`, completion/cancellation timestamp
    #[serde(
        rename = "sp",
        default,
        deserialize_with = "deserialize_optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub stop_date: Option<Option<f64>>,

    /// `ts`, parent task ids, one or many as in a create
    #[serde(
        rename = "ts",
        default,
        deserialize_with = "crate::wire::deserialize_patch_vec_or_single",
        skip_serializing_if = "Option::is_none"
    )]
    pub task_ids: Option<Vec<ThingsId>>,

    /// `ix`, sort index
    #[serde(rename = "ix", skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,

    /// `cd`, creation timestamp
    #[serde(rename = "cd", skip_serializing_if = "Option::is_none")]
    pub creation_date: Option<f64>,

    /// `md`, modification timestamp
    #[serde(rename = "md", skip_serializing_if = "Option::is_none")]
    pub modification_date: Option<f64>,
}
