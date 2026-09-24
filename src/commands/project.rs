use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Result, bail};
use clap::Args;
use iocraft::prelude::*;

use crate::{
    app::Cli,
    arg_types::IdentifierToken,
    commands::{Command, detailed_json_conflict, write_json},
    common::{kind_with_article, shown_title},
    ui::{
        render_element_to_string,
        views::{
            json::common::build_tasks_json,
            project::{ProjectHeadingGroup, ProjectView},
        },
    },
};

#[derive(Args)]
#[command(about = "Show all tasks in a project")]
pub struct ProjectArgs {
    /// Project ID (or unique ID prefix)
    pub project_id: IdentifierToken,
    /// Show notes and checklists beneath each task
    #[arg(long, short = 'd')]
    pub detailed: bool,
}

impl Command for ProjectArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn std::io::Write,
        ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        let store = Arc::new(cli.load_store()?);
        let today = ctx.today();
        let (task_opt, err, ambiguous) = store.resolve_task_identifier(&self.project_id);
        let Some(project) = task_opt else {
            let candidates = ambiguous
                .iter()
                .map(|task| format!("\n  {}  ({})", shown_title(&task.title), task.uuid))
                .collect::<String>();
            bail!("{err}{candidates}");
        };

        if !project.is_project() {
            bail!(
                "Item is {}: {}",
                kind_with_article(&project),
                shown_title(&project.title)
            );
        }
        if store.in_trash(&project) {
            bail!("Project is in the Trash: {}", shown_title(&project.title));
        }

        let children = store
            .tasks(None)
            .into_iter()
            .filter(|t| {
                !t.is_recurrence_template()
                    && !store.in_trash(t)
                    && store.effective_project_uuid(t).as_ref() == Some(&project.uuid)
            })
            .collect::<Vec<_>>();

        let json = cli.json;
        if json {
            detailed_json_conflict(json, self.detailed)?;

            let mut sorted_children = children.clone();
            sorted_children.sort_by_key(|t| t.index);
            write_json(out, &build_tasks_json(&sorted_children, &store, &today))?;
            return Ok(());
        }

        let headings = store
            .tasks_by_uuid
            .values()
            .filter(|t| t.is_heading() && !t.trashed && t.project.as_ref() == Some(&project.uuid))
            .cloned()
            .map(|h| (h.uuid.clone(), h))
            .collect::<BTreeMap<_, _>>();

        let mut ungrouped = Vec::new();
        let mut by_heading: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for t in children.clone() {
            if let Some(heading_uuid) = &t.action_group
                && headings.contains_key(heading_uuid)
            {
                by_heading.entry(heading_uuid.clone()).or_default().push(t);
                continue;
            }
            ungrouped.push(t);
        }

        let mut sorted_headings = headings.values().collect::<Vec<_>>();
        sorted_headings.sort_by(|a, b| (a.index, &a.uuid).cmp(&(b.index, &b.uuid)));
        ungrouped.sort_by_key(|t| t.index);
        for items in by_heading.values_mut() {
            items.sort_by_key(|t| t.index);
        }

        // a heading shows whether it holds a to-do or not
        // reorder moves an empty one too
        let heading_groups = sorted_headings
            .iter()
            .map(|heading| ProjectHeadingGroup {
                uuid: heading.uuid.clone(),
                title: heading.title.clone(),
                items: by_heading
                    .get(&heading.uuid)
                    .map(|tasks| tasks.iter().collect::<Vec<_>>())
                    .unwrap_or_default(),
            })
            .collect::<Vec<_>>();

        let mut ui = element! {
            ContextProvider(value: Context::owned(store.clone())) {
                ContextProvider(value: Context::owned(today)) {
                    ProjectView(
                        project: &project,
                        ungrouped: ungrouped.iter().collect::<Vec<_>>(),
                        heading_groups,
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
