mod entities;
mod state;

use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
};

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
pub use entities::{
    Area, AreaStateProps, ChecklistItem, ChecklistItemStateProps, ProjectProgress, StateProperties,
    Tag, TagStateProps, Task, TaskStateProps,
};
pub use state::{
    RawState, degraded_checklist_owners, degraded_ids, fold_item, fold_items,
    unread_template_instances,
};

use crate::{
    common::one_line,
    common::{day_of, day_timestamp},
    ids::{
        ThingsId,
        matching::{prefix_matches, shortest_unique_prefixes},
    },
    repeat::next_occurrence_of_rule,
    wire::{
        task::{TaskStart, TaskStatus},
        wire_object::EntityType,
    },
};

#[derive(Debug, Default)]
pub struct ThingsStore {
    pub tasks_by_uuid: HashMap<ThingsId, Task>,
    pub areas_by_uuid: HashMap<ThingsId, Area>,
    pub tags_by_uuid: HashMap<ThingsId, Tag>,
    pub project_progress_by_uuid: HashMap<ThingsId, ProjectProgress>,
    pub short_ids: HashMap<ThingsId, String>,
    pub markable_ids: HashSet<ThingsId>,
    pub markable_ids_sorted: Vec<ThingsId>,
    pub area_ids_sorted: Vec<ThingsId>,
    pub task_ids_sorted: Vec<ThingsId>,
}

/// an instant from the wire, None outside the years 1 to 9999
///
/// outside those years a local offset would leave chrono's range
fn ts_to_dt(ts: Option<f64>) -> Option<DateTime<Utc>> {
    let ts = ts.filter(|ts| (-62_135_596_800.0..=253_402_300_799.0).contains(ts))?;
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
                !self.in_trash(task) && !task.is_heading() && task.entity.can_upgrade_to_task7()
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
            // a template stands for instances still to come
            // it is no to-do of the project yet
            if !task.is_todo() || task.is_recurrence_template() {
                continue;
            }

            let Some(project_uuid) = self.effective_project_uuid(task) else {
                continue;
            };
            // a to-do in the Trash counts for a project in the Trash alone
            let project_trashed = self
                .tasks_by_uuid
                .get(&project_uuid)
                .is_some_and(|project| project.trashed);
            if self.in_trash(task) && !project_trashed {
                continue;
            }

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

        for owner in
            degraded_checklist_owners(raw_state).chain(unread_template_instances(raw_state))
        {
            if let Some(task) = self.tasks_by_uuid.get_mut(&owner) {
                task.degraded = true;
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
            // items of one index keep a fixed order by id
            items.sort_by(|a, b| (a.index, &a.uuid).cmp(&(b.index, &b.uuid)));
            if let Some(task) = self.tasks_by_uuid.get_mut(task_uuid) {
                task.checklist_items = items.clone();
            }
        }

        // an instance outlives a purged template as a plain to-do
        // a template the state still holds stays linked, whether it reads or not
        for task in self.tasks_by_uuid.values_mut() {
            task.recurrence_templates
                .retain(|id| raw_state.contains_key(id));
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
            deadline_suppressed: p.deadline_suppressed,
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

    /// the to-dos and projects outside the Trash of `status`, or of every status
    pub fn tasks(&self, status: Option<TaskStatus>) -> Vec<Task> {
        let mut out: Vec<Task> = self
            .tasks_by_uuid
            .values()
            .filter(|task| {
                !self.in_trash(task)
                    && !task.is_heading()
                    && status.is_none_or(|status| task.status == status)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| (a.index, &a.uuid).cmp(&(b.index, &b.uuid)));
        out
    }

    /// one row per repeating template for the next instance after today, the one the pass or an Apple client makes next
    ///
    /// the row keeps a rule visible between instances
    pub fn projected_repeats(&self, today: NaiveDate) -> Vec<Task> {
        self.tasks_by_uuid
            .values()
            .filter(|template| {
                template.is_recurrence_template()
                    && !template.trashed
                    && template.status == TaskStatus::Incomplete
                    && !template.instance_creation_paused
                    && !self.in_closed_container(template)
            })
            .filter_map(|template| {
                let rule = template.recurrence_rule.as_ref()?;
                let count = template.instance_creation_count;
                // icsd is where the search for the next instance starts
                // the pass reads it that way
                let search_from = template.instance_creation_start_date.and_then(day_of)?;
                let due = next_occurrence_of_rule(rule, search_from.pred_opt()?, count)?;
                // an instance due today or earlier belongs to today
                // the row shows the one after it
                let next = if due > today {
                    due
                } else {
                    next_occurrence_of_rule(rule, today, count.checked_add(1)?)?
                };
                let mut projected = template.clone();
                projected.start_date = DateTime::from_timestamp(day_timestamp(next), 0);
                projected.start = TaskStart::Someday;
                Some(projected)
            })
            .collect()
    }

    pub fn inbox(&self) -> Vec<Task> {
        let mut out: Vec<Task> = self
            .tasks_by_uuid
            .values()
            .filter(|t| self.in_inbox(t))
            .cloned()
            .collect();
        out.sort_by(|a, b| (a.index, &a.uuid).cmp(&(b.index, &b.uuid)));
        out
    }

    pub fn anytime(&self, today: &DateTime<Utc>) -> Vec<Task> {
        let project_visible = |task: &Task, store: &ThingsStore| {
            if store.in_closed_container(task) {
                return false;
            }
            let Some(project_uuid) = store.effective_project_uuid(task) else {
                return true;
            };
            let Some(project) = store.tasks_by_uuid.get(&project_uuid) else {
                return true;
            };
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
                !self.in_trash(t)
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

    /// completed and canceled items
    ///
    /// filtered by the local calendar day of their completion instant
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
                if task.is_heading() || self.in_trashed_container(task) {
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

    /// the Trash as the app lists it, most recently changed first, each trashed project with the to-dos it took along
    pub fn trash(&self) -> Vec<(Task, Vec<Task>)> {
        let in_trashed_project = |task: &Task| {
            self.effective_project_uuid(task)
                .and_then(|id| self.tasks_by_uuid.get(&id))
                .is_some_and(|project| project.trashed && project.is_project())
        };
        let mut entries: Vec<&Task> = self
            .tasks_by_uuid
            .values()
            .filter(|task| !task.is_heading() && self.in_trash(task) && !in_trashed_project(task))
            .collect();
        entries.sort_by(|a, b| {
            (Reverse(a.modification_date), a.index, &a.uuid).cmp(&(
                Reverse(b.modification_date),
                b.index,
                &b.uuid,
            ))
        });
        entries
            .into_iter()
            .map(|entry| {
                let mut held: Vec<Task> = if entry.is_project() {
                    self.tasks_by_uuid
                        .values()
                        .filter(|task| {
                            !task.is_heading()
                                && self.effective_project_uuid(task).as_ref() == Some(&entry.uuid)
                        })
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };
                held.sort_by(|a, b| (a.index, &a.uuid).cmp(&(b.index, &b.uuid)));
                (entry.clone(), held)
            })
            .collect()
    }

    /// in the Inbox list as the Inbox view reads it
    ///
    /// a marked object without a create shows here as elsewhere
    /// a capture that is done, canceled or in the Trash is out of it
    pub fn in_inbox(&self, task: &Task) -> bool {
        task.status == TaskStatus::Incomplete
            && task.start == TaskStart::Inbox
            && !self.in_trash(task)
            && self.effective_project_uuid(task).is_none()
            && self.effective_area_uuid(task).is_none()
            && !task.is_project()
            && !task.is_heading()
            && !task.is_blank()
    }

    /// in the Today list as the Today view reads it
    ///
    /// a to-do that is blank, done or canceled, in the Trash or in a closed project or heading is out of it
    pub fn in_today(&self, task: &Task, today: &DateTime<Utc>) -> bool {
        task.status == TaskStatus::Incomplete
            && task.is_today(today)
            && !task.is_blank()
            && !task.trashed
            && !self.in_closed_container(task)
    }

    /// a to-do, project or area still carries the tag id
    pub fn tag_is_carried(&self, id: &ThingsId) -> bool {
        self.tasks_by_uuid
            .values()
            .any(|task| task.tags.contains(id))
            || self
                .areas_by_uuid
                .values()
                .any(|area| area.tags.contains(id))
    }

    /// in the Trash itself or through a trashed project or heading
    pub fn in_trash(&self, task: &Task) -> bool {
        task.trashed || self.in_trashed_container(task)
    }

    /// under a trashed heading or in a trashed project
    ///
    /// that puts a to-do in the Trash along with them
    pub fn in_trashed_container(&self, task: &Task) -> bool {
        let heading = task
            .action_group
            .as_ref()
            .and_then(|id| self.tasks_by_uuid.get(id));
        let project = self
            .effective_project_uuid(task)
            .and_then(|id| self.tasks_by_uuid.get(&id));
        heading.is_some_and(|heading| heading.trashed)
            || project.is_some_and(|project| project.trashed)
    }

    /// in a trashed container or a project or heading that is no longer open
    ///
    /// that keeps a to-do out of the lists and a template out of the repeat pass
    pub fn in_closed_container(&self, task: &Task) -> bool {
        let closed = |id: &ThingsId| {
            self.tasks_by_uuid
                .get(id)
                .is_some_and(|container| container.status != TaskStatus::Incomplete)
        };
        self.in_trashed_container(task)
            || task.action_group.as_ref().is_some_and(closed)
            || self
                .effective_project_uuid(task)
                .as_ref()
                .is_some_and(closed)
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
                !t.trashed
                    && t.is_project()
                    && !t.is_recurrence_template()
                    && status.map(|s| t.status == s).unwrap_or(true)
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

    /// the parents above a tag, nearest first
    ///
    /// the list stops where the chain ends or turns back on itself
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
        let resolved = self.resolve_prefix(
            identifier,
            |id| {
                self.markable_ids
                    .contains(id)
                    .then(|| self.tasks_by_uuid.get(id))
                    .flatten()
            },
            &self.markable_ids_sorted,
            "Item",
        );
        // an item in the Trash takes no writes and is named as such rather than as missing
        if resolved.0.is_none()
            && resolved.2.is_empty()
            && let (Some(task), _, _) = self.resolve_task_identifier(identifier)
            && self.in_trash(&task)
        {
            return (
                None,
                format!("Item is in the Trash: {}", one_line(&task.title)),
                Vec::new(),
            );
        }
        resolved
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
