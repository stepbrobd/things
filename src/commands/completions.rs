use anyhow::Result;
use clap::{Args, CommandFactory, ValueEnum};
use clap_complete::{Shell, generate};
use clap_complete_nushell::Nushell;

use crate::{app::Cli, commands::Command};

#[derive(Clone, Copy, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Elvish,
    Fish,
    Nushell,
    Powershell,
    Zsh,
}

#[derive(Args)]
#[command(about = "Generate shell completion scripts")]
pub struct CompletionsArgs {
    #[arg(value_enum)]
    pub shell: CompletionShell,
}

impl Command for CompletionsArgs {
    fn run_with_ctx(
        &self,
        _cli: &Cli,
        out: &mut dyn std::io::Write,
        _ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        let mut cmd = Cli::command();
        let bin_name = cmd.get_name().to_string();
        match self.shell {
            CompletionShell::Bash => generate(Shell::Bash, &mut cmd, bin_name, out),
            CompletionShell::Elvish => generate(Shell::Elvish, &mut cmd, bin_name, out),
            CompletionShell::Fish => generate(Shell::Fish, &mut cmd, bin_name, out),
            CompletionShell::Nushell => generate(Nushell, &mut cmd, bin_name, out),
            CompletionShell::Powershell => generate(Shell::PowerShell, &mut cmd, bin_name, out),
            CompletionShell::Zsh => generate(Shell::Zsh, &mut cmd, bin_name, out),
        }
        Ok(())
    }
}
