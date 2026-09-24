use std::io::Write;

use anyhow::{Result, bail};
use clap::Args;

use crate::{
    app::Cli,
    arg_types::IdentifierToken,
    commands::{Command, write_json},
    common::{DIM, ICONS, colored, fmt_date, fmt_date_local, one_line},
    ui::views::json::common::build_tasks_json,
    wire::task::{TaskStart, TaskStatus, TaskType},
};

#[derive(Debug, Args)]
#[command(about = "Show one task or project in full")]
pub struct ShowArgs {
    /// Task or project ID (or unique ID prefix)
    pub item_id: IdentifierToken,
}

impl Command for ShowArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn Write,
        ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        let store = cli.load_store()?;
        let today = ctx.today();
        let (task, err, _) = store.resolve_task_identifier(&self.item_id);
        let Some(task) = task else {
            bail!("{err}");
        };

        if cli.json {
            let mut json = build_tasks_json(std::slice::from_ref(&task), &store, &today);
            write_json(out, &json.remove(0))?;
            return Ok(());
        }

        let no_color = cli.no_color();
        let field = |name: &str| colored(format!("{name}:"), &[DIM], no_color);
        writeln!(
            out,
            "{}  {}",
            one_line(&task.title),
            colored(&task.uuid, &[DIM], no_color)
        )?;
        let kind = match task.item_type {
            TaskType::Todo => "to-do".to_string(),
            TaskType::Project => "project".to_string(),
            TaskType::Heading => "heading".to_string(),
            TaskType::Unknown(raw) => format!("unknown kind {raw}"),
        };
        let mut status = match task.status {
            TaskStatus::Incomplete => "open".to_string(),
            TaskStatus::Completed => "done".to_string(),
            TaskStatus::Canceled => "canceled".to_string(),
            TaskStatus::Unknown(raw) => format!("unknown status {raw}"),
        };
        if store.in_trash(&task) {
            status.push_str(", in the Trash");
        }
        if task.degraded {
            status.push_str(", did not replay completely");
        }
        writeln!(out, "{} {kind}, {status}", field("Kind"))?;

        let mut when = match (task.start, task.start_date) {
            // a start this CLI does not know still shows the day it carries
            (TaskStart::Unknown(raw), Some(day)) if day.date_naive() == today.date_naive() => {
                format!("unknown start {raw}, today")
            }
            (TaskStart::Unknown(raw), Some(day)) => {
                format!("unknown start {raw}, {}", day.format("%Y-%m-%d"))
            }
            (TaskStart::Unknown(raw), None) => format!("unknown start {raw}"),
            (TaskStart::Inbox, _) => "inbox".to_string(),
            (_, Some(day)) if day.date_naive() == today.date_naive() => "today".to_string(),
            (_, Some(day)) => day.format("%Y-%m-%d").to_string(),
            (TaskStart::Someday, None) => "someday".to_string(),
            (TaskStart::Anytime, None) => "anytime".to_string(),
        };
        if task.evening {
            when.push_str(" evening");
        }
        if let Some(time) = task.reminder() {
            when.push_str(&format!(" @{time}"));
        }
        writeln!(out, "{} {when}", field("When"))?;
        if task.deadline.is_some() {
            writeln!(out, "{} {}", field("Deadline"), fmt_date(task.deadline))?;
        }
        if let Some(project) = store.effective_project_uuid(&task) {
            writeln!(
                out,
                "{} {}",
                field("Project"),
                one_line(&store.resolve_project_title(&project))
            )?;
        }
        if let Some(heading) = task
            .action_group
            .as_ref()
            .and_then(|id| store.get_task(&id.to_string()))
        {
            writeln!(out, "{} {}", field("Heading"), one_line(&heading.title))?;
        }
        if let Some(area) = store.effective_area_uuid(&task) {
            writeln!(
                out,
                "{} {}",
                field("Area"),
                one_line(&store.resolve_area_title(&area))
            )?;
        }
        if !task.tags.is_empty() {
            let tags: Vec<String> = task
                .tags
                .iter()
                .map(|tag| one_line(&store.resolve_tag_title(tag)))
                .collect();
            writeln!(out, "{} {}", field("Tags"), tags.join(", "))?;
        }
        if let Some(rule) = &task.recurrence_rule {
            writeln!(
                out,
                "{} {}",
                field("Repeat"),
                rule.human_readable()
                    .unwrap_or_else(|_| "repeats".to_string())
            )?;
        }
        if let Some(rule) = task
            .recurrence_templates
            .first()
            .and_then(|id| store.get_task(&id.to_string()))
            .and_then(|template| template.recurrence_rule)
        {
            writeln!(
                out,
                "{} {}",
                field("Instance of"),
                rule.human_readable()
                    .unwrap_or_else(|_| "a repeat".to_string())
            )?;
        }
        // instants show under their local day
        // the deadline above is a day stamp
        let mut dates = vec![format!("created {}", fmt_date_local(task.creation_date))];
        if task.modification_date.is_some() {
            dates.push(format!(
                "modified {}",
                fmt_date_local(task.modification_date)
            ));
        }
        if task.stop_date.is_some() {
            let closed = if task.is_canceled() {
                "canceled"
            } else {
                "completed"
            };
            dates.push(format!("{closed} {}", fmt_date_local(task.stop_date)));
        }
        writeln!(out, "{} {}", field("Dates"), dates.join(", "))?;
        if let Some(notes) = task
            .notes
            .as_deref()
            .filter(|notes| !notes.trim().is_empty())
        {
            writeln!(out, "{}", field("Notes"))?;
            for line in notes.lines() {
                writeln!(out, "  {}", one_line(line))?;
            }
        }
        if !task.checklist_items.is_empty() {
            writeln!(out, "{}", field("Checklist"))?;
            for item in &task.checklist_items {
                let mark = if item.is_completed() {
                    ICONS.checklist_done
                } else if item.is_canceled() {
                    ICONS.checklist_canceled
                } else {
                    ICONS.checklist_open
                };
                writeln!(out, "  {mark} {}", one_line(&item.title))?;
            }
        }
        Ok(())
    }
}
