use num_enum::{FromPrimitive, IntoPrimitive};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ids::ThingsId,
    wire::{
        deserialize_default_on_null, deserialize_optional_field, notes::TaskNotes,
        recurrence::RecurrenceRule, serialize_day_stamp,
    },
};

/// task wire properties (`p` fields for task entities through `Task7`)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskProps {
    /// `tt`, title
    #[serde(rename = "tt", default)]
    pub title: String,

    /// `nt`, notes payload as a structured text object
    ///
    /// the plain XML string of older histories is not read
    #[serde(rename = "nt", default)]
    pub notes: Option<TaskNotes>,

    /// `tp`, task type (`Todo`, `Project`, `Heading`)
    #[serde(rename = "tp", default)]
    pub item_type: TaskType,

    /// `ss`, task status (`Incomplete`, `Canceled`, `Completed`)
    #[serde(rename = "ss", default)]
    pub status: TaskStatus,

    /// `sp`, completion/cancellation timestamp
    #[serde(rename = "sp", default)]
    pub stop_date: Option<f64>,

    /// `st`, list location (`Inbox`, `Anytime`, `Someday`)
    #[serde(rename = "st", default)]
    pub start_location: TaskStart,

    /// `sr`, scheduled/start day timestamp
    #[serde(rename = "sr", default)]
    pub scheduled_date: Option<i64>,

    /// `tir`, today index reference day timestamp
    #[serde(rename = "tir", default)]
    pub today_index_reference: Option<i64>,

    /// `dd`, deadline day timestamp
    #[serde(rename = "dd", default)]
    pub deadline: Option<i64>,

    /// `dds`, the day the due deadline was taken out of Today, usually null
    #[serde(rename = "dds", default)]
    pub deadline_suppressed_date: Option<Value>,

    /// `pr`, parent project ids (typically 0 or 1)
    #[serde(rename = "pr", default)]
    pub parent_project_ids: Vec<ThingsId>,

    /// `ar`, area ids (typically 0 or 1)
    #[serde(rename = "ar", default)]
    pub area_ids: Vec<ThingsId>,

    /// `agr`, heading/action-group ids (typically 0 or 1)
    #[serde(rename = "agr", default)]
    pub action_group_ids: Vec<ThingsId>,

    /// `tg`, applied tag ids
    #[serde(rename = "tg", default)]
    pub tag_ids: Vec<ThingsId>,

    /// `ix`, structural sort index in its container
    #[serde(rename = "ix", default)]
    pub sort_index: i32,

    /// `ti`, Today-view sort index
    #[serde(
        rename = "ti",
        default,
        deserialize_with = "deserialize_default_on_null"
    )]
    pub today_sort_index: i32,

    /// `do`, due date offset
    ///
    /// observed as `0` in typical payloads
    #[serde(
        rename = "do",
        default,
        deserialize_with = "deserialize_default_on_null"
    )]
    pub due_date_offset: i32,

    /// `rr`, the repeat rule of a template, null on every other task
    #[serde(rename = "rr", default)]
    pub recurrence_rule: Option<RecurrenceRule>,

    /// `rmd`, reminder metadata
    ///
    /// observed as null for normal task/project creates
    #[serde(rename = "rmd", default)]
    pub reminder_metadata: Option<Value>,

    /// `rp`, Task7 repeater payload
    #[serde(rename = "rp", default)]
    pub repeater: Option<Value>,

    /// `rt`, the template an instance belongs to
    #[serde(rename = "rt", default)]
    pub recurrence_template_ids: Vec<ThingsId>,

    /// `icsd`, the day the search for a template's next instance starts
    #[serde(rename = "icsd", default)]
    pub instance_creation_start_date: Option<i64>,

    /// `acrd`, after-completion reference date timestamp for repeat scheduling
    #[serde(rename = "acrd", default)]
    pub after_completion_reference_date: Option<i64>,

    /// `icc`, the number of instances a template has made
    #[serde(
        rename = "icc",
        default,
        deserialize_with = "deserialize_default_on_null"
    )]
    pub instance_creation_count: i32,

    /// `icp`, instance creation paused flag
    #[serde(rename = "icp", default)]
    pub instance_creation_paused: bool,

    /// `ato`, alarm time offset in seconds from day start
    #[serde(rename = "ato", default)]
    pub alarm_time_offset: Option<i64>,

    /// `lai`, last alarm interaction timestamp
    #[serde(rename = "lai", default)]
    pub last_alarm_interaction: Option<f64>,

    /// `sb`, evening section bit (`1` evening, `0` normal)
    #[serde(
        rename = "sb",
        default,
        deserialize_with = "deserialize_default_on_null"
    )]
    pub evening_bit: i32,

    /// `lt`, leaves tombstone when deleted
    #[serde(
        rename = "lt",
        default,
        deserialize_with = "deserialize_default_on_null"
    )]
    pub leaves_tombstone: bool,

    /// `tr`, trashed state
    #[serde(rename = "tr", default)]
    pub trashed: bool,

    /// `dl`, deadline list metadata
    ///
    /// rarely used, often empty
    #[serde(rename = "dl", default)]
    pub deadline_list: Vec<Value>,

    /// `xx`, conflict override metadata (CRDT internals)
    #[serde(rename = "xx", default)]
    pub conflict_overrides: Option<Value>,

    /// `cd`, creation timestamp
    #[serde(rename = "cd", default)]
    pub creation_date: Option<f64>,

    /// `md`, last user-modification timestamp
    #[serde(rename = "md", default)]
    pub modification_date: Option<f64>,
}

/// sparse patch fields for task `t=1` updates
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskPatch {
    /// `tt`, title
    #[serde(rename = "tt", skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// `nt`, notes payload
    #[serde(rename = "nt", skip_serializing_if = "Option::is_none")]
    pub notes: Option<TaskNotes>,

    /// `st`, start location
    #[serde(rename = "st", skip_serializing_if = "Option::is_none")]
    pub start_location: Option<TaskStart>,

    /// `sr`, scheduled day timestamp
    ///
    /// `null` clears date
    #[serde(
        rename = "sr",
        default,
        deserialize_with = "deserialize_optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub scheduled_date: Option<Option<i64>>,

    /// `tir`, today reference day timestamp
    ///
    /// `null` clears today placement
    #[serde(
        rename = "tir",
        default,
        deserialize_with = "deserialize_optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub today_index_reference: Option<Option<i64>>,

    /// `pr`, parent project ids
    #[serde(rename = "pr", skip_serializing_if = "Option::is_none")]
    pub parent_project_ids: Option<Vec<ThingsId>>,

    /// `ar`, area ids
    #[serde(rename = "ar", skip_serializing_if = "Option::is_none")]
    pub area_ids: Option<Vec<ThingsId>>,

    /// `agr`, heading/action-group ids
    #[serde(rename = "agr", skip_serializing_if = "Option::is_none")]
    pub action_group_ids: Option<Vec<ThingsId>>,

    /// `tg`, tag ids
    #[serde(rename = "tg", skip_serializing_if = "Option::is_none")]
    pub tag_ids: Option<Vec<ThingsId>>,

    /// `sb`, evening section bit (`1` evening, `0` normal)
    #[serde(rename = "sb", skip_serializing_if = "Option::is_none")]
    pub evening_bit: Option<i32>,

    /// `ato`, alarm time offset in seconds from day start
    ///
    /// `null` clears the reminder
    #[serde(
        rename = "ato",
        default,
        deserialize_with = "deserialize_optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub alarm_time_offset: Option<Option<i64>>,

    /// `do`, due date offset in days, the deadline of a repeat's instances counted from their day
    #[serde(rename = "do", skip_serializing_if = "Option::is_none")]
    pub due_date_offset: Option<i32>,

    /// `tp`, task type
    #[serde(rename = "tp", skip_serializing_if = "Option::is_none")]
    pub item_type: Option<TaskType>,

    /// `ss`, task status
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

    /// `dd`, deadline day timestamp
    ///
    /// read as any number and written whole
    #[serde(
        rename = "dd",
        default,
        deserialize_with = "deserialize_optional_field",
        serialize_with = "serialize_day_stamp",
        skip_serializing_if = "Option::is_none"
    )]
    pub deadline: Option<Option<f64>>,

    /// `dds`, the day the due deadline was taken out of Today, null when it was not
    #[serde(
        rename = "dds",
        default,
        deserialize_with = "deserialize_optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub deadline_suppressed_date: Option<Option<Value>>,

    /// `ix`, sort index
    #[serde(rename = "ix", skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,

    /// `ti`, today sort index
    #[serde(rename = "ti", skip_serializing_if = "Option::is_none")]
    pub today_sort_index: Option<i32>,

    /// `rr`, the repeat rule of a template
    #[serde(
        rename = "rr",
        default,
        deserialize_with = "deserialize_optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub recurrence_rule: Option<Option<RecurrenceRule>>,

    /// `rp`, Task7 repeater payload
    ///
    /// `null` clears the repeater
    #[serde(
        rename = "rp",
        default,
        deserialize_with = "deserialize_optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub repeater: Option<Option<Value>>,

    /// `rt`, the template an instance belongs to
    #[serde(rename = "rt", skip_serializing_if = "Option::is_none")]
    pub recurrence_template_ids: Option<Vec<ThingsId>>,

    /// `icsd`, the day the search for the next instance starts
    #[serde(
        rename = "icsd",
        default,
        deserialize_with = "deserialize_optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub instance_creation_start_date: Option<Option<i64>>,

    /// `acrd`, after-completion reference date timestamp
    #[serde(
        rename = "acrd",
        default,
        deserialize_with = "deserialize_optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub after_completion_reference_date: Option<Option<i64>>,

    /// `icc`, the number of instances made so far
    #[serde(rename = "icc", skip_serializing_if = "Option::is_none")]
    pub instance_creation_count: Option<i32>,

    /// `icp`, instance creation paused
    #[serde(rename = "icp", skip_serializing_if = "Option::is_none")]
    pub instance_creation_paused: Option<bool>,

    /// `lt`, leaves tombstone
    #[serde(rename = "lt", skip_serializing_if = "Option::is_none")]
    pub leaves_tombstone: Option<bool>,

    /// `tr`, trashed
    #[serde(rename = "tr", skip_serializing_if = "Option::is_none")]
    pub trashed: Option<bool>,

    /// `cd`, creation timestamp
    #[serde(
        rename = "cd",
        default,
        deserialize_with = "deserialize_optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub creation_date: Option<Option<f64>>,

    /// `md`, modification timestamp
    #[serde(
        rename = "md",
        default,
        deserialize_with = "deserialize_optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub modification_date: Option<Option<f64>>,
}

impl TaskPatch {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.notes.is_none()
            && self.start_location.is_none()
            && self.scheduled_date.is_none()
            && self.alarm_time_offset.is_none()
            && self.today_index_reference.is_none()
            && self.parent_project_ids.is_none()
            && self.area_ids.is_none()
            && self.action_group_ids.is_none()
            && self.tag_ids.is_none()
            && self.evening_bit.is_none()
            && self.item_type.is_none()
            && self.status.is_none()
            && self.stop_date.is_none()
            && self.deadline.is_none()
            && self.deadline_suppressed_date.is_none()
            && self.sort_index.is_none()
            && self.today_sort_index.is_none()
            && self.recurrence_rule.is_none()
            && self.repeater.is_none()
            && self.recurrence_template_ids.is_none()
            && self.instance_creation_start_date.is_none()
            && self.after_completion_reference_date.is_none()
            && self.instance_creation_count.is_none()
            && self.instance_creation_paused.is_none()
            && self.leaves_tombstone.is_none()
            && self.trashed.is_none()
            && self.creation_date.is_none()
            && self.modification_date.is_none()
            && self.due_date_offset.is_none()
    }
}

/// task kind used in `tp`
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, FromPrimitive, IntoPrimitive,
)]
#[repr(i32)]
#[serde(from = "i32", into = "i32")]
pub enum TaskType {
    /// regular leaf task
    Todo = 0,
    /// project container
    Project = 1,
    /// heading/section under a project
    Heading = 2,

    /// unknown value preserved for forward compatibility
    #[num_enum(catch_all)]
    Unknown(i32),
}

#[allow(clippy::derivable_impls)]
impl Default for TaskType {
    fn default() -> Self {
        Self::Todo
    }
}

/// task status used in `ss`
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, FromPrimitive, IntoPrimitive,
)]
#[repr(i32)]
#[serde(from = "i32", into = "i32")]
pub enum TaskStatus {
    /// open/incomplete
    Incomplete = 0,
    /// canceled
    Canceled = 2,
    /// completed
    Completed = 3,

    /// unknown value preserved for forward compatibility
    #[num_enum(catch_all)]
    Unknown(i32),
}

#[allow(clippy::derivable_impls)]
impl Default for TaskStatus {
    fn default() -> Self {
        Self::Incomplete
    }
}

/// start location used in `st`
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, FromPrimitive, IntoPrimitive,
)]
#[repr(i32)]
#[serde(from = "i32", into = "i32")]
pub enum TaskStart {
    /// Inbox list
    Inbox = 0,
    /// Anytime list
    Anytime = 1,
    /// Someday list
    Someday = 2,

    /// unknown value preserved for forward compatibility
    #[num_enum(catch_all)]
    Unknown(i32),
}

#[allow(clippy::derivable_impls)]
impl Default for TaskStart {
    fn default() -> Self {
        Self::Inbox
    }
}
