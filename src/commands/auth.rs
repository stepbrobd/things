use std::{
    fs::OpenOptions,
    io::{self, Write},
};

use anyhow::{Context as _, Result, bail};
use clap::Args;

use crate::{app::Cli, auth::write_verified_auth, client::ThingsCloudClient, commands::Command};

#[derive(Args)]
#[command(about = "Configure Things Cloud credentials")]
pub struct AuthArgs {}

/// the terminal that rpassword prompts on
#[cfg(unix)]
const TERMINAL: &str = "/dev/tty";
#[cfg(not(unix))]
const TERMINAL: &str = "CONOUT$";

impl Command for AuthArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn std::io::Write,
        _ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        // the email prompt goes to the terminal
        // rpassword writes the password prompt there too
        // standard output keeps the result alone
        let mut terminal = OpenOptions::new()
            .write(true)
            .open(TERMINAL)
            .context("Failed to open the terminal for the prompts")?;
        write!(terminal, "Things Cloud email: ")
            .and_then(|()| terminal.flush())
            .context("Failed to write the prompt to the terminal")?;
        let mut email = String::new();
        io::stdin()
            .read_line(&mut email)
            .context("Failed to read the email from standard input")?;
        // an input that ends before an email asks for no password
        if email.trim().is_empty() {
            bail!("The Things Cloud email is empty.");
        }

        let password = rpassword::prompt_password("Things Cloud password: ")
            .context("Failed to read the password from the terminal")?;

        // the prompt strips the newline
        // trailing spaces are part of the password
        let path = write_verified_auth(email.trim(), &password, |email, password| {
            if cli.no_cloud {
                return Ok(());
            }
            let mut client = ThingsCloudClient::new(email.to_string(), password.to_string())?;
            client.authenticate()?;
            Ok(())
        })?;
        writeln!(out, "Saved auth to {}", path.display())?;
        Ok(())
    }
}
