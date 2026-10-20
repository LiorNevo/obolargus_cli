//! obolargus-cli: batch operations, backtesting, data import, and headless
//! evaluation.
//!
//! Command surface contract:
//! `specs/001-boilerplate-submodules/contracts/cli-contract.md`.
//!
//! The `sync` subcommand's market-data behavior is specified in
//! `specs/017-yahoo-data-source/spec.md` (US1, FR-030..FR-032) and its
//! configuration in
//! `specs/017-yahoo-data-source/contracts/config-contract-additions.md`.

use clap::{Parser, Subcommand};

mod sync;

#[derive(Debug, Parser)]
#[command(
    name = "obolargus-cli",
    version,
    about = "CLI for batch operations, backtesting, data import, and headless evaluation"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Import data from CSV/OFX (placeholder)
    Import,
    /// Evaluate a rule headlessly (placeholder)
    Eval,
    /// Run a backtest (placeholder)
    Backtest,
    /// Generate a report (placeholder)
    Report,
    /// Synchronize market data (placeholder)
    Sync(sync::SyncArgs),
}

/// Runs the CLI, returning an error message for non-zero exit paths.
pub fn run(args: Vec<String>) -> Result<(), String> {
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                print!("{error}");
                return Ok(());
            }
            return Err(error.to_string());
        }
    };
    match cli.command {
        Command::Import => println!("obolargus-cli: import — placeholder acknowledged"),
        Command::Eval => println!("obolargus-cli: eval — placeholder acknowledged"),
        Command::Backtest => println!("obolargus-cli: backtest — placeholder acknowledged"),
        Command::Report => println!("obolargus-cli: report — placeholder acknowledged"),
        Command::Sync(args) => return sync::run(args),
    }
    Ok(())
}

fn main() {
    if let Err(message) = run(std::env::args().collect()) {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Command};

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    #[test]
    fn parses_every_placeholder_subcommand() {
        for arg in ["import", "eval", "backtest", "report", "sync"] {
            let cli = parse(&["obolargus-cli", arg]).expect("subcommand should parse");
            assert!(
                matches!(
                    cli.command,
                    Command::Import
                        | Command::Eval
                        | Command::Backtest
                        | Command::Report
                        | Command::Sync(_)
                ),
                "unexpected command for {arg}"
            );
        }
    }

    #[test]
    fn rejects_unknown_subcommand() {
        assert!(parse(&["obolargus-cli", "unknown-command"]).is_err());
    }

    #[test]
    fn run_acknowledges_placeholder_subcommand() {
        assert!(super::run(vec!["obolargus-cli".into(), "import".into()]).is_ok());
    }

    #[test]
    fn run_reports_unknown_command_as_error() {
        assert!(super::run(vec!["obolargus-cli".into(), "nope".into()]).is_err());
    }

    #[test]
    fn sync_without_asset_keeps_placeholder_acknowledgement() {
        // The CLI contract requires every placeholder subcommand to
        // acknowledge and exit 0. `sync` with no `--asset` preserves
        // that behavior; the real refresh path needs `--asset`.
        assert!(super::run(vec!["obolargus-cli".into(), "sync".into()]).is_ok());
    }
}
