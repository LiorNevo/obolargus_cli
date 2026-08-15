# obolargus-cli

Command-line interface for the Obolargus platform. A git submodule of the
parent Obolargus repository. Command surface per `contracts/cli-contract.md`.

## Contents

- `src/main.rs` — clap surface (`import | eval | backtest | report | sync`,
  `--version`) with placeholder handlers that acknowledge and exit 0;
  unknown commands exit non-zero.

## Development

- Tests: `cargo test --all-targets`
- Lint: `cargo clippy --all-targets -- -D warnings`
- Coverage: `cargo llvm-cov --all-targets` (threshold 90%+)
- Docs: `cargo doc --no-deps`

From the parent repo: `make test|lint|test-coverage PROJ=obolargus-cli`.