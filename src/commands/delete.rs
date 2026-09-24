use std::collections::{BTreeMap, HashSet};

use anyhow::Result;
use clap::Args;

use crate::{
    app::Cli,
    arg_types::IdentifierToken,
    commands::Command,
    common::{DIM, GREEN, ICONS, colored, counted, one_line},
    ids::ThingsId,
    store::Task,
    wire::{
        task::TaskPatch,
        wire_object::{EntityType, WireObject},
    },
};

#[derive(Debug, Args)]
#[command(about = "Move to-dos, projects, and headings to the Trash, or delete an area")]
pub struct DeleteArgs {
    /// Item UUID(s) (or unique UUID prefixes)
    #[arg(required = true)]
    pub item_ids: Vec<IdentifierToken>,
}

/// the trash flag on a task, what the app writes when the delete key is pressed
fn trash(now: f64) -> WireObject {
    WireObject::update(
        EntityType::Task7,
        TaskPatch {
            trashed: Some(true),
            modification_date: Some(Some(now)),
            ..Default::default()
        },
    )
}

#[derive(Debug, Clone)]
struct DeletePlan {
    targets: Vec<(String, EntityType, String, usize)>,
    changes: BTreeMap<String, WireObject>,
}

/// every identifier is checked before anything is trashed, a batch lands whole or not at all
fn build_delete_plan(
    args: &DeleteArgs,
    store: &crate::store::ThingsStore,
    now: f64,
) -> std::result::Result<DeletePlan, String> {
    let mut targets: Vec<(String, EntityType, String)> = Vec::new();
    let mut seen = HashSet::new();

    for identifier in &args.item_ids {
        let (task, task_err, task_ambiguous) = store.resolve_task_identifier(identifier.as_str());
        let (area, area_err, area_ambiguous) = store.resolve_area_identifier(identifier.as_str());

        // several candidates of one kind count as a match, the other kind cannot take the identifier
        let task_match = task.is_some() || !task_ambiguous.is_empty();
        let area_match = area.is_some() || !area_ambiguous.is_empty();

        if task_match && area_match {
            return Err(format!(
                "Ambiguous identifier '{}' (matches tasks and areas).",
                identifier.as_str()
            ));
        }

        if task.is_none() && area.is_none() {
            return Err(if !task_ambiguous.is_empty() {
                task_err
            } else if !area_ambiguous.is_empty() {
                area_err
            } else {
                format!("Item not found: {}", identifier.as_str())
            });
        }

        if let Some(task) = task {
            if !task.entity.can_upgrade_to_task7() {
                return Err(format!(
                    "Unsupported task entity for deletion: {}",
                    task.entity
                ));
            }
            if store.in_trash(&task) {
                return Err(format!("Item already deleted: {}", one_line(&task.title)));
            }
            if task.has_repeater() {
                return Err(format!(
                    "Task7 repeater tasks are blocked from deletion until repeater bookkeeping is supported: {}",
                    one_line(&task.title)
                ));
            }
            if !seen.insert(task.uuid.clone()) {
                continue;
            }
            targets.push((task.uuid.to_string(), EntityType::Task7, task.title.clone()));
            continue;
        }

        if let Some(area) = area {
            if !seen.insert(area.uuid.clone()) {
                continue;
            }
            targets.push((area.uuid.to_string(), EntityType::Area3, area.title.clone()));
        }
    }

    // a project takes along everything whose effective project it is, the headings and the to-dos under them included, an area everything whose effective area it is, projects and their contents included, and a heading the to-dos under it, as in the app
    let contents = |parent: &ThingsId, in_area: bool, heading: bool| -> Vec<Task> {
        store
            .tasks_by_uuid
            .values()
            .filter(|task| {
                !store.in_trash(task)
                    && task.uuid != *parent
                    && if in_area {
                        store.effective_area_uuid(task).as_ref() == Some(parent)
                    } else if heading {
                        task.action_group.as_ref() == Some(parent)
                    } else {
                        store.effective_project_uuid(task).as_ref() == Some(parent)
                    }
            })
            .cloned()
            .collect()
    };
    let mut changes = BTreeMap::new();
    let mut counts = Vec::new();
    for (uuid, entity, _title) in &targets {
        let mut taken = 0usize;
        let parent: ThingsId = uuid.parse().expect("resolved id");
        let in_area = *entity == EntityType::Area3;
        if in_area {
            changes.insert(uuid.clone(), WireObject::delete(entity.clone()));
        } else {
            changes.insert(uuid.clone(), trash(now));
        }
        let target = store.tasks_by_uuid.get(&parent);
        let heading = target.is_some_and(Task::is_heading);
        if in_area || heading || target.is_some_and(Task::is_project) {
            for child in contents(&parent, in_area, heading) {
                if child.has_repeater() {
                    return Err(format!(
                        "Task7 repeater tasks are blocked from deletion until repeater bookkeeping is supported: {}",
                        one_line(&child.title)
                    ));
                }
                if changes.insert(child.uuid.to_string(), trash(now)).is_none() {
                    taken += 1;
                }
            }
        }
        counts.push(taken);
    }

    let targets = targets
        .into_iter()
        .zip(counts)
        .map(|((uuid, entity, title), taken)| (uuid, entity, title, taken))
        .collect();
    Ok(DeletePlan { targets, changes })
}

impl Command for DeleteArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn std::io::Write,
        ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        let store = cli.load_store()?;
        let plan =
            build_delete_plan(self, &store, ctx.now_timestamp()).map_err(anyhow::Error::msg)?;

        ctx.commit_changes(plan.changes, None)
            .map_err(|e| anyhow::anyhow!("Failed to delete items: {e}"))?;

        for (uuid, _entity, title, taken) in plan.targets {
            let along = if taken > 0 {
                colored(
                    format!("  (with {})", counted(taken, "item")),
                    &[DIM],
                    cli.no_color(),
                )
            } else {
                String::new()
            };
            writeln!(
                out,
                "{} {}  {}{}",
                colored(
                    format!("{} Deleted", ICONS.deleted),
                    &[GREEN],
                    cli.no_color()
                ),
                one_line(&title),
                colored(&uuid, &[DIM], cli.no_color()),
                along
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::{
        store::{ThingsStore, fold_items},
        wire::{
            area::AreaProps,
            task::{TaskProps, TaskStart, TaskStatus, TaskType},
        },
    };

    const TASK_A: &str = "A7h5eCi24RvAWKC3Hv3muf";
    const TASK_B: &str = "KGvAPpMrzHAKMdgMiERP1V";
    const AREA_A: &str = "MpkEei6ybkFS2n6SXvwfLf";

    struct FailingCtx;

    impl crate::cmd_ctx::CmdCtx for FailingCtx {
        fn now_timestamp(&self) -> f64 {
            1.0
        }

        fn today_timestamp(&self) -> i64 {
            unreachable!()
        }

        fn next_id(&mut self) -> String {
            unreachable!()
        }

        fn current_head_index(&self) -> i64 {
            unreachable!()
        }

        fn commit_changes(
            &mut self,
            _changes: BTreeMap<String, WireObject>,
            _ancestor_index: Option<i64>,
        ) -> Result<i64> {
            anyhow::bail!("cloud unavailable")
        }
    }

    #[test]
    fn delete_propagates_commit_failure_without_success_output() {
        let journal = tempfile::NamedTempFile::new().expect("journal file");
        let item = BTreeMap::from([task(TASK_A, "Alpha", false)]);
        serde_json::to_writer(&journal, &vec![item]).expect("write journal");
        let cli = Cli::parse_from([
            "things",
            "--load-journal",
            journal.path().to_str().expect("journal path"),
        ]);
        let args = DeleteArgs {
            item_ids: vec![IdentifierToken::from(TASK_A)],
        };
        let mut out = Vec::new();

        let error = args
            .run_with_ctx(&cli, &mut out, &mut FailingCtx)
            .expect_err("failed commit must fail the command");

        assert_eq!(
            error.to_string(),
            "Failed to delete items: cloud unavailable"
        );
        assert!(out.is_empty());
    }

    fn build_store(entries: Vec<(String, WireObject)>) -> ThingsStore {
        let mut item = BTreeMap::new();
        for (uuid, obj) in entries {
            item.insert(uuid, obj);
        }
        ThingsStore::from_raw_state(&fold_items([item]))
    }

    fn task(uuid: &str, title: &str, trashed: bool) -> (String, WireObject) {
        (
            uuid.to_string(),
            WireObject::create(
                EntityType::Task6,
                TaskProps {
                    title: title.to_string(),
                    item_type: TaskType::Todo,
                    status: TaskStatus::Incomplete,
                    start_location: TaskStart::Inbox,
                    sort_index: 0,
                    trashed,
                    creation_date: Some(1.0),
                    modification_date: Some(1.0),
                    ..Default::default()
                },
            ),
        )
    }

    fn area(uuid: &str, title: &str) -> (String, WireObject) {
        (
            uuid.to_string(),
            WireObject::create(
                EntityType::Area3,
                AreaProps {
                    title: title.to_string(),
                    sort_index: 0,
                    ..Default::default()
                },
            ),
        )
    }

    #[test]
    fn delete_payloads_match_snapshot_cases() {
        let single = build_delete_plan(
            &DeleteArgs {
                item_ids: vec![IdentifierToken::from(TASK_A)],
            },
            &build_store(vec![task(TASK_A, "Alpha", false)]),
            1.0,
        )
        .expect("plan");
        assert_eq!(
            serde_json::to_value(single.changes).expect("to value"),
            serde_json::json!({ TASK_A: {"t":1,"e":"Task7","p":{"md":1.0,"tr":true}} })
        );

        let multi = build_delete_plan(
            &DeleteArgs {
                item_ids: vec![IdentifierToken::from(TASK_A), IdentifierToken::from(AREA_A)],
            },
            &build_store(vec![task(TASK_A, "Alpha", false), area(AREA_A, "Work")]),
            1.0,
        )
        .expect("plan");
        assert_eq!(
            serde_json::to_value(multi.changes).expect("to value"),
            serde_json::json!({
                TASK_A: {"t":1,"e":"Task7","p":{"md":1.0,"tr":true}},
                AREA_A: {"t":2,"e":"Area3","p":{}}
            })
        );

        // the heading links the to-do to the project, there is no direct project field on it
        let heading = "JFdhhhp37fpryAKu8UXwzK";
        let via_heading = "74rgJf6Qh9wYp2TcVk8mNB";
        let project_id = "By8mN2qRk5Wv7Xc9Dt3HpL";
        let cascade = build_delete_plan(
            &DeleteArgs {
                item_ids: vec![IdentifierToken::from(project_id)],
            },
            &build_store(vec![
                (
                    project_id.to_string(),
                    WireObject::create(
                        EntityType::Task7,
                        TaskProps {
                            title: "Remodel".to_string(),
                            item_type: TaskType::Project,
                            creation_date: Some(1.0),
                            ..Default::default()
                        },
                    ),
                ),
                (
                    heading.to_string(),
                    WireObject::create(
                        EntityType::Task7,
                        TaskProps {
                            title: "Plumbing".to_string(),
                            item_type: TaskType::Heading,
                            parent_project_ids: vec![project_id.parse().expect("id")],
                            creation_date: Some(1.0),
                            ..Default::default()
                        },
                    ),
                ),
                (
                    via_heading.to_string(),
                    WireObject::create(
                        EntityType::Task7,
                        TaskProps {
                            title: "Order sink".to_string(),
                            action_group_ids: vec![heading.parse().expect("id")],
                            creation_date: Some(1.0),
                            ..Default::default()
                        },
                    ),
                ),
                task(TASK_A, "Unrelated", false),
            ]),
            1.0,
        )
        .expect("plan");
        assert_eq!(cascade.targets[0].3, 2);
        assert_eq!(
            cascade.changes.keys().cloned().collect::<Vec<_>>(),
            vec![via_heading, project_id, heading]
        );
        // the heading alone takes the to-dos under it
        let heading_only = build_delete_plan(
            &DeleteArgs {
                item_ids: vec![IdentifierToken::from(heading)],
            },
            &build_store(vec![
                (
                    heading.to_string(),
                    WireObject::create(
                        EntityType::Task7,
                        TaskProps {
                            title: "Plumbing".to_string(),
                            item_type: TaskType::Heading,
                            creation_date: Some(1.0),
                            ..Default::default()
                        },
                    ),
                ),
                (
                    via_heading.to_string(),
                    WireObject::create(
                        EntityType::Task7,
                        TaskProps {
                            title: "Order sink".to_string(),
                            action_group_ids: vec![heading.parse().expect("id")],
                            creation_date: Some(1.0),
                            ..Default::default()
                        },
                    ),
                ),
            ]),
            1.0,
        )
        .expect("plan");
        assert_eq!(
            heading_only.changes.keys().cloned().collect::<Vec<_>>(),
            vec![via_heading, heading]
        );

        // one bad target fails the batch before anything is trashed
        let trashed_target = build_delete_plan(
            &DeleteArgs {
                item_ids: vec![IdentifierToken::from(TASK_A), IdentifierToken::from(TASK_B)],
            },
            &build_store(vec![
                task(TASK_A, "Active", false),
                task(TASK_B, "Trashed", true),
            ]),
            1.0,
        )
        .expect_err("a trashed target");
        assert_eq!(trashed_target, "Item already deleted: Trashed");

        // a prefix of two to-dos and one area names no single item
        let ambiguous = build_delete_plan(
            &DeleteArgs {
                item_ids: vec![IdentifierToken::from("Ab")],
            },
            &build_store(vec![
                task("Ab11111111111111111111", "Groceries", false),
                task("Ab21111111111111111111", "Laundry", false),
                area("Ab31111111111111111111", "Finance"),
            ]),
            1.0,
        )
        .expect_err("an ambiguous prefix");
        assert!(
            ambiguous.starts_with("Ambiguous identifier 'Ab'"),
            "{ambiguous}"
        );
    }
}
