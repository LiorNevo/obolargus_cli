//! Contract tests for `obolargus-cli schema export`.

use std::path::PathBuf;
use std::process::{Command, Output};

use obolargus_core::data_dictionary::export_system_schema;

const ARTIFACT: &str = include_str!("../../obolargus_core/schema/system-data-dictionary.json");

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_obolargus-cli"))
        .args(args)
        .output()
        .expect("run cli")
}

fn temporary_path(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "obolargus-schema-cli-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("temp directory");
    directory.join("schema.json")
}

#[test]
fn stdout_export_matches_the_library_and_artifact() {
    let output = run(&["schema", "export"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let schema = export_system_schema().expect("schema");
    assert_eq!(stdout, schema.canonical_json());
    assert_eq!(stdout, ARTIFACT);
}

#[test]
fn output_flag_writes_the_same_bytes_and_confirms_on_stderr() {
    let path = temporary_path("file");
    let output = run(&["schema", "export", "--output", path.to_str().expect("path")]);
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    assert!(stderr.contains("wrote"), "stderr: {stderr}");
    let written = std::fs::read_to_string(&path).expect("written file");
    let schema = export_system_schema().expect("schema");
    assert_eq!(written, schema.canonical_json());
    let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
}

#[test]
fn help_and_version_exit_zero() {
    for arguments in [
        vec!["schema", "--help"],
        vec!["schema", "export", "--help"],
        vec!["--version"],
    ] {
        let output = run(&arguments);
        assert!(output.status.success(), "{arguments:?} failed");
    }
}

#[test]
fn invalid_arguments_exit_two() {
    let output = run(&["schema", "export", "--output"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn write_failure_exits_one_and_leaves_no_partial_file() {
    let path = std::env::temp_dir()
        .join(format!("obolargus-schema-missing-{}", std::process::id()))
        .join("nested")
        .join("schema.json");
    let output = run(&["schema", "export", "--output", path.to_str().expect("path")]);
    assert_eq!(output.status.code(), Some(1));
    assert!(!path.exists(), "partial file was created");
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    assert!(stderr.contains("failed to write"), "stderr: {stderr}");
}
