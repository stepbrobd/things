use std::{io::Write, sync::Arc};

use anyhow::Result;
use clap::Args;
use iocraft::prelude::*;

use crate::{
    app::Cli,
    commands::{Command, DetailedArgs, detailed_json_conflict, write_json},
    ordering::today_view_order,
    ui::{
        render_element_to_string,
        views::{json::common::build_tasks_json, today::TodayView},
    },
    wire::task::TaskStatus,
};

#[derive(Default, Args)]
#[command(about = "Show the Today view (default)")]
pub struct TodayArgs {
    #[command(flatten)]
    pub detailed: DetailedArgs,
}

impl Command for TodayArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn Write,
        ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        let store = Arc::new(cli.load_store()?);
        let today = ctx.today();

        let mut today_items: Vec<_> = store
            .tasks(Some(TaskStatus::Incomplete))
            .into_iter()
            .filter(|t| store.in_today(t, &today))
            .collect();

        today_items.sort_by_key(today_view_order);

        let json = cli.json;
        if json {
            detailed_json_conflict(json, self.detailed.detailed)?;
            write_json(out, &build_tasks_json(&today_items, &store, &today))?;
            return Ok(());
        }

        let mut ui = element! {
            ContextProvider(value: Context::owned(store.clone())) {
                ContextProvider(value: Context::owned(today)) {
                    TodayView(
                        items: &today_items,
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
