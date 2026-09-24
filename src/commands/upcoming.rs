use std::{io::Write, sync::Arc};

use anyhow::Result;
use clap::Args;
use iocraft::prelude::*;

use crate::{
    app::Cli,
    commands::{Command, DetailedArgs, detailed_json_conflict, write_json},
    ui::{
        render_element_to_string,
        views::{json::common::build_tasks_json, upcoming::UpcomingView},
    },
    wire::task::TaskStatus,
};

#[derive(Debug, Default, Args)]
#[command(about = "Show tasks scheduled for the future")]
pub struct UpcomingArgs {
    #[command(flatten)]
    pub detailed: DetailedArgs,
}

impl Command for UpcomingArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn Write,
        ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        let store = Arc::new(cli.load_store()?);
        let today = ctx.today();
        let now_ts = today.timestamp();

        let mut tasks = Vec::new();
        for t in store.tasks(Some(TaskStatus::Incomplete)) {
            // a template shows through its projection alone
            if t.in_someday() || t.is_recurrence_template() || store.in_closed_container(&t) {
                continue;
            }
            let Some(start_date) = t.start_date else {
                continue;
            };
            if start_date.timestamp() > now_ts {
                tasks.push(t);
            }
        }
        tasks.extend(store.projected_repeats(today.date_naive()));
        tasks.sort_by(|a, b| {
            (a.start_date, a.index, &a.uuid).cmp(&(b.start_date, b.index, &b.uuid))
        });

        let json = cli.json;
        if json {
            detailed_json_conflict(json, self.detailed.detailed)?;
            write_json(out, &build_tasks_json(&tasks, &store, &today))?;
            return Ok(());
        }

        let mut ui = element! {
            ContextProvider(value: Context::owned(store.clone())) {
                ContextProvider(value: Context::owned(today)) {
                    UpcomingView(
                        items: &tasks,
                        detailed: self.detailed.detailed,
                    )
                }
            }
        };

        let rendered = render_element_to_string(&mut ui, cli.no_color());
        writeln!(out, "{}", rendered)?;
        Ok(())
    }
}
