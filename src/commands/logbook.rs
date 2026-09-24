use std::{io::Write, sync::Arc};

use anyhow::{Result, bail};
use clap::Args;
use iocraft::prelude::*;

use crate::{
    app::Cli,
    commands::{Command, DetailedArgs, detailed_json_conflict, write_json},
    common::parse_day,
    ui::{
        render_element_to_string,
        views::{json::common::build_tasks_json, logbook::LogbookView},
    },
};

#[derive(Args)]
#[command(about = "Show the Logbook")]
pub struct LogbookArgs {
    #[command(flatten)]
    pub detailed: DetailedArgs,
    #[arg(
        long = "from",
        short = 'f',
        help = "Show items completed or canceled on or after this date (YYYY-MM-DD)"
    )]
    pub from_date: Option<String>,
    #[arg(
        long = "to",
        short = 't',
        help = "Show items completed or canceled on or before this date (YYYY-MM-DD)"
    )]
    pub to_date: Option<String>,
}

impl Command for LogbookArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn Write,
        ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        let store = Arc::new(cli.load_store()?);
        let today = ctx.today();

        let from_day = self
            .from_date
            .as_deref()
            .map(|day| parse_day(day, "--from"))
            .transpose()
            .map_err(anyhow::Error::msg)?;
        let to_day = self
            .to_date
            .as_deref()
            .map(|day| parse_day(day, "--to"))
            .transpose()
            .map_err(anyhow::Error::msg)?;

        if let (Some(from), Some(to)) = (from_day, to_day)
            && from > to
        {
            bail!("--from date must be before or equal to --to date");
        }

        let tasks = store.logbook(from_day, to_day);

        let json = cli.json;
        if json {
            detailed_json_conflict(json, self.detailed.detailed)?;
            write_json(out, &build_tasks_json(&tasks, &store, &today))?;
            return Ok(());
        }

        let mut ui = element! {
            ContextProvider(value: Context::owned(store.clone())) {
                ContextProvider(value: Context::owned(today)) {
                    LogbookView(
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
