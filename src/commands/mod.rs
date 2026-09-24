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

use crate::{
    app::Cli,
    cmd_ctx::{CmdCtx, DefaultCmdCtx},
};

pub trait Command {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn std::io::Write,
        ctx: &mut dyn CmdCtx,
    ) -> Result<()>;

    fn run(&self, cli: &Cli, out: &mut dyn std::io::Write) -> Result<()> {
        let mut ctx = DefaultCmdCtx::from_cli(cli);
        self.run_with_ctx(cli, out, &mut ctx)
    }
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
        help = "Comma-separated tags to add (titles or UUID prefixes)"
    )]
    pub add_tags: Option<String>,
    #[arg(
        long = "remove-tags",
        short = 'r',
        help = "Comma-separated tags to remove (titles or UUID prefixes)"
    )]
    pub remove_tags: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    #[command(about = "Show the Inbox")]
    Inbox(inbox::InboxArgs),
    #[command(about = "Show the Today view (default)")]
    Today(today::TodayArgs),
    #[command(about = "Show tasks scheduled for the future")]
    Upcoming(upcoming::UpcomingArgs),
    #[command(about = "Show the Anytime view")]
    Anytime(anytime::AnytimeArgs),
    #[command(about = "Show the Someday view")]
    Someday(someday::SomedayArgs),
    #[command(about = "Show the Logbook")]
    Logbook(logbook::LogbookArgs),
    #[command(about = "Show the Trash")]
    Trash(trash::TrashArgs),
    #[command(about = "Show, create, or edit projects")]
    Projects(projects::ProjectsArgs),
    #[command(about = "Show all tasks in a project")]
    Project(project::ProjectArgs),
    #[command(about = "Show or create areas")]
    Areas(areas::AreasArgs),
    #[command(about = "Show projects and tasks in an area")]
    Area(area::AreaArgs),
    #[command(about = "Show or edit tags")]
    Tags(tags::TagsArgs),
    #[command(about = "Create a new task")]
    New(new::NewArgs),
    #[command(about = "Edit a task title, container, notes, tags, or checklist items")]
    Edit(edit::EditArgs),
    #[command(about = "Mark a task done, incomplete, or canceled")]
    Mark(mark::MarkArgs),
    #[command(about = "Reorder item relative to another item")]
    Reorder(reorder::ReorderArgs),
    #[command(about = "Move to-dos, projects and headings to the Trash, or delete an area")]
    Delete(delete::DeleteArgs),
    #[command(about = "Configure Things Cloud credentials")]
    Auth(auth::AuthArgs),
    #[command(about = "Search and filter tasks")]
    Find(find::FindArgs),
    #[command(about = "Show one task or project in full")]
    Show(show::ShowArgs),
    #[command(about = "Generate shell completion scripts")]
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
