use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use iocraft::prelude::*;

use crate::{
    app::Cli,
    arg_types::IdentifierToken,
    commands::{Command, TagDeltaArgs, detailed_json_conflict, write_json},
    common::{
        DIM, GREEN, ICONS, colored, day_timestamp, one_line, parse_day, resolve_removable_tag_ids,
        resolve_tag_ids, task6_note,
    },
    ids::ThingsId,
    ui::{
        render_element_to_string,
        views::{
            json::common::build_tasks_json,
            projects::{ProjectsAreaGroup, ProjectsView},
        },
    },
    wire::{
        notes::{StructuredTaskNotes, TaskNotes},
        task::{TaskPatch, TaskProps, TaskStart, TaskStatus, TaskType},
        wire_object::{EntityType, WireObject},
    },
};

#[derive(Debug, Subcommand)]
pub enum ProjectsSubcommand {
    #[command(about = "Show all active projects")]
    List(ProjectsListArgs),
    #[command(about = "Create a new project")]
    New(ProjectsNewArgs),
    #[command(about = "Edit a project title, notes, area, or tags")]
    Edit(ProjectsEditArgs),
}

#[derive(Debug, Args)]
#[command(about = "Show, create, or edit projects")]
pub struct ProjectsArgs {
    /// Show notes for each project
    #[arg(long, short = 'd')]
    pub detailed: bool,
    #[command(subcommand)]
    pub command: Option<ProjectsSubcommand>,
}

#[derive(Debug, Default, Args)]
pub struct ProjectsListArgs {
    /// Show notes for each project
    #[arg(long, short = 'd')]
    pub detailed: bool,
}

#[derive(Debug, Args)]
pub struct ProjectsNewArgs {
    /// Project title
    pub title: String,
    #[arg(long, short = 'a', help = "Area ID or prefix to place the project in")]
    pub area: Option<IdentifierToken>,
    #[arg(
        long,
        short = 'w',
        help = "When: anytime (default), someday, today, or YYYY-MM-DD"
    )]
    pub when: Option<String>,
    #[arg(long, short = 'n', default_value = "", help = "Project notes")]
    pub notes: String,
    #[arg(
        long,
        short = 't',
        help = "Comma-separated tags (titles or ID prefixes)"
    )]
    pub tags: Option<String>,
    #[arg(long = "deadline", short = 'd', help = "Deadline date (YYYY-MM-DD)")]
    pub deadline_date: Option<String>,
}

#[derive(Debug, Args)]
pub struct ProjectsEditArgs {
    /// Project ID (or unique ID prefix)
    pub project_id: IdentifierToken,
    #[arg(long, short = 't', help = "Replace title")]
    pub title: Option<String>,
    #[arg(
        long = "move",
        short = 'm',
        help = "Move to clear or area ID or prefix"
    )]
    pub move_target: Option<IdentifierToken>,
    #[arg(long, short = 'n', help = "Replace notes (use empty string to clear)")]
    pub notes: Option<String>,
    #[command(flatten)]
    pub tag_delta: TagDeltaArgs,
}

#[derive(Debug, Clone)]
struct ProjectsEditPlan {
    project: crate::store::Task,
    update: TaskPatch,
    labels: Vec<String>,
}

fn build_projects_edit_plan(
    args: &ProjectsEditArgs,
    store: &crate::store::ThingsStore,
    now: f64,
) -> std::result::Result<ProjectsEditPlan, String> {
    let (project_opt, err, _) = store.resolve_mark_identifier(&args.project_id);
    let Some(project) = project_opt else {
        return Err(err);
    };
    if !project.is_project() {
        return Err("The specified ID is not a project.".to_string());
    }

    let mut update = TaskPatch::default();
    let mut labels: Vec<String> = Vec::new();

    if let Some(title) = &args.title {
        let title = title.trim();
        if title.is_empty() {
            return Err("Project title cannot be empty.".to_string());
        }
        update.title = Some(title.to_string());
        labels.push("title".to_string());
    }

    if let Some(notes) = &args.notes {
        update.notes = Some(if notes.is_empty() {
            TaskNotes::Structured(StructuredTaskNotes {
                object_type: Some("tx".to_string()),
                format_type: 1,
                ch: Some(0),
                v: Some(String::new()),
                ps: Vec::new(),
                unknown_fields: Default::default(),
            })
        } else {
            task6_note(notes)
        });
        labels.push("notes".to_string());
    }

    if let Some(move_target) = &args.move_target {
        let move_raw = &**move_target;
        let move_l = move_raw.to_lowercase();
        if move_l == "inbox" {
            return Err("Projects cannot be moved to Inbox.".to_string());
        }
        if move_l == "clear" {
            update.area_ids = Some(vec![]);
            labels.push("move=clear".to_string());
        } else {
            let (item, _, item_candidates) = store.resolve_task_identifier(move_raw);
            let (area, _, area_candidates) = store.resolve_area_identifier(move_raw);
            if !item_candidates.is_empty() {
                return Err(format!(
                    "Ambiguous --move target '{move_raw}' (matches several items)."
                ));
            }
            if !area_candidates.is_empty() {
                return Err(format!(
                    "Ambiguous --move target '{move_raw}' (matches several areas)."
                ));
            }
            match (item, area) {
                (Some(_), Some(_)) => {
                    return Err(format!(
                        "Ambiguous --move target '{move_raw}' (matches an item and an area)."
                    ));
                }
                // a project, heading or to-do is no place for a project, wherever it is
                (Some(_), None) => {
                    return Err("Projects can only be moved to an area or clear.".to_string());
                }
                (None, Some(area)) => {
                    update.area_ids = Some(vec![area.uuid]);
                    labels.push(format!("move={move_raw}"));
                }
                (None, None) => return Err(format!("Container not found: {move_raw}")),
            }
        }
    }

    let mut current_tags = project.tags.clone();
    if let Some(add_tags) = &args.tag_delta.add_tags {
        let (ids, err) = resolve_tag_ids(store, add_tags);
        if !err.is_empty() {
            return Err(err);
        }
        for id in ids {
            if !current_tags.iter().any(|t| t == &id) {
                current_tags.push(id);
            }
        }
        labels.push("add-tags".to_string());
    }
    if let Some(remove_tags) = &args.tag_delta.remove_tags {
        let (ids, err) = resolve_removable_tag_ids(store, remove_tags);
        if !err.is_empty() {
            return Err(err);
        }
        current_tags.retain(|t| !ids.iter().any(|id| id == t));
        labels.push("remove-tags".to_string());
    }
    if args.tag_delta.add_tags.is_some() || args.tag_delta.remove_tags.is_some() {
        update.tag_ids = Some(current_tags);
    }

    if update.is_empty() {
        return Err("No edit changes requested.".to_string());
    }

    update.modification_date = Some(Some(now));

    Ok(ProjectsEditPlan {
        project,
        update,
        labels,
    })
}

impl Command for ProjectsArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn std::io::Write,
        ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        // `projects --detailed` and `projects list --detailed` show details
        // `projects --detailed list` does not
        // the subcommand's own flag wins over the one before it
        let effective_detailed = match self.command.as_ref() {
            None => self.detailed,
            Some(ProjectsSubcommand::List(la)) => la.detailed,
            _ => false,
        };
        let effective_json = match self.command.as_ref() {
            None | Some(ProjectsSubcommand::List(_)) => cli.json,
            _ => false,
        };

        match &self.command {
            None | Some(ProjectsSubcommand::List(_)) => {
                let store = Arc::new(cli.load_store()?);
                let today = ctx.today();
                let projects = store.projects(Some(TaskStatus::Incomplete));

                if effective_json {
                    detailed_json_conflict(effective_json, effective_detailed)?;
                    write_json(out, &build_tasks_json(&projects, &store, &today))?;
                    return Ok(());
                }

                let mut by_area: BTreeMap<Option<ThingsId>, Vec<_>> = BTreeMap::new();
                for p in &projects {
                    by_area.entry(p.area.clone()).or_default().push(p.clone());
                }

                let mut id_scope = projects.iter().map(|p| p.uuid.clone()).collect::<Vec<_>>();
                id_scope.extend(by_area.keys().flatten().cloned());
                let id_prefix_len = store.unique_prefix_length(&id_scope);

                let no_area = by_area.remove(&None).unwrap_or_default();

                // areas in their own order, the one the app shows
                let mut area_entries: Vec<(ThingsId, Vec<_>)> = by_area
                    .into_iter()
                    .filter_map(|(k, v)| k.map(|uuid| (uuid, v)))
                    .collect();
                area_entries.sort_by_key(|(uuid, _)| {
                    store
                        .areas_by_uuid
                        .get(uuid)
                        .map(|a| a.index)
                        .unwrap_or(i32::MAX)
                });

                let area_groups = area_entries
                    .into_iter()
                    .map(|(area_uuid, area_projects)| ProjectsAreaGroup {
                        area_title: store.resolve_area_title(&area_uuid),
                        area_uuid,
                        projects: area_projects,
                    })
                    .collect::<Vec<_>>();

                let mut ui = element! {
                        ContextProvider(value: Context::owned(store.clone())) {
                            ContextProvider(value: Context::owned(today)) {
                                ProjectsView(
                                projects_count: projects.len(),
                                no_area_projects: no_area,
                                area_groups,
                                detailed: effective_detailed,
                                id_prefix_len,
                            )
                        }
                    }
                };
                let rendered = render_element_to_string(&mut ui, cli.no_color());
                writeln!(out, "{}", rendered)?;
            }
            Some(ProjectsSubcommand::New(args)) => {
                let title = args.title.trim();
                if title.is_empty() {
                    bail!("Project title cannot be empty.");
                }

                let store = cli.load_store()?;
                let now = ctx.now_timestamp();
                let mut props = TaskProps {
                    title: title.to_string(),
                    notes: Some(task6_note(&args.notes)),
                    item_type: TaskType::Project,
                    status: TaskStatus::Incomplete,
                    start_location: TaskStart::Anytime,
                    sort_index: 0,
                    conflict_overrides: Some(serde_json::json!({"_t": "oo", "sn": {}})),
                    creation_date: Some(now),
                    modification_date: Some(now),
                    ..Default::default()
                };

                if let Some(area_id) = &args.area {
                    let (area_opt, err, _) = store.resolve_area_identifier(area_id);
                    let Some(area) = area_opt else {
                        bail!("{err}");
                    };
                    props.area_ids = vec![area.uuid];
                }

                if let Some(when_raw) = &args.when {
                    let when = when_raw.trim().to_lowercase();
                    if when == "anytime" {
                        props.start_location = TaskStart::Anytime;
                    } else if when == "someday" {
                        props.start_location = TaskStart::Someday;
                    } else if when == "today" {
                        let ts = ctx.today_timestamp();
                        props.start_location = TaskStart::Anytime;
                        props.scheduled_date = Some(ts);
                        props.today_index_reference = Some(ts);
                    } else {
                        let Ok(day) = parse_day(when_raw.trim(), "--when") else {
                            bail!(
                                "Invalid --when: {} (expected anytime, someday, today or YYYY-MM-DD)",
                                one_line(when_raw)
                            );
                        };
                        let ts = day_timestamp(day);
                        // a day that has come puts the project in Anytime
                        // the app does the same
                        props.start_location = if ts <= ctx.today_timestamp() {
                            TaskStart::Anytime
                        } else {
                            TaskStart::Someday
                        };
                        props.scheduled_date = Some(ts);
                        props.today_index_reference = Some(ts);
                    }
                }

                if let Some(tags) = &args.tags {
                    let (tag_ids, err) = resolve_tag_ids(&store, tags);
                    if !err.is_empty() {
                        bail!("{err}");
                    }
                    props.tag_ids = tag_ids;
                }

                if let Some(deadline) = &args.deadline_date {
                    let day = parse_day(deadline, "--deadline").map_err(anyhow::Error::msg)?;
                    props.deadline = Some(day_timestamp(day));
                }

                let uuid = ctx.next_id();

                let mut changes = BTreeMap::new();
                changes.insert(uuid.clone(), WireObject::create(EntityType::Task7, props));
                ctx.commit_changes(changes)
                    .with_context(|| "Failed to create project")?;

                writeln!(
                    out,
                    "{} {}  {}",
                    colored(format!("{} Created", ICONS.done), &[GREEN], cli.no_color()),
                    one_line(title),
                    colored(&uuid, &[DIM], cli.no_color())
                )?;
            }
            Some(ProjectsSubcommand::Edit(args)) => {
                let store = cli.load_store()?;
                let plan = build_projects_edit_plan(args, &store, ctx.now_timestamp())
                    .map_err(anyhow::Error::msg)?;

                let mut changes = BTreeMap::new();
                changes.insert(
                    plan.project.uuid.to_string(),
                    WireObject::update(EntityType::Task7, plan.update.clone()),
                );
                ctx.commit_changes(changes)
                    .with_context(|| "Failed to edit project")?;

                let title = plan.update.title.as_deref().unwrap_or(&plan.project.title);
                writeln!(
                    out,
                    "{} {}  {} {}",
                    colored(format!("{} Edited", ICONS.done), &[GREEN], cli.no_color()),
                    one_line(title),
                    colored(&plan.project.uuid, &[DIM], cli.no_color()),
                    colored(
                        format!("({})", plan.labels.join(", ")),
                        &[DIM],
                        cli.no_color()
                    )
                )?;
            }
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
            area::AreaProps,
            tags::TagProps,
            task::{TaskProps, TaskStart, TaskStatus, TaskType},
            wire_object::{EntityType, WireItem, WireObject},
        },
    };

    const NOW: f64 = 1_700_000_222.0;
    const PROJECT_UUID: &str = "KGvAPpMrzHAKMdgMiERP1V";

    fn build_store(entries: Vec<(String, WireObject)>) -> ThingsStore {
        let mut item: WireItem = BTreeMap::new();
        for (uuid, obj) in entries {
            item.insert(uuid, obj);
        }
        ThingsStore::from_raw_state(&fold_items([item]))
    }

    fn project(uuid: &str, title: &str, tags: Vec<&str>) -> (String, WireObject) {
        (
            uuid.to_string(),
            WireObject::create(
                EntityType::Task6,
                TaskProps {
                    title: title.to_string(),
                    item_type: TaskType::Project,
                    status: TaskStatus::Incomplete,
                    start_location: TaskStart::Anytime,
                    sort_index: 0,
                    tag_ids: tags
                        .iter()
                        .map(|t| {
                            t.parse::<ThingsId>()
                                .expect("test tag id should parse as ThingsId")
                        })
                        .collect(),
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

    fn tag(uuid: &str, title: &str) -> (String, WireObject) {
        (
            uuid.to_string(),
            WireObject::create(
                EntityType::Tag4,
                TagProps {
                    title: title.to_string(),
                    sort_index: 0,
                    ..Default::default()
                },
            ),
        )
    }

    #[test]
    fn projects_edit_payload_variants() {
        let target_area_uuid = "JFdhhhp37fpryAKu8UXwzK";
        let store = build_store(vec![
            project(PROJECT_UUID, "Roadmap", vec![]),
            area(target_area_uuid, "Personal"),
        ]);

        let title_plan = build_projects_edit_plan(
            &ProjectsEditArgs {
                project_id: PROJECT_UUID.parse().expect("id"),
                title: Some("Roadmap v2".to_string()),
                move_target: None,
                notes: None,
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
            },
            &store,
            NOW,
        )
        .expect("title plan");
        let p = serde_json::to_value(&title_plan.update).expect("patch");
        assert_eq!(p.get("tt"), Some(&json!("Roadmap v2")));
        assert_eq!(p.get("md"), Some(&json!(NOW)));

        let clear_plan = build_projects_edit_plan(
            &ProjectsEditArgs {
                project_id: PROJECT_UUID.parse().expect("id"),
                title: None,
                move_target: Some("clear".parse().expect("id")),
                notes: None,
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
            },
            &store,
            NOW,
        )
        .expect("clear plan");
        assert_eq!(
            serde_json::to_value(&clear_plan.update)
                .expect("patch")
                .get("ar"),
            Some(&json!([]))
        );

        let move_plan = build_projects_edit_plan(
            &ProjectsEditArgs {
                project_id: PROJECT_UUID.parse().expect("id"),
                title: None,
                move_target: Some(target_area_uuid.parse().expect("id")),
                notes: None,
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
            },
            &store,
            NOW,
        )
        .expect("move area plan");
        assert_eq!(
            serde_json::to_value(&move_plan.update)
                .expect("patch")
                .get("ar"),
            Some(&json!([target_area_uuid]))
        );
    }

    #[test]
    fn projects_edit_tags_and_errors() {
        let tag1 = "WukwpDdL5Z88nX3okGMKTC";
        let tag2 = "JiqwiDaS3CAyjCmHihBDnB";
        let store = build_store(vec![
            project(PROJECT_UUID, "Roadmap", vec![tag1, tag2]),
            tag(tag1, "Work"),
            tag(tag2, "Focus"),
        ]);

        let remove_plan = build_projects_edit_plan(
            &ProjectsEditArgs {
                project_id: PROJECT_UUID.parse().expect("id"),
                title: None,
                move_target: None,
                notes: None,
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: Some("Work".to_string()),
                },
            },
            &store,
            NOW,
        )
        .expect("remove tags");
        assert_eq!(
            serde_json::to_value(&remove_plan.update)
                .expect("patch")
                .get("tg"),
            Some(&json!([tag2]))
        );

        let no_change = build_projects_edit_plan(
            &ProjectsEditArgs {
                project_id: PROJECT_UUID.parse().expect("id"),
                title: None,
                move_target: None,
                notes: None,
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
            },
            &store,
            NOW,
        )
        .expect_err("no changes");
        assert_eq!(no_change, "No edit changes requested.");

        let inbox = build_projects_edit_plan(
            &ProjectsEditArgs {
                project_id: PROJECT_UUID.parse().expect("id"),
                title: None,
                move_target: Some("inbox".parse().expect("id")),
                notes: None,
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
            },
            &store,
            NOW,
        )
        .expect_err("cannot move inbox");
        assert_eq!(inbox, "Projects cannot be moved to Inbox.");
    }
}
