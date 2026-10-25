//! Contract tests for the `sync` subcommand's command surface.
//!
//! Implements the executable proof of the CLI-side error and dry-run
//! behaviour of spec 017 (FR-030..FR-032) and the `cli-contract.md`
//! rules that every `sync` invocation must honour.
//!
//! Every case here is reachable **without** a database: the argument
//! validation runs before the runtime context (and therefore before any
//! connection) is built, and the offline lookup path returns before the
//! registry is needed.
//!
//! The database-backed half of the command surface lives in
//! `tests/sync_db.rs`.

use obolargus_cli::sync::{LookupArgs, ReconcileArgs, SyncArgs, SyncCommand};

/// Builds the `sync lookup` arguments.
fn lookup(query: &str, format: &str, offline: bool) -> SyncArgs {
    SyncArgs {
        assets: Vec::new(),
        command: Some(SyncCommand::Lookup(LookupArgs {
            query: query.to_string(),
            format: format.to_string(),
            limit: 10,
            offline,
        })),
    }
}

/// Builds the `sync reconcile` arguments.
fn reconcile(asset: &str, as_of: &str, policy: &str) -> SyncArgs {
    SyncArgs {
        assets: Vec::new(),
        command: Some(SyncCommand::Reconcile(ReconcileArgs {
            asset: asset.to_string(),
            as_of: as_of.to_string(),
            policy: policy.to_string(),
        })),
    }
}

/// An empty query is rejected before any provider is contacted (US2).
#[test]
fn lookup_rejects_an_empty_query() {
    let error = obolargus_cli::sync::run(lookup("", "json", true)).expect_err("err");
    assert_eq!(error, "--query must not be empty");
}

/// A whitespace-only query is rejected too — the query is trimmed first.
#[test]
fn lookup_rejects_a_whitespace_only_query() {
    let error = obolargus_cli::sync::run(lookup("   \t ", "json", true)).expect_err("err");
    assert_eq!(error, "--query must not be empty");
}

/// An unknown output format is rejected (US2).
#[test]
fn lookup_rejects_an_unknown_format() {
    let error = obolargus_cli::sync::run(lookup("AAPL", "xml", true)).expect_err("err");
    assert_eq!(error, "unknown --format 'xml' (expected 'json' or 'table')");
}

/// The offline lookup prints the documented shape without contacting any
/// provider (US2).
#[test]
fn lookup_offline_succeeds_without_a_provider() {
    assert!(obolargus_cli::sync::run(lookup("AAPL", "json", true)).is_ok());
}

/// The offline lookup also supports the table format (US2).
#[test]
fn lookup_offline_supports_the_table_format() {
    assert!(obolargus_cli::sync::run(lookup("AAPL", "table", true)).is_ok());
}

/// A `--asset` that is not a UUID and not a `<scheme>:<value>` pair is
/// rejected (FR-030).
#[test]
fn refresh_rejects_a_malformed_asset_argument() {
    let error = obolargus_cli::sync::run(SyncArgs {
        assets: vec!["not-an-id".to_string()],
        command: None,
    })
    .expect_err("err");
    assert_eq!(
        error,
        "--asset 'not-an-id' is neither a UUID nor a '<scheme>:<value>' pair"
    );
}

/// An empty `--asset` is rejected (FR-030).
#[test]
fn refresh_rejects_an_empty_asset_argument() {
    let error = obolargus_cli::sync::run(SyncArgs {
        assets: vec![String::new()],
        command: None,
    })
    .expect_err("err");
    assert_eq!(error, "--asset must not be empty");
}

/// A `<scheme>:<value>` pair with an empty value is rejected (FR-030).
#[test]
fn refresh_rejects_an_incomplete_typed_asset_argument() {
    let error = obolargus_cli::sync::run(SyncArgs {
        assets: vec!["TICKER:".to_string()],
        command: None,
    })
    .expect_err("err");
    assert_eq!(
        error,
        "--asset 'TICKER:' is neither a UUID nor a '<scheme>:<value>' pair"
    );
}

/// `sync` with no `--asset` keeps the historical placeholder
/// acknowledgement (the CLI contract requires exit 0).
#[test]
fn refresh_without_an_asset_acknowledges_the_placeholder() {
    assert!(
        obolargus_cli::sync::run(SyncArgs {
            assets: Vec::new(),
            command: None,
        })
        .is_ok()
    );
}

/// A `--asset` that is not a valid UUID is rejected by `sync reconcile`.
#[test]
fn reconcile_rejects_a_malformed_asset_argument() {
    let error = obolargus_cli::sync::run(reconcile("not-a-uuid", "2026-01-02", "primary_wins"))
        .expect_err("err");
    assert_eq!(error, "--asset 'not-a-uuid' is not a valid UUID");
}

/// An `--as-of` that is not a `YYYY-MM-DD` date is rejected (FR-021).
#[test]
fn reconcile_rejects_a_malformed_as_of_date() {
    let uuid = "00000000-0000-0000-0000-000000000001";
    let error =
        obolargus_cli::sync::run(reconcile(uuid, "2026-13-01", "primary_wins")).expect_err("err");
    assert_eq!(error, "--as-of '2026-13-01' is not a valid YYYY-MM-DD date");
}

/// The deferred `majority` policy is rejected with a pointer to the
/// shipped policies (research D9).
#[test]
fn reconcile_rejects_the_deferred_majority_policy() {
    let uuid = "00000000-0000-0000-0000-000000000001";
    let error =
        obolargus_cli::sync::run(reconcile(uuid, "2026-01-02", "majority")).expect_err("err");
    assert_eq!(
        error,
        "ReconciliationPolicy::Majority is deferred; use primary_wins or first_non_empty"
    );
}

/// An unknown policy name is rejected (FR-020).
#[test]
fn reconcile_rejects_an_unknown_policy() {
    let uuid = "00000000-0000-0000-0000-000000000001";
    let error =
        obolargus_cli::sync::run(reconcile(uuid, "2026-01-02", "nonsense")).expect_err("err");
    assert_eq!(
        error,
        "unknown --policy 'nonsense' (expected 'primary_wins' or 'first_non_empty')"
    );
}

/// The `sync` subcommand accepts the documented flag spellings through
/// `clap` (FR-030..FR-032).
#[test]
fn sync_accepts_the_documented_flags() {
    use clap::Parser;

    let cli = obolargus_cli::Cli::try_parse_from([
        "obolargus-cli",
        "sync",
        "--asset",
        "TICKER:AAPL",
        "--asset",
        "00000000-0000-0000-0000-000000000001",
    ])
    .expect("parse");
    match cli.command {
        obolargus_cli::Command::Sync(args) => {
            assert_eq!(args.assets.len(), 2);
            assert!(args.command.is_none());
        }
        other => panic!("expected the sync command, got {other:?}"),
    }

    let cli = obolargus_cli::Cli::try_parse_from([
        "obolargus-cli",
        "sync",
        "lookup",
        "--query",
        "AAPL",
        "--format",
        "table",
        "--limit",
        "3",
        "--offline",
    ])
    .expect("parse");
    match cli.command {
        obolargus_cli::Command::Sync(args) => match args.command {
            Some(SyncCommand::Lookup(lookup_args)) => {
                assert_eq!(lookup_args.query, "AAPL");
                assert_eq!(lookup_args.format, "table");
                assert_eq!(lookup_args.limit, 3);
                assert!(lookup_args.offline);
            }
            other => panic!("expected the lookup command, got {other:?}"),
        },
        other => panic!("expected the sync command, got {other:?}"),
    }

    let cli = obolargus_cli::Cli::try_parse_from([
        "obolargus-cli",
        "sync",
        "reconcile",
        "--asset",
        "00000000-0000-0000-0000-000000000001",
        "--as-of",
        "2026-01-02",
        "--policy",
        "first_non_empty",
    ])
    .expect("parse");
    match cli.command {
        obolargus_cli::Command::Sync(args) => match args.command {
            Some(SyncCommand::Reconcile(reconcile_args)) => {
                assert_eq!(reconcile_args.asset, "00000000-0000-0000-0000-000000000001");
                assert_eq!(reconcile_args.as_of, "2026-01-02");
                assert_eq!(reconcile_args.policy, "first_non_empty");
            }
            other => panic!("expected the reconcile command, got {other:?}"),
        },
        other => panic!("expected the sync command, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// `sync load-range` — FR-014 of spec 018
// ---------------------------------------------------------------------------

/// Builds the `sync load-range` arguments.
fn load_range(
    identifiers: &[&str],
    from: &str,
    to: &str,
    concurrency: usize,
    format: &str,
) -> SyncArgs {
    SyncArgs {
        assets: Vec::new(),
        command: Some(SyncCommand::LoadRange(obolargus_cli::sync::LoadRangeArgs {
            identifiers: identifiers.iter().map(|s| s.to_string()).collect(),
            from: from.to_string(),
            to: to.to_string(),
            concurrency,
            source: None,
            create_missing_assets: false,
            format: format.to_string(),
        })),
    }
}

/// An unknown output format is rejected before any date is attempted.
#[test]
fn load_range_rejects_an_unknown_format() {
    let error = obolargus_cli::sync::run(load_range(
        &["TICKER:AAPL"],
        "2026-10-01",
        "2026-10-02",
        2,
        "xml",
    ))
    .expect_err("err");
    assert_eq!(error, "unknown --format 'xml' (expected 'json' or 'table')");
}

/// At least one identifier is required.
#[test]
fn load_range_requires_an_identifier() {
    let error = obolargus_cli::sync::run(load_range(&[], "2026-10-01", "2026-10-02", 2, "json"))
        .expect_err("err");
    assert_eq!(error, "--identifier is required at least once");
}

/// An unparseable `--from` is rejected.
#[test]
fn load_range_rejects_an_unparseable_from() {
    let error = obolargus_cli::sync::run(load_range(
        &["TICKER:AAPL"],
        "10/01/2026",
        "2026-10-02",
        2,
        "json",
    ))
    .expect_err("err");
    assert!(error.contains("--from"), "error: {error}");
}

/// A reversed range is rejected before the database is needed.
#[test]
fn load_range_rejects_a_reversed_range() {
    let error = obolargus_cli::sync::run(load_range(
        &["TICKER:AAPL"],
        "2026-10-03",
        "2026-10-02",
        2,
        "json",
    ))
    .expect_err("err");
    assert_eq!(error, "--from 2026-10-03 is later than --to 2026-10-02");
}

/// A future `--to` is rejected before the database is needed.
#[test]
fn load_range_rejects_a_future_to() {
    let future = (chrono::Utc::now().date_naive() + chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    let error = obolargus_cli::sync::run(load_range(
        &["TICKER:AAPL"],
        "2026-10-01",
        &future,
        2,
        "json",
    ))
    .expect_err("err");
    assert!(error.contains("is later than today"), "error: {error}");
}

/// The window must be in `1..=MAX_CONCURRENCY`.
#[test]
fn load_range_rejects_an_invalid_window() {
    for concurrency in [
        0,
        obolargus_core::market_data::load_range::MAX_CONCURRENCY + 1,
    ] {
        let error = obolargus_cli::sync::run(load_range(
            &["TICKER:AAPL"],
            "2026-10-01",
            "2026-10-02",
            concurrency,
            "json",
        ))
        .expect_err("err");
        assert!(
            error.contains("--concurrency must be between"),
            "error: {error}"
        );
    }
}
