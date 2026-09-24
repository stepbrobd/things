use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    ids::ThingsId,
    wire::{
        area::AreaProps,
        checklist::ChecklistItemProps,
        notes::TaskNotes,
        recurrence::RecurrenceRule,
        tags::TagProps,
        task::{TaskProps, TaskStart, TaskStatus, TaskType},
        wire_object::{EntityType, Properties},
    },
};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct StateObject {
    pub entity_type: Option<EntityType>,
    pub properties: StateProperties,
    /// an update to this object could not be applied, or its create did not parse
    ///
    /// what is shown may be behind the history
    #[serde(default)]
    pub degraded: bool,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum StateProperties {
    Task(Box<TaskStateProps>),
    ChecklistItem(ChecklistItemStateProps),
    Area(AreaStateProps),
    Tag(TagStateProps),
    Other,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct TaskStateProps {
    pub title: String,
    pub notes: Option<String>,
    pub item_type: TaskType,
    pub status: TaskStatus,
    pub stop_date: Option<f64>,
    pub start_location: TaskStart,
    pub scheduled_date: Option<f64>,
    pub today_index_reference: Option<i64>,
    pub deadline: Option<f64>,
    /// `dds` is set
    ///
    /// the due deadline was taken out of Today
    #[serde(default)]
    pub deadline_suppressed: bool,
    pub parent_project_ids: Vec<ThingsId>,
    pub area_ids: Vec<ThingsId>,
    pub action_group_ids: Vec<ThingsId>,
    pub tag_ids: Vec<ThingsId>,
    pub sort_index: i32,
    pub today_sort_index: i32,
    pub recurrence_rule: Option<RecurrenceRule>,
    pub repeater: Option<serde_json::Value>,
    pub recurrence_template_ids: Vec<ThingsId>,
    pub instance_creation_start_date: Option<i64>,
    pub after_completion_reference_date: Option<i64>,
    pub instance_creation_count: i32,
    pub instance_creation_paused: bool,
    pub evening_bit: i32,
    pub alarm_time_offset: Option<i64>,
    /// `do`, the deadline of a repeat's instances as days after their day
    pub due_date_offset: i32,
    pub leaves_tombstone: bool,
    pub trashed: bool,
    pub creation_date: Option<f64>,
    pub modification_date: Option<f64>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ChecklistItemStateProps {
    pub title: String,
    pub status: TaskStatus,
    pub stop_date: Option<f64>,
    pub task_ids: Vec<ThingsId>,
    pub sort_index: i32,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct AreaStateProps {
    pub title: String,
    pub tag_ids: Vec<ThingsId>,
    pub sort_index: i32,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct TagStateProps {
    pub title: String,
    pub shortcut: Option<String>,
    pub sort_index: i32,
    pub parent_ids: Vec<ThingsId>,
}

#[derive(Debug, Clone)]
pub struct Tag {
    pub uuid: ThingsId,
    pub title: String,
    pub shortcut: Option<String>,
    pub index: i32,
    pub parent_uuid: Option<ThingsId>,
}

#[derive(Debug, Clone)]
pub struct Area {
    pub uuid: ThingsId,
    pub title: String,
    pub tags: Vec<ThingsId>,
    pub index: i32,
}

#[derive(Debug, Clone)]
pub struct ChecklistItem {
    pub uuid: ThingsId,
    pub title: String,
    pub task_uuid: ThingsId,
    pub status: TaskStatus,
    pub index: i32,
}

impl ChecklistItem {
    pub fn is_completed(&self) -> bool {
        self.status == TaskStatus::Completed
    }

    pub fn is_canceled(&self) -> bool {
        self.status == TaskStatus::Canceled
    }
}

#[derive(Clone, Default)]
pub struct ProjectProgress {
    pub total: i32,
    pub done: i32,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub uuid: ThingsId,
    pub title: String,
    pub status: TaskStatus,
    pub start: TaskStart,
    pub item_type: TaskType,
    pub entity: EntityType,
    pub notes: Option<String>,
    pub project: Option<ThingsId>,
    pub area: Option<ThingsId>,
    pub action_group: Option<ThingsId>,
    pub tags: Vec<ThingsId>,
    pub trashed: bool,
    pub deadline: Option<DateTime<Utc>>,
    pub deadline_suppressed: bool,
    pub start_date: Option<DateTime<Utc>>,
    pub stop_date: Option<DateTime<Utc>>,
    pub creation_date: Option<DateTime<Utc>>,
    pub modification_date: Option<DateTime<Utc>>,
    pub index: i32,
    pub today_index: i32,
    pub today_index_reference: Option<i64>,
    pub leaves_tombstone: bool,
    pub instance_creation_paused: bool,
    pub instance_creation_start_date: Option<i64>,
    pub instance_creation_count: i32,
    pub evening: bool,
    pub alarm_time_offset: Option<i64>,
    pub recurrence_rule: Option<RecurrenceRule>,
    pub repeater: Option<serde_json::Value>,
    pub recurrence_templates: Vec<ThingsId>,
    /// `do`, the deadline of a repeat's instances as days after their day
    pub due_date_offset: i32,
    /// the object's replay did not complete
    ///
    /// what is shown may be behind the history
    /// no write goes through it
    pub degraded: bool,
    pub checklist_items: Vec<ChecklistItem>,
}

impl Task {
    pub fn reminder(&self) -> Option<String> {
        self.alarm_time_offset
            .map(|secs| format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60))
    }

    /// a blank row with nothing in it
    ///
    /// notes, a checklist, a tag or a deadline alone make it a row that counts
    /// a project always counts
    /// it holds its to-dos
    pub fn is_blank(&self) -> bool {
        !self.is_project()
            && self.title.trim().is_empty()
            && self.notes.as_deref().unwrap_or("").trim().is_empty()
            && self.checklist_items.is_empty()
            && self.tags.is_empty()
            && self.deadline.is_none()
    }

    pub fn is_completed(&self) -> bool {
        self.status == TaskStatus::Completed
    }

    pub fn is_canceled(&self) -> bool {
        self.status == TaskStatus::Canceled
    }

    pub fn is_todo(&self) -> bool {
        self.item_type == TaskType::Todo
    }

    /// the number of an item kind this CLI does not know, None for a to-do, project or heading
    pub fn unknown_kind(&self) -> Option<i32> {
        match self.item_type {
            TaskType::Unknown(raw) => Some(raw),
            _ => None,
        }
    }

    pub fn is_project(&self) -> bool {
        self.item_type == TaskType::Project
    }

    pub fn is_heading(&self) -> bool {
        self.item_type == TaskType::Heading
    }

    pub fn in_someday(&self) -> bool {
        self.start == TaskStart::Someday && self.start_date.is_none()
    }

    /// the day rule of the Today list
    ///
    /// `ThingsStore::in_today` completes it
    ///
    /// started on or before today, or undated with a deadline on or before today that nobody took out of Today
    /// templates are excluded
    pub fn is_today(&self, today: &DateTime<Utc>) -> bool {
        if self.is_recurrence_template() {
            return false;
        }
        match self.start_date {
            Some(start_date) => {
                matches!(self.start, TaskStart::Anytime | TaskStart::Someday)
                    && start_date <= *today
            }
            None => {
                !self.deadline_suppressed
                    && self.deadline.is_some_and(|deadline| deadline <= *today)
            }
        }
    }

    pub fn is_staged_for_today(&self, today: &DateTime<Utc>) -> bool {
        let Some(start_date) = self.start_date else {
            return false;
        };
        self.start == TaskStart::Someday && start_date <= *today
    }

    pub fn is_recurrence_template(&self) -> bool {
        self.recurrence_rule.is_some() && self.recurrence_templates.is_empty()
    }

    pub fn is_recurrence_instance(&self) -> bool {
        self.recurrence_rule.is_none() && !self.recurrence_templates.is_empty()
    }

    pub fn has_repeater(&self) -> bool {
        self.repeater.is_some()
    }
}

fn i64_to_f64_opt(value: Option<i64>) -> Option<f64> {
    value.map(|v| v as f64)
}

fn parse_notes_from_wire(notes: &Option<TaskNotes>) -> Option<String> {
    notes.as_ref().and_then(TaskNotes::to_plain_text)
}

impl From<TaskProps> for TaskStateProps {
    fn from(props: TaskProps) -> Self {
        Self {
            title: props.title,
            notes: parse_notes_from_wire(&props.notes),
            item_type: props.item_type,
            status: props.status,
            stop_date: props.stop_date,
            start_location: props.start_location,
            scheduled_date: i64_to_f64_opt(props.scheduled_date),
            today_index_reference: props.today_index_reference,
            deadline: i64_to_f64_opt(props.deadline),
            deadline_suppressed: props.deadline_suppressed_date.is_some(),
            parent_project_ids: props.parent_project_ids,
            area_ids: props.area_ids,
            action_group_ids: props.action_group_ids,
            tag_ids: props.tag_ids,
            sort_index: props.sort_index,
            today_sort_index: props.today_sort_index,
            recurrence_rule: props.recurrence_rule,
            repeater: props.repeater,
            recurrence_template_ids: props.recurrence_template_ids,
            instance_creation_start_date: props.instance_creation_start_date,
            after_completion_reference_date: props.after_completion_reference_date,
            instance_creation_count: props.instance_creation_count,
            instance_creation_paused: props.instance_creation_paused,
            evening_bit: props.evening_bit,
            alarm_time_offset: props.alarm_time_offset,
            due_date_offset: props.due_date_offset,
            leaves_tombstone: props.leaves_tombstone,
            trashed: props.trashed,
            creation_date: props.creation_date,
            modification_date: props.modification_date,
        }
    }
}

impl From<ChecklistItemProps> for ChecklistItemStateProps {
    fn from(props: ChecklistItemProps) -> Self {
        Self {
            title: props.title,
            status: props.status,
            stop_date: props.stop_date,
            task_ids: props.task_ids,
            sort_index: props.sort_index,
        }
    }
}

impl From<AreaProps> for AreaStateProps {
    fn from(props: AreaProps) -> Self {
        Self {
            title: props.title,
            tag_ids: props.tag_ids,
            sort_index: props.sort_index,
        }
    }
}

impl From<TagProps> for TagStateProps {
    fn from(props: TagProps) -> Self {
        Self {
            title: props.title,
            shortcut: props.shortcut,
            sort_index: props.sort_index,
            parent_ids: props.parent_ids,
        }
    }
}

impl From<Properties> for StateProperties {
    fn from(payload: Properties) -> Self {
        use Properties::*;
        use StateProperties::*;

        match payload {
            TaskCreate(props) => Task(Box::new((*props).into())),
            TaskUpdate(patch) => Task(Box::new((*patch).into())),
            ChecklistCreate(props) => ChecklistItem(props.into()),
            ChecklistUpdate(patch) => ChecklistItem(patch.into()),
            AreaCreate(props) => Area(props.into()),
            AreaUpdate(patch) => Area(patch.into()),
            TagCreate(props) => Tag(props.into()),
            TagUpdate(patch) => Tag(patch.into()),
            TombstoneCreate(_) | CommandCreate(_) | Ignored(_) | Unknown(_) | Delete => Other,
        }
    }
}
