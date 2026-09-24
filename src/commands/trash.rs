use std::{io::Write, sync::Arc};

use anyhow::Result;
use clap::Args;
use iocraft::prelude::*;

use crate::{
    app::Cli,
    commands::{Command, DetailedArgs, detailed_json_conflict, write_json},
    store::Task,
    ui::{
        render_element_to_string,
        views::{json::common::build_tasks_json, trash::TrashView},
    },
};

#[derive(Args)]
#[command(about = "Show the Trash")]
pub struct TrashArgs {
    #[command(flatten)]
    pub detailed: DetailedArgs,
}

impl Command for TrashArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn Write,
        ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        let store = Arc::new(cli.load_store()?);
        let today = ctx.today();
        let trash = store.trash();

        if cli.json {
            detailed_json_conflict(cli.json, self.detailed.detailed)?;
            // the rows in the order the view shows them, a trashed project followed by the to-dos it took along
            let rows = trash
                .iter()
                .flat_map(|(entry, held)| std::iter::once(entry).chain(held))
                .cloned()
                .collect::<Vec<Task>>();
            write_json(out, &build_tasks_json(&rows, &store, &today))?;
            return Ok(());
        }

        let entries = trash
            .iter()
            .map(|(entry, held)| (entry, held.iter().collect()))
            .collect::<Vec<_>>();
        let mut ui = element! {
            ContextProvider(value: Context::owned(store.clone())) {
                ContextProvider(value: Context::owned(today)) {
                    TrashView(entries, detailed: self.detailed.detailed)
                }
            }
        };
        let rendered = render_element_to_string(&mut ui, cli.no_color());
        writeln!(out, "{}", rendered)?;

        Ok(())
    }
}
