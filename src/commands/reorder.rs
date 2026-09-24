use std::{
    cmp::{Ordering, Reverse},
    collections::BTreeMap,
};

use anyhow::{Context as _, Result};
use chrono::{DateTime, TimeZone, Utc};
use clap::{ArgGroup, Args};

use crate::{
    app::Cli,
    arg_types::IdentifierToken,
    commands::Command,
    common::{DIM, GREEN, ICONS, colored, one_line, shown_title},
    ids::ThingsId,
    ordering::{allocate, in_today_order, today_group, today_view_order},
    wire::{
        task::{TaskPatch, TaskStatus},
        wire_object::{EntityType, WireObject},
    },
};

#[derive(Args)]
#[command(about = "Reorder item relative to another item")]
#[command(group(ArgGroup::new("anchor").args(["before_id", "after_id"]).required(true).multiple(false)))]
pub struct ReorderArgs {
    /// Item ID (or unique ID prefix)
    pub item_id: IdentifierToken,
    #[arg(long, short = 'b', help = "Anchor item ID or prefix to place before")]
    pub before_id: Option<IdentifierToken>,
    #[arg(long, short = 'a', help = "Anchor item ID or prefix to place after")]
    pub after_id: Option<IdentifierToken>,
}

/// the item already sits right next to the anchor in every list of `lists` that holds both
///
/// before it when `before` is set, after it otherwise
/// no list holding both leaves `own`, the item's own list in the order a write keeps, to decide
fn already_in_place(
    lists: &[Vec<&ThingsId>],
    own: &[&ThingsId],
    item: &ThingsId,
    anchor: &ThingsId,
    before: bool,
) -> bool {
    let positions = |order: &[&ThingsId]| {
        let at = |id: &ThingsId| order.iter().position(|entry| *entry == id);
        Some((at(item)?, at(anchor)?))
    };
    let adjacent = |(item_at, anchor_at): (usize, usize)| {
        if before {
            item_at + 1 == anchor_at
        } else {
            anchor_at + 1 == item_at
        }
    };
    let both: Vec<(usize, usize)> = lists.iter().filter_map(|order| positions(order)).collect();
    if both.is_empty() {
        return positions(own).is_some_and(adjacent);
    }
    both.into_iter().all(adjacent)
}

/// every list that shows `item`, each in its own order
///
/// a project view shows the to-dos under each heading at every status
/// an area view shows its own to-dos at every status and start
/// Anytime groups its to-dos by container
/// Someday shows its to-dos in one list
/// a repeat template and what is in the Trash show in no list
fn lists_showing(
    store: &crate::store::ThingsStore,
    item: &crate::store::Task,
    today: &DateTime<Utc>,
) -> Vec<Vec<crate::store::Task>> {
    let shown = |task: &crate::store::Task| !store.in_trash(task) && !task.is_recurrence_template();
    let project = store.effective_project_uuid(item);
    let area = store.effective_area_uuid(item);
    let same_container = |task: &crate::store::Task| {
        store.effective_project_uuid(task) == project
            && (project.is_some() || store.effective_area_uuid(task) == area)
    };
    let rows = |keep: &dyn Fn(&crate::store::Task) -> bool| {
        let mut rows: Vec<crate::store::Task> = store
            .tasks_by_uuid
            .values()
            .filter(|task| shown(task) && keep(task))
            .cloned()
            .collect();
        rows.sort_by(|a, b| (a.index, &a.uuid).cmp(&(b.index, &b.uuid)));
        rows
    };
    if item.is_heading() {
        return vec![rows(&|task| {
            task.is_heading() && task.project == item.project
        })];
    }
    if item.is_project() {
        return vec![
            rows(&|task| task.is_project() && task.area == item.area),
            store
                .someday()
                .into_iter()
                .filter(|task| task.is_project())
                .collect(),
        ];
    }
    let to_do = |task: &crate::store::Task| !task.is_project() && !task.is_heading();
    let mut lists = vec![
        store.inbox(),
        store
            .anytime(today)
            .into_iter()
            .filter(|task| same_container(task))
            .collect(),
        store
            .someday()
            .into_iter()
            .filter(|task| to_do(task))
            .collect(),
    ];
    if project.is_some() || area.is_some() {
        lists.push(rows(&|task| {
            to_do(task)
                && same_container(task)
                && (project.is_none() || task.action_group == item.action_group)
        }));
    }
    lists
}

/// every patch of a reorder in one commit
///
/// a rebalance lands whole or not at all
#[derive(Debug)]
struct ReorderPlan {
    item: crate::store::Task,
    changes: BTreeMap<String, WireObject>,
    reorder_label: String,
    already_in_place: bool,
}

fn build_reorder_plan(
    args: &ReorderArgs,
    store: &crate::store::ThingsStore,
    now: f64,
    today_ts: i64,
) -> std::result::Result<ReorderPlan, String> {
    let today = Utc
        .timestamp_opt(today_ts, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|d| Utc.from_utc_datetime(&d))
        .unwrap_or_else(Utc::now);
    let (item_opt, err, _) = store.resolve_task_identifier(&args.item_id);
    let Some(item) = item_opt else {
        return Err(err);
    };

    let anchor_id = args
        .before_id
        .as_deref()
        .or(args.after_id.as_deref())
        .unwrap_or_default();
    let (anchor_opt, err, _) = store.resolve_task_identifier(anchor_id);
    let Some(anchor) = anchor_opt else {
        return Err(err);
    };

    if item.uuid == anchor.uuid {
        return Err("Cannot reorder an item relative to itself.".to_string());
    }
    // an item or anchor of a kind the writes do not know may sit in a list they cannot see
    // so may one that did not replay completely
    // one that is closed or in the Trash takes no place in a list
    for (role, task) in [("Item", &item), ("Anchor", &anchor)] {
        if let Some(raw) = task.unknown_kind() {
            return Err(format!(
                "{role} is of unknown kind {raw}: {}",
                one_line(&task.title)
            ));
        }
        if !task.entity.can_upgrade_to_task7() {
            return Err(format!(
                "{role} is of kind {}: {}",
                task.entity,
                one_line(&task.title)
            ));
        }
        if task.degraded {
            return Err(format!(
                "{role} did not replay completely: {}",
                one_line(&task.title)
            ));
        }
        if let Some(state) = store.closed_state(task) {
            return Err(format!("{role} is {state}: {}", one_line(&task.title)));
        }
        // a repeat template shows in no list
        if task.is_recurrence_template() {
            return Err(format!(
                "{role} is a repeat template: {}",
                one_line(&task.title)
            ));
        }
    }
    // a heading orders among headings alone
    if item.is_heading() != anchor.is_heading() {
        let (role, heading) = if anchor.is_heading() {
            ("Anchor", &anchor)
        } else {
            ("Item", &item)
        };
        return Err(format!(
            "{role} is a heading: {}",
            shown_title(&heading.title)
        ));
    }

    // only an incomplete to-do that Today lists is ordered within it
    // the others are ordered within their own list
    let is_today_orderable = |task: &crate::store::Task| store.in_today(task, &today);
    let is_today_reorder = is_today_orderable(&item) && is_today_orderable(&anchor);
    // a to-do Today lists by its deadline alone has no place in Today's order to put anything next to
    if is_today_reorder && !in_today_order(&anchor) {
        return Err(format!(
            "Anchor is in Today by its deadline alone, which gives it no place in Today's order: {}",
            one_line(&anchor.title)
        ));
    }

    if is_today_reorder {
        let today_label = |today_ref: i64, today_index: i32| {
            format!(
                "({}={}, today_ref={today_ref}, today_index={today_index})",
                if args.before_id.is_some() {
                    "before"
                } else {
                    "after"
                },
                one_line(&anchor.title)
            )
        };
        // a move to where the item already sits in the Today view writes nothing
        let mut section: Vec<&crate::store::Task> = store
            .tasks_by_uuid
            .values()
            .filter(|task| {
                is_today_orderable(task)
                    && !task.is_heading()
                    && !task.is_blank()
                    && task.evening == anchor.evening
            })
            .collect();
        section.sort_by_key(|task| today_view_order(task));
        let ids: Vec<&ThingsId> = section.iter().map(|task| &task.uuid).collect();
        if already_in_place(
            std::slice::from_ref(&ids),
            &ids,
            &item.uuid,
            &anchor.uuid,
            args.before_id.is_some(),
        ) {
            return Ok(ReorderPlan {
                reorder_label: today_label(today_group(&item, today_ts), item.today_index),
                item,
                changes: BTreeMap::new(),
                already_in_place: true,
            });
        }

        let anchor_tir = today_group(&anchor, today_ts);
        // the item joins the anchor's day group and takes a slot next to the anchor among that group's today indexes
        let mut group: Vec<&crate::store::Task> = store
            .tasks_by_uuid
            .values()
            .filter(|task| {
                task.uuid != item.uuid
                    && is_today_orderable(task)
                    && in_today_order(task)
                    && today_group(task, today_ts) == anchor_tir
            })
            .collect();
        group.sort_by_key(|task| (task.today_index, Reverse(task.index), task.uuid.clone()));
        let anchor_pos = group
            .iter()
            .position(|task| task.uuid == anchor.uuid)
            .ok_or_else(|| "Anchor not found in reorder list.".to_string())?;
        let hole = if args.before_id.is_some() {
            anchor_pos
        } else {
            anchor_pos + 1
        };
        let run: Vec<(ThingsId, i32)> = group
            .iter()
            .map(|task| (task.uuid.clone(), task.today_index))
            .collect();
        let (new_ti, moved) = allocate(&run, hole);
        // a rebalance writes to every row it moves
        // one of a kind the writes do not know is refused as in the structural run
        if !moved.is_empty()
            && let Some(task) = group
                .iter()
                .find(|task| !task.entity.can_upgrade_to_task7() || task.unknown_kind().is_some())
        {
            return Err(match task.unknown_kind() {
                Some(raw) => format!("Cannot rebalance around an item of unknown kind {raw}"),
                None => format!("Cannot rebalance around an item of kind {}", task.entity),
            });
        }

        let sb = if item.evening != anchor.evening {
            Some(if anchor.evening { 1 } else { 0 })
        } else {
            None
        };
        let mut changes = BTreeMap::new();
        for (uuid, today_index) in moved {
            changes.insert(
                uuid.to_string(),
                WireObject::update(
                    EntityType::Task7,
                    TaskPatch {
                        today_sort_index: Some(today_index),
                        modification_date: Some(Some(now)),
                        ..Default::default()
                    },
                ),
            );
        }
        changes.insert(
            item.uuid.to_string(),
            WireObject::update(
                EntityType::Task7,
                TaskPatch {
                    today_index_reference: Some(Some(anchor_tir)),
                    today_sort_index: Some(new_ti),
                    evening_bit: sb,
                    modification_date: Some(Some(now)),
                    ..Default::default()
                },
            ),
        );

        return Ok(ReorderPlan {
            reorder_label: today_label(anchor_tir, new_ti),
            item,
            changes,
            already_in_place: false,
        });
    }

    let bucket = |task: &crate::store::Task| -> Vec<String> {
        if task.is_heading() {
            return vec![
                "heading".to_string(),
                task.project
                    .clone()
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            ];
        }
        if task.is_project() {
            return vec![
                "project".to_string(),
                task.area.clone().map(|v| v.to_string()).unwrap_or_default(),
            ];
        }
        if let Some(project_uuid) = store.effective_project_uuid(task) {
            return vec![
                "task-project".to_string(),
                project_uuid.to_string(),
                task.action_group
                    .clone()
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            ];
        }
        // an area view shows its to-dos of every start in one order
        if let Some(area_uuid) = store.effective_area_uuid(task) {
            return vec!["task-area".to_string(), area_uuid.to_string()];
        }
        vec!["task-root".to_string(), i32::from(task.start).to_string()]
    };

    let item_bucket = bucket(&item);
    let anchor_bucket = bucket(&anchor);
    if item_bucket != anchor_bucket {
        return Err(format!(
            "Anchor is in another list: {}",
            one_line(&anchor.title)
        ));
    }

    // a project or area view lists its to-dos, headings and projects of every status
    // the Inbox, Anytime and Someday list incomplete ones
    // a repeat template shows in no list
    let every_status = matches!(
        item_bucket.first().map(String::as_str),
        Some("task-project" | "task-area" | "heading" | "project")
    );
    let mut siblings = store
        .tasks_by_uuid
        .values()
        .filter(|t| {
            !store.in_trash(t)
                && !t.is_recurrence_template()
                && (every_status || t.status == TaskStatus::Incomplete)
                && bucket(t) == item_bucket
        })
        .cloned()
        .collect::<Vec<_>>();
    siblings.sort_by(|a, b| match a.index.cmp(&b.index) {
        Ordering::Equal => a.uuid.cmp(&b.uuid),
        other => other,
    });

    let structural_label = |index: i32| {
        if args.before_id.is_some() {
            format!("(before={}, index={})", one_line(&anchor.title), index)
        } else {
            format!("(after={}, index={})", one_line(&anchor.title), index)
        }
    };
    // a move to where the item already sits writes nothing
    // it sits there when every list that shows both has no row between them
    let shown = lists_showing(store, &item, &today);
    let lists: Vec<Vec<&ThingsId>> = shown
        .iter()
        .map(|list| list.iter().map(|task| &task.uuid).collect())
        .collect();
    let own: Vec<&ThingsId> = siblings.iter().map(|task| &task.uuid).collect();
    if already_in_place(
        &lists,
        &own,
        &item.uuid,
        &anchor.uuid,
        args.before_id.is_some(),
    ) {
        return Ok(ReorderPlan {
            reorder_label: structural_label(item.index),
            item,
            changes: BTreeMap::new(),
            already_in_place: true,
        });
    }

    let mut order = siblings
        .into_iter()
        .filter(|t| t.uuid != item.uuid)
        .collect::<Vec<_>>();
    let anchor_pos = order.iter().position(|t| t.uuid == anchor.uuid);
    let Some(anchor_pos) = anchor_pos else {
        return Err("Anchor not found in reorder list.".to_string());
    };
    let insert_at = if args.before_id.is_some() {
        anchor_pos
    } else {
        anchor_pos + 1
    };
    order.insert(insert_at, item.clone());
    let run: Vec<(ThingsId, i32)> = order
        .iter()
        .filter(|task| task.uuid != item.uuid)
        .map(|task| (task.uuid.clone(), task.index))
        .collect();
    let (new_index, moved) = allocate(&run, insert_at);
    if !moved.is_empty()
        && let Some(task) = order
            .iter()
            .find(|task| !task.entity.can_upgrade_to_task7() || task.unknown_kind().is_some())
    {
        return Err(match task.unknown_kind() {
            Some(raw) => format!("Cannot rebalance around an item of unknown kind {raw}"),
            None => format!("Cannot rebalance around an item of kind {}", task.entity),
        });
    }
    let mut index_updates: Vec<(String, i32)> = moved
        .into_iter()
        .map(|(uuid, index)| (uuid.to_string(), index))
        .collect();
    if new_index != item.index {
        index_updates.push((item.uuid.to_string(), new_index));
    }

    let mut changes = BTreeMap::new();
    for (task_uuid, task_index) in index_updates {
        changes.insert(
            task_uuid,
            WireObject::update(
                EntityType::Task7,
                TaskPatch {
                    sort_index: Some(task_index),
                    modification_date: Some(Some(now)),
                    ..Default::default()
                },
            ),
        );
    }

    // a plan that writes nothing leaves the item where it sits in its own list
    // another list that shows both still has rows between them
    if changes.is_empty() {
        return Err(format!(
            "Item already sits {} the anchor in its own list, while another list shows rows between them: {}",
            if args.before_id.is_some() {
                "before"
            } else {
                "after"
            },
            one_line(&item.title)
        ));
    }

    Ok(ReorderPlan {
        reorder_label: structural_label(new_index),
        item,
        changes,
        already_in_place: false,
    })
}

impl Command for ReorderArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn std::io::Write,
        ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        let store = cli.load_store()?;
        let plan = build_reorder_plan(self, &store, ctx.now_timestamp(), ctx.today_timestamp())
            .map_err(anyhow::Error::msg)?;

        // a move to where the item already sits writes nothing
        if !plan.changes.is_empty() {
            ctx.commit_changes(plan.changes)
                .with_context(|| "Failed to reorder item")?;
        }

        writeln!(
            out,
            "{} {}  {} {}",
            colored(
                format!(
                    "{} {}",
                    ICONS.done,
                    if plan.already_in_place {
                        "Already in place"
                    } else {
                        "Reordered"
                    }
                ),
                &[GREEN],
                cli.no_color()
            ),
            one_line(&plan.item.title),
            colored(&plan.item.uuid, &[DIM], cli.no_color()),
            colored(&plan.reorder_label, &[DIM], cli.no_color())
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        store::{ThingsStore, fold_items},
        wire::task::{TaskProps, TaskStart, TaskStatus, TaskType},
    };

    const NOW: f64 = 1_700_000_444.0;
    const TASK_A: &str = "A7h5eCi24RvAWKC3Hv3muf";
    const TASK_B: &str = "KGvAPpMrzHAKMdgMiERP1V";
    const TASK_C: &str = "MpkEei6ybkFS2n6SXvwfLf";
    const TODAY: i64 = 1_699_920_000; // 2023-11-14 00:00:00 UTC (midnight)

    fn build_store(entries: Vec<(String, WireObject)>) -> ThingsStore {
        let mut item = BTreeMap::new();
        for (uuid, obj) in entries {
            item.insert(uuid, obj);
        }
        ThingsStore::from_raw_state(&fold_items([item]))
    }

    #[allow(clippy::too_many_arguments)]
    fn task(
        uuid: &str,
        title: &str,
        st: i32,
        ss: i32,
        ix: i32,
        sr: Option<i64>,
        tir: Option<i64>,
        ti: i32,
    ) -> (String, WireObject) {
        task_for_entity(uuid, title, st, ss, ix, sr, tir, ti, EntityType::Task6)
    }

    #[allow(clippy::too_many_arguments)]
    fn task_for_entity(
        uuid: &str,
        title: &str,
        st: i32,
        ss: i32,
        ix: i32,
        sr: Option<i64>,
        tir: Option<i64>,
        ti: i32,
        entity: EntityType,
    ) -> (String, WireObject) {
        (
            uuid.to_string(),
            WireObject::create(
                entity,
                TaskProps {
                    title: title.to_string(),
                    item_type: TaskType::Todo,
                    status: TaskStatus::from(ss),
                    start_location: TaskStart::from(st),
                    sort_index: ix,
                    scheduled_date: sr,
                    today_index_reference: tir,
                    today_sort_index: ti,
                    creation_date: Some(1.0),
                    modification_date: Some(1.0),
                    ..Default::default()
                },
            ),
        )
    }

    #[test]
    fn reorder_before_after_and_today_payloads() {
        let store = build_store(vec![
            task(TASK_A, "A", 0, 0, 1024, None, None, 0),
            task(TASK_B, "B", 0, 0, 2048, None, None, 0),
            task(TASK_C, "C", 0, 0, 3072, None, None, 0),
        ]);

        let before = build_reorder_plan(
            &ReorderArgs {
                item_id: TASK_C.parse().expect("id"),
                before_id: Some(TASK_B.parse().expect("id")),
                after_id: None,
            },
            &store,
            NOW,
            TODAY,
        )
        .expect("before plan");
        assert_eq!(
            serde_json::to_value(before.changes).expect("to value"),
            json!({ TASK_C: {"t":1,"e":"Task7","p":{"ix":1536,"md":NOW}} })
        );

        let store_today = build_store(vec![
            task(TASK_A, "A", 1, 0, 100, Some(TODAY), Some(TODAY), 10),
            task(TASK_B, "B", 1, 0, 200, Some(TODAY), Some(TODAY), 20),
        ]);
        let today_plan = build_reorder_plan(
            &ReorderArgs {
                item_id: TASK_A.parse().expect("id"),
                before_id: None,
                after_id: Some(TASK_B.parse().expect("id")),
            },
            &store_today,
            NOW,
            TODAY,
        )
        .expect("today plan");
        assert_eq!(
            serde_json::to_value(today_plan.changes).expect("to value"),
            json!({ TASK_A: {"t":1,"e":"Task7","p":{"tir":TODAY,"ti":21,"md":NOW}} })
        );
    }

    #[test]
    fn reorder_rebalance_and_errors() {
        let store = build_store(vec![
            task(TASK_A, "A", 0, 0, 1024, None, None, 0),
            task(TASK_B, "B", 0, 0, 1025, None, None, 0),
            task(TASK_C, "C", 0, 0, 1026, None, None, 0),
        ]);
        let rebalance = build_reorder_plan(
            &ReorderArgs {
                item_id: TASK_C.parse().expect("id"),
                before_id: None,
                after_id: Some(TASK_A.parse().expect("id")),
            },
            &store,
            NOW,
            TODAY,
        )
        .expect("rebalance");
        // one commit moves every sibling that changes, whatever head the writer is at
        assert_eq!(
            serde_json::to_value(rebalance.changes).expect("to value"),
            json!({
                TASK_C: {"t":1,"e":"Task7","p":{"ix":2048,"md":NOW}},
                TASK_B: {"t":1,"e":"Task7","p":{"ix":3072,"md":NOW}},
            })
        );

        let future_sibling_store = build_store(vec![
            task(TASK_A, "A", 0, 0, 1024, None, None, 0),
            task_for_entity(
                TASK_B,
                "Future",
                0,
                0,
                1025,
                None,
                None,
                0,
                EntityType::Unknown("Task8".to_string()),
            ),
            task(TASK_C, "C", 0, 0, 1026, None, None, 0),
        ]);
        let future_error = build_reorder_plan(
            &ReorderArgs {
                item_id: TASK_C.parse().expect("id"),
                before_id: None,
                after_id: Some(TASK_A.parse().expect("id")),
            },
            &future_sibling_store,
            NOW,
            TODAY,
        )
        .expect_err("dense reorder around future-version sibling");
        assert_eq!(
            future_error,
            "Cannot rebalance around an item of kind Task8"
        );

        let err = build_reorder_plan(
            &ReorderArgs {
                item_id: TASK_A.parse().expect("id"),
                before_id: Some(TASK_A.parse().expect("id")),
                after_id: None,
            },
            &store,
            NOW,
            TODAY,
        )
        .expect_err("self reorder");
        assert_eq!(err, "Cannot reorder an item relative to itself.");
    }
}
