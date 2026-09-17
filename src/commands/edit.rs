use std::{
    collections::{BTreeMap, HashMap, HashSet},
    str::FromStr,
};

use anyhow::Result;
use clap::Args;
use serde_json::json;

use crate::{
    app::Cli,
    arg_types::IdentifierToken,
    commands::{Command, TagDeltaArgs},
    common::{
        DIM, GREEN, ICONS, colored, day_of, day_timestamp, parse_day, parse_instant,
        parse_reminder, resolve_tag_ids, task6_note,
    },
    ids::ThingsId,
    repeat::{Bound, ChecklistCopy, RepeatSpec, TemplateSource, bound, checklist_items, template},
    store::Task,
    wire::{
        checklist::{ChecklistItemPatch, ChecklistItemProps},
        notes::{StructuredTaskNotes, TaskNotes},
        task::{TaskPatch, TaskStart, TaskStatus},
        wire_object::{EntityType, WireObject},
    },
};

#[derive(Debug, Args)]
#[command(
    about = "Edit a task: title, notes, container, tags, checklist, when, deadline, reminder or repeat"
)]
pub struct EditArgs {
    #[arg(help = "Task UUID(s) (or unique UUID prefixes)")]
    pub task_ids: Vec<IdentifierToken>,
    #[arg(long, short = 't', help = "Replace title (single task only)")]
    pub title: Option<String>,
    #[arg(
        long,
        short = 'n',
        help = "Replace notes (single task only; use empty string to clear)"
    )]
    pub notes: Option<String>,
    #[arg(
        long = "move",
        short = 'm',
        help = "Move to Inbox, clear, project UUID/prefix, or area UUID/prefix"
    )]
    pub move_target: Option<String>,
    #[command(flatten)]
    pub tag_delta: TagDeltaArgs,
    #[arg(
        long = "add-checklist",
        short = 'c',
        value_name = "TITLE",
        help = "Add a checklist item (repeatable, single task only)"
    )]
    pub add_checklist: Vec<String>,
    #[arg(
        long = "remove-checklist",
        short = 'x',
        value_name = "IDS",
        help = "Remove checklist items by comma-separated short IDs (single task only)"
    )]
    pub remove_checklist: Option<String>,
    #[arg(
        long = "rename-checklist",
        short = 'k',
        value_name = "ID:TITLE",
        help = "Rename a checklist item: short-id:new title (repeatable, single task only)"
    )]
    pub rename_checklist: Vec<String>,
    #[arg(
        long = "completed-on",
        value_name = "DATETIME",
        help = "Set when a completed task was completed (single task only, RFC 3339 or YYYY-MM-DD)"
    )]
    pub completed_on: Option<String>,
    #[arg(
        long = "created-on",
        value_name = "DATETIME",
        help = "Set when the task was created (single task only, RFC 3339 or YYYY-MM-DD)"
    )]
    pub created_on: Option<String>,
    #[arg(
        long,
        short = 'w',
        help = "When: anytime, today, evening, someday, or YYYY-MM-DD"
    )]
    pub when: Option<String>,
    #[arg(long = "deadline", short = 'd', help = "Deadline date (YYYY-MM-DD)")]
    pub deadline_date: Option<String>,
    #[arg(long = "clear-deadline", short = 'D', help = "Clear deadline")]
    pub clear_deadline: bool,
    #[arg(
        long = "reminder",
        value_name = "HH:MM",
        help = "Reminder time on the scheduled day (HH:MM)"
    )]
    pub reminder: Option<String>,
    #[arg(long = "clear-reminder", help = "Clear reminder")]
    pub clear_reminder: bool,
    #[arg(
        long = "repeat",
        value_name = "RULE",
        help = "Repeat: daily, weekly[:mon,thu], monthly[:15|last], yearly[:MM-DD] or after:2w, with /N for every N (single task only)"
    )]
    pub repeat: Option<String>,
    #[arg(
        long = "times",
        value_name = "N",
        help = "End the repeat after N times"
    )]
    pub times: Option<i32>,
    #[arg(
        long = "until",
        value_name = "YYYY-MM-DD",
        help = "End the repeat on a day"
    )]
    pub until: Option<String>,
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

#[derive(Debug, Clone)]
struct EditPlan {
    tasks: Vec<crate::store::Task>,
    changes: BTreeMap<String, WireObject>,
    labels: Vec<String>,
}

/// the transition to an undated to-do, which clears the day, the place in Today, the evening flag and the reminder, whether a move to the Inbox or a when of anytime or someday brought it about
fn unschedule(update: &mut TaskPatch, task: &Task) {
    update.scheduled_date = Some(None);
    update.today_index_reference = Some(None);
    update.evening_bit = Some(0);
    if task.alarm_time_offset.is_some() {
        update.alarm_time_offset = Some(None);
    }
}

/// when, deadline, reminder and repeat, the fields the app's popovers edit
///
/// `checklist` is the to-do's checklist as this edit leaves it, which a new
/// template copies
#[allow(clippy::too_many_arguments)]
fn apply_schedule(
    args: &EditArgs,
    task: &Task,
    update: &mut TaskPatch,
    changes: &mut BTreeMap<String, WireObject>,
    labels: &mut Vec<String>,
    checklist: &[ChecklistCopy],
    now: f64,
    today_ts: i64,
    next_id: &mut dyn FnMut() -> String,
) -> std::result::Result<(), String> {
    let scheduling = args.when.is_some()
        || args.deadline_date.is_some()
        || args.clear_deadline
        || args.reminder.is_some()
        || args.clear_reminder
        || args.repeat.is_some();
    if !scheduling {
        if args.times.is_some() || args.until.is_some() {
            return Err("--times and --until need --repeat".to_string());
        }
        return Ok(());
    }
    if task.has_repeater() {
        return Err(
            "Task7 repeater tasks are blocked from scheduling until repeater bookkeeping is supported."
                .to_string(),
        );
    }
    let mut label = |text: String| {
        if !labels.contains(&text) {
            labels.push(text);
        }
    };

    if let Some(when_raw) = &args.when {
        let when = when_raw.trim();
        let when_l = when.to_lowercase();
        if when_l == "anytime" || when_l == "someday" {
            update.start_location = Some(if when_l == "anytime" {
                TaskStart::Anytime
            } else {
                TaskStart::Someday
            });
            unschedule(update, task);
            label(format!("when={when_l}"));
        } else if when_l == "today" || when_l == "evening" {
            update.start_location = Some(TaskStart::Anytime);
            update.scheduled_date = Some(Some(today_ts));
            update.today_index_reference = Some(Some(today_ts));
            update.evening_bit = Some(if when_l == "evening" { 1 } else { 0 });
            label(format!("when={when_l}"));
        } else {
            let when_day = match parse_day(Some(when), "--when") {
                Ok(Some(day)) => day,
                Ok(None) => {
                    return Err(
                        "--when requires anytime, someday, today, evening, or YYYY-MM-DD"
                            .to_string(),
                    );
                }
                Err(e) => return Err(e),
            };
            let day_ts = day_timestamp(when_day);
            update.start_location = Some(if day_ts <= today_ts {
                TaskStart::Anytime
            } else {
                TaskStart::Someday
            });
            update.scheduled_date = Some(Some(day_ts));
            update.today_index_reference = Some(Some(day_ts));
            update.evening_bit = Some(0);
            label(format!("when={when}"));
        }
    }

    if let Some(deadline) = &args.deadline_date {
        let day = match parse_day(Some(deadline), "--deadline") {
            Ok(Some(day)) => day,
            Ok(None) => return Err("--deadline requires YYYY-MM-DD".to_string()),
            Err(e) => return Err(e),
        };
        update.deadline = Some(Some(day_timestamp(day) as f64));
        label(format!("deadline={deadline}"));
    }
    if args.clear_deadline {
        update.deadline = Some(None);
        label("deadline=none".to_string());
    }

    if let Some(reminder) = &args.reminder {
        let dated = match update.scheduled_date {
            Some(day) => day.is_some(),
            None => task.start_date.is_some(),
        };
        if !dated {
            return Err(
                "--reminder requires a scheduled day, set --when today or YYYY-MM-DD".to_string(),
            );
        }
        update.alarm_time_offset = Some(Some(parse_reminder(reminder)?));
        label(format!("reminder={reminder}"));
    }
    if args.clear_reminder {
        update.alarm_time_offset = Some(None);
        label("reminder=none".to_string());
    }

    if let Some(rule_text) = &args.repeat {
        if task.is_recurrence_template() || task.is_recurrence_instance() {
            return Err("This to-do already repeats.".to_string());
        }
        let spec: RepeatSpec = rule_text.parse()?;
        let bound = bound(args.times, args.until.as_deref())?;
        let when_day = match update.scheduled_date {
            Some(Some(day)) => day_of(day),
            Some(None) => None,
            None => task.start_date.map(|day| day.date_naive()),
        };
        let Some(when_day) = when_day else {
            return Err(
                "--repeat requires a scheduled day, set --when today or YYYY-MM-DD".to_string(),
            );
        };
        let first = spec.first_occurrence(when_day);
        if first != when_day {
            return Err(format!(
                "{when_day} is not a day of {rule_text}, the next one is {first}, set --when {first}"
            ));
        }
        if let Bound::Until(until) = bound
            && until < first
        {
            return Err(format!(
                "--until {until} is before the first occurrence {first}"
            ));
        }
        let deadline = match update.deadline {
            Some(deadline) => deadline.is_some(),
            None => task.deadline.is_some(),
        };
        if deadline {
            return Err(
                "A repeating to-do keeps its deadline as an offset the CLI does not write yet, clear the deadline before adding --repeat."
                    .to_string(),
            );
        }
        let rule = spec.rule(first, day_of(today_ts).expect("today"), bound);
        label(format!(
            "repeat={}",
            rule.human_readable().unwrap_or_else(|_| rule_text.clone())
        ));
        // the template mirrors the to-do as this edit leaves it, not as it was
        let source = TemplateSource {
            title: update.title.clone().unwrap_or_else(|| task.title.clone()),
            notes: update
                .notes
                .clone()
                .or_else(|| task.notes.as_deref().map(task6_note)),
            tag_ids: update.tag_ids.clone().unwrap_or_else(|| task.tags.clone()),
            parent_project_ids: update
                .parent_project_ids
                .clone()
                .unwrap_or_else(|| task.project.iter().cloned().collect()),
            area_ids: update
                .area_ids
                .clone()
                .unwrap_or_else(|| task.area.iter().cloned().collect()),
            action_group_ids: update
                .action_group_ids
                .clone()
                .unwrap_or_else(|| task.action_group.iter().cloned().collect()),
            alarm_time_offset: match update.alarm_time_offset {
                Some(alarm) => alarm,
                None => task.alarm_time_offset,
            },
            sort_index: task.index,
            today_sort_index: task.today_index,
            conflict_overrides: Some(json!({"_t": "oo", "sn": {}})),
            checklist: checklist.to_vec(),
        };
        let template_uuid = next_id();
        let template_id = ThingsId::from_str(&template_uuid).map_err(|e| e.to_string())?;
        update.recurrence_template_ids = Some(vec![template_id.clone()]);
        changes.insert(
            template_uuid,
            WireObject::create(EntityType::Task7, template(&spec, rule, first, source, now)),
        );
        changes.extend(checklist_items(checklist, &template_id, now, next_id));
    } else if args.times.is_some() || args.until.is_some() {
        return Err("--times and --until need --repeat".to_string());
    }
    Ok(())
}

impl Command for EditArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn std::io::Write,
        ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        let store = cli.load_store()?;
        let now = ctx.now_timestamp();
        let today = ctx.today_timestamp();
        let mut id_gen = || ctx.next_id();
        let plan = match build_edit_plan(self, &store, now, today, &mut id_gen) {
            Ok(plan) => plan,
            Err(err) => {
                eprintln!("{err}");
                return Ok(());
            }
        };

        if let Err(e) = ctx.commit_changes(plan.changes.clone(), None) {
            eprintln!("Failed to edit item: {e}");
            return Ok(());
        }

        let label_str = colored(
            format!("({})", plan.labels.join(", ")),
            &[DIM],
            cli.no_color(),
        );
        for task in plan.tasks {
            let title_display = plan
                .changes
                .get(&task.uuid.to_string())
                .and_then(|obj| obj.properties_map().get("tt").cloned())
                .and_then(|v| v.as_str().map(ToString::to_string))
                .unwrap_or(task.title);
            writeln!(
                out,
                "{} {}  {} {}",
                colored(format!("{} Edited", ICONS.done), &[GREEN], cli.no_color()),
                title_display,
                colored(&task.uuid, &[DIM], cli.no_color()),
                label_str
            )?;
        }

        Ok(())
    }
}

fn build_edit_plan(
    args: &EditArgs,
    store: &crate::store::ThingsStore,
    now: f64,
    today_ts: i64,
    next_id: &mut dyn FnMut() -> String,
) -> std::result::Result<EditPlan, String> {
    let multiple = args.task_ids.len() > 1;
    if multiple && args.title.is_some() {
        return Err("--title requires a single task ID.".to_string());
    }
    if multiple && args.notes.is_some() {
        return Err("--notes requires a single task ID.".to_string());
    }
    if multiple && (args.completed_on.is_some() || args.created_on.is_some()) {
        return Err("--completed-on/--created-on require a single task ID.".to_string());
    }
    if multiple && args.repeat.is_some() {
        return Err("--repeat requires a single task ID.".to_string());
    }
    if multiple
        && (!args.add_checklist.is_empty()
            || args.remove_checklist.is_some()
            || !args.rename_checklist.is_empty())
    {
        return Err(
            "--add-checklist/--remove-checklist/--rename-checklist require a single task ID."
                .to_string(),
        );
    }

    let mut tasks = Vec::new();
    for identifier in &args.task_ids {
        let (task_opt, err, _) = store.resolve_mark_identifier(identifier.as_str());
        let Some(task) = task_opt else {
            return Err(err);
        };
        if task.is_project() {
            return Err("Use 'projects edit' to edit a project.".to_string());
        }
        tasks.push(task);
    }

    let mut shared_update = TaskPatch::default();
    let mut move_from_inbox_st: Option<TaskStart> = None;
    let mut labels: Vec<String> = Vec::new();
    let move_raw = args.move_target.clone().unwrap_or_default();
    let move_l = move_raw.to_lowercase();

    if !move_raw.trim().is_empty() {
        if move_l == "inbox" {
            if args.when.is_some() || args.reminder.is_some() || args.repeat.is_some() {
                return Err(
                    "--move inbox cannot be combined with --when, --reminder or --repeat, an Inbox to-do has no day."
                        .to_string(),
                );
            }
            shared_update.parent_project_ids = Some(vec![]);
            shared_update.area_ids = Some(vec![]);
            shared_update.action_group_ids = Some(vec![]);
            shared_update.start_location = Some(TaskStart::Inbox);
            labels.push("move=inbox".to_string());
        } else if move_l == "clear" {
            labels.push("move=clear".to_string());
        } else {
            let (project_opt, _, _) = store.resolve_mark_identifier(&move_raw);
            let (area_opt, _, _) = store.resolve_area_identifier(&move_raw);

            let project_uuid = project_opt.as_ref().and_then(|p| {
                if p.is_project() {
                    Some(p.uuid.clone())
                } else {
                    None
                }
            });
            let area_uuid = area_opt.as_ref().map(|a| a.uuid.clone());

            if project_uuid.is_some() && area_uuid.is_some() {
                return Err(format!(
                    "Ambiguous --move target '{}' (matches project and area).",
                    move_raw
                ));
            }
            if project_opt.is_some() && project_uuid.is_none() {
                return Err(
                    "--move target must be Inbox, clear, a project ID, or an area ID.".to_string(),
                );
            }

            if let Some(project_uuid) = project_uuid {
                let project_id = project_uuid;
                shared_update.parent_project_ids = Some(vec![project_id]);
                shared_update.area_ids = Some(vec![]);
                shared_update.action_group_ids = Some(vec![]);
                move_from_inbox_st = Some(TaskStart::Anytime);
                labels.push(format!("move={move_raw}"));
            } else if let Some(area_uuid) = area_uuid {
                let area_id = area_uuid;
                shared_update.area_ids = Some(vec![area_id]);
                shared_update.parent_project_ids = Some(vec![]);
                shared_update.action_group_ids = Some(vec![]);
                move_from_inbox_st = Some(TaskStart::Anytime);
                labels.push(format!("move={move_raw}"));
            } else {
                return Err(format!("Container not found: {move_raw}"));
            }
        }
    }

    let mut add_tag_ids = Vec::new();
    let mut remove_tag_ids = Vec::new();
    if let Some(raw) = &args.tag_delta.add_tags {
        let (ids, err) = resolve_tag_ids(store, raw);
        if !err.is_empty() {
            return Err(err);
        }
        add_tag_ids = ids;
        labels.push("add-tags".to_string());
    }
    if let Some(raw) = &args.tag_delta.remove_tags {
        let (ids, err) = resolve_tag_ids(store, raw);
        if !err.is_empty() {
            return Err(err);
        }
        remove_tag_ids = ids;
        if !labels.iter().any(|l| l == "remove-tags") {
            labels.push("remove-tags".to_string());
        }
    }

    let mut rename_map: HashMap<String, String> = HashMap::new();
    for token in &args.rename_checklist {
        let Some((short_id, new_title)) = token.split_once(':') else {
            return Err(format!(
                "--rename-checklist requires 'id:new title' format, got: {token:?}"
            ));
        };
        let short_id = short_id.trim();
        let new_title = new_title.trim();
        if short_id.is_empty() || new_title.is_empty() {
            return Err(format!(
                "--rename-checklist requires 'id:new title' format, got: {token:?}"
            ));
        }
        rename_map.insert(short_id.to_string(), new_title.to_string());
    }

    let mut changes: BTreeMap<String, WireObject> = BTreeMap::new();

    for task in &tasks {
        let mut update = shared_update.clone();

        if let Some(title) = &args.title {
            let title = title.trim();
            if title.is_empty() {
                return Err("Task title cannot be empty.".to_string());
            }
            update.title = Some(title.to_string());
            if !labels.iter().any(|l| l == "title") {
                labels.push("title".to_string());
            }
        }

        if let Some(completed_on) = &args.completed_on {
            if !task.is_completed() && !task.is_canceled() {
                return Err("--completed-on needs a completed or canceled task.".to_string());
            }
            update.stop_date = Some(Some(parse_instant(completed_on, "--completed-on")?));
            labels.push("completed-on".to_string());
        }
        if let Some(created_on) = &args.created_on {
            update.creation_date = Some(Some(parse_instant(created_on, "--created-on")?));
            labels.push("created-on".to_string());
        }

        if let Some(notes) = &args.notes {
            if notes.is_empty() {
                update.notes = Some(TaskNotes::Structured(StructuredTaskNotes {
                    object_type: Some("tx".to_string()),
                    format_type: 1,
                    ch: Some(0),
                    v: Some(String::new()),
                    ps: Vec::new(),
                    unknown_fields: Default::default(),
                }));
            } else {
                update.notes = Some(task6_note(notes));
            }
            if !labels.iter().any(|l| l == "notes") {
                labels.push("notes".to_string());
            }
        }

        if move_l == "clear" {
            update.parent_project_ids = Some(vec![]);
            update.area_ids = Some(vec![]);
            update.action_group_ids = Some(vec![]);
            if task.start == TaskStart::Inbox {
                update.start_location = Some(TaskStart::Anytime);
            }
        }
        if move_l == "inbox" {
            unschedule(&mut update, task);
        }

        if let Some(move_from_inbox_st) = move_from_inbox_st
            && task.start == TaskStart::Inbox
        {
            update.start_location = Some(move_from_inbox_st);
        }

        if !add_tag_ids.is_empty() || !remove_tag_ids.is_empty() {
            let mut current = task.tags.clone();
            for uuid in &add_tag_ids {
                if !current.iter().any(|c| c == uuid) {
                    current.push(uuid.clone());
                }
            }
            current.retain(|uuid| !remove_tag_ids.iter().any(|r| r == uuid));
            update.tag_ids = Some(current);
        }

        // the checklist as this edit leaves it, kept alongside the changes for a template to copy
        let mut checklist = task
            .checklist_items
            .iter()
            .map(|item| (item.uuid.clone(), item.title.clone(), item.index))
            .collect::<Vec<_>>();

        if let Some(remove_raw) = &args.remove_checklist {
            let (items, err) = resolve_checklist_items(task, remove_raw);
            if !err.is_empty() {
                return Err(err);
            }
            let removed = items.into_iter().map(|i| i.uuid).collect::<HashSet<_>>();
            for uuid in &removed {
                changes.insert(
                    uuid.to_string(),
                    WireObject::delete(EntityType::ChecklistItem3),
                );
            }
            checklist.retain(|(uuid, _, _)| !removed.contains(uuid));
            if !labels.iter().any(|l| l == "remove-checklist") {
                labels.push("remove-checklist".to_string());
            }
        }

        if !rename_map.is_empty() {
            for (short_id, new_title) in &rename_map {
                let matches = task
                    .checklist_items
                    .iter()
                    .filter(|i| i.uuid.starts_with(short_id))
                    .cloned()
                    .collect::<Vec<_>>();
                if matches.is_empty() {
                    return Err(format!("Checklist item not found: '{short_id}'"));
                }
                if matches.len() > 1 {
                    return Err(format!("Ambiguous checklist item prefix: '{short_id}'"));
                }
                changes.insert(
                    matches[0].uuid.to_string(),
                    WireObject::update(
                        EntityType::ChecklistItem3,
                        ChecklistItemPatch {
                            title: Some(new_title.to_string()),
                            modification_date: Some(now),
                            ..Default::default()
                        },
                    ),
                );
                for (uuid, title, _) in checklist.iter_mut() {
                    if *uuid == matches[0].uuid {
                        *title = new_title.to_string();
                    }
                }
            }
            if !labels.iter().any(|l| l == "rename-checklist") {
                labels.push("rename-checklist".to_string());
            }
        }

        if !args.add_checklist.is_empty() {
            let max_ix = task
                .checklist_items
                .iter()
                .map(|i| i.index)
                .max()
                .unwrap_or(0);
            for (idx, title) in args.add_checklist.iter().enumerate() {
                let title = title.trim();
                if title.is_empty() {
                    return Err("Checklist item title cannot be empty.".to_string());
                }
                let index = max_ix + idx as i32 + 1;
                let uuid = next_id();
                changes.insert(
                    uuid.clone(),
                    WireObject::create(
                        EntityType::ChecklistItem3,
                        ChecklistItemProps {
                            title: title.to_string(),
                            task_ids: vec![task.uuid.clone()],
                            status: TaskStatus::Incomplete,
                            sort_index: index,
                            creation_date: Some(now),
                            modification_date: Some(now),
                            ..Default::default()
                        },
                    ),
                );
                checklist.push((
                    ThingsId::from_str(&uuid).map_err(|e| e.to_string())?,
                    title.to_string(),
                    index,
                ));
            }
            if !labels.iter().any(|l| l == "add-checklist") {
                labels.push("add-checklist".to_string());
            }
        }

        let checklist = checklist
            .into_iter()
            .map(|(_, title, index)| ChecklistCopy { title, index })
            .collect::<Vec<_>>();
        apply_schedule(
            args,
            task,
            &mut update,
            &mut changes,
            &mut labels,
            &checklist,
            now,
            today_ts,
            next_id,
        )?;

        let has_checklist_changes = !args.add_checklist.is_empty()
            || args.remove_checklist.is_some()
            || !rename_map.is_empty();
        if update.is_empty() && !has_checklist_changes {
            return Err("No edit changes requested.".to_string());
        }

        if !update.is_empty() {
            update.modification_date = Some(Some(now));
            changes.insert(
                task.uuid.to_string(),
                WireObject::update(EntityType::Task7, update),
            );
        }
    }

    Ok(EditPlan {
        tasks,
        changes,
        labels,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::{
        ids::ThingsId,
        store::{ThingsStore, fold_items},
        wire::{
            area::AreaProps,
            checklist::ChecklistItemProps,
            tags::TagProps,
            task::{TaskProps, TaskStart, TaskStatus, TaskType},
            wire_object::{EntityType, OperationType, WireItem, WireObject},
        },
    };

    const NOW: f64 = 1_700_000_222.0;
    const TODAY: i64 = 1_699_920_000;
    const TASK_UUID: &str = "A7h5eCi24RvAWKC3Hv3muf";
    const TASK_UUID2: &str = "3H9jsMx3kYMrQ4M7DReSRn";
    const PROJECT_UUID: &str = "KGvAPpMrzHAKMdgMiERP1V";
    const AREA_UUID: &str = "MpkEei6ybkFS2n6SXvwfLf";
    const CHECK_A: &str = "5uwoHPi5m5i8QJa6Rae6Cn";
    const CHECK_B: &str = "CwhFwmHxjHkR7AFn9aJH9Q";

    fn build_store(entries: Vec<(String, WireObject)>) -> ThingsStore {
        let mut item: WireItem = BTreeMap::new();
        for (uuid, obj) in entries {
            item.insert(uuid, obj);
        }
        let raw = fold_items([item]);
        ThingsStore::from_raw_state(&raw)
    }

    fn task(uuid: &str, title: &str) -> (String, WireObject) {
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
                    creation_date: Some(1.0),
                    modification_date: Some(1.0),
                    ..Default::default()
                },
            ),
        )
    }

    fn task_with(uuid: &str, title: &str, tag_ids: Vec<&str>) -> (String, WireObject) {
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
                    tag_ids: tag_ids
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

    fn project(uuid: &str, title: &str) -> (String, WireObject) {
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

    fn assert_task_update(plan: &EditPlan, uuid: &str) -> BTreeMap<String, serde_json::Value> {
        let obj = plan.changes.get(uuid).expect("missing task change");
        assert_eq!(obj.operation_type, OperationType::Update);
        assert_eq!(obj.entity_type, Some(EntityType::Task7));
        obj.properties_map()
    }

    #[test]
    fn edit_title_and_notes_payloads() {
        let store = build_store(vec![task(TASK_UUID, "Old title")]);
        let args = EditArgs {
            task_ids: vec![IdentifierToken::from(TASK_UUID)],
            title: Some("New title".to_string()),
            notes: Some("new notes".to_string()),
            move_target: None,
            tag_delta: TagDeltaArgs {
                add_tags: None,
                remove_tags: None,
            },
            add_checklist: vec![],
            remove_checklist: None,
            rename_checklist: vec![],
            completed_on: None,
            created_on: None,
            when: None,
            deadline_date: None,
            clear_deadline: false,
            reminder: None,
            clear_reminder: false,
            repeat: None,
            times: None,
            until: None,
        };
        let mut id_gen = || "X".to_string();
        let plan = build_edit_plan(&args, &store, NOW, TODAY, &mut id_gen).expect("plan");
        let p = assert_task_update(&plan, TASK_UUID);
        assert_eq!(p.get("tt"), Some(&json!("New title")));
        assert_eq!(p.get("md"), Some(&json!(NOW)));
        assert!(p.contains_key("nt"));
    }

    #[test]
    fn edit_move_targets_payload() {
        let store = build_store(vec![
            task(TASK_UUID, "Movable"),
            project(PROJECT_UUID, "Roadmap"),
            area(AREA_UUID, "Work"),
        ]);

        let mut id_gen = || "X".to_string();
        let inbox = build_edit_plan(
            &EditArgs {
                task_ids: vec![IdentifierToken::from(TASK_UUID)],
                title: None,
                notes: None,
                move_target: Some("inbox".to_string()),
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
                add_checklist: vec![],
                remove_checklist: None,
                rename_checklist: vec![],
                completed_on: None,
                created_on: None,
                when: None,
                deadline_date: None,
                clear_deadline: false,
                reminder: None,
                clear_reminder: false,
                repeat: None,
                times: None,
                until: None,
            },
            &store,
            NOW,
            TODAY,
            &mut id_gen,
        )
        .expect("inbox plan");
        let p = assert_task_update(&inbox, TASK_UUID);
        assert_eq!(p.get("st"), Some(&json!(0)));
        assert_eq!(p.get("pr"), Some(&json!([])));
        assert_eq!(p.get("ar"), Some(&json!([])));

        let clear = build_edit_plan(
            &EditArgs {
                task_ids: vec![IdentifierToken::from(TASK_UUID)],
                title: None,
                notes: None,
                move_target: Some("clear".to_string()),
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
                add_checklist: vec![],
                remove_checklist: None,
                rename_checklist: vec![],
                completed_on: None,
                created_on: None,
                when: None,
                deadline_date: None,
                clear_deadline: false,
                reminder: None,
                clear_reminder: false,
                repeat: None,
                times: None,
                until: None,
            },
            &store,
            NOW,
            TODAY,
            &mut id_gen,
        )
        .expect("clear plan");
        let p = assert_task_update(&clear, TASK_UUID);
        assert_eq!(p.get("st"), Some(&json!(1)));

        let project_move = build_edit_plan(
            &EditArgs {
                task_ids: vec![IdentifierToken::from(TASK_UUID)],
                title: None,
                notes: None,
                move_target: Some(PROJECT_UUID.to_string()),
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
                add_checklist: vec![],
                remove_checklist: None,
                rename_checklist: vec![],
                completed_on: None,
                created_on: None,
                when: None,
                deadline_date: None,
                clear_deadline: false,
                reminder: None,
                clear_reminder: false,
                repeat: None,
                times: None,
                until: None,
            },
            &store,
            NOW,
            TODAY,
            &mut id_gen,
        )
        .expect("project move plan");
        let p = assert_task_update(&project_move, TASK_UUID);
        assert_eq!(p.get("pr"), Some(&json!([PROJECT_UUID])));
        assert_eq!(p.get("st"), Some(&json!(1)));
    }

    #[test]
    fn a_repeat_added_by_an_edit_mirrors_the_edited_to_do() {
        let store = build_store(vec![
            task(TASK_UUID, "Old title"),
            checklist(CHECK_A, TASK_UUID, "Step one", 1),
            checklist(CHECK_B, TASK_UUID, "Step two", 2),
        ]);
        let args = EditArgs {
            task_ids: vec![IdentifierToken::from(TASK_UUID)],
            title: Some("New title".to_string()),
            notes: Some("New notes".to_string()),
            move_target: None,
            tag_delta: TagDeltaArgs {
                add_tags: None,
                remove_tags: None,
            },
            add_checklist: vec!["Step three".to_string()],
            remove_checklist: Some(CHECK_B[..6].to_string()),
            rename_checklist: vec![format!("{}:Step won", &CHECK_A[..6])],
            completed_on: None,
            created_on: None,
            when: Some("today".to_string()),
            deadline_date: None,
            clear_deadline: false,
            reminder: None,
            clear_reminder: false,
            repeat: Some("daily".to_string()),
            times: None,
            until: None,
        };
        let id = |n: u128| ThingsId::from_u128(n).to_string();
        let mut ids = (1..).map(id);
        let mut id_gen = || ids.next().expect("id");
        let plan = build_edit_plan(&args, &store, NOW, TODAY, &mut id_gen).expect("plan");

        // the first id is the added item on the to-do, the second the template, the third and fourth its checklist copies
        let template = plan.changes.get(&id(2)).expect("template").properties_map();
        assert_eq!(template.get("tt"), Some(&json!("New title")));
        assert_eq!(template["nt"]["v"], json!("New notes"));
        assert!(template.get("rr").is_some_and(|rule| !rule.is_null()));
        let copies = [3, 4]
            .iter()
            .map(|n| plan.changes.get(&id(*n)).expect("copy").properties_map())
            .collect::<Vec<_>>();
        assert_eq!(copies[0].get("tt"), Some(&json!("Step won")));
        assert_eq!(copies[0].get("ts"), Some(&json!([id(2)])));
        assert_eq!(copies[1].get("tt"), Some(&json!("Step three")));
        assert_eq!(copies[1].get("ix"), Some(&json!(3)));
        assert!(!plan.changes.contains_key(&id(5)));

        // a deadline has no place on the template yet, the edit is refused
        let dated = EditArgs {
            deadline_date: Some("2027-01-15".to_string()),
            ..args
        };
        let err = build_edit_plan(&dated, &store, NOW, TODAY, &mut id_gen).expect_err("deadline");
        assert!(err.contains("clear the deadline"));
    }

    #[test]
    fn moving_to_the_inbox_clears_the_day_and_the_reminder() {
        let scheduled = (
            TASK_UUID.to_string(),
            WireObject::create(
                EntityType::Task7,
                TaskProps {
                    title: "Dated".to_string(),
                    start_location: TaskStart::Anytime,
                    scheduled_date: Some(TODAY),
                    today_index_reference: Some(TODAY),
                    evening_bit: 1,
                    alarm_time_offset: Some(32400),
                    creation_date: Some(1.0),
                    modification_date: Some(1.0),
                    ..Default::default()
                },
            ),
        );
        let store = build_store(vec![scheduled]);
        let args = |move_target: Option<&str>, when: Option<&str>| EditArgs {
            task_ids: vec![IdentifierToken::from(TASK_UUID)],
            title: None,
            notes: None,
            move_target: move_target.map(str::to_string),
            tag_delta: TagDeltaArgs {
                add_tags: None,
                remove_tags: None,
            },
            add_checklist: vec![],
            remove_checklist: None,
            rename_checklist: vec![],
            completed_on: None,
            created_on: None,
            when: when.map(str::to_string),
            deadline_date: None,
            clear_deadline: false,
            reminder: None,
            clear_reminder: false,
            repeat: None,
            times: None,
            until: None,
        };
        let mut id_gen = || "X".to_string();
        for (move_target, when) in [(Some("inbox"), None), (None, Some("someday"))] {
            let plan = build_edit_plan(&args(move_target, when), &store, NOW, TODAY, &mut id_gen)
                .expect("plan");
            let p = assert_task_update(&plan, TASK_UUID);
            assert_eq!(p.get("sr"), Some(&json!(null)));
            assert_eq!(p.get("tir"), Some(&json!(null)));
            assert_eq!(p.get("sb"), Some(&json!(0)));
            assert_eq!(p.get("ato"), Some(&json!(null)));
        }
        let err = build_edit_plan(
            &args(Some("inbox"), Some("today")),
            &store,
            NOW,
            TODAY,
            &mut id_gen,
        )
        .expect_err("inbox and a day");
        assert!(err.starts_with("--move inbox cannot be combined"));
    }

    #[test]
    fn edit_multi_id_move_and_rejections() {
        let store = build_store(vec![
            task(TASK_UUID, "Task One"),
            task(TASK_UUID2, "Task Two"),
            project(PROJECT_UUID, "Roadmap"),
        ]);

        let mut id_gen = || "X".to_string();
        let plan = build_edit_plan(
            &EditArgs {
                task_ids: vec![
                    IdentifierToken::from(TASK_UUID),
                    IdentifierToken::from(TASK_UUID2),
                ],
                title: None,
                notes: None,
                move_target: Some(PROJECT_UUID.to_string()),
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
                add_checklist: vec![],
                remove_checklist: None,
                rename_checklist: vec![],
                completed_on: None,
                created_on: None,
                when: None,
                deadline_date: None,
                clear_deadline: false,
                reminder: None,
                clear_reminder: false,
                repeat: None,
                times: None,
                until: None,
            },
            &store,
            NOW,
            TODAY,
            &mut id_gen,
        )
        .expect("multi move");
        assert_eq!(plan.changes.len(), 2);

        let err = build_edit_plan(
            &EditArgs {
                task_ids: vec![
                    IdentifierToken::from(TASK_UUID),
                    IdentifierToken::from(TASK_UUID2),
                ],
                title: Some("New".to_string()),
                notes: None,
                move_target: None,
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
                add_checklist: vec![],
                remove_checklist: None,
                rename_checklist: vec![],
                completed_on: None,
                created_on: None,
                when: None,
                deadline_date: None,
                clear_deadline: false,
                reminder: None,
                clear_reminder: false,
                repeat: None,
                times: None,
                until: None,
            },
            &store,
            NOW,
            TODAY,
            &mut id_gen,
        )
        .expect_err("title should reject");
        assert_eq!(err, "--title requires a single task ID.");
    }

    #[test]
    fn edit_tag_payloads() {
        let tag1 = "WukwpDdL5Z88nX3okGMKTC";
        let tag2 = "JiqwiDaS3CAyjCmHihBDnB";
        let store = build_store(vec![
            task_with(TASK_UUID, "A", vec![tag1]),
            tag(tag1, "Work"),
            tag(tag2, "Focus"),
        ]);

        let mut id_gen = || "X".to_string();
        let plan = build_edit_plan(
            &EditArgs {
                task_ids: vec![IdentifierToken::from(TASK_UUID)],
                title: None,
                notes: None,
                move_target: None,
                tag_delta: TagDeltaArgs {
                    add_tags: Some("Focus".to_string()),
                    remove_tags: Some("Work".to_string()),
                },
                add_checklist: vec![],
                remove_checklist: None,
                rename_checklist: vec![],
                completed_on: None,
                created_on: None,
                when: None,
                deadline_date: None,
                clear_deadline: false,
                reminder: None,
                clear_reminder: false,
                repeat: None,
                times: None,
                until: None,
            },
            &store,
            NOW,
            TODAY,
            &mut id_gen,
        )
        .expect("tag plan");

        let p = assert_task_update(&plan, TASK_UUID);
        assert_eq!(p.get("tg"), Some(&json!([tag2])));
    }

    #[test]
    fn edit_checklist_mutations() {
        let store = build_store(vec![
            task(TASK_UUID, "A"),
            checklist(CHECK_A, TASK_UUID, "Step one", 1),
            checklist(CHECK_B, TASK_UUID, "Step two", 2),
        ]);

        let new_check = |n: u128| ThingsId::from_u128(n).to_string();
        let mut ids = vec![new_check(1), new_check(2)].into_iter();
        let mut id_gen = || ids.next().expect("next id");
        let plan = build_edit_plan(
            &EditArgs {
                task_ids: vec![IdentifierToken::from(TASK_UUID)],
                title: None,
                notes: None,
                move_target: None,
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
                add_checklist: vec!["Step three".to_string(), "Step four".to_string()],
                remove_checklist: Some(format!("{},{}", &CHECK_A[..6], &CHECK_B[..6])),
                rename_checklist: vec![format!("{}:Renamed", &CHECK_A[..6])],
                completed_on: None,
                created_on: None,
                when: None,
                deadline_date: None,
                clear_deadline: false,
                reminder: None,
                clear_reminder: false,
                repeat: None,
                times: None,
                until: None,
            },
            &store,
            NOW,
            TODAY,
            &mut id_gen,
        )
        .expect("checklist plan");

        assert!(matches!(
            plan.changes.get(CHECK_A).map(|o| o.operation_type),
            Some(OperationType::Update)
        ));
        assert!(matches!(
            plan.changes.get(CHECK_B).map(|o| o.operation_type),
            Some(OperationType::Delete)
        ));
        assert!(plan.changes.contains_key(&new_check(1)));
        assert!(plan.changes.contains_key(&new_check(2)));
    }

    #[test]
    fn edit_no_changes_project_and_move_errors() {
        let store = build_store(vec![task(TASK_UUID, "A")]);
        let mut id_gen = || "X".to_string();
        let err = build_edit_plan(
            &EditArgs {
                task_ids: vec![IdentifierToken::from(TASK_UUID)],
                title: None,
                notes: None,
                move_target: None,
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
                add_checklist: vec![],
                remove_checklist: None,
                rename_checklist: vec![],
                completed_on: None,
                created_on: None,
                when: None,
                deadline_date: None,
                clear_deadline: false,
                reminder: None,
                clear_reminder: false,
                repeat: None,
                times: None,
                until: None,
            },
            &store,
            NOW,
            TODAY,
            &mut id_gen,
        )
        .expect_err("no changes");
        assert_eq!(err, "No edit changes requested.");

        let store = build_store(vec![task(TASK_UUID, "A"), project(PROJECT_UUID, "Roadmap")]);
        let err = build_edit_plan(
            &EditArgs {
                task_ids: vec![IdentifierToken::from(PROJECT_UUID)],
                title: Some("New".to_string()),
                notes: None,
                move_target: None,
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
                add_checklist: vec![],
                remove_checklist: None,
                rename_checklist: vec![],
                completed_on: None,
                created_on: None,
                when: None,
                deadline_date: None,
                clear_deadline: false,
                reminder: None,
                clear_reminder: false,
                repeat: None,
                times: None,
                until: None,
            },
            &store,
            NOW,
            TODAY,
            &mut id_gen,
        )
        .expect_err("project edit reject");
        assert_eq!(err, "Use 'projects edit' to edit a project.");

        let store = build_store(vec![
            task(TASK_UUID, "Movable"),
            task(PROJECT_UUID, "Not a project"),
        ]);
        let err = build_edit_plan(
            &EditArgs {
                task_ids: vec![IdentifierToken::from(TASK_UUID)],
                title: None,
                notes: None,
                move_target: Some(PROJECT_UUID.to_string()),
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
                add_checklist: vec![],
                remove_checklist: None,
                rename_checklist: vec![],
                completed_on: None,
                created_on: None,
                when: None,
                deadline_date: None,
                clear_deadline: false,
                reminder: None,
                clear_reminder: false,
                repeat: None,
                times: None,
                until: None,
            },
            &store,
            NOW,
            TODAY,
            &mut id_gen,
        )
        .expect_err("invalid move target kind");
        assert_eq!(
            err,
            "--move target must be Inbox, clear, a project ID, or an area ID."
        );
    }

    #[test]
    fn edit_move_target_ambiguous() {
        let ambiguous_project = "ABCD1234efgh5678JKLMno";
        let ambiguous_area = "ABCD1234pqrs9123TUVWxy";
        let store = build_store(vec![
            task(TASK_UUID, "Movable"),
            project(ambiguous_project, "Project match"),
            area(ambiguous_area, "Area match"),
        ]);
        let mut id_gen = || "X".to_string();
        let err = build_edit_plan(
            &EditArgs {
                task_ids: vec![IdentifierToken::from(TASK_UUID)],
                title: None,
                notes: None,
                move_target: Some("ABCD1234".to_string()),
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
                add_checklist: vec![],
                remove_checklist: None,
                rename_checklist: vec![],
                completed_on: None,
                created_on: None,
                when: None,
                deadline_date: None,
                clear_deadline: false,
                reminder: None,
                clear_reminder: false,
                repeat: None,
                times: None,
                until: None,
            },
            &store,
            NOW,
            TODAY,
            &mut id_gen,
        )
        .expect_err("ambiguous move target");
        assert_eq!(
            err,
            "Ambiguous --move target 'ABCD1234' (matches project and area)."
        );
    }

    #[test]
    fn checklist_single_task_constraint_and_empty_title() {
        let store = build_store(vec![task(TASK_UUID, "A"), task(TASK_UUID2, "B")]);
        let mut id_gen = || "X".to_string();

        let err = build_edit_plan(
            &EditArgs {
                task_ids: vec![
                    IdentifierToken::from(TASK_UUID),
                    IdentifierToken::from(TASK_UUID2),
                ],
                title: None,
                notes: None,
                move_target: None,
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
                add_checklist: vec!["Step".to_string()],
                remove_checklist: None,
                rename_checklist: vec![],
                completed_on: None,
                created_on: None,
                when: None,
                deadline_date: None,
                clear_deadline: false,
                reminder: None,
                clear_reminder: false,
                repeat: None,
                times: None,
                until: None,
            },
            &store,
            NOW,
            TODAY,
            &mut id_gen,
        )
        .expect_err("single task constraint");
        assert_eq!(
            err,
            "--add-checklist/--remove-checklist/--rename-checklist require a single task ID."
        );

        let store = build_store(vec![task(TASK_UUID, "A")]);
        let err = build_edit_plan(
            &EditArgs {
                task_ids: vec![IdentifierToken::from(TASK_UUID)],
                title: Some("   ".to_string()),
                notes: None,
                move_target: None,
                tag_delta: TagDeltaArgs {
                    add_tags: None,
                    remove_tags: None,
                },
                add_checklist: vec![],
                remove_checklist: None,
                rename_checklist: vec![],
                completed_on: None,
                created_on: None,
                when: None,
                deadline_date: None,
                clear_deadline: false,
                reminder: None,
                clear_reminder: false,
                repeat: None,
                times: None,
                until: None,
            },
            &store,
            NOW,
            TODAY,
            &mut id_gen,
        )
        .expect_err("empty title");
        assert_eq!(err, "Task title cannot be empty.");
    }

    #[test]
    fn checklist_patch_has_expected_fields() {
        let patch = ChecklistItemPatch {
            title: Some("Step".to_string()),
            status: Some(TaskStatus::Incomplete),
            stop_date: None,
            task_ids: Some(vec![
                TASK_UUID
                    .parse::<crate::ids::ThingsId>()
                    .expect("test task id should parse as ThingsId"),
            ]),
            sort_index: Some(3),
            creation_date: Some(NOW),
            modification_date: Some(NOW),
        };
        let props = patch.into_properties();
        assert_eq!(props.get("tt"), Some(&json!("Step")));
        assert_eq!(props.get("ss"), Some(&json!(0)));
        assert_eq!(props.get("ix"), Some(&json!(3)));
    }
}
