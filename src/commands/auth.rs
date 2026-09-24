use std::io::{self, Write};

use anyhow::Result;
use clap::Args;

use crate::{app::Cli, auth::write_verified_auth, client::ThingsCloudClient, commands::Command};

#[derive(Args)]
#[command(about = "Configure Things Cloud credentials")]
pub struct AuthArgs {}

impl Command for AuthArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn std::io::Write,
        _ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        print!("Things Cloud email: ");
        io::stdout().flush()?;
        let mut email = String::new();
        io::stdin().read_line(&mut email)?;

        let password = rpassword::prompt_password("Things Cloud password: ")?;

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
