use std::sync::Arc;

use anyhow::{Result, bail};
use clap::Args;
use iocraft::prelude::*;

use crate::{
    app::Cli,
    arg_types::IdentifierToken,
    commands::{Command, detailed_json_conflict, write_json},
    common::{ICONS, shown_title},
    ui::{
        render_element_to_string,
        views::{area::AreaView, json::common::build_tasks_json},
    },
    wire::task::TaskStatus,
};

#[derive(Args)]
#[command(about = "Show projects and tasks in an area")]
pub struct AreaArgs {
    /// Area ID (or unique ID prefix)
    pub area_id: IdentifierToken,
    /// Show notes and checklists beneath each item
    #[arg(long, short = 'd')]
    pub detailed: bool,
    /// Include completed and canceled items
    #[arg(long, short = 'a')]
    pub all: bool,
}

impl Command for AreaArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn std::io::Write,
        ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        let store = Arc::new(cli.load_store()?);
        let today = ctx.today();
        let (area_opt, err, ambiguous) = store.resolve_area_identifier(&self.area_id);
        let Some(area) = area_opt else {
            // the message counts every match and lists ten at most
            let candidates = ambiguous
                .iter()
                .take(10)
                .map(|area| {
                    format!(
                        "\n  {} {}  ({})",
                        ICONS.area,
                        shown_title(&area.title),
                        area.uuid
                    )
                })
                .collect::<String>();
            bail!("{err}{candidates}");
        };

        let status_filter = if self.all {
            None
        } else {
            Some(TaskStatus::Incomplete)
        };
        let mut projects = store
            .projects(status_filter)
            .into_iter()
            .filter(|p| p.area.as_ref() == Some(&area.uuid))
            .collect::<Vec<_>>();
        projects.sort_by_key(|p| p.index);

        let mut loose_tasks = store
            .tasks(status_filter)
            .into_iter()
            .filter(|t| {
                if t.is_recurrence_template() {
                    return false;
                }
                t.area.as_ref() == Some(&area.uuid)
                    && !t.is_project()
                    && !store.in_trash(t)
                    && store.effective_project_uuid(t).is_none()
            })
            .collect::<Vec<_>>();
        loose_tasks.sort_by_key(|t| t.index);

        let json = cli.json;
        if json {
            detailed_json_conflict(json, self.detailed)?;

            let mut items = projects;
            items.extend(loose_tasks);
            items.sort_by(|a, b| {
                let a_proj = if a.is_project() { 0 } else { 1 };
                let b_proj = if b.is_project() { 0 } else { 1 };
                (a_proj, a.index, &a.uuid).cmp(&(b_proj, b.index, &b.uuid))
            });
            write_json(out, &build_tasks_json(&items, &store, &today))?;
            return Ok(());
        }

        let mut ui = element! {
            ContextProvider(value: Context::owned(store.clone())) {
                ContextProvider(value: Context::owned(today)) {
                    AreaView(
                        area: &area,
                        tasks: loose_tasks.iter().collect::<Vec<_>>(),
                        projects: projects.iter().collect::<Vec<_>>(),
                        detailed: self.detailed,
                    )
                }
            }
        };
        let rendered = render_element_to_string(&mut ui, cli.no_color());
        writeln!(out, "{}", rendered)?;

        Ok(())
    }
}
