//! Database-backed contract tests for the `sync` subcommand.
//!
//! Implements the executable proof of the CLI half of spec 017
//! (FR-030..FR-032): the refresh path resolves `--asset` arguments against
//! the database, connects to the configured database, and drives the
//! refresh orchestrator.
//!
//! The command surface's argument-validation half lives in
//! `tests/sync_cli.rs`; the offline lookup path needs no database at all.
//!
//! The tests are gated on `TEST_DATABASE_URL` / `DATABASE_URL` and skip
//! (as a passing no-op) when no database is configured, per the
//! `obolargus_cli/AGENTS.md` convention.
//!
//! **These are plain `#[test]` functions, not `#[tokio::test]`.** The
//! `sync` command surface is synchronous: `sync::run` builds its own
//! tokio runtime and drives the async work with `block_on`. Calling it
//! from inside an existing runtime panics ("Cannot start a runtime from
//! within a runtime"), so the tests own a runtime for their own database
//! work and call the CLI outside of it.
//!
//! **Serial execution required.** These tests mutate the process
//! environment (`DATABASE_URL`, `OBOLARGUS_SYNC_DRY_RUN`,
//! `MARKET_DATA_SOURCES`) because the CLI reads its configuration through
//! `envconfig`. The workspace sets `RUST_TEST_THREADS=1` in the root
//! `.cargo/config.toml`, so no other thread can observe an inconsistent
//! environment.
//!
//! They also share the `TEST_DATABASE_URL` database with
//! `obolargus_core`'s and `obolargus_server`'s integration tests, which
//! truncate the same tables. Run one crate's suite at a time — the root
//! `Makefile`'s `test` and `coverage` targets do exactly that.

use obolargus_cli::sync::{ReconcileArgs, SyncArgs, SyncCommand};

/// The market-data configuration used by the tests: a single **disabled**
/// source. A disabled source is filtered out of the registry, so the
/// refresh finds no candidate and fails fast — no network call is made.
const DISABLED_SOURCE_CONFIG: &str =
    r#"[{"source_id":"disabled_source","provider_type":"yahoo_finance","enabled":false}]"#;

/// Connects to the test database, applies the migrations and returns the
/// pool together with the URL it was opened with.
///
/// Returns `None` when no database is configured (the test then skips).
async fn migrated_pool() -> Option<(sqlx::PgPool, String)> {
    let url = std::env::var("TEST_DATABASE_URL")
        .ok()
        .or_else(|| std::env::var("DATABASE_URL").ok())?;
    let pool = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
    {
        Ok(pool) => pool,
        Err(error) => {
            eprintln!("SKIPPED: could not connect to the test database: {error}");
            return None;
        }
    };
    if let Err(error) = sqlx::migrate!("../obolargus_core/migrations")
        .run(&pool)
        .await
    {
        eprintln!("SKIPPED: could not apply the migrations: {error}");
        return None;
    }
    Some((pool, url))
}

/// Points the CLI at a controlled environment for the duration of a test.
///
/// `set_var` / `remove_var` are unsafe on edition 2024 because they race
/// with concurrent readers of the process environment. The test harness
/// runs the suite serially (`RUST_TEST_THREADS=1` in the root
/// `.cargo/config.toml`), so the unsafe is sound here — no other thread
/// can observe an inconsistent state.
struct CliEnv {
    original_database_url: Option<String>,
    original_dry_run: Option<String>,
    original_sources: Option<String>,
}

impl CliEnv {
    /// Installs the environment: `DATABASE_URL` points at `database_url`
    /// (or is removed when `None`), the dry-run flag is set to
    /// `dry_run`, and the market-data sources are the disabled source.
    fn install(database_url: Option<&str>, dry_run: bool) -> Self {
        let original_database_url = std::env::var("DATABASE_URL").ok();
        let original_dry_run = std::env::var("OBOLARGUS_SYNC_DRY_RUN").ok();
        let original_sources = std::env::var("MARKET_DATA_SOURCES").ok();

        unsafe {
            match database_url {
                Some(url) => std::env::set_var("DATABASE_URL", url),
                None => std::env::remove_var("DATABASE_URL"),
            }
            std::env::set_var(
                "OBOLARGUS_SYNC_DRY_RUN",
                if dry_run { "true" } else { "false" },
            );
            std::env::set_var("MARKET_DATA_SOURCES", DISABLED_SOURCE_CONFIG);
        }

        Self {
            original_database_url,
            original_dry_run,
            original_sources,
        }
    }
}

impl Drop for CliEnv {
    fn drop(&mut self) {
        unsafe {
            match &self.original_database_url {
                Some(value) => std::env::set_var("DATABASE_URL", value),
                None => std::env::remove_var("DATABASE_URL"),
            }
            match &self.original_dry_run {
                Some(value) => std::env::set_var("OBOLARGUS_SYNC_DRY_RUN", value),
                None => std::env::remove_var("OBOLARGUS_SYNC_DRY_RUN"),
            }
            match &self.original_sources {
                Some(value) => std::env::set_var("MARKET_DATA_SOURCES", value),
                None => std::env::remove_var("MARKET_DATA_SOURCES"),
            }
        }
    }
}

/// Truncates every table the tests touch so each case starts clean.
async fn truncate(pool: &sqlx::PgPool) {
    for table in [
        "asset_reference_data",
        "asset_identifiers",
        "positions",
        "events",
        "assets",
        "issuers",
        "portfolios",
        "data_source_reconciliation_audits",
        "data_source_refresh_runs",
        "data_sources",
    ] {
        let query = format!("TRUNCATE TABLE {table} RESTART IDENTITY CASCADE");
        // Safe: `table` comes from the fixed constant list above, never
        // from user input.
        sqlx::query(sqlx::AssertSqlSafe(query))
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("truncate {table}: {error}"));
    }
}

/// Creates an asset carrying a `TICKER` identifier.
async fn seed_asset_with_ticker(pool: &sqlx::PgPool, ticker: &str) -> uuid::Uuid {
    let asset_id: (uuid::Uuid,) = sqlx::query_as(
        "INSERT INTO assets (id, asset_type, display_name, currency) \
         VALUES (gen_random_uuid(), 'EQUITY', $1, 'USD') RETURNING id",
    )
    .bind(format!("Asset {ticker}"))
    .fetch_one(pool)
    .await
    .expect("insert asset");

    sqlx::query(
        "INSERT INTO asset_identifiers (id, asset_id, scheme, value) \
         VALUES (gen_random_uuid(), $1, 'TICKER', $2)",
    )
    .bind(asset_id.0)
    .bind(ticker)
    .execute(pool)
    .await
    .expect("insert identifier");

    asset_id.0
}

/// The dry-run refresh resolves a bare `--asset` UUID and reports what it
/// would do without writing anything (FR-030).
#[test]
fn refresh_dry_run_resolves_a_uuid_asset() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let Some((pool, url)) = runtime.block_on(migrated_pool()) else {
        eprintln!("SKIPPED: no test database configured");
        return;
    };
    runtime.block_on(truncate(&pool));
    let asset_id = runtime.block_on(seed_asset_with_ticker(&pool, "AAPL"));

    let _env = CliEnv::install(Some(&url), true);

    let result = obolargus_cli::sync::run(SyncArgs {
        assets: vec![asset_id.to_string()],
        command: None,
    });
    assert!(result.is_ok(), "the dry run should succeed: {result:?}");

    // The dry run wrote no refresh-run audit row.
    let runs: i64 = runtime.block_on(async {
        sqlx::query_scalar("SELECT COUNT(*) FROM data_source_refresh_runs")
            .fetch_one(&pool)
            .await
            .expect("count runs")
    });
    assert_eq!(runs, 0, "a dry run must not write an audit row");

    runtime.block_on(truncate(&pool));
}

/// The dry-run refresh resolves a scheme-qualified `--asset` against the
/// `asset_identifiers` table (FR-030).
#[test]
fn refresh_dry_run_resolves_a_scheme_qualified_asset() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let Some((pool, url)) = runtime.block_on(migrated_pool()) else {
        eprintln!("SKIPPED: no test database configured");
        return;
    };
    runtime.block_on(truncate(&pool));
    runtime.block_on(seed_asset_with_ticker(&pool, "AAPL"));

    let _env = CliEnv::install(Some(&url), true);

    let result = obolargus_cli::sync::run(SyncArgs {
        assets: vec!["TICKER:AAPL".to_string()],
        command: None,
    });
    assert!(result.is_ok(), "the dry run should succeed: {result:?}");

    runtime.block_on(truncate(&pool));
}

/// A `--asset` pair that no asset claims is rejected with a message that
/// names the pair (FR-030).
#[test]
fn refresh_rejects_an_unknown_scheme_qualified_asset() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let Some((pool, url)) = runtime.block_on(migrated_pool()) else {
        eprintln!("SKIPPED: no test database configured");
        return;
    };
    runtime.block_on(truncate(&pool));
    runtime.block_on(seed_asset_with_ticker(&pool, "AAPL"));

    let _env = CliEnv::install(Some(&url), true);

    let error = obolargus_cli::sync::run(SyncArgs {
        assets: vec!["TICKER:ZZZZ".to_string()],
        command: None,
    })
    .expect_err("err");
    assert_eq!(error, "no asset matches identifier 'TICKER:ZZZZ'");

    runtime.block_on(truncate(&pool));
}

/// A scheme-qualified `--asset` cannot be resolved without a database URL
/// (FR-030).
#[test]
fn refresh_reports_a_missing_database_url_for_a_typed_asset() {
    let _env = CliEnv::install(None, true);

    let error = obolargus_cli::sync::run(SyncArgs {
        assets: vec!["TICKER:AAPL".to_string()],
        command: None,
    })
    .expect_err("err");
    assert_eq!(
        error,
        "DATABASE_URL is not set; cannot resolve scheme-qualified identifiers"
    );
}

/// The refresh needs a database URL even for a bare UUID (FR-030).
#[test]
fn refresh_reports_a_missing_database_url() {
    let _env = CliEnv::install(None, false);

    let error = obolargus_cli::sync::run(SyncArgs {
        assets: vec!["00000000-0000-0000-0000-000000000001".to_string()],
        command: None,
    })
    .expect_err("err");
    assert_eq!(
        error,
        "DATABASE_URL is not set; the sync refresh needs a database"
    );
}

/// The refresh connects to the configured database and drives the refresh
/// orchestrator, writing the audit row (FR-007, FR-030..FR-032).
///
/// The configured source is **disabled**, so the registry has no enabled
/// candidate: the asset is counted as `source_unavailable` and no network
/// call is made.
#[test]
fn refresh_writes_the_audit_row() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let Some((pool, url)) = runtime.block_on(migrated_pool()) else {
        eprintln!("SKIPPED: no test database configured");
        return;
    };
    runtime.block_on(truncate(&pool));
    let asset_id = runtime.block_on(seed_asset_with_ticker(&pool, "AAPL"));

    let _env = CliEnv::install(Some(&url), false);

    let result = obolargus_cli::sync::run(SyncArgs {
        assets: vec![asset_id.to_string()],
        command: None,
    });
    assert!(result.is_ok(), "the refresh should succeed: {result:?}");

    // The refresh wrote a closed audit row.
    let row: Option<(i32, i32, i32, bool)> = runtime.block_on(async {
        sqlx::query_as(
            "SELECT assets_attempted, assets_missing_identifier, \
                    assets_source_unavailable, ended_at IS NOT NULL \
             FROM data_source_refresh_runs",
        )
        .fetch_optional(&pool)
        .await
        .expect("query the audit row")
    });
    let (attempted, missing, unavailable, ended) = row.expect("the refresh wrote an audit row");
    assert_eq!(attempted, 1);
    // The only configured source is disabled, so no enabled source can
    // answer the asset's `TICKER` identifier: the asset is counted as
    // `missing_identifier` and no provider is ever contacted.
    assert_eq!(missing, 1, "no enabled source supports the identifier");
    assert_eq!(unavailable, 0, "no provider was contacted");
    assert!(ended, "the run must be closed");

    runtime.block_on(truncate(&pool));
}

/// The reconciliation dry run reports the enabled-source count without
/// touching the database (FR-021).
#[test]
fn reconcile_dry_run_reports_the_enabled_source_count() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let Some((pool, url)) = runtime.block_on(migrated_pool()) else {
        eprintln!("SKIPPED: no test database configured");
        return;
    };
    runtime.block_on(truncate(&pool));

    let _env = CliEnv::install(Some(&url), true);

    let result = obolargus_cli::sync::run(SyncArgs {
        assets: Vec::new(),
        command: Some(SyncCommand::Reconcile(ReconcileArgs {
            asset: "00000000-0000-0000-0000-000000000001".to_string(),
            as_of: "2026-01-02".to_string(),
            policy: "primary_wins".to_string(),
        })),
    });
    assert!(result.is_ok(), "the dry run should succeed: {result:?}");

    runtime.block_on(truncate(&pool));
}

/// The reconciliation connects to the database and reports the typed
/// failure when the registry does not have two enabled sources (FR-020).
#[test]
fn reconcile_reports_the_typed_failure() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let Some((pool, url)) = runtime.block_on(migrated_pool()) else {
        eprintln!("SKIPPED: no test database configured");
        return;
    };
    runtime.block_on(truncate(&pool));

    let _env = CliEnv::install(Some(&url), false);

    let error = obolargus_cli::sync::run(SyncArgs {
        assets: Vec::new(),
        command: Some(SyncCommand::Reconcile(ReconcileArgs {
            asset: "00000000-0000-0000-0000-000000000001".to_string(),
            as_of: "2026-01-02".to_string(),
            policy: "primary_wins".to_string(),
        })),
    })
    .expect_err("err");
    assert!(
        error.starts_with("reconcile error:"),
        "the typed failure must be surfaced: {error}"
    );
    assert!(
        error.contains("at least 2 enabled sources"),
        "the typed failure must name the cause: {error}"
    );

    runtime.block_on(truncate(&pool));
}
