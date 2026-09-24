pub mod anytime;
pub mod area;
pub mod areas;
pub mod auth;
pub mod completions;
pub mod delete;
pub mod edit;
pub mod find;
pub mod inbox;
pub mod logbook;
pub mod mark;
pub mod new;
pub mod project;
pub mod projects;
pub mod reorder;
pub mod show;
pub mod someday;
pub mod tags;
pub mod today;
pub mod trash;
pub mod upcoming;

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use serde::Serialize;

use crate::{app::Cli, cmd_ctx::CmdCtx};

pub trait Command {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn std::io::Write,
        ctx: &mut dyn CmdCtx,
    ) -> Result<()>;
}

pub(crate) fn detailed_json_conflict(json: bool, detailed: bool) -> Result<()> {
    if json && detailed {
        bail!("--detailed is not supported with --json.");
    }
    Ok(())
}

pub(crate) fn write_json<T: Serialize>(out: &mut dyn std::io::Write, value: &T) -> Result<()> {
    serde_json::to_writer_pretty(&mut *out, value)?;
    writeln!(out)?;
    Ok(())
}

#[derive(Debug, Default, Clone, Args)]
pub struct DetailedArgs {
    /// Show notes beneath each task
    #[arg(long, short = 'd')]
    pub detailed: bool,
}

#[derive(Debug, Default, Clone, Args)]
pub struct TagDeltaArgs {
    #[arg(
        long = "add-tags",
        short = 'a',
        help = "Comma-separated tags to add (titles or ID prefixes)"
    )]
    pub add_tags: Option<String>,
    #[arg(
        long = "remove-tags",
        short = 'r',
        help = "Comma-separated tags to remove (titles or ID prefixes)"
    )]
    pub remove_tags: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Inbox(inbox::InboxArgs),
    Today(today::TodayArgs),
    Upcoming(upcoming::UpcomingArgs),
    Anytime(anytime::AnytimeArgs),
    Someday(someday::SomedayArgs),
    Logbook(logbook::LogbookArgs),
    Trash(trash::TrashArgs),
    Projects(projects::ProjectsArgs),
    Project(project::ProjectArgs),
    Areas(areas::AreasArgs),
    Area(area::AreaArgs),
    Tags(tags::TagsArgs),
    New(new::NewArgs),
    Edit(edit::EditArgs),
    Mark(mark::MarkArgs),
    Reorder(reorder::ReorderArgs),
    Delete(delete::DeleteArgs),
    Auth(auth::AuthArgs),
    Find(find::FindArgs),
    Show(show::ShowArgs),
    Completions(completions::CompletionsArgs),
}

impl Command for Commands {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn std::io::Write,
        ctx: &mut dyn CmdCtx,
    ) -> Result<()> {
        match self {
            Commands::Inbox(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Today(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Upcoming(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Anytime(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Someday(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Logbook(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Trash(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Projects(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Project(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Areas(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Area(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Tags(args) => args.run_with_ctx(cli, out, ctx),
            Commands::New(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Edit(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Mark(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Reorder(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Delete(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Auth(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Find(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Show(args) => args.run_with_ctx(cli, out, ctx),
            Commands::Completions(args) => args.run_with_ctx(cli, out, ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::app::Cli;

    /// the flag combinations clap refuses before any command runs
    #[test]
    fn conflicting_and_missing_arguments_are_refused_at_parse_time() {
        let refused = |args: &[&str]| {
            let mut argv = vec!["things"];
            argv.extend_from_slice(args);
            Cli::try_parse_from(argv).is_err()
        };
        assert!(refused(&["edit"]));
        assert!(refused(&[
            "edit",
            "A",
            "--deadline",
            "2027-01-15",
            "--clear-deadline"
        ]));
        assert!(refused(&[
            "edit",
            "A",
            "--reminder",
            "09:00",
            "--clear-reminder"
        ]));
        assert!(refused(&["edit", "A", "--times", "3"]));
        assert!(refused(&[
            "edit",
            "A",
            "--repeat",
            "daily",
            "--times",
            "3",
            "--until",
            "2027-01-15"
        ]));
        assert!(refused(&["new", "x", "--before", "A", "--after", "B"]));
        assert!(refused(&["new", "x", "--until", "2027-01-15"]));
        assert!(refused(&["reorder", "A"]));
        assert!(refused(&[
            "reorder",
            "A",
            "--before-id",
            "B",
            "--after-id",
            "C"
        ]));
        assert!(refused(&["mark", "--done"]));
        assert!(refused(&["delete"]));
        assert!(!refused(&[
            "edit", "A", "--repeat", "daily", "--times", "3"
        ]));
        assert!(!refused(&["reorder", "A", "--after-id", "B"]));
    }
}
