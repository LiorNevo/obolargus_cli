//! obolargus-cli: batch operations, backtesting, data import, and headless
//! evaluation.
//!
//! Command surface contract:
//! `specs/001-boilerplate-submodules/contracts/cli-contract.md`.

use clap::{Parser, Subcommand};

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
    Sync,
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
        Command::Sync => println!("obolargus-cli: sync — placeholder acknowledged"),
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
        for (arg, expected) in [
            ("import", Command::Import),
            ("eval", Command::Eval),
            ("backtest", Command::Backtest),
            ("report", Command::Report),
            ("sync", Command::Sync),
        ] {
            let cli = parse(&["obolargus-cli", arg]).expect("subcommand should parse");
            assert!(
                matches!(
                    cli.command,
                    Command::Import
                        | Command::Eval
                        | Command::Backtest
                        | Command::Report
                        | Command::Sync
                ),
                "unexpected command for {arg}"
            );
            let _ = expected;
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
}
