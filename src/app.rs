use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    io::{IsTerminal, Read},
    path::PathBuf,
};

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
#[command(before_help = concat!("Things ", env!("CARGO_PKG_VERSION")))]
#[command(disable_help_subcommand = true)]
#[command(about = "Command-line interface for Things 3 via Cloud API")]
#[command(
    after_help = "Environment:\n  THINGS_EMAIL, THINGS_PASSWORD    Things Cloud credentials, over the auth file\n  THINGS_LOG                       Log filter directive, for example debug\n  THINGS_LOG_FORMAT                pretty, simplified or json\n  NO_COLOR                         Disable color\n  XDG_CONFIG_HOME, XDG_STATE_HOME  Where the auth file and the sync log live"
)]
pub struct Cli {
    /// Output JSON when supported by the selected command
    #[arg(long, global = true)]
    pub json: bool,
    /// For testing: disable cloud sync and cloud writes
    #[arg(long, hide = true)]
    pub no_cloud: bool,
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
    /// set when the sync failed and the cached state is in use
    #[arg(skip)]
    pub offline: Cell<bool>,
}

impl Cli {
    /// color goes to a terminal unless NO_COLOR is set
    pub fn no_color(&self) -> bool {
        !std::io::stdout().is_terminal()
            || std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
    }

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

        let cache_dir = append_log_dir();
        if self.no_cloud {
            return fold_state_from_append_log(&cache_dir);
        }

        let (email, password) = load_auth()?;
        let mut client = ThingsCloudClient::new(email, password)?;
        match get_state_with_append_log(&mut client, cache_dir.clone()) {
            Ok(state) => Ok(state),
            Err(err) => {
                eprintln!("Sync failed, showing the cached state: {err:#}");
                self.offline.set(true);
                fold_state_from_append_log(&cache_dir)
            }
        }
    }

    pub fn load_store(&self) -> Result<ThingsStore> {
        let state = self.load_state()?;
        Ok(ThingsStore::from_raw_state(&state))
    }
}

pub fn run() -> Result<()> {
    let mut cli = Cli::parse();
    logging::init();
    let command = cli
        .command
        .take()
        .unwrap_or(Commands::Today(Default::default()));
    let mut ctx = DefaultCmdCtx::from_cli(&cli);
    if !matches!(command, Commands::Auth(_) | Commands::Completions(_)) {
        materialize_due(&cli, &mut ctx)?;
    }
    command.run_with_ctx(&cli, &mut std::io::stdout(), &mut ctx)
}

/// create the instances repeating templates are due for, the way the Apple clients do on their day
fn materialize_due(cli: &Cli, ctx: &mut dyn CmdCtx) -> Result<()> {
    let mut state = cli.load_state()?;
    if cli.offline.get() {
        return Ok(());
    }
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
