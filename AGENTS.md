# AGENTS.md — obolargus-cli

Operational rules for contributors and AI agents working in this crate.

## Conventions (from `.specify/memory/constitution.md`)

- No `unsafe`, no `unwrap`, no `panic` in production code — use `Result`.
- Coverage must stay 90%+ (verified with `cargo-llvm-cov`).
- Every public item needs a rustdoc comment; documentation is code-driven.
- `rustfmt` and `clippy --all-targets -- -D warnings` must stay clean.

## Contract rules

- Honor `contracts/cli-contract.md`: `--version` prints semver and exits 0;
  each placeholder command acknowledges and exits 0; unknown commands exit
  non-zero. Executed via `tests/cli_exec.rs`.

## Commands

- Tests: `cargo test --all-targets --no-fail-fast`
- Lint: `cargo clippy --all-targets -- -D warnings`
- Coverage: `cargo llvm-cov --all-targets --no-fail-fast`
- Try it: `cargo run -- import`.
- Export the data-dictionary schema: `cargo run -- schema export` or
  `cargo run -- schema export --output <PATH>`. It reads no database or
  network state and must emit the same canonical bytes as the library,
  HTTP, and committed artifact surfaces.
