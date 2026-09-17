use std::{cell::RefCell, collections::BTreeMap, io::Read, path::PathBuf};

use anyhow::{Context, Result};
use clap::Parser;

use crate::{
    auth::load_auth,
    client::ThingsCloudClient,
    cmd_ctx::{CmdCtx, DefaultCmdCtx},
    commands::{Command, Commands},
    common::ICONS,
    dirs::append_log_dir,
    log_cache::{fold_state_from_append_log, get_state_with_append_log},
    logging, repeat,
    store::{RawState, ThingsStore, fold_item, fold_items},
    wire::wire_object::WireItem,
};

#[derive(Debug, Parser)]
#[command(name = "things")]
#[command(bin_name = "things")]
#[command(version)]
#[command(before_help = concat!("things ", env!("CARGO_PKG_VERSION")))]
#[command(disable_help_subcommand = true)]
#[command(about = concat!(
    "things v",
    env!("CARGO_PKG_VERSION"),
    ": Command-line interface for Things 3 via Cloud API"
))]
pub struct Cli {
    /// Disable color output
    #[arg(long)]
    pub no_color: bool,
    /// Output JSON when supported by the selected command
    #[arg(long, global = true)]
    pub json: bool,
    /// Skip cloud sync and use local cache only
    #[arg(long)]
    pub no_sync: bool,
    /// Leave due instances of repeating to-dos for the Apple clients to create
    #[arg(long, global = true)]
    pub no_materialize: bool,
    /// For testing: disable cloud sync and cloud writes
    #[arg(long, hide = true)]
    pub no_cloud: bool,
    /// Set the log level filter
    #[arg(long, value_enum, default_value_t = logging::Level::Info)]
    pub log_level: logging::Level,
    /// Set the logging output format
    #[arg(long, value_enum, default_value_t = logging::LogFormat::Auto)]
    pub log_format: logging::LogFormat,
    /// For testing: advanced tracing filter directive
    #[arg(long, global = true, hide = true, value_name = "DIRECTIVE")]
    pub log_filter: Option<String>,
    /// For testing: override "today" UTC midnight timestamp
    #[arg(long, global = true, hide = true, value_name = "TIMESTAMP")]
    pub today_ts: Option<i64>,
    /// For testing: override current UNIX timestamp
    #[arg(long, global = true, hide = true, value_name = "TIMESTAMP")]
    pub now_ts: Option<f64>,
    /// For testing: derive new ids from a seed instead of random bytes
    #[arg(long, global = true, hide = true, value_name = "SEED")]
    pub id_seed: Option<u64>,
    /// For testing: load state from a JSON journal file instead of syncing.
    /// The file must contain a JSON array of WireItem objects (each is a
    /// map of uuid -> WireObject).
    #[arg(long, value_name = "FILE", hide = true)]
    pub load_journal: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Commands>,
    /// the state loaded by this run, so a command after the materialization pass does not sync twice
    #[arg(skip)]
    pub state_cache: RefCell<Option<RawState>>,
}

impl Cli {
    pub fn load_state(&self) -> Result<RawState> {
        if let Some(state) = self.state_cache.borrow().as_ref() {
            return Ok(state.clone());
        }
        let state = self.load_state_fresh()?;
        *self.state_cache.borrow_mut() = Some(state.clone());
        Ok(state)
    }

    fn load_state_fresh(&self) -> Result<RawState> {
        if let Some(journal_path) = &self.load_journal {
            let raw = if journal_path == std::path::Path::new("-") {
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .with_context(|| "failed to read journal JSON from stdin")?;
                buf
            } else {
                std::fs::read_to_string(journal_path).with_context(|| {
                    format!("failed to read journal file {}", journal_path.display())
                })?
            };
            let items: Vec<WireItem> =
                serde_json::from_str(&raw).with_context(|| "failed to parse journal JSON")?;
            return Ok(fold_items(items));
        }

        if self.no_sync || self.no_cloud {
            let cache_dir = append_log_dir();
            return fold_state_from_append_log(&cache_dir);
        }

        let (email, password) = load_auth()?;
        let mut client = ThingsCloudClient::new(email, password)?;
        let cache_dir = append_log_dir();
        get_state_with_append_log(&mut client, cache_dir)
    }

    pub fn load_store(&self) -> Result<ThingsStore> {
        let state = self.load_state()?;
        Ok(ThingsStore::from_raw_state(&state))
    }
}

pub fn run() -> Result<()> {
    let mut cli = Cli::parse();
    logging::init(cli.log_level, cli.log_format, cli.log_filter.as_deref());
    let command = cli
        .command
        .take()
        .unwrap_or(Commands::Today(Default::default()));
    let mut ctx = DefaultCmdCtx::from_cli(&cli);
    if !matches!(
        command,
        Commands::SetAuth(_) | Commands::Completions(_) | Commands::Webserver(_)
    ) {
        materialize_due(&cli, &mut ctx)?;
    }
    command.run_with_ctx(&cli, &mut std::io::stdout(), &mut ctx)
}

/// create the instances repeating templates are due for, the way the Apple clients do on their day
fn materialize_due(cli: &Cli, ctx: &mut dyn CmdCtx) -> Result<()> {
    if cli.no_sync || cli.no_materialize {
        return Ok(());
    }
    let mut state = cli.load_state()?;
    let store = ThingsStore::from_raw_state(&state);
    let today = ctx.today().date_naive();
    let now = ctx.now_timestamp();
    let mut next_id = || ctx.next_id();
    let due = repeat::due_instances(&store, today, now, &mut next_id);
    if due.is_empty() {
        return Ok(());
    }
    let mut changes = BTreeMap::new();
    for materialized in &due {
        changes.extend(materialized.changes.clone());
    }
    ctx.commit_changes(changes.clone(), None)
        .with_context(|| "failed to create due instances of repeating to-dos")?;
    fold_item(changes, &mut state);
    *cli.state_cache.borrow_mut() = Some(state);
    for materialized in due {
        eprintln!(
            "{} Created {} for {}  {}",
            ICONS.repeat, materialized.title, materialized.day, materialized.instance_id
        );
    }
    Ok(())
}
