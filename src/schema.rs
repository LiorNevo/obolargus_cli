//! The `schema` subcommand family.
//!
//! Contract: `specs/028-data-dictionary-json-schema/contracts/export-surfaces.md`
//! §3. The command reads no database or network state.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::CliError;

/// Arguments for `obolargus-cli schema`.
#[derive(Debug, Args, Clone)]
pub struct SchemaArgs {
    /// The nested verb.
    #[command(subcommand)]
    pub command: SchemaCommand,
}

/// The `schema` verbs.
#[derive(Debug, Subcommand, Clone)]
pub enum SchemaCommand {
    /// Export the complete system data dictionary as JSON Schema.
    Export {
        /// Write to a file instead of stdout.
        #[arg(long, value_name = "PATH")]
        output: Option<PathBuf>,
    },
}

/// Runs the `schema` subcommand family.
pub fn run(args: SchemaArgs) -> Result<(), CliError> {
    match args.command {
        SchemaCommand::Export { output } => export(output),
    }
}

fn export(output: Option<PathBuf>) -> Result<(), CliError> {
    let schema = obolargus_core::data_dictionary::export_system_schema()
        .map_err(|error| CliError::infrastructure(format!("schema export failed: {error}")))?;
    let canonical = schema.canonical_json();
    match output {
        Some(path) => {
            fs::write(&path, canonical).map_err(|error| {
                CliError::infrastructure(format!("failed to write {}: {error}", path.display()))
            })?;
            eprintln!("wrote {}", path.display());
        }
        None => {
            let mut stdout = io::stdout().lock();
            stdout.write_all(canonical.as_bytes()).map_err(|error| {
                CliError::infrastructure(format!("failed to write standard output: {error}"))
            })?;
            stdout.flush().map_err(|error| {
                CliError::infrastructure(format!("failed to flush standard output: {error}"))
            })?;
        }
    }
    Ok(())
}
