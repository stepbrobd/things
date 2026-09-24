use std::collections::HashMap;

use tracing::warn;

use crate::{
    ids::ThingsId,
    store::entities::{
        AreaStateProps, ChecklistItemStateProps, StateObject, StateProperties, TagStateProps,
        TaskStateProps,
    },
    wire::{
        area::AreaPatch,
        checklist::ChecklistItemPatch,
        notes::TaskNotesApplyError,
        tags::TagPatch,
        task::TaskPatch,
        wire_object::{EntityType, OperationType, Properties, WireItem, WireObject},
    },
};

pub type RawState = HashMap<ThingsId, StateObject>;

/// apply a task patch, keeping the old note when a note delta does not apply
fn apply_task_patch(
    task: &mut TaskStateProps,
    patch: TaskPatch,
) -> Result<(), TaskNotesApplyError> {
    let mut outcome = Ok(());
    if let Some(title) = patch.title {
        task.title = title;
    }
    if let Some(notes) = patch.notes {
        match notes.apply_to(task.notes.as_deref()) {
            Ok(updated) => task.notes = updated,
            Err(error) => outcome = Err(error),
        }
    }
    if let Some(start_location) = patch.start_location {
        task.start_location = start_location;
    }
    if let Some(scheduled_date) = patch.scheduled_date {
        task.scheduled_date = scheduled_date.map(|v| v as f64);
    }
    if let Some(today_index_reference) = patch.today_index_reference {
        task.today_index_reference = today_index_reference;
    }
    if let Some(parent_project_ids) = patch.parent_project_ids {
        task.parent_project_ids = parent_project_ids;
    }
    if let Some(area_ids) = patch.area_ids {
        task.area_ids = area_ids;
    }
    if let Some(action_group_ids) = patch.action_group_ids {
        task.action_group_ids = action_group_ids;
    }
    if let Some(tag_ids) = patch.tag_ids {
        task.tag_ids = tag_ids;
    }
    if let Some(evening_bit) = patch.evening_bit {
        task.evening_bit = evening_bit;
    }
    if let Some(alarm_time_offset) = patch.alarm_time_offset {
        task.alarm_time_offset = alarm_time_offset;
    }
    if let Some(due_date_offset) = patch.due_date_offset {
        task.due_date_offset = due_date_offset;
    }
    if let Some(modification_date) = patch.modification_date {
        task.modification_date = modification_date;
    }

    if let Some(item_type) = patch.item_type {
        task.item_type = item_type;
    }
    if let Some(status) = patch.status {
        task.status = status;
    }
    if let Some(stop_date) = patch.stop_date {
        task.stop_date = stop_date;
    }
    if let Some(deadline) = patch.deadline {
        task.deadline = deadline;
    }
    if let Some(suppressed) = patch.deadline_suppressed_date {
        task.deadline_suppressed = suppressed.is_some_and(|day| !day.is_null());
    }
    if let Some(sort_index) = patch.sort_index {
        task.sort_index = sort_index;
    }
    if let Some(today_sort_index) = patch.today_sort_index {
        task.today_sort_index = today_sort_index;
    }
    if let Some(recurrence_rule) = patch.recurrence_rule {
        task.recurrence_rule = recurrence_rule;
    }
    if let Some(repeater) = patch.repeater {
        task.repeater = repeater;
    }
    if let Some(recurrence_template_ids) = patch.recurrence_template_ids {
        task.recurrence_template_ids = recurrence_template_ids;
    }
    if let Some(instance_creation_start_date) = patch.instance_creation_start_date {
        task.instance_creation_start_date = instance_creation_start_date;
    }
    if let Some(after_completion_reference_date) = patch.after_completion_reference_date {
        task.after_completion_reference_date = after_completion_reference_date;
    }
    if let Some(instance_creation_count) = patch.instance_creation_count {
        task.instance_creation_count = instance_creation_count;
    }
    if let Some(instance_creation_paused) = patch.instance_creation_paused {
        task.instance_creation_paused = instance_creation_paused;
    }
    if let Some(leaves_tombstone) = patch.leaves_tombstone {
        task.leaves_tombstone = leaves_tombstone;
    }
    if let Some(trashed) = patch.trashed {
        task.trashed = trashed;
    }
    if let Some(creation_date) = patch.creation_date {
        task.creation_date = creation_date;
    }
    outcome
}

fn apply_checklist_patch(item: &mut ChecklistItemStateProps, patch: ChecklistItemPatch) {
    if let Some(title) = patch.title {
        item.title = title;
    }
    if let Some(status) = patch.status {
        item.status = status;
    }
    if let Some(stop_date) = patch.stop_date {
        item.stop_date = stop_date;
    }
    if let Some(task_ids) = patch.task_ids {
        item.task_ids = task_ids;
    }
    if let Some(sort_index) = patch.sort_index {
        item.sort_index = sort_index;
    }
}

fn apply_area_patch(area: &mut AreaStateProps, patch: AreaPatch) {
    if let Some(title) = patch.title {
        area.title = title;
    }
    if let Some(tag_ids) = patch.tag_ids {
        area.tag_ids = tag_ids;
    }
    if let Some(sort_index) = patch.sort_index {
        area.sort_index = sort_index;
    }
}

fn apply_tag_patch(tag: &mut TagStateProps, patch: TagPatch) {
    if let Some(title) = patch.title {
        tag.title = title;
    }
    if let Some(parent_ids) = patch.parent_ids {
        tag.parent_ids = parent_ids;
    }
    if let Some(shortcut) = patch.shortcut {
        tag.shortcut = shortcut;
    }
    if let Some(sort_index) = patch.sort_index {
        tag.sort_index = sort_index;
    }
}

/// the state of an object whose create never arrived, a patch applied to nothing
impl From<TaskPatch> for TaskStateProps {
    fn from(patch: TaskPatch) -> Self {
        let mut task = Self::default();
        if let Err(error) = apply_task_patch(&mut task, patch) {
            warn!(target: "things::replay", "note delta on an object without a create not applied: {error:?}");
        }
        task
    }
}

impl From<ChecklistItemPatch> for ChecklistItemStateProps {
    fn from(patch: ChecklistItemPatch) -> Self {
        let mut item = Self::default();
        apply_checklist_patch(&mut item, patch);
        item
    }
}

impl From<AreaPatch> for AreaStateProps {
    fn from(patch: AreaPatch) -> Self {
        let mut area = Self::default();
        apply_area_patch(&mut area, patch);
        area
    }
}

impl From<TagPatch> for TagStateProps {
    fn from(patch: TagPatch) -> Self {
        let mut tag = Self::default();
        apply_tag_patch(&mut tag, patch);
        tag
    }
}

fn wire_object_properties(obj: &WireObject) -> StateProperties {
    match obj.properties() {
        Ok(payload) => payload.into(),
        Err(_) => StateProperties::Other,
    }
}

/// an object of a stored kind whose payload did not parse keeps the fields that do and is marked, as is one an update reaches before any create, a task whose note cannot be read, a task of a kind this CLI does not read and a tombstone whose payload does not parse
fn insert_state_object(state: &mut RawState, uuid: &ThingsId, obj: WireObject) {
    let stored = obj.entity_type.as_ref().is_some_and(EntityType::is_stored);
    let unparsed = stored && matches!(obj.payload, Properties::Unknown(_));
    let properties = if unparsed {
        obj.readable_properties()
            .map_or(StateProperties::Other, Into::into)
    } else {
        wire_object_properties(&obj)
    };
    let create_less = stored && obj.operation_type == OperationType::Update;
    let unreadable_note = matches!(
        &obj.payload,
        Properties::TaskCreate(props)
            if props.notes.as_ref().is_some_and(|notes| notes.apply_to(None).is_err())
    );
    let future = obj
        .entity_type
        .as_ref()
        .is_some_and(|entity| entity.is_task_family() && !stored);
    // a tombstone whose payload does not parse names nothing to purge
    // its own mark reports it as not replayed
    let unread_tombstone = matches!(obj.entity_type, Some(EntityType::Tombstone2))
        && matches!(obj.payload, Properties::Unknown(_));
    let degraded = unparsed || create_less || unreadable_note || future || unread_tombstone;
    if unparsed {
        warn!(target: "things::replay", uuid = %uuid, "the object's payload did not parse, the fields that do are kept");
    } else if unreadable_note {
        warn!(target: "things::replay", uuid = %uuid, "the task's note cannot be read");
    } else if create_less {
        warn!(target: "things::replay", uuid = %uuid, "an update reached the object before any create, it is kept partial");
    } else if future {
        warn!(target: "things::replay", uuid = %uuid, "a task of a kind this CLI does not read, it is kept opaque");
    } else if unread_tombstone {
        warn!(target: "things::replay", uuid = %uuid, "the tombstone's payload did not parse, nothing is purged");
    }
    state.insert(
        uuid.clone(),
        StateObject {
            entity_type: obj.entity_type,
            properties,
            degraded,
        },
    );
}

/// an update the object cannot take leaves it marked, a note delta that does not apply, a patch for a known entity that did not parse, or a payload of another kind than the object holds
fn apply_update_payload(
    uuid: &ThingsId,
    existing: &mut StateObject,
    payload: Properties,
    entity_type: Option<&EntityType>,
) {
    let failure = match (&mut existing.properties, payload) {
        (StateProperties::Task(task), Properties::TaskUpdate(patch)) => {
            apply_task_patch(task, *patch)
                .err()
                .map(|error| format!("note delta not applied: {error:?}"))
        }
        (StateProperties::ChecklistItem(item), Properties::ChecklistUpdate(patch)) => {
            apply_checklist_patch(item, patch);
            None
        }
        (StateProperties::Area(area), Properties::AreaUpdate(patch)) => {
            apply_area_patch(area, patch);
            None
        }
        (StateProperties::Tag(tag), Properties::TagUpdate(patch)) => {
            apply_tag_patch(tag, patch);
            None
        }
        (_, Properties::Ignored(_)) => None,
        (StateProperties::Other, Properties::Unknown(_)) => entity_type
            .is_some_and(EntityType::is_stored)
            .then(|| "the patch did not parse".to_string()),
        (_, Properties::Unknown(_)) => Some(
            if entity_type.is_some_and(EntityType::is_stored) {
                "the patch did not parse"
            } else {
                "an update of a kind this CLI does not read"
            }
            .to_string(),
        ),
        (_, payload) => {
            existing.properties = payload.into();
            Some("the payload is of another kind than the object".to_string())
        }
    };
    if let Some(failure) = failure {
        warn!(target: "things::replay", uuid = %uuid, "{failure}");
        existing.degraded = true;
    }
}

pub fn fold_item(item: WireItem, state: &mut RawState) {
    // the objects of a commit fold in key order
    // its purges apply last, after any move out of what they purge
    let mut purged = Vec::new();
    for (key, obj) in item {
        let Ok(uuid) = key.parse::<ThingsId>() else {
            warn!(target: "things::replay", %key, "an object whose id is not base58 is skipped");
            continue;
        };
        match obj.operation_type {
            OperationType::Create => {
                if let Properties::TombstoneCreate(tombstone) = &obj.payload {
                    purged.push(tombstone.deleted_object_id.clone());
                    continue;
                }
                insert_state_object(state, &uuid, obj);
            }
            OperationType::Update => {
                if let Some(existing) = state.get_mut(&uuid) {
                    match obj.properties() {
                        Ok(payload) => {
                            apply_update_payload(&uuid, existing, payload, obj.entity_type.as_ref())
                        }
                        Err(error) => {
                            warn!(target: "things::replay", uuid = %uuid, "the patch did not parse: {error}");
                            existing.degraded = true;
                        }
                    }
                    if obj.entity_type.is_some() {
                        existing.entity_type = obj.entity_type.clone();
                    }
                } else {
                    insert_state_object(state, &uuid, obj);
                }
            }
            OperationType::Delete => {
                state.remove(&uuid);
            }
            OperationType::Unknown(operation) => {
                warn!(target: "things::replay", uuid = %uuid, operation, "an operation this CLI does not read reached the object");
                if let Some(existing) = state.get_mut(&uuid) {
                    existing.degraded = true;
                } else {
                    // an object the fold has not seen keeps the fields that parse as an update of its kind
                    // its mark reports it as not replayed
                    let update = WireObject {
                        operation_type: OperationType::Update,
                        ..obj
                    };
                    let properties = update
                        .readable_properties()
                        .map_or(StateProperties::Other, Into::into);
                    state.insert(
                        uuid,
                        StateObject {
                            entity_type: update.entity_type,
                            properties,
                            degraded: true,
                        },
                    );
                }
            }
        }
    }
    for target in &purged {
        purge(state, target);
    }
}

/// a `Tombstone2` names an object an Apple client deleted for good, emptying the Trash for instance
///
/// the object goes with what only exists through it, the checklist of a to-do and the to-dos and headings of a project or heading
/// those would otherwise come back as open items without their container
/// an area or a tag goes alone
/// a to-do in an area is not a part of it
fn purge(state: &mut RawState, target: &ThingsId) {
    let mut gone = vec![target.clone()];
    while let Some(id) = gone.pop() {
        state.remove(&id);
        gone.extend(state.iter().filter_map(|(child, object)| {
            let held = match &object.properties {
                StateProperties::Task(task) => {
                    task.parent_project_ids.contains(&id) || task.action_group_ids.contains(&id)
                }
                StateProperties::ChecklistItem(item) => item.task_ids.contains(&id),
                _ => false,
            };
            held.then(|| child.clone())
        }));
    }
}

/// the objects whose replay did not complete, which no command may write through
pub fn degraded_ids(state: &RawState) -> Vec<ThingsId> {
    let mut ids: Vec<ThingsId> = state
        .iter()
        .filter(|(_, object)| object.degraded)
        .map(|(uuid, _)| uuid.clone())
        .collect();
    ids.extend(degraded_checklist_owners(state));
    ids.extend(unread_template_instances(state));
    ids.sort();
    ids.dedup();
    ids
}

/// the tasks a marked checklist item belongs to, whose checklist is not whole either
pub fn degraded_checklist_owners(state: &RawState) -> impl Iterator<Item = ThingsId> + '_ {
    state
        .values()
        .filter(|object| object.degraded)
        .filter_map(|object| match &object.properties {
            StateProperties::ChecklistItem(item) => Some(item.task_ids.iter().cloned()),
            _ => None,
        })
        .flatten()
}

/// the to-dos whose repeat template the state holds as no readable task
///
/// without their template they would read as plain to-dos
pub fn unread_template_instances(state: &RawState) -> impl Iterator<Item = ThingsId> + '_ {
    state
        .iter()
        .filter(|(_, object)| match &object.properties {
            StateProperties::Task(task) => task.recurrence_template_ids.iter().any(|id| {
                state.get(id).is_some_and(|template| {
                    !matches!(template.properties, StateProperties::Task(_))
                })
            }),
            _ => false,
        })
        .map(|(uuid, _)| uuid.clone())
}

pub fn fold_items(items: impl IntoIterator<Item = WireItem>) -> RawState {
    let mut state = RawState::new();
    for item in items {
        fold_item(item, &mut state);
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        store::ThingsStore,
        wire::{
            task::{TaskStatus, TaskType},
            wire_object::EntityType,
        },
    };

    const TASK_ID: &str = "A7h5eCi24RvAWKC3Hv3muf";

    fn wire_item(json: &str) -> WireItem {
        serde_json::from_str(json).expect("test wire item should deserialize")
    }

    #[test]
    fn a_due_deadline_brings_an_undated_to_do_into_today_until_suppressed() {
        // 2026-03-25 at UTC midnight, the deadline that day and the evening bit on a template
        let today = chrono::DateTime::from_timestamp(1_774_396_800, 0).expect("today");
        let due = format!(
            r#"{{"{TASK_ID}":{{"t":0,"e":"Task7","p":{{"tt":"Rent","st":1,"dd":1774396800}}}},"Te11111111111111111111":{{"t":0,"e":"Task7","p":{{"tt":"Template","st":2,"sb":1,"rr":{{"fu":16,"fa":1,"of":[{{"dy":0}}],"tp":0}}}}}}}}"#
        );
        let suppressed =
            format!(r#"{{"{TASK_ID}":{{"t":1,"e":"Task7","p":{{"dds":1774396800}}}}}}"#);
        let store = ThingsStore::from_raw_state(&fold_items([wire_item(&due)]));
        assert!(store.get_task(TASK_ID).expect("to-do").is_today(&today));
        assert!(
            !store
                .get_task("Te11111111111111111111")
                .expect("template")
                .is_today(&today)
        );
        let store =
            ThingsStore::from_raw_state(&fold_items([wire_item(&due), wire_item(&suppressed)]));
        assert!(!store.get_task(TASK_ID).expect("to-do").is_today(&today));
    }

    #[test]
    fn a_tombstone_removes_what_it_names_and_what_only_exists_through_it() {
        let items = [
            r#"{"Pj11111111111111111111":{"t":0,"e":"Task7","p":{"tt":"Kitchen","tp":1,"st":1,"tr":true}}}"#,
            r#"{"Hd11111111111111111111":{"t":0,"e":"Task7","p":{"tt":"Tiles","tp":2,"st":1,"pr":["Pj11111111111111111111"]}}}"#,
            r#"{"Ta11111111111111111111":{"t":0,"e":"Task7","p":{"tt":"Measure","st":1,"pr":["Pj11111111111111111111"]}}}"#,
            r#"{"Tb11111111111111111111":{"t":0,"e":"Task7","p":{"tt":"Order","st":1,"agr":["Hd11111111111111111111"]}}}"#,
            r#"{"Ck11111111111111111111":{"t":0,"e":"ChecklistItem3","p":{"tt":"Tape","ts":["Ta11111111111111111111"],"ss":0,"ix":1}}}"#,
            r#"{"Tc11111111111111111111":{"t":0,"e":"Task7","p":{"tt":"Trashed alone","st":1,"tr":true}}}"#,
            r#"{"Ar11111111111111111111":{"t":0,"e":"Area3","p":{"tt":"Home"}}}"#,
            r#"{"Td11111111111111111111":{"t":0,"e":"Task7","p":{"tt":"In the area","st":1,"ar":["Ar11111111111111111111"]}}}"#,
            r#"{"Tm11111111111111111111":{"t":0,"e":"Tombstone2","p":{"dloid":"Pj11111111111111111111","dld":1774396800}}}"#,
            r#"{"Tn11111111111111111111":{"t":0,"e":"Tombstone2","p":{"dloid":"Ar11111111111111111111","dld":1774396800}}}"#,
        ];
        let state = fold_items(items.map(wire_item));
        let present = |id: &str| state.contains_key(&id.parse::<ThingsId>().expect("id"));
        for purged in [
            "Pj11111111111111111111",
            "Hd11111111111111111111",
            "Ta11111111111111111111",
            "Tb11111111111111111111",
            "Ck11111111111111111111",
            "Ar11111111111111111111",
            "Tm11111111111111111111",
            "Tn11111111111111111111",
        ] {
            assert!(!present(purged), "{purged} stayed");
        }
        assert!(
            present("Tc11111111111111111111"),
            "a trashed to-do stays in the Trash"
        );
        assert!(
            present("Td11111111111111111111"),
            "a to-do outlives its area"
        );
    }

    #[test]
    fn an_operation_this_cli_does_not_read_marks_an_object_it_has_not_seen() {
        let items = [
            r#"{"Ta11111111111111111111":{"t":3,"e":"Task7","p":{"tt":"Order tiles"}}}"#,
            r#"{"Tc11111111111111111111":{"t":0,"e":"Task7","p":"garbled"}}"#,
        ];
        let state = fold_items(items.map(wire_item));
        assert_eq!(
            degraded_ids(&state),
            vec![
                "Ta11111111111111111111".parse::<ThingsId>().expect("id"),
                "Tc11111111111111111111".parse::<ThingsId>().expect("id"),
            ]
        );
        // the fields that parse keep the object listed as a marked row
        let store = ThingsStore::from_raw_state(&state);
        let task = store.get_task("Ta11111111111111111111").expect("listed");
        assert_eq!(task.title, "Order tiles");
        assert!(task.degraded);
    }

    #[test]
    fn a_tombstone_that_does_not_parse_is_marked_and_purges_nothing() {
        let items = [
            r#"{"Ta11111111111111111111":{"t":0,"e":"Task7","p":{"tt":"Order tiles","st":1,"tr":true}}}"#,
            r#"{"Tm11111111111111111111":{"t":0,"e":"Tombstone2","p":{"dloid":5,"dld":1774396800}}}"#,
        ];
        let state = fold_items(items.map(wire_item));
        let tombstone = "Tm11111111111111111111".parse::<ThingsId>().expect("id");
        assert!(
            state.contains_key(&"Ta11111111111111111111".parse::<ThingsId>().expect("id")),
            "the to-do stays"
        );
        assert_eq!(degraded_ids(&state), vec![tombstone]);
    }

    #[test]
    fn an_instance_keeps_a_template_the_state_holds_and_is_marked_while_it_does_not_read() {
        // an object without a kind is held and read as no task
        let items = [
            r#"{"Tt11111111111111111111":{"t":0,"p":{"tt":"Water plants"}}}"#,
            r#"{"Ti11111111111111111111":{"t":0,"e":"Task7","p":{"tt":"Water plants","st":1,"rt":["Tt11111111111111111111"]}}}"#,
        ];
        let state = fold_items(items.map(wire_item));
        let store = ThingsStore::from_raw_state(&state);
        let instance = store.get_task("Ti11111111111111111111").expect("instance");
        assert!(instance.is_recurrence_instance());
        assert!(instance.degraded);
        assert_eq!(
            degraded_ids(&state),
            vec!["Ti11111111111111111111".parse::<ThingsId>().expect("id")]
        );
    }

    #[test]
    fn a_tombstone_purges_what_it_names_whatever_its_deletion_time_holds() {
        let items = [
            r#"{"Ta11111111111111111111":{"t":0,"e":"Task7","p":{"tt":"Order tiles","st":1,"tr":true}}}"#,
            r#"{"Tm11111111111111111111":{"t":0,"e":"Tombstone2","p":{"dloid":"Ta11111111111111111111","dld":"1774396800"}}}"#,
        ];
        let state = fold_items(items.map(wire_item));
        assert!(state.is_empty(), "{state:?}");
    }

    #[test]
    fn a_purge_waits_for_the_rest_of_its_commit() {
        let items = [
            r#"{"Pk11111111111111111111":{"t":0,"e":"Task7","p":{"tt":"Garden","tp":1,"st":1}}}"#,
            r#"{"Hd11111111111111111111":{"t":0,"e":"Task7","p":{"tt":"Spring","tp":2,"st":1,"pr":["Pk11111111111111111111"]}}}"#,
            r#"{"Tc11111111111111111111":{"t":0,"e":"Task7","p":{"tt":"Plant bulbs","st":1,"agr":["Hd11111111111111111111"]}}}"#,
            // the tombstone's key sorts before the to-do the same commit moves out of the heading
            r#"{"A111111111111111111111":{"t":0,"e":"Tombstone2","p":{"dloid":"Hd11111111111111111111","dld":1774396800}},"Tc11111111111111111111":{"t":1,"e":"Task7","p":{"agr":[],"pr":["Pk11111111111111111111"]}}}"#,
        ];
        let state = fold_items(items.map(wire_item));
        let heading = "Hd11111111111111111111".parse::<ThingsId>().expect("id");
        assert!(!state.contains_key(&heading));
        let moved = state
            .get(&"Tc11111111111111111111".parse::<ThingsId>().expect("id"))
            .expect("the moved to-do outlives the heading");
        assert!(!moved.degraded);
        let StateProperties::Task(task) = &moved.properties else {
            panic!("the to-do keeps its task state");
        };
        assert_eq!(task.title, "Plant bulbs");
    }

    fn task6_create() -> WireItem {
        wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":0,"e":"Task6","p":{{"tt":"Send tracking number","tp":0,"ss":0,"st":1,"cd":1.0,"md":1.0}}}}}}"#
        ))
    }

    #[test]
    fn task7_update_preserves_task_state_and_promotes_entity() {
        let update = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":1,"e":"Task7","p":{{"ss":0,"sp":null,"md":2.0}}}}}}"#
        ));
        let state = fold_items([task6_create(), update]);
        let task_id = TASK_ID.parse::<ThingsId>().expect("valid task id");
        let object = state.get(&task_id).expect("task state should remain");

        assert_eq!(object.entity_type, Some(EntityType::Task7));
        let StateProperties::Task(properties) = &object.properties else {
            panic!("Task7 update replaced typed task state");
        };
        assert_eq!(properties.title, "Send tracking number");
        assert_eq!(properties.status, TaskStatus::Incomplete);
        assert_eq!(properties.modification_date, Some(2.0));

        let store = ThingsStore::from_raw_state(&state);
        let task = store
            .get_task(TASK_ID)
            .expect("Task7 task should be visible");
        assert_eq!(task.item_type, TaskType::Todo);
        assert_eq!(task.entity, EntityType::Task7);
    }

    #[test]
    fn unknown_future_task_update_keeps_known_state_and_marks_it() {
        let update = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":1,"e":"Task8","p":{{"future":true}}}}}}"#
        ));
        let state = fold_items([task6_create(), update]);
        let task_id = TASK_ID.parse::<ThingsId>().expect("valid task id");
        let object = state.get(&task_id).expect("task state should remain");

        assert_eq!(
            object.entity_type,
            Some(EntityType::Unknown("Task8".to_string()))
        );
        assert!(matches!(object.properties, StateProperties::Task(_)));
        assert!(
            ThingsStore::from_raw_state(&state)
                .get_task(TASK_ID)
                .is_some()
        );
        // what the update changed is unknown, writes on top of it could clobber it
        assert!(object.degraded);
        assert_eq!(degraded_ids(&state), vec![task_id]);
    }

    #[test]
    fn malformed_task7_patch_is_preserved_as_unknown_and_ignored() {
        let update = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":1,"e":"Task7","p":{{"ss":"future"}}}}}}"#
        ));
        let object = update.get(TASK_ID).expect("Task7 update");
        assert!(matches!(object.payload, Properties::Unknown(_)));

        let state = fold_items([task6_create(), update]);
        let task_id = TASK_ID.parse::<ThingsId>().expect("valid task id");
        let StateProperties::Task(properties) = &state[&task_id].properties else {
            panic!("malformed Task7 patch replaced typed task state");
        };
        assert_eq!(properties.title, "Send tracking number");
        assert_eq!(properties.status, TaskStatus::Incomplete);
        // the object is marked, what is shown may be behind the history
        assert!(state[&task_id].degraded);
        assert_eq!(degraded_ids(&state), vec![task_id]);
    }

    #[test]
    fn a_malformed_checklist_patch_marks_the_item_instead_of_failing_the_fold() {
        const ITEM_ID: &str = "5uwoHPi5m5i8QJa6Rae6Cn";
        let create = wire_item(&format!(
            r#"{{"{ITEM_ID}":{{"t":0,"e":"ChecklistItem3","p":{{"tt":"Step","ss":0,"ts":["{TASK_ID}"],"ix":0}}}}}}"#
        ));
        let update = wire_item(&format!(
            r#"{{"{ITEM_ID}":{{"t":1,"e":"ChecklistItem3","p":{{"ss":"future"}}}}}}"#
        ));
        assert!(matches!(
            update.get(ITEM_ID).expect("update").payload,
            Properties::Unknown(_)
        ));

        let state = fold_items([task6_create(), create, update]);
        let item_id = ITEM_ID.parse::<ThingsId>().expect("valid id");
        let StateProperties::ChecklistItem(item) = &state[&item_id].properties else {
            panic!("the malformed patch replaced the item's typed state");
        };
        assert_eq!(item.title, "Step");
        assert!(state[&item_id].degraded);
        // the item's to-do is refused with it
        let mut marked = vec![item_id, TASK_ID.parse::<ThingsId>().expect("valid id")];
        marked.sort();
        assert_eq!(degraded_ids(&state), marked);
    }

    #[test]
    fn an_update_before_any_create_leaves_a_marked_partial_task() {
        let update = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":1,"e":"Task7","p":{{"tt":"Partial","md":2.0}}}}}}"#
        ));
        let state = fold_items([update]);
        let task_id = TASK_ID.parse::<ThingsId>().expect("valid task id");
        let StateProperties::Task(properties) = &state[&task_id].properties else {
            panic!("the patch becomes a partial task");
        };
        assert_eq!(properties.title, "Partial");
        assert!(state[&task_id].degraded);
        // a settings object without a create is not a task and carries no mark
        let settings =
            wire_item(r#"{"Se11111111111111111111":{"t":1,"e":"Settings5","p":{"x":1}}}"#);
        let state = fold_items([settings]);
        assert!(state.values().all(|object| !object.degraded));
    }

    #[test]
    fn an_unparseable_create_keeps_what_parses_and_is_marked() {
        let create = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":0,"e":"Task7","p":{{"tt":"Odd","ss":"future"}}}}}}"#
        ));
        let state = fold_items([create]);
        let task_id = TASK_ID.parse::<ThingsId>().expect("valid task id");
        let StateProperties::Task(task) = &state[&task_id].properties else {
            panic!("the title parses and stays in view");
        };
        assert_eq!(task.title, "Odd");
        assert!(state[&task_id].degraded);

        // a note in a format this CLI does not read marks its task too
        let create = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":0,"e":"Task7","p":{{"tt":"Odd","nt":{{"_t":"tx","t":3,"v":"x"}}}}}}}}"#
        ));
        assert!(fold_items([create])[&task_id].degraded);

        // so does a checklist item of it that did not replay
        let item = "Ck11111111111111111111";
        let items = [
            task6_create(),
            wire_item(&format!(
                r#"{{"{item}":{{"t":0,"e":"ChecklistItem3","p":{{"tt":"Step","ts":["{TASK_ID}"],"ss":"x"}}}}}}"#
            )),
        ];
        let state = fold_items(items);
        assert!(!state[&task_id].degraded);
        assert!(degraded_ids(&state).contains(&task_id));
        assert!(
            ThingsStore::from_raw_state(&state)
                .get_task(TASK_ID)
                .expect("task")
                .degraded
        );
    }

    #[test]
    fn structured_note_delta_updates_the_existing_note() {
        let checksum = crc32fast::hash("café! todo".as_bytes());
        let create = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":0,"e":"Task6","p":{{"tt":"Write notes","tp":0,"ss":0,"st":1,"nt":{{"_t":"tx","t":1,"ch":0,"v":"café todo"}}}}}}}}"#
        ));
        let update = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":1,"e":"Task7","p":{{"nt":{{"_t":"tx","t":2,"ps":[{{"p":5,"l":0,"r":"!","ch":{checksum}}}]}}}}}}}}"#
        ));

        let state = fold_items([create, update]);
        let task_id = TASK_ID.parse::<ThingsId>().expect("valid task id");
        let StateProperties::Task(properties) = &state[&task_id].properties else {
            panic!("task state should remain typed");
        };

        assert_eq!(properties.notes.as_deref(), Some("café! todo"));
        assert!(!state[&task_id].degraded);
    }

    #[test]
    fn invalid_structured_note_delta_preserves_the_existing_note() {
        let create = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":0,"e":"Task6","p":{{"tt":"Write notes","tp":0,"ss":0,"st":1,"nt":{{"_t":"tx","t":1,"ch":0,"v":"original"}}}}}}}}"#
        ));
        let update = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":1,"e":"Task7","p":{{"nt":{{"_t":"tx","t":2,"ps":[{{"p":0,"l":8,"r":"corrupt","ch":0}}]}}}}}}}}"#
        ));

        let state = fold_items([create, update]);
        let task_id = TASK_ID.parse::<ThingsId>().expect("valid task id");
        let StateProperties::Task(properties) = &state[&task_id].properties else {
            panic!("task state should remain typed");
        };

        assert_eq!(properties.notes.as_deref(), Some("original"));
        assert!(state[&task_id].degraded);
    }

    #[test]
    fn recurrence_maintenance_updates_are_folded_into_task_state() {
        let create = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":0,"e":"Task6","p":{{"tt":"Repeat","tp":0,"ss":0,"st":1,"icsd":10,"acrd":20,"icc":2}}}}}}"#
        ));
        let update = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":1,"e":"Task6","p":{{"icsd":11,"acrd":null,"icc":3}}}}}}"#
        ));

        let state = fold_items([create, update]);
        let task_id = TASK_ID.parse::<ThingsId>().expect("valid task id");
        let StateProperties::Task(properties) = &state[&task_id].properties else {
            panic!("task state should remain typed");
        };

        assert_eq!(properties.instance_creation_start_date, Some(11));
        assert_eq!(properties.after_completion_reference_date, None);
        assert_eq!(properties.instance_creation_count, 3);
    }

    #[test]
    fn explicit_null_modification_date_clears_task_state() {
        let update = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":1,"e":"Task7","p":{{"md":null}}}}}}"#
        ));

        let state = fold_items([task6_create(), update]);
        let task_id = TASK_ID.parse::<ThingsId>().expect("valid task id");
        let StateProperties::Task(properties) = &state[&task_id].properties else {
            panic!("task state should remain typed");
        };

        assert_eq!(properties.modification_date, None);
    }

    #[test]
    fn checklist_stop_date_updates_are_folded_and_clearable() {
        let create = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":0,"e":"ChecklistItem3","p":{{"tt":"Step","ss":0,"sp":10.0,"ts":["{TASK_ID}"],"ix":0}}}}}}"#
        ));
        let set = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":1,"e":"ChecklistItem3","p":{{"ss":3,"sp":11.0}}}}}}"#
        ));
        let clear = wire_item(&format!(
            r#"{{"{TASK_ID}":{{"t":1,"e":"ChecklistItem3","p":{{"ss":0,"sp":null}}}}}}"#
        ));

        let state = fold_items([create, set, clear]);
        let item_id = TASK_ID.parse::<ThingsId>().expect("valid checklist id");
        let StateProperties::ChecklistItem(properties) = &state[&item_id].properties else {
            panic!("checklist state should remain typed");
        };

        assert_eq!(properties.status, TaskStatus::Incomplete);
        assert_eq!(properties.stop_date, None);
    }
}
