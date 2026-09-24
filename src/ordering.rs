//! the Today view's order, the lists that show a row and the sort index allocation shared by new, edit and reorder, for the structural index within a container and the today index within a day's group
//!
//! a slot between two neighbors when the gap allows one
//! otherwise the run is rebalanced
//! the members that move come back as patches

use std::cmp::Reverse;

use chrono::{DateTime, Utc};

use crate::{
    common::shown_title,
    ids::ThingsId,
    store::{Task, ThingsStore},
};

/// the day group a today index counts in, `tir` when a client set it, otherwise the scheduled day, otherwise today
pub fn today_group(task: &Task, today_ts: i64) -> i64 {
    task.today_index_reference
        .or_else(|| task.start_date.map(|day| day.timestamp()))
        .unwrap_or(today_ts)
}

/// a place in Today's order
///
/// a to-do Today lists by its deadline alone has none
pub fn in_today_order(task: &Task) -> bool {
    task.today_index_reference.is_some() || task.start_date.is_some()
}

/// the order in which the Today view lists its to-dos
///
/// ties fall to the id
pub fn today_view_order(task: &Task) -> (Reverse<i64>, i32, Reverse<i32>, ThingsId) {
    (
        Reverse(task.today_index_reference.unwrap_or(0)),
        task.today_index,
        Reverse(task.index),
        task.uuid.clone(),
    )
}

/// the distance between members of a rebalanced run
pub const STRIDE: i32 = 1024;

/// an index between two neighbors, before the first, after the last, or 0 in an empty run, None when the neighbors are adjacent or the type has no room left
pub fn slot_between(prev: Option<i32>, next: Option<i32>) -> Option<i32> {
    match (prev, next) {
        (None, None) => Some(0),
        (None, Some(next)) => next.checked_sub(1),
        (Some(prev), None) => prev.checked_add(1),
        (Some(prev), Some(next)) => {
            if i64::from(prev) + 1 >= i64::from(next) {
                return None;
            }
            i32::try_from((i64::from(prev) + i64::from(next)) / 2).ok()
        }
    }
}

/// the index of a newcomer at `hole` in `run`, the members in their order with their current indexes, plus the members that move when the run has to be rebalanced
pub fn allocate(run: &[(ThingsId, i32)], hole: usize) -> (i32, Vec<(ThingsId, i32)>) {
    let prev = hole
        .checked_sub(1)
        .and_then(|at| run.get(at))
        .map(|(_, index)| *index);
    let next = run.get(hole).map(|(_, index)| *index);
    if let Some(slot) = slot_between(prev, next) {
        return (slot, Vec::new());
    }
    let target = |position: usize| {
        i32::try_from(position + 1).map_or(i32::MAX, |n| n.saturating_mul(STRIDE))
    };
    let moved = run
        .iter()
        .enumerate()
        .filter_map(|(position, (id, index))| {
            let slot = target(if position < hole {
                position
            } else {
                position + 1
            });
            (*index != slot).then(|| (id.clone(), slot))
        })
        .collect();
    (target(hole), moved)
}

/// every list that shows `row`, each in its own order
///
/// `row` is among the rows when the store holds it
/// a project view shows the to-dos under each heading at every status
/// an area view shows its own to-dos at every status and start
/// Anytime groups its to-dos by container
/// Someday shows its to-dos in one list
/// Upcoming shows the to-dos and projects of a day in one list
/// the projected repeats of that day are among them
/// a repeat template and what is in the Trash show in no list
pub fn lists_showing(store: &ThingsStore, row: &Task, today: &DateTime<Utc>) -> Vec<Vec<Task>> {
    let sorted = |mut rows: Vec<Task>| {
        rows.sort_by(|a, b| (a.index, &a.uuid).cmp(&(b.index, &b.uuid)));
        rows
    };
    let listed = |keep: &dyn Fn(&Task) -> bool| {
        keep(row).then(|| {
            sorted(
                store
                    .tasks_by_uuid
                    .values()
                    .filter(|task| keep(task))
                    .cloned()
                    .collect(),
            )
        })
    };
    let shown = |task: &Task| !store.in_trash(task) && !task.is_recurrence_template();
    let day = |task: &Task| task.start_date.map(|start| start.date_naive());
    let upcoming = store.in_upcoming(row, today).then(|| {
        sorted(
            store
                .tasks_by_uuid
                .values()
                .filter(|task| store.in_upcoming(task, today) && day(task) == day(row))
                .cloned()
                .chain(
                    store
                        .projected_repeats(today.date_naive())
                        .into_iter()
                        .filter(|task| day(task) == day(row)),
                )
                .collect(),
        )
    });
    if row.is_heading() {
        return [listed(&|task| {
            shown(task) && task.is_heading() && task.project == row.project
        })]
        .into_iter()
        .flatten()
        .collect();
    }
    if row.is_project() {
        return [
            listed(&|task| shown(task) && task.is_project() && task.area == row.area),
            listed(&|task| store.in_someday(task) && task.is_project()),
            upcoming,
        ]
        .into_iter()
        .flatten()
        .collect();
    }
    let project = store.effective_project_uuid(row);
    let area = store.effective_area_uuid(row);
    let same_container = |task: &Task| {
        store.effective_project_uuid(task) == project
            && (project.is_some() || store.effective_area_uuid(task) == area)
    };
    let to_do = |task: &Task| !task.is_project() && !task.is_heading();
    [
        listed(&|task| store.in_inbox(task)),
        listed(&|task| store.in_anytime(task, today) && same_container(task)),
        listed(&|task| store.in_someday(task) && to_do(task)),
        (project.is_some() || area.is_some())
            .then(|| {
                listed(&|task| {
                    shown(task)
                        && to_do(task)
                        && same_container(task)
                        && (project.is_none() || task.action_group == row.action_group)
                })
            })
            .flatten(),
        upcoming,
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// the index that puts a row right before or after `anchor` in its own list `own` and in every list of `lists` that holds the anchor, and the rows a rebalance moves
///
/// `own` and `lists` leave out the row being placed
/// a slot between the anchor and its nearest neighbor in all of them moves nothing
/// without one, `own` is rebalanced when it holds every row of those lists
/// the rebalance keeps their order then
/// otherwise the placement is refused
/// it is refused as well when another list that shows a row the rebalance moves holds a row outside `own`
/// `placed` names the row being placed when the store holds it
pub fn place_next_to(
    store: &ThingsStore,
    today: &DateTime<Utc>,
    own: &[Task],
    lists: &[Vec<Task>],
    anchor: &Task,
    placed: Option<&ThingsId>,
    before: bool,
) -> Result<(i32, Vec<(ThingsId, i32)>), String> {
    let at = |list: &[Task]| list.iter().position(|task| task.uuid == anchor.uuid);
    let own_at = at(own).expect("the anchor is among the rows of its list");
    let holding: Vec<&[Task]> = lists
        .iter()
        .map(Vec::as_slice)
        .filter(|list| at(list).is_some())
        .collect();
    let neighbor = |list: &[Task]| {
        let at = at(list)?;
        if before {
            at.checked_sub(1).map(|prev| list[prev].index)
        } else {
            list.get(at + 1).map(|next| next.index)
        }
    };
    let neighbors = std::iter::once(own)
        .chain(holding.iter().copied())
        .filter_map(neighbor);
    let slot = if before {
        slot_between(neighbors.max(), Some(anchor.index))
    } else {
        slot_between(Some(anchor.index), neighbors.min())
    };
    if let Some(slot) = slot {
        return Ok((slot, Vec::new()));
    }
    let in_own = |task: &Task| own.iter().any(|row| row.uuid == task.uuid);
    let run: Vec<(ThingsId, i32)> = own
        .iter()
        .map(|task| (task.uuid.clone(), task.index))
        .collect();
    let (index, moved) = allocate(&run, if before { own_at } else { own_at + 1 });
    // a moved row keeps its place in another list only while that list holds nothing outside `own`
    // Today breaks a tie of day group and today index by `ix`
    let outside = |task: &Task| !in_own(task) && Some(&task.uuid) != placed;
    let today_ts = today.timestamp();
    let reorders = moved
        .iter()
        .filter_map(|(id, _)| own.iter().find(|task| task.uuid == *id))
        .any(|row| {
            lists_showing(store, row, today)
                .iter()
                .any(|list| list.iter().any(outside))
                || (store.in_today(row, today)
                    && store.tasks_by_uuid.values().any(|other| {
                        outside(other)
                            && store.in_today(other, today)
                            && today_group(other, today_ts) == today_group(row, today_ts)
                            && other.today_index == row.today_index
                    }))
        });
    if holding.iter().any(|list| !list.iter().all(in_own)) || reorders {
        return Err(format!(
            "Cannot rebalance next to the anchor without reordering another list: {}",
            shown_title(&anchor.title)
        ));
    }
    Ok((index, moved))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> ThingsId {
        ThingsId::from_u128(n)
    }

    #[test]
    fn a_gap_gives_a_slot_and_adjacent_neighbors_respace_the_run() {
        assert_eq!(slot_between(None, None), Some(0));
        assert_eq!(slot_between(None, Some(1024)), Some(1023));
        assert_eq!(slot_between(Some(1024), None), Some(1025));
        assert_eq!(slot_between(Some(1024), Some(2048)), Some(1536));
        assert_eq!(slot_between(Some(-5), Some(-2)), Some(-3));
        assert_eq!(slot_between(Some(1024), Some(1025)), None);
        assert_eq!(slot_between(Some(7), Some(7)), None);
        assert_eq!(slot_between(Some(i32::MAX), None), None);
        assert_eq!(slot_between(None, Some(i32::MIN)), None);
        assert_eq!(
            slot_between(Some(i32::MAX - 2), Some(i32::MAX)),
            Some(i32::MAX - 1)
        );

        let run = vec![(id(1), 1024), (id(2), 1025), (id(3), 1026)];
        let (slot, moved) = allocate(&run, 1);
        assert_eq!(slot, 2048);
        assert_eq!(moved, vec![(id(2), 3072), (id(3), 4096)]);

        let (slot, moved) = allocate(&run, 3);
        assert_eq!(slot, 1027);
        assert!(moved.is_empty());

        // no room above the last member
        // the whole run is rebalanced
        let (slot, moved) = allocate(&[(id(1), i32::MAX)], 1);
        assert_eq!(slot, 2048);
        assert_eq!(moved, vec![(id(1), 1024)]);
    }
}
