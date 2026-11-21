//! Binary entry point. All behaviour lives in the `pu_get` library so it can be
//! tested without spawning a process.

use std::process::ExitCode;

use clap::Parser;
use pu_get::cli::{self, Cli, Command};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let interactive = cli
        .command
        .as_ref()
        .is_none_or(|command| matches!(command, Command::Browse));
    cli::init_tracing(cli.verbose, interactive);

    match cli::run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pu-get: error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
