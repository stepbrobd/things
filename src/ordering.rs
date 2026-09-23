//! sort index allocation shared by new and reorder, for the structural index within a container and the today index within a day's group
//! a slot between two neighbors when the gap allows one, otherwise the run is respaced and the members that move come back as patches

use crate::{ids::ThingsId, store::Task};

/// the day group a today index counts in, `tir` when a client set it, otherwise the scheduled day, otherwise today
pub fn today_group(task: &Task, today_ts: i64) -> i64 {
    task.today_index_reference
        .or_else(|| task.start_date.map(|day| day.timestamp()))
        .unwrap_or(today_ts)
}

/// the distance between members of a respaced run
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

/// the index of a newcomer at `hole` in `run`, the members in their order with their current indexes, plus the members that move when the run has to be respaced
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

        // no room above the last member, the whole run respaces
        let (slot, moved) = allocate(&[(id(1), i32::MAX)], 1);
        assert_eq!(slot, 2048);
        assert_eq!(moved, vec![(id(1), 1024)]);
    }
}
