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
//!
//! The crate is split into a library target (this module) and a thin
//! binary target (`src/main.rs`). The binary only parses `argv` and maps
//! the outcome onto the process exit code; everything else lives here so
//! the integration tests under `tests/` can drive the command surface
//! directly.

use clap::{Parser, Subcommand};

pub mod sync;
pub mod temporal;

/// The `sync` subcommand family.
pub use sync::{SyncArgs, SyncCommand};
/// The `temporal` subcommand family.
pub use temporal::{TemporalArgs, TemporalCommand};

/// A CLI failure carrying the process exit code from
/// `specs/021-bitemporal-market-data/contracts/cli-contract.md` §8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    /// The process exit code.
    pub code: i32,
    /// The message printed to stderr.
    pub message: String,
}

impl CliError {
    /// Exit code 1: infrastructure or unexpected failure.
    pub fn infrastructure(message: impl Into<String>) -> Self {
        Self {
            code: 1,
            message: message.into(),
        }
    }
    /// Exit code 2: usage error.
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: message.into(),
        }
    }
    /// Exit code 3: no value at the requested instant.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: 3,
            message: message.into(),
        }
    }
    /// Exit code 5: verification failed.
    pub fn verification_failed(message: impl Into<String>) -> Self {
        Self {
            code: 5,
            message: message.into(),
        }
    }
    /// Exit code 1 with an explicit "not delivered yet" message. Kept
    /// distinct so the wording is uniform across forwards-compatible verbs.
    pub fn not_implemented(message: impl Into<String>) -> Self {
        Self {
            code: 1,
            message: format!("not implemented: {}", message.into()),
        }
    }
}

impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self::infrastructure(message)
    }
}

impl From<&str> for CliError {
    fn from(message: &str) -> Self {
        Self::infrastructure(message.to_string())
    }
}

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
    /// Bitemporal facts, positions, valuations, retention, and verification.
    Temporal(temporal::TemporalArgs),
}

/// Runs the CLI, returning an error message for non-zero exit paths.
pub fn run(args: Vec<String>) -> Result<(), CliError> {
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
            return Err(CliError::usage(error.to_string()));
        }
    };
    match cli.command {
        Command::Import => println!("obolargus-cli: import — placeholder acknowledged"),
        Command::Eval => println!("obolargus-cli: eval — placeholder acknowledged"),
        Command::Backtest => println!("obolargus-cli: backtest — placeholder acknowledged"),
        Command::Report => println!("obolargus-cli: report — placeholder acknowledged"),
        Command::Sync(args) => return sync::run(args).map_err(CliError::from),
        Command::Temporal(args) => return temporal::run(args),
    }
    Ok(())
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

    /// `--help` prints the usage text and exits 0 (the CLI contract).
    #[test]
    fn help_flag_prints_the_usage_and_succeeds() {
        assert!(super::run(vec!["obolargus-cli".into(), "--help".into()]).is_ok());
    }

    /// `--version` prints the semver and exits 0 (the CLI contract).
    #[test]
    fn version_flag_prints_the_version_and_succeeds() {
        assert!(super::run(vec!["obolargus-cli".into(), "--version".into()]).is_ok());
    }

    /// The `sync` subcommand's own `--help` also exits 0.
    #[test]
    fn sync_help_flag_prints_the_usage_and_succeeds() {
        assert!(super::run(vec!["obolargus-cli".into(), "sync".into(), "--help".into()]).is_ok());
    }
}
