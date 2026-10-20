//! obolargus-cli binary entry point.
//!
//! The command surface lives in the library target (`obolargus_cli`); this
//! binary only maps the outcome onto the process exit code so the crate
//! keeps a thin, testable separation between "what the CLI does" and "how
//! the process reports it".

#![forbid(unsafe_code)]

use obolargus_cli::run;

fn main() {
    if let Err(message) = run(std::env::args().collect()) {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
