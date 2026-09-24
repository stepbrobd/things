use std::collections::{BTreeMap, HashSet};

use anyhow::{Context as _, Result, bail};
use clap::{ArgGroup, Args};

use crate::{
    app::Cli,
    arg_types::IdentifierToken,
    commands::Command,
    common::{DIM, GREEN, ICONS, colored, one_line},
    wire::{
        checklist::ChecklistItemPatch,
        recurrence::RecurrenceType,
        task::{TaskPatch, TaskStatus},
        wire_object::{EntityType, WireObject},
    },
};

#[derive(Args)]
#[command(about = "Mark a task completed, incomplete, or canceled")]
#[command(group(ArgGroup::new("status").args(["done", "incomplete", "canceled", "check_ids", "uncheck_ids", "check_cancel_ids"]).required(true).multiple(false)))]
pub struct MarkArgs {
    /// Task ID(s) (or unique ID prefixes)
    #[arg(required = true)]
    pub task_ids: Vec<IdentifierToken>,
    #[arg(long, short = 'd', help = "Mark task(s) as completed")]
    pub done: bool,
    #[arg(long, short = 'i', help = "Mark task(s) as incomplete")]
    pub incomplete: bool,
    #[arg(long, short = 'c', help = "Mark task(s) as canceled")]
    pub canceled: bool,
    #[arg(
        long = "check",
        short = 'k',
        help = "Mark checklist items completed by comma-separated ID prefixes"
    )]
    pub check_ids: Option<String>,
    #[arg(
        long = "uncheck",
        short = 'u',
        help = "Mark checklist items incomplete by comma-separated ID prefixes"
    )]
    pub uncheck_ids: Option<String>,
    #[arg(
        long = "check-cancel",
        short = 'x',
        help = "Mark checklist items canceled by comma-separated ID prefixes"
    )]
    pub check_cancel_ids: Option<String>,
}

fn resolve_checklist_items(
    task: &crate::store::Task,
    raw_ids: &str,
) -> (Vec<crate::store::ChecklistItem>, String) {
    let tokens = raw_ids
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return (Vec::new(), "No checklist item IDs provided.".to_string());
    }

    let mut resolved = Vec::new();
    let mut seen = HashSet::new();
    for token in tokens {
        let matches = task
            .checklist_items
            .iter()
            .filter(|item| item.uuid.starts_with(token))
            .cloned()
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return (Vec::new(), format!("Checklist item not found: '{token}'"));
        }
        if matches.len() > 1 {
            return (
                Vec::new(),
                format!("Ambiguous checklist item prefix: '{token}'"),
            );
        }
        let item = matches[0].clone();
        if seen.insert(item.uuid.clone()) {
            resolved.push(item);
        }
    }

    (resolved, String::new())
}

/// a status change of a repeat's instance
///
/// the app follows it with template bookkeeping for after completion rules
fn validate_recurring_instance(
    task: &crate::store::Task,
    store: &crate::store::ThingsStore,
) -> (bool, String) {
    if task.recurrence_templates.len() != 1 {
        return (
            false,
            format!(
                "This repeating item names {} templates where one is expected.",
                task.recurrence_templates.len()
            ),
        );
    }

    // the store keeps a template id while the state holds the template
    // one that did not replay completely is no task
    let Some(template) = store.get_task(&task.recurrence_templates[0].to_string()) else {
        return (
            false,
            "The template of this repeating item did not replay completely.".to_string(),
        );
    };

    let Some(rr) = template.recurrence_rule else {
        return (
            false,
            "The template of this repeating item has no repeat rule.".to_string(),
        );
    };

    match rr.recurrence_type {
        RecurrenceType::FixedSchedule => (true, String::new()),
        RecurrenceType::AfterCompletion => (
            false,
            "Only an Apple client completes or cancels an item that repeats after completion."
                .to_string(),
        ),
        RecurrenceType::Unknown(v) => (
            false,
            format!("The template has a repeat type the CLI does not know: rr.tp={v:?}."),
        ),
    }
}

fn validate_mark_target(
    task: &crate::store::Task,
    action: &str,
    store: &crate::store::ThingsStore,
) -> String {
    if task.has_repeater() {
        return "Cannot change the status of an item with a repeater, whose bookkeeping the Apple clients keep."
            .to_string();
    }
    if task.is_recurrence_template() {
        return "A repeat template takes no status, mark one of its instances instead.".to_string();
    }
    if matches!(task.status, TaskStatus::Unknown(_)) {
        return "Item is of an unknown status.".to_string();
    }
    if action == "completed" && task.status == TaskStatus::Completed {
        return "Item is already completed.".to_string();
    }
    if action == "incomplete" && task.status == TaskStatus::Incomplete {
        return "Item is already incomplete.".to_string();
    }
    if action == "canceled" && task.status == TaskStatus::Canceled {
        return "Item is already canceled.".to_string();
    }
    if task.is_recurrence_instance() {
        let (ok, reason) = validate_recurring_instance(task, store);
        if !ok {
            return reason;
        }
    }
    String::new()
}

struct MarkCommitPlan {
    changes: BTreeMap<String, WireObject>,
}

fn build_mark_status_plan(
    args: &MarkArgs,
    store: &crate::store::ThingsStore,
    now: f64,
) -> (MarkCommitPlan, Vec<crate::store::Task>, Vec<String>) {
    let action = if args.done {
        "completed"
    } else if args.incomplete {
        "incomplete"
    } else {
        "canceled"
    };

    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    let mut errors = Vec::new();
    for identifier in &args.task_ids {
        let (task_opt, err, _) = store.resolve_mark_identifier(identifier.as_str());
        let Some(task) = task_opt else {
            errors.push(err);
            continue;
        };
        if !seen.insert(task.uuid.clone()) {
            continue;
        }
        targets.push(task);
    }

    let mut updates = Vec::new();
    let mut successes = Vec::new();

    for task in targets {
        let validation_error = validate_mark_target(&task, action, store);
        if !validation_error.is_empty() {
            errors.push(format!("{} ({})", validation_error, one_line(&task.title)));
            continue;
        }

        let (task_status, stop_date) = if action == "completed" {
            (TaskStatus::Completed, Some(now))
        } else if action == "incomplete" {
            (TaskStatus::Incomplete, None)
        } else {
            (TaskStatus::Canceled, Some(now))
        };

        // a project closes with its incomplete to-dos
        // those to-dos take its status as in the app once it asks
        if task.is_project() && action != "incomplete" {
            let mut open = store
                .tasks_by_uuid
                .values()
                .filter(|child| {
                    child.is_todo()
                        && !store.in_trash(child)
                        && child.status == TaskStatus::Incomplete
                        && !child.is_recurrence_template()
                        && !seen.contains(&child.uuid)
                        && store.effective_project_uuid(child).as_ref() == Some(&task.uuid)
                })
                .cloned()
                .collect::<Vec<_>>();
            open.sort_by(|a, b| (a.index, &a.uuid).cmp(&(b.index, &b.uuid)));
            for child in open {
                let validation_error = validate_mark_target(&child, action, store);
                if !validation_error.is_empty() {
                    errors.push(format!(
                        "{} ({}, in {})",
                        validation_error,
                        one_line(&child.title),
                        one_line(&task.title)
                    ));
                    continue;
                }
                seen.insert(child.uuid.clone());
                updates.push((child.uuid.clone(), task_status, stop_date));
                successes.push(child);
            }
        }

        updates.push((task.uuid.clone(), task_status, stop_date));
        successes.push(task);
    }

    let mut changes = BTreeMap::new();
    for (uuid, status, stop_date) in updates {
        changes.insert(
            uuid.to_string(),
            WireObject::update(
                EntityType::Task7,
                TaskPatch {
                    status: Some(status),
                    stop_date: Some(stop_date),
                    modification_date: Some(Some(now)),
                    ..Default::default()
                },
            ),
        );
    }

    (MarkCommitPlan { changes }, successes, errors)
}

fn build_mark_checklist_plan(
    args: &MarkArgs,
    task: &crate::store::Task,
    checklist_raw: &str,
    now: f64,
) -> std::result::Result<(MarkCommitPlan, Vec<crate::store::ChecklistItem>, String), String> {
    let (items, err) = resolve_checklist_items(task, checklist_raw);
    if !err.is_empty() {
        return Err(err);
    }

    let (label, status): (&str, TaskStatus) = if args.check_ids.is_some() {
        ("checked", TaskStatus::Completed)
    } else if args.uncheck_ids.is_some() {
        ("unchecked", TaskStatus::Incomplete)
    } else {
        ("canceled", TaskStatus::Canceled)
    };

    let mut changes = BTreeMap::new();
    for item in &items {
        if matches!(item.status, TaskStatus::Unknown(_)) {
            return Err(format!(
                "Checklist item is of an unknown status: {}",
                one_line(&item.title)
            ));
        }
        if item.status == status {
            return Err(format!(
                "Checklist item is already {label}: {}",
                one_line(&item.title)
            ));
        }
        // a closed item carries the moment it closed
        // a to-do does too
        let stop_date = (status != TaskStatus::Incomplete).then_some(now);
        changes.insert(
            item.uuid.to_string(),
            WireObject::update(
                EntityType::ChecklistItem3,
                ChecklistItemPatch {
                    status: Some(status),
                    stop_date: Some(stop_date),
                    modification_date: Some(now),
                    ..Default::default()
                },
            ),
        );
    }

    Ok((MarkCommitPlan { changes }, items, label.to_string()))
}

impl Command for MarkArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn std::io::Write,
        ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        let store = cli.load_store()?;
        let checklist_raw = self
            .check_ids
            .as_ref()
            .or(self.uncheck_ids.as_ref())
            .or(self.check_cancel_ids.as_ref());

        if let Some(checklist_raw) = checklist_raw {
            // ids that name one to-do count once
            let mut tasks: Vec<crate::store::Task> = Vec::new();
            for identifier in &self.task_ids {
                let (task_opt, err, _) = store.resolve_mark_identifier(identifier.as_str());
                let Some(task) = task_opt else {
                    bail!("{err}");
                };
                if !tasks.iter().any(|seen| seen.uuid == task.uuid) {
                    tasks.push(task);
                }
            }
            if tasks.len() != 1 {
                bail!(
                    "Checklist flags (--check, --uncheck, --check-cancel) require exactly one to-do ID."
                );
            }
            let task = tasks.remove(0);

            if task.checklist_items.is_empty() {
                bail!("Item has no checklist: {}", one_line(&task.title));
            }

            let (plan, items, label) =
                build_mark_checklist_plan(self, &task, checklist_raw, ctx.now_timestamp())
                    .map_err(anyhow::Error::msg)?;

            ctx.commit_changes(plan.changes)
                .with_context(|| "Failed to mark checklist items")?;

            let title = match label.as_str() {
                "checked" => format!("{} Checked", ICONS.checklist_done),
                "unchecked" => format!("{} Unchecked", ICONS.checklist_open),
                _ => format!("{} Canceled", ICONS.checklist_canceled),
            };

            for item in items {
                writeln!(
                    out,
                    "{} {}  {}",
                    colored(&title, &[GREEN], cli.no_color()),
                    one_line(&item.title),
                    colored(&item.uuid, &[DIM], cli.no_color())
                )?;
            }
            return Ok(());
        }

        let action = if self.done {
            "completed"
        } else if self.incomplete {
            "incomplete"
        } else {
            "canceled"
        };

        let (plan, successes, errors) = build_mark_status_plan(self, &store, ctx.now_timestamp());
        // every target is checked before any is written
        // a batch lands whole or not at all
        if !errors.is_empty() {
            bail!("{}", errors.join("\n"));
        }

        ctx.commit_changes(plan.changes)
            .with_context(|| format!("Failed to mark items {action}"))?;

        let label = match action {
            "completed" => format!("{} Completed", ICONS.done),
            "incomplete" => format!("{} Incomplete", ICONS.incomplete),
            _ => format!("{} Canceled", ICONS.canceled),
        };
        for task in successes {
            writeln!(
                out,
                "{} {}  {}",
                colored(&label, &[GREEN], cli.no_color()),
                one_line(&task.title),
                colored(&task.uuid, &[DIM], cli.no_color())
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        ids::ThingsId,
        store::{ThingsStore, fold_items},
        wire::{
            checklist::ChecklistItemProps,
            recurrence::{RecurrenceRule, RecurrenceType},
            task::{TaskProps, TaskStart, TaskStatus, TaskType},
        },
    };

    const NOW: f64 = 1_700_000_111.0;
    const TASK_A: &str = "A7h5eCi24RvAWKC3Hv3muf";
    const CHECK_A: &str = "MpkEei6ybkFS2n6SXvwfLf";
    const CHECK_B: &str = "JFdhhhp37fpryAKu8UXwzK";
    const TPL_A: &str = "MpkEei6ybkFS2n6SXvwfLf";
    const TPL_B: &str = "JFdhhhp37fpryAKu8UXwzK";

    fn build_store(entries: Vec<(String, WireObject)>) -> ThingsStore {
        let mut item = BTreeMap::new();
        for (uuid, obj) in entries {
            item.insert(uuid, obj);
        }
        ThingsStore::from_raw_state(&fold_items([item]))
    }

    fn task(uuid: &str, title: &str, status: i32) -> (String, WireObject) {
        task_for_entity(uuid, title, status, EntityType::Task6)
    }

    fn task_for_entity(
        uuid: &str,
        title: &str,
        status: i32,
        entity: EntityType,
    ) -> (String, WireObject) {
        (
            uuid.to_string(),
            WireObject::create(
                entity,
                TaskProps {
                    title: title.to_string(),
                    item_type: TaskType::Todo,
                    status: TaskStatus::from(status),
                    start_location: TaskStart::Inbox,
                    sort_index: 0,
                    creation_date: Some(1.0),
                    modification_date: Some(1.0),
                    ..Default::default()
                },
            ),
        )
    }

    fn task_with_props(
        uuid: &str,
        title: &str,
        recurrence_rule: Option<RecurrenceRule>,
        recurrence_templates: Vec<&str>,
    ) -> (String, WireObject) {
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
                    recurrence_rule,
                    recurrence_template_ids: recurrence_templates
                        .iter()
                        .map(|t| {
                            t.parse::<ThingsId>()
                                .expect("test recurrence template id should parse")
                        })
                        .collect(),
                    creation_date: Some(1.0),
                    modification_date: Some(1.0),
                    ..Default::default()
                },
            ),
        )
    }

    fn checklist(uuid: &str, task_uuid: &str, title: &str, ix: i32) -> (String, WireObject) {
        (
            uuid.to_string(),
            WireObject::create(
                EntityType::ChecklistItem3,
                ChecklistItemProps {
                    title: title.to_string(),
                    task_ids: vec![
                        task_uuid
                            .parse::<ThingsId>()
                            .expect("test task id should parse as ThingsId"),
                    ],
                    status: TaskStatus::Incomplete,
                    sort_index: ix,
                    creation_date: Some(1.0),
                    modification_date: Some(1.0),
                    ..Default::default()
                },
            ),
        )
    }

    #[test]
    fn an_instance_whose_template_did_not_replay_is_refused() {
        // an object without a kind is held and read as no task
        let template: WireObject =
            serde_json::from_str(r#"{"t":0,"p":{"tt":"Water plants"}}"#).expect("template");
        let store = build_store(vec![
            (TPL_A.to_string(), template),
            task_with_props(TASK_A, "Water plants", None, vec![TPL_A]),
        ]);
        let task = store.get_task(TASK_A).expect("instance");
        let (valid, message) = validate_recurring_instance(&task, &store);
        assert!(!valid);
        assert_eq!(
            message,
            "The template of this repeating item did not replay completely."
        );
    }

    #[test]
    fn mark_status_payloads() {
        let done_store = build_store(vec![task(TASK_A, "Alpha", 0)]);
        let (done_plan, _, errs) = build_mark_status_plan(
            &MarkArgs {
                task_ids: vec![TASK_A.parse().expect("id")],
                done: true,
                incomplete: false,
                canceled: false,
                check_ids: None,
                uncheck_ids: None,
                check_cancel_ids: None,
            },
            &done_store,
            NOW,
        );
        assert!(errs.is_empty());
        assert_eq!(
            serde_json::to_value(done_plan.changes).expect("to value"),
            json!({ TASK_A: {"t":1,"e":"Task7","p":{"ss":3,"sp":NOW,"md":NOW}} })
        );

        let incomplete_store = build_store(vec![task(TASK_A, "Alpha", 3)]);
        let (incomplete_plan, _, _) = build_mark_status_plan(
            &MarkArgs {
                task_ids: vec![TASK_A.parse().expect("id")],
                done: false,
                incomplete: true,
                canceled: false,
                check_ids: None,
                uncheck_ids: None,
                check_cancel_ids: None,
            },
            &incomplete_store,
            NOW,
        );
        assert_eq!(
            serde_json::to_value(incomplete_plan.changes).expect("to value"),
            json!({ TASK_A: {"t":1,"e":"Task7","p":{"ss":0,"sp":null,"md":NOW}} })
        );
    }

    #[test]
    fn mark_accepts_task7_and_rejects_opaque_repeaters() {
        let task7_store = build_store(vec![task_for_entity(
            TASK_A,
            "Current task",
            0,
            EntityType::Task7,
        )]);
        let args = MarkArgs {
            task_ids: vec![TASK_A.parse().expect("id")],
            done: true,
            incomplete: false,
            canceled: false,
            check_ids: None,
            uncheck_ids: None,
            check_cancel_ids: None,
        };
        let (plan, _, errors) = build_mark_status_plan(&args, &task7_store, NOW);
        assert!(errors.is_empty());
        assert_eq!(plan.changes[TASK_A].entity_type, Some(EntityType::Task7));

        let repeating = (
            TASK_A.to_string(),
            WireObject::create(
                EntityType::Task7,
                TaskProps {
                    title: "Repeater".to_string(),
                    repeater: Some(json!({"version": 1})),
                    ..TaskProps::default()
                },
            ),
        );
        let (plan, _, errors) = build_mark_status_plan(&args, &build_store(vec![repeating]), NOW);
        assert!(plan.changes.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("an item with a repeater"));
    }

    #[test]
    fn mark_checklist_payloads() {
        let store = build_store(vec![
            task(TASK_A, "Task with checklist", 0),
            checklist(CHECK_A, TASK_A, "One", 1),
            checklist(CHECK_B, TASK_A, "Two", 2),
        ]);
        let task = store.get_task(TASK_A).expect("task");

        let (checked_plan, _, _) = build_mark_checklist_plan(
            &MarkArgs {
                task_ids: vec![TASK_A.parse().expect("id")],
                done: false,
                incomplete: false,
                canceled: false,
                check_ids: Some(format!("{},{}", &CHECK_A[..6], &CHECK_B[..6])),
                uncheck_ids: None,
                check_cancel_ids: None,
            },
            &task,
            &format!("{},{}", &CHECK_A[..6], &CHECK_B[..6]),
            NOW,
        )
        .expect("checked plan");
        assert_eq!(
            serde_json::to_value(checked_plan.changes).expect("to value"),
            json!({
                CHECK_A: {"t":1,"e":"ChecklistItem3","p":{"ss":3,"sp":NOW,"md":NOW}},
                CHECK_B: {"t":1,"e":"ChecklistItem3","p":{"ss":3,"sp":NOW,"md":NOW}}
            })
        );
    }

    #[test]
    fn a_closed_project_takes_its_open_to_dos_along() {
        const PROJECT: &str = "Pj11111111111111111111";
        const HEADING: &str = "Hd11111111111111111111";
        const OPEN: &str = "Ae11111111111111111111";
        const UNDER: &str = "Un11111111111111111111";
        const DONE: &str = "Dn11111111111111111111";
        let object = |title: &str,
                      item_type: TaskType,
                      status: TaskStatus,
                      project: Option<&str>,
                      heading: Option<&str>| {
            WireObject::create(
                EntityType::Task7,
                TaskProps {
                    title: title.to_string(),
                    item_type,
                    status,
                    parent_project_ids: project
                        .map(|id| id.parse().expect("id"))
                        .into_iter()
                        .collect(),
                    action_group_ids: heading
                        .map(|id| id.parse().expect("id"))
                        .into_iter()
                        .collect(),
                    ..Default::default()
                },
            )
        };
        let store = |project: TaskStatus| {
            build_store(vec![
                (
                    PROJECT.to_string(),
                    object("Remodel", TaskType::Project, project, None, None),
                ),
                (
                    HEADING.to_string(),
                    object(
                        "Plumbing",
                        TaskType::Heading,
                        TaskStatus::Incomplete,
                        Some(PROJECT),
                        None,
                    ),
                ),
                (
                    OPEN.to_string(),
                    object(
                        "Buy tiles",
                        TaskType::Todo,
                        TaskStatus::Incomplete,
                        Some(PROJECT),
                        None,
                    ),
                ),
                (
                    UNDER.to_string(),
                    object(
                        "Order sink",
                        TaskType::Todo,
                        TaskStatus::Incomplete,
                        None,
                        Some(HEADING),
                    ),
                ),
                (
                    DONE.to_string(),
                    object(
                        "Measure",
                        TaskType::Todo,
                        TaskStatus::Completed,
                        Some(PROJECT),
                        None,
                    ),
                ),
            ])
        };
        let status = |done: bool, incomplete: bool| MarkArgs {
            task_ids: vec![PROJECT.parse().expect("id")],
            done,
            incomplete,
            canceled: !done && !incomplete,
            check_ids: None,
            uncheck_ids: None,
            check_cancel_ids: None,
        };
        let (plan, _, errors) =
            build_mark_status_plan(&status(false, false), &store(TaskStatus::Incomplete), NOW);
        assert!(errors.is_empty(), "{errors:?}");
        let mut keys = plan.changes.keys().cloned().collect::<Vec<_>>();
        keys.sort();
        let mut expected = vec![OPEN.to_string(), PROJECT.to_string(), UNDER.to_string()];
        expected.sort();
        assert_eq!(keys, expected);
        assert_eq!(plan.changes[UNDER].properties_map()["ss"], json!(2));
        // reopening a project leaves its to-dos as they are
        // the app does the same
        let (plan, _, errors) =
            build_mark_status_plan(&status(false, true), &store(TaskStatus::Completed), NOW);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(plan.changes.keys().collect::<Vec<_>>(), vec![PROJECT]);
    }

    #[test]
    fn mark_recurring_rejection_cases() {
        let store = build_store(vec![task_with_props(
            TASK_A,
            "Recurring template",
            Some(RecurrenceRule {
                recurrence_type: RecurrenceType::FixedSchedule,
                ..Default::default()
            }),
            vec![],
        )]);
        let status = |done: bool| MarkArgs {
            task_ids: vec![TASK_A.parse().expect("id")],
            done,
            incomplete: false,
            canceled: !done,
            check_ids: None,
            uncheck_ids: None,
            check_cancel_ids: None,
        };
        // a template takes no status, canceled no more than completed
        for done in [true, false] {
            let (plan, _, errs) = build_mark_status_plan(&status(done), &store, NOW);
            assert!(plan.changes.is_empty());
            assert_eq!(
                errs,
                vec![
                    "A repeat template takes no status, mark one of its instances instead. (Recurring template)"
                ]
            );
        }

        // both templates exist
        // an instance of two is malformed
        let template = |uuid| {
            task_with_props(
                uuid,
                "Recurring template",
                Some(RecurrenceRule {
                    recurrence_type: RecurrenceType::FixedSchedule,
                    ..Default::default()
                }),
                vec![],
            )
        };
        let store = build_store(vec![
            template(TPL_A),
            template(TPL_B),
            task_with_props(TASK_A, "Recurring instance", None, vec![TPL_A, TPL_B]),
        ]);
        for done in [true, false] {
            let (_, _, errs) = build_mark_status_plan(&status(done), &store, NOW);
            assert_eq!(
                errs,
                vec![
                    "This repeating item names 2 templates where one is expected. (Recurring instance)"
                ]
            );
        }
    }
}
