//! CLI execution contract tests.
//!
//! See `contracts/cli-contract.md`.

use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_obolargus-cli"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn version_flag_prints_semver_and_exits_zero() {
    let output = run(&["--version"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("obolargus-cli"), "stdout: {stdout}");
}

#[test]
fn every_placeholder_command_acknowledges_and_exits_zero() {
    for command in ["import", "eval", "backtest", "report", "sync"] {
        let output = run(&[command]);
        assert!(output.status.success(), "command {command} exited non-zero");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("placeholder acknowledged"),
            "command {command} stdout: {stdout}"
        );
    }
}

#[test]
fn unknown_command_exits_non_zero() {
    let output = run(&["frobnicate"]);
    assert!(!output.status.success(), "expected non-zero exit");
}
