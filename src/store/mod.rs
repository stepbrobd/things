mod entities;
mod state;

use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
};

use chrono::{DateTime, Days, Local, NaiveDate, TimeZone, Utc};
pub use entities::{
    Area, AreaStateProps, ChecklistItem, ChecklistItemStateProps, ProjectProgress, StateObject,
    StateProperties, Tag, TagStateProps, Task, TaskStateProps,
};
pub use state::{RawState, degraded_ids, fold_item, fold_items};

use crate::{
    common::day_timestamp,
    ids::{
        ThingsId,
        matching::{prefix_matches, shortest_unique_prefixes},
    },
    repeat::next_occurrence_of_rule,
    wire::{
        task::{TaskStart, TaskStatus, TaskType},
        wire_object::EntityType,
    },
};

#[derive(Debug, Default)]
pub struct ThingsStore {
    pub tasks_by_uuid: HashMap<ThingsId, Task>,
    pub areas_by_uuid: HashMap<ThingsId, Area>,
    pub tags_by_uuid: HashMap<ThingsId, Tag>,
    pub tags_by_title: HashMap<String, ThingsId>,
    pub project_progress_by_uuid: HashMap<ThingsId, ProjectProgress>,
    pub short_ids: HashMap<ThingsId, String>,
    pub markable_ids: HashSet<ThingsId>,
    pub markable_ids_sorted: Vec<ThingsId>,
    pub area_ids_sorted: Vec<ThingsId>,
    pub task_ids_sorted: Vec<ThingsId>,
}

fn ts_to_dt(ts: Option<f64>) -> Option<DateTime<Utc>> {
    let ts = ts?;
    let mut secs = ts.floor() as i64;
    let mut nanos = ((ts - secs as f64) * 1_000_000_000_f64).round() as u32;
    if nanos >= 1_000_000_000 {
        secs += 1;
        nanos = 0;
    }
    Utc.timestamp_opt(secs, nanos).single()
}

impl ThingsStore {
    pub fn from_raw_state(raw_state: &RawState) -> Self {
        let mut store = Self::default();
        store.build(raw_state);
        store.build_project_progress_index();
        store.short_ids = shortest_unique_prefixes(&store.short_id_domain(raw_state));
        store.build_mark_indexes();
        store.area_ids_sorted = store.areas_by_uuid.keys().cloned().collect();
        store.area_ids_sorted.sort();
        store.task_ids_sorted = store.tasks_by_uuid.keys().cloned().collect();
        store.task_ids_sorted.sort();
        store
    }

    fn short_id_domain(&self, raw_state: &RawState) -> Vec<ThingsId> {
        let mut ids = Vec::new();
        for (uuid, obj) in raw_state {
            if let Some(EntityType::Tombstone2) = obj.entity_type.as_ref() {
                continue;
            }

            ids.push(uuid.clone());
        }
        ids
    }

    fn build_mark_indexes(&mut self) {
        let markable: Vec<&Task> = self
            .tasks_by_uuid
            .values()
            .filter(|task| {
                !task.trashed && !task.is_heading() && task.entity.can_upgrade_to_task7()
            })
            .collect();

        self.markable_ids = markable.iter().map(|t| t.uuid.clone()).collect();
        self.markable_ids_sorted = self.markable_ids.iter().cloned().collect();
        self.markable_ids_sorted.sort();
    }

    fn build_project_progress_index(&mut self) {
        let mut totals: HashMap<ThingsId, i32> = HashMap::new();
        let mut dones: HashMap<ThingsId, i32> = HashMap::new();

        for task in self.tasks_by_uuid.values() {
            if task.trashed || !task.is_todo() {
                continue;
            }

            let Some(project_uuid) = self.effective_project_uuid(task) else {
                continue;
            };

            *totals.entry(project_uuid.clone()).or_insert(0) += 1;
            if task.is_completed() {
                *dones.entry(project_uuid).or_insert(0) += 1;
            }
        }

        self.project_progress_by_uuid = totals
            .into_iter()
            .map(|(project_uuid, total)| {
                let done = *dones.get(&project_uuid).unwrap_or(&0);
                (project_uuid, ProjectProgress { total, done })
            })
            .collect();
    }

    fn build(&mut self, raw_state: &RawState) {
        let mut checklist_items: Vec<ChecklistItem> = Vec::new();

        for (uuid, obj) in raw_state {
            match obj.entity_type.as_ref() {
                Some(entity) if entity.is_task_family() => {
                    let StateProperties::Task(props) = &obj.properties else {
                        continue;
                    };
                    let task = self.parse_task(uuid, props, entity, obj.degraded);
                    self.tasks_by_uuid.insert(uuid.clone(), task);
                }
                Some(EntityType::Area3) => {
                    let StateProperties::Area(props) = &obj.properties else {
                        continue;
                    };
                    let area = self.parse_area(uuid, props);
                    self.areas_by_uuid.insert(uuid.clone(), area);
                }
                Some(EntityType::Tag3 | EntityType::Tag4) => {
                    let StateProperties::Tag(props) = &obj.properties else {
                        continue;
                    };
                    let tag = self.parse_tag(uuid, props);
                    if !tag.title.is_empty() {
                        self.tags_by_title
                            .insert(tag.title.clone(), tag.uuid.clone());
                    }
                    self.tags_by_uuid.insert(uuid.clone(), tag);
                }
                Some(
                    EntityType::ChecklistItem
                    | EntityType::ChecklistItem2
                    | EntityType::ChecklistItem3,
                ) => {
                    if let StateProperties::ChecklistItem(props) = &obj.properties {
                        checklist_items.push(self.parse_checklist_item(uuid, props));
                    }
                }
                _ => {}
            }
        }

        let mut by_task: HashMap<ThingsId, Vec<ChecklistItem>> = HashMap::new();
        for item in checklist_items {
            if self.tasks_by_uuid.contains_key(&item.task_uuid) {
                by_task
                    .entry(item.task_uuid.clone())
                    .or_default()
                    .push(item);
            }
        }

        for (task_uuid, items) in by_task.iter_mut() {
            items.sort_by_key(|i| i.index);
            if let Some(task) = self.tasks_by_uuid.get_mut(task_uuid) {
                task.checklist_items = items.clone();
            }
        }
    }

    fn parse_task(
        &self,
        uuid: &ThingsId,
        p: &TaskStateProps,
        entity: &EntityType,
        degraded: bool,
    ) -> Task {
        Task {
            uuid: uuid.clone(),
            title: p.title.clone(),
            status: p.status,
            start: p.start_location,
            item_type: p.item_type,
            entity: entity.clone(),
            notes: p.notes.clone(),
            project: p.parent_project_ids.first().cloned(),
            area: p.area_ids.first().cloned(),
            action_group: p.action_group_ids.first().cloned(),
            tags: p.tag_ids.clone(),
            trashed: p.trashed,
            deadline: ts_to_dt(p.deadline),
            start_date: ts_to_dt(p.scheduled_date),
            stop_date: ts_to_dt(p.stop_date),
            creation_date: ts_to_dt(p.creation_date),
            modification_date: ts_to_dt(p.modification_date),
            index: p.sort_index,
            today_index: p.today_sort_index,
            today_index_reference: p.today_index_reference,
            leaves_tombstone: p.leaves_tombstone,
            instance_creation_paused: p.instance_creation_paused,
            instance_creation_start_date: p.instance_creation_start_date,
            instance_creation_count: p.instance_creation_count,
            evening: p.evening_bit != 0,
            alarm_time_offset: p.alarm_time_offset,
            due_date_offset: p.due_date_offset,
            degraded,
            recurrence_rule: p.recurrence_rule.clone(),
            repeater: p.repeater.clone(),
            recurrence_templates: p.recurrence_template_ids.clone(),
            checklist_items: Vec::new(),
        }
    }

    fn parse_checklist_item(&self, uuid: &ThingsId, p: &ChecklistItemStateProps) -> ChecklistItem {
        ChecklistItem {
            uuid: uuid.clone(),
            title: p.title.clone(),
            task_uuid: p.task_ids.first().cloned().unwrap_or_default(),
            status: p.status,
            index: p.sort_index,
        }
    }

    fn parse_area(&self, uuid: &ThingsId, p: &AreaStateProps) -> Area {
        Area {
            uuid: uuid.clone(),
            title: p.title.clone(),
            tags: p.tag_ids.clone(),
            index: p.sort_index,
        }
    }

    fn parse_tag(&self, uuid: &ThingsId, p: &TagStateProps) -> Tag {
        Tag {
            uuid: uuid.clone(),
            title: p.title.clone(),
            shortcut: p.shortcut.clone(),
            index: p.sort_index,
            parent_uuid: p.parent_ids.first().cloned(),
        }
    }

    pub fn tasks(
        &self,
        status: Option<TaskStatus>,
        trashed: Option<bool>,
        item_type: Option<TaskType>,
    ) -> Vec<Task> {
        let mut out: Vec<Task> = self
            .tasks_by_uuid
            .values()
            .filter(|task| {
                if let Some(expect_trashed) = trashed
                    && task.trashed != expect_trashed
                {
                    return false;
                }
                if let Some(expect_status) = status
                    && task.status != expect_status
                {
                    return false;
                }
                if let Some(expect_type) = item_type
                    && task.item_type != expect_type
                {
                    return false;
                }
                if task.is_heading() {
                    return false;
                }
                true
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| (a.index, &a.uuid).cmp(&(b.index, &b.uuid)));
        out
    }

    /// one row per repeating template for its next occurrence after today, so a rule stays visible between instances
    pub fn projected_repeats(&self, today: NaiveDate) -> Vec<Task> {
        let floor = today - Days::new(1);
        self.tasks_by_uuid
            .values()
            .filter(|template| {
                template.is_recurrence_template()
                    && !template.trashed
                    && template.status == TaskStatus::Incomplete
                    && !template.instance_creation_paused
            })
            .filter_map(|template| {
                let last_instance = self
                    .tasks_by_uuid
                    .values()
                    .filter(|task| {
                        !task.trashed && task.recurrence_templates.contains(&template.uuid)
                    })
                    .filter_map(|task| task.start_date)
                    .map(|day| day.date_naive())
                    .max();
                let after = last_instance.map_or(floor, |last| last.max(floor));
                let next = next_occurrence_of_rule(
                    template.recurrence_rule.as_ref()?,
                    after,
                    template.instance_creation_count,
                )?;
                if next <= today {
                    return None;
                }
                let mut projected = template.clone();
                projected.start_date = DateTime::from_timestamp(day_timestamp(next), 0);
                projected.start = TaskStart::Someday;
                Some(projected)
            })
            .collect()
    }

    pub fn today(&self, today: &DateTime<Utc>) -> Vec<Task> {
        let mut out: Vec<Task> = self
            .tasks_by_uuid
            .values()
            .filter(|t| {
                !t.trashed
                    && t.status == TaskStatus::Incomplete
                    && !t.is_heading()
                    && !t.is_project()
                    && !t.is_blank()
                    && t.is_today(today)
            })
            .cloned()
            .collect();

        // equal keys keep a fixed order, the id breaks the tie
        fn key(task: &Task) -> (i32, Reverse<i64>, Reverse<i32>, &ThingsId) {
            if task.today_index == 0 {
                let sr_ts = task.start_date.map(|d| d.timestamp()).unwrap_or(0);
                (0, Reverse(sr_ts), Reverse(task.index), &task.uuid)
            } else {
                (
                    1,
                    Reverse(task.today_index as i64),
                    Reverse(task.index),
                    &task.uuid,
                )
            }
        }
        out.sort_by(|a, b| key(a).cmp(&key(b)));
        out
    }

    pub fn inbox(&self) -> Vec<Task> {
        let mut out: Vec<Task> = self
            .tasks_by_uuid
            .values()
            .filter(|t| {
                !t.trashed
                    && t.status == TaskStatus::Incomplete
                    && t.start == TaskStart::Inbox
                    && self.effective_project_uuid(t).is_none()
                    && self.effective_area_uuid(t).is_none()
                    && !t.is_project()
                    && !t.is_heading()
                    && !t.is_blank()
                    && t.creation_date.is_some()
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| (a.index, &a.uuid).cmp(&(b.index, &b.uuid)));
        out
    }

    pub fn anytime(&self, today: &DateTime<Utc>) -> Vec<Task> {
        let project_visible = |task: &Task, store: &ThingsStore| {
            let Some(project_uuid) = store.effective_project_uuid(task) else {
                return true;
            };
            let Some(project) = store.tasks_by_uuid.get(&project_uuid) else {
                return true;
            };
            if project.trashed || project.status != TaskStatus::Incomplete {
                return false;
            }
            if project.start == TaskStart::Someday {
                return false;
            }
            if let Some(start_date) = project.start_date
                && start_date > *today
            {
                return false;
            }
            true
        };

        let mut out: Vec<Task> = self
            .tasks_by_uuid
            .values()
            .filter(|t| {
                !t.trashed
                    && t.status == TaskStatus::Incomplete
                    && t.start == TaskStart::Anytime
                    && !t.is_project()
                    && !t.is_heading()
                    && !t.is_blank()
                    && (t.start_date.is_none() || t.start_date <= Some(*today))
                    && project_visible(t, self)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| (a.index, &a.uuid).cmp(&(b.index, &b.uuid)));
        out
    }

    pub fn someday(&self) -> Vec<Task> {
        let mut out: Vec<Task> = self
            .tasks_by_uuid
            .values()
            .filter(|t| {
                !t.trashed
                    && t.status == TaskStatus::Incomplete
                    && t.start == TaskStart::Someday
                    && !t.is_heading()
                    && !t.is_blank()
                    && !t.is_recurrence_template()
                    && t.start_date.is_none()
                    && (t.is_project() || self.effective_project_uuid(t).is_none())
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| (a.index, &a.uuid).cmp(&(b.index, &b.uuid)));
        out
    }

    /// completed and canceled items, filtered by the local calendar day of their completion instant
    pub fn logbook(&self, from_date: Option<NaiveDate>, to_date: Option<NaiveDate>) -> Vec<Task> {
        let mut out: Vec<Task> = self
            .tasks_by_uuid
            .values()
            .filter(|task| {
                if task.trashed
                    || !(task.status == TaskStatus::Completed
                        || task.status == TaskStatus::Canceled)
                {
                    return false;
                }
                if task.is_heading() {
                    return false;
                }
                let Some(stop_date) = task.stop_date else {
                    return false;
                };

                // the offset in force at that instant, not today's
                let stop_day = stop_date.with_timezone(&Local).date_naive();
                if from_date.is_some_and(|from_day| stop_day < from_day) {
                    return false;
                }
                if to_date.is_some_and(|to_day| stop_day > to_day) {
                    return false;
                }

                true
            })
            .cloned()
            .collect();

        out.sort_by_key(|t| {
            let stop_key = t
                .stop_date
                .map(|d| (d.timestamp(), d.timestamp_subsec_nanos()))
                .unwrap_or((0, 0));
            (Reverse(stop_key), Reverse(t.index), t.uuid.clone())
        });
        out
    }

    pub fn effective_project_uuid(&self, task: &Task) -> Option<ThingsId> {
        if let Some(project) = &task.project {
            return Some(project.clone());
        }
        if let Some(action_group) = &task.action_group
            && let Some(heading) = self.tasks_by_uuid.get(action_group)
            && let Some(project) = &heading.project
        {
            return Some(project.clone());
        }
        None
    }

    pub fn effective_area_uuid(&self, task: &Task) -> Option<ThingsId> {
        if let Some(area) = &task.area {
            return Some(area.clone());
        }

        if let Some(project_uuid) = self.effective_project_uuid(task)
            && let Some(project) = self.tasks_by_uuid.get(&project_uuid)
            && let Some(area) = &project.area
        {
            return Some(area.clone());
        }

        if let Some(action_group) = &task.action_group
            && let Some(heading) = self.tasks_by_uuid.get(action_group)
            && let Some(area) = &heading.area
        {
            return Some(area.clone());
        }

        None
    }

    pub fn projects(&self, status: Option<TaskStatus>) -> Vec<Task> {
        let mut out: Vec<Task> = self
            .tasks_by_uuid
            .values()
            .filter(|t| {
                !t.trashed && t.is_project() && status.map(|s| t.status == s).unwrap_or(true)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| (a.index, &a.uuid).cmp(&(b.index, &b.uuid)));
        out
    }

    pub fn areas(&self) -> Vec<Area> {
        let mut out: Vec<Area> = self.areas_by_uuid.values().cloned().collect();
        out.sort_by(|a, b| (a.index, &a.uuid).cmp(&(b.index, &b.uuid)));
        out
    }

    pub fn tags(&self) -> Vec<Tag> {
        let mut out: Vec<Tag> = self
            .tags_by_uuid
            .values()
            .filter(|t| !t.title.trim().is_empty())
            .cloned()
            .collect();
        out.sort_by(|a, b| (a.index, &a.uuid).cmp(&(b.index, &b.uuid)));
        out
    }

    /// the parents above a tag, nearest first, stopping where the chain ends or turns back on itself
    pub fn tag_ancestors(&self, tag: &ThingsId) -> Vec<ThingsId> {
        let mut seen = HashSet::from([tag.clone()]);
        let mut chain = Vec::new();
        let mut current = self
            .tags_by_uuid
            .get(tag)
            .and_then(|tag| tag.parent_uuid.clone());
        while let Some(parent) = current {
            if !seen.insert(parent.clone()) {
                break;
            }
            current = self
                .tags_by_uuid
                .get(&parent)
                .and_then(|tag| tag.parent_uuid.clone());
            chain.push(parent);
        }
        chain
    }

    pub fn get_task(&self, uuid: &str) -> Option<Task> {
        uuid.parse::<ThingsId>()
            .ok()
            .and_then(|id| self.tasks_by_uuid.get(&id).cloned())
    }

    pub fn get_area(&self, uuid: &str) -> Option<Area> {
        uuid.parse::<ThingsId>()
            .ok()
            .and_then(|id| self.areas_by_uuid.get(&id).cloned())
    }

    pub fn get_tag(&self, uuid: &str) -> Option<Tag> {
        uuid.parse::<ThingsId>()
            .ok()
            .and_then(|id| self.tags_by_uuid.get(&id).cloned())
    }

    pub fn resolve_tag_title<T: ToString>(&self, uuid: T) -> String {
        let raw = uuid.to_string();
        raw.parse::<ThingsId>()
            .ok()
            .and_then(|id| self.tags_by_uuid.get(&id))
            .filter(|t| !t.title.trim().is_empty())
            .map(|t| t.title.clone())
            .unwrap_or(raw)
    }

    pub fn resolve_area_title<T: ToString>(&self, uuid: T) -> String {
        let raw = uuid.to_string();
        raw.parse::<ThingsId>()
            .ok()
            .and_then(|id| self.areas_by_uuid.get(&id))
            .map(|a| a.title.clone())
            .unwrap_or(raw)
    }

    pub fn resolve_project_title<T: ToString>(&self, uuid: T) -> String {
        let raw = uuid.to_string();
        if let Ok(id) = raw.parse::<ThingsId>()
            && let Some(task) = self.tasks_by_uuid.get(&id)
            && !task.title.trim().is_empty()
        {
            return task.title.clone();
        }
        if raw.is_empty() {
            return "(project)".to_string();
        }
        let short: String = raw.chars().take(8).collect();
        format!("(project {short})")
    }

    pub fn short_id<T: ToString>(&self, uuid: T) -> String {
        let raw = uuid.to_string();
        raw.parse::<ThingsId>()
            .ok()
            .and_then(|id| self.short_ids.get(&id).cloned())
            .unwrap_or(raw)
    }

    pub fn project_progress<T: ToString>(&self, project_uuid: T) -> ProjectProgress {
        project_uuid
            .to_string()
            .parse::<ThingsId>()
            .ok()
            .and_then(|id| self.project_progress_by_uuid.get(&id).cloned())
            .unwrap_or_default()
    }

    pub fn unique_prefix_length<T: ToString>(&self, ids: &[T]) -> usize {
        if ids.is_empty() {
            return 0;
        }
        let mut max_need = 1usize;
        for id in ids {
            if let Ok(parsed) = id.to_string().parse::<ThingsId>()
                && let Some(short) = self.short_ids.get(&parsed)
            {
                max_need = max_need.max(short.len());
            } else {
                max_need = max_need.max(6);
            }
        }
        max_need
    }

    /// one item by full id or unique prefix among `sorted_ids`, looked up through `lookup`, or the message and up to ten candidates when the prefix is ambiguous
    fn resolve_prefix<'a, T: Clone + 'a>(
        &'a self,
        identifier: &str,
        lookup: impl Fn(&ThingsId) -> Option<&'a T>,
        sorted_ids: &[ThingsId],
        label: &str,
    ) -> (Option<T>, String, Vec<T>) {
        let ident = identifier.trim();
        if ident.is_empty() {
            return (
                None,
                format!("Missing {} identifier.", label.to_lowercase()),
                Vec::new(),
            );
        }

        if let Ok(exact_id) = ident.parse::<ThingsId>()
            && let Some(exact) = lookup(&exact_id)
        {
            return (Some(exact.clone()), String::new(), Vec::new());
        }

        let matches: Vec<&ThingsId> = prefix_matches(sorted_ids, ident);
        if matches.len() == 1
            && let Some(item) = lookup(matches[0])
        {
            return (Some(item.clone()), String::new(), Vec::new());
        }

        if matches.len() > 1 {
            let mut out = Vec::new();
            for m in matches.iter().take(10) {
                if let Some(item) = lookup(m) {
                    out.push(item.clone());
                }
            }
            let remaining = matches.len().saturating_sub(out.len());
            let mut msg = format!("Ambiguous {} id prefix.", label.to_lowercase());
            if remaining > 0 {
                msg.push_str(&format!(
                    " ({} matches, showing first {})",
                    matches.len(),
                    out.len()
                ));
            }
            return (None, msg, out);
        }

        (
            None,
            format!("{} not found: {}", label, identifier),
            Vec::new(),
        )
    }

    pub fn resolve_mark_identifier(&self, identifier: &str) -> (Option<Task>, String, Vec<Task>) {
        self.resolve_prefix(
            identifier,
            |id| {
                self.markable_ids
                    .contains(id)
                    .then(|| self.tasks_by_uuid.get(id))
                    .flatten()
            },
            &self.markable_ids_sorted,
            "Item",
        )
    }

    pub fn resolve_area_identifier(&self, identifier: &str) -> (Option<Area>, String, Vec<Area>) {
        self.resolve_prefix(
            identifier,
            |id| self.areas_by_uuid.get(id),
            &self.area_ids_sorted,
            "Area",
        )
    }

    pub fn resolve_task_identifier(&self, identifier: &str) -> (Option<Task>, String, Vec<Task>) {
        self.resolve_prefix(
            identifier,
            |id| self.tasks_by_uuid.get(id),
            &self.task_ids_sorted,
            "Task",
        )
    }
}
