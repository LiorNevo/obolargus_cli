//! The `sync` subcommand family: refresh market data, look up a ticker, and
//! reconcile a fact across sources.
//!
//! Implements US1 / US2 / US3b of `specs/017-yahoo-data-source/spec.md`
//! (FR-030..FR-032) and the `OBOLARGUS_SYNC_DRY_RUN` rule from
//! `specs/017-yahoo-data-source/contracts/config-contract-additions.md`.

use clap::{Args, Subcommand};
use envconfig::Envconfig;
use serde::Serialize;
use uuid::Uuid;

use obolargus_core::domain::id::{AssetId, DomainId};
use obolargus_core::market_data::load::{
    IdentifierLoadConfig, IdentifierLoadReport, run_identifier_load,
};
use obolargus_core::market_data::reconcile::{
    ReconcileConfig, ReconcileSummary, ReconciliationPolicy, run_reconcile,
};
use obolargus_core::market_data::refresh::{RefreshConfig, RefreshSummary, run_refresh};
use obolargus_core::market_data::types::{TickerLookupHit, TickerLookupRequest, TypedIdentifier};

/// Arguments for the `sync` subcommand.
#[derive(Debug, Args, Default, Clone)]
pub struct SyncArgs {
    /// Asset to refresh. Either `<scheme>:<value>` (e.g. `TICKER:AAPL`) or
    /// a bare asset UUID. May be repeated; when omitted together with a
    /// subcommand, every asset with a supported identifier is refreshed.
    #[arg(long = "asset", value_name = "ASSET")]
    pub assets: Vec<String>,
    /// The nested subcommand (`lookup`, `reconcile`).
    #[command(subcommand)]
    pub command: Option<SyncCommand>,
}

/// The nested `sync` subcommands.
#[derive(Debug, Subcommand, Clone)]
pub enum SyncCommand {
    /// Look up an unknown ticker to help create a new asset (US2).
    Lookup(LookupArgs),
    /// Reconcile one fact across the configured sources (US3b).
    Reconcile(ReconcileArgs),
    /// Load reference data for a batch of identifiers as of one date (US2 of
    /// spec 018).
    Load(LoadArgs),
}

/// Arguments for `sync load`.
///
/// Implements FR-011 of `specs/018-load-reference-data/spec.md`.
#[derive(Debug, Args, Clone)]
pub struct LoadArgs {
    /// One identifier, as `<type>:<value>` (e.g. `TICKER:XNAS:AAPL`). May be
    /// repeated; one identifier per flag occurrence.
    #[arg(long = "identifier", value_name = "IDENTIFIER")]
    pub identifiers: Vec<String>,
    /// The single as-of date (YYYY-MM-DD).
    #[arg(long = "date", value_name = "YYYY-MM-DD")]
    pub date: String,
    /// A preferred upstream source id. It is consulted **first** for every
    /// identifier; the normal fail-over continues when it cannot answer.
    #[arg(long = "source", value_name = "SOURCE_ID")]
    pub source: Option<String>,
    /// Output format: `json` (default) or `table`.
    #[arg(long = "format", value_name = "FORMAT", default_value = "json")]
    pub format: String,
}

/// Arguments for `sync lookup`.
#[derive(Debug, Args, Clone)]
pub struct LookupArgs {
    /// Free-text query (ticker or company-name fragment).
    #[arg(long = "query", value_name = "Q")]
    pub query: String,
    /// Output format: `json` (default) or `table`.
    #[arg(long = "format", value_name = "FORMAT", default_value = "json")]
    pub format: String,
    /// Maximum number of hits to print.
    #[arg(long = "limit", value_name = "N", default_value_t = 10)]
    pub limit: usize,
    /// Print the same shape without contacting any provider.
    #[arg(long = "offline", default_value_t = false)]
    pub offline: bool,
}

/// Arguments for `sync reconcile`.
#[derive(Debug, Args, Clone)]
pub struct ReconcileArgs {
    /// Subject asset (UUID).
    #[arg(long = "asset", value_name = "ASSET")]
    pub asset: String,
    /// The dual-temporal `as_of_date` (YYYY-MM-DD).
    #[arg(long = "as-of", value_name = "YYYY-MM-DD")]
    pub as_of: String,
    /// Reconciliation policy: `primary_wins` or `first_non_empty`.
    #[arg(long = "policy", value_name = "POLICY", default_value = "primary_wins")]
    pub policy: String,
}

/// CLI-local configuration, loaded via `envconfig`.
///
/// The `OBOLARGUS_SYNC_DRY_RUN` flag is CLI-only; everything else comes
/// from the server's `AppConfig` (which is a superset).
#[derive(Debug, Clone, PartialEq, Eq, Envconfig)]
pub struct SyncConfig {
    /// When `true`, the refresh writes no rows; it only prints the
    /// resolved summary.
    #[envconfig(from = "OBOLARGUS_SYNC_DRY_RUN", default = "false")]
    pub dry_run: bool,
}

/// The JSON summary printed by `sync` on success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SyncOutput {
    /// Whether this was a dry run (no rows written).
    pub dry_run: bool,
    /// The refresh run's audit row id.
    pub run_id: Uuid,
    /// Total assets attempted.
    pub assets_attempted: i64,
    /// Assets refreshed (a new row was written).
    pub assets_refreshed: i64,
    /// Assets whose bar already matched the existing latest row.
    pub assets_no_op: i64,
    /// Assets with no supported identifier.
    pub assets_missing_identifier: i64,
    /// Assets where the source returned a typed retryable error.
    pub assets_source_unavailable: i64,
}

impl From<&RefreshSummary> for SyncOutput {
    fn from(summary: &RefreshSummary) -> Self {
        Self {
            dry_run: false,
            run_id: summary.run_id,
            assets_attempted: summary.assets_attempted,
            assets_refreshed: summary.assets_refreshed,
            assets_no_op: summary.assets_no_op,
            assets_missing_identifier: summary.assets_missing_identifier,
            assets_source_unavailable: summary.assets_source_unavailable,
        }
    }
}

/// The JSON output of `sync lookup`.
///
/// Not `Eq` because `TickerLookupHit::relevance` is an upstream-provided
/// `f64` relevance score (not a monetary value, so the constitution's
/// decimal rule does not apply).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LookupOutput {
    /// The query that was issued.
    pub query: String,
    /// The hits, sorted by relevance descending then symbol ascending.
    pub hits: Vec<TickerLookupHit>,
}

/// The JSON output of `sync reconcile`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReconcileOutput {
    /// The reconciliation audit row's id.
    pub reconciliation_id: Uuid,
    /// The chosen canonical value, as the serialized JSON payload.
    pub canonical_value: String,
    /// The number of sources consulted.
    pub sources_tried: usize,
}

impl From<&ReconcileSummary> for ReconcileOutput {
    fn from(summary: &ReconcileSummary) -> Self {
        Self {
            reconciliation_id: summary.reconciliation_id,
            canonical_value: summary.canonical_value.clone(),
            sources_tried: summary.per_source_answers.len(),
        }
    }
}

/// The JSON output of `sync load`.
///
/// Mirrors the API's response body field for field — the two surfaces are
/// interchangeable witnesses of one shared operation (SC-002).
///
/// Implements FR-007 / FR-011 of `specs/018-load-reference-data/spec.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoadOutput {
    /// The report schema version.
    pub report_version: u32,
    /// The run's audit row id (`null` for a dry run).
    pub run_id: Option<Uuid>,
    /// Whether this was a dry run (nothing was persisted).
    pub dry_run: bool,
    /// The run's overall status.
    pub status: obolargus_core::market_data::load::RunStatus,
    /// The caller-named preferred source (`null` when none was named).
    pub requested_source: Option<String>,
    /// When the run started.
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// When the run ended.
    pub ended_at: chrono::DateTime<chrono::Utc>,
    /// The run's wall-clock duration.
    pub duration_ms: u64,
    /// The identifiers as received, before de-duplication.
    pub identifiers_requested_raw: usize,
    /// The repeated entries in the request.
    pub identifiers_duplicated: usize,
    /// The unique identifiers that reached the fetch stage.
    pub identifiers_attempted: usize,
    /// The outcome counters.
    pub counters: LoadCounters,
    /// Per-source attempt counts.
    pub per_source_attempted: std::collections::BTreeMap<String, usize>,
    /// Per-source failure counts.
    pub per_source_failed: std::collections::BTreeMap<String, usize>,
    /// One line per unique requested identifier.
    pub identifiers: Vec<obolargus_core::market_data::load::LoadIdentifierOutcome>,
}

/// The outcome counters of a load run.
///
/// Implements FR-006 of `specs/018-load-reference-data/spec.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoadCounters {
    /// A row was written.
    pub loaded: usize,
    /// The identical fact was already known.
    pub no_op: usize,
    /// No asset claims the identifier.
    pub not_resolved: usize,
    /// The source holds no value for that date.
    pub no_data_for_date: usize,
    /// No enabled source claims the identifier's scheme.
    pub unsupported_scheme: usize,
    /// The source refused the request.
    pub rejected: usize,
    /// A transient or infrastructure failure.
    pub source_unavailable: usize,
    /// The source's budget was exhausted.
    pub rate_limited: usize,
    /// The store refused the write.
    pub storage_failure: usize,
}

impl From<&IdentifierLoadReport> for LoadOutput {
    fn from(report: &IdentifierLoadReport) -> Self {
        Self {
            report_version: report.report_version,
            run_id: report.run_id,
            dry_run: false,
            status: report.status,
            requested_source: report.requested_source.clone(),
            started_at: report.started_at,
            ended_at: report.ended_at,
            duration_ms: report.duration_ms,
            identifiers_requested_raw: report.identifiers_requested_raw,
            identifiers_duplicated: report.identifiers_duplicated,
            identifiers_attempted: report.identifiers_attempted,
            counters: LoadCounters {
                loaded: report.loaded,
                no_op: report.no_op,
                not_resolved: report.not_resolved,
                no_data_for_date: report.no_data_for_date,
                unsupported_scheme: report.unsupported_scheme,
                rejected: report.rejected,
                source_unavailable: report.source_unavailable,
                rate_limited: report.rate_limited,
                storage_failure: report.storage_failure,
            },
            per_source_attempted: report.per_source_attempted.clone(),
            per_source_failed: report.per_source_failed.clone(),
            identifiers: report.identifiers.clone(),
        }
    }
}

/// Parses an `--identifier` argument into a typed identifier.
///
/// The grammar splits at the **last** colon, so a venue-qualified type
/// survives: `TICKER:XNAS:AAPL` is the type `TICKER:XNAS` and the value
/// `AAPL`. The type is upper-cased; the value is trimmed and stays
/// case-sensitive.
///
/// Implements FR-002 / FR-011 of `specs/018-load-reference-data/spec.md`.
fn parse_identifier(raw: &str) -> Result<TypedIdentifier, String> {
    let trimmed = raw.trim();
    let Some((scheme, value)) = trimmed.rsplit_once(':') else {
        return Err(format!(
            "--identifier '{trimmed}' must be '<type>:<value>' (e.g. TICKER:AAPL)"
        ));
    };
    let scheme = scheme.trim().to_uppercase();
    let value = value.trim();
    if scheme.is_empty() {
        return Err(format!(
            "--identifier '{trimmed}' has an empty type (expected '<type>:<value>')"
        ));
    }
    if value.is_empty() {
        return Err(format!(
            "--identifier '{trimmed}' has an empty value (expected '<type>:<value>')"
        ));
    }
    Ok(TypedIdentifier::new(scheme, value))
}

/// A parsed `--asset` argument.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedAsset {
    /// A bare asset UUID.
    Id(Uuid),
    /// A scheme-qualified identifier (e.g. `TICKER:AAPL`).
    Typed {
        /// Scheme (e.g. `"TICKER"`).
        scheme: String,
        /// Value (e.g. `"AAPL"`).
        value: String,
    },
}

/// Parses a `--asset` argument into an asset id.
///
/// Accepts a bare UUID, or `<scheme>:<value>` — which is resolved against
/// the database by the identifier lookup.
fn parse_asset(raw: &str) -> Result<ParsedAsset, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("--asset must not be empty".to_string());
    }
    if let Ok(uuid) = Uuid::parse_str(trimmed) {
        return Ok(ParsedAsset::Id(uuid));
    }
    if let Some((scheme, value)) = trimmed.split_once(':')
        && !scheme.is_empty()
        && !value.is_empty()
    {
        return Ok(ParsedAsset::Typed {
            scheme: scheme.to_string(),
            value: value.to_string(),
        });
    }
    Err(format!(
        "--asset '{trimmed}' is neither a UUID nor a '<scheme>:<value>' pair"
    ))
}

/// Runs the `sync` subcommand.
///
/// Returns `Ok(())` on success (the summary is printed as JSON) and an
/// error message on a typed failure.
pub fn run(args: SyncArgs) -> Result<(), String> {
    let config = SyncConfig::init_from_env().map_err(|e| format!("config error: {e}"))?;

    // Nested subcommands take precedence over the flat refresh path.
    if let Some(command) = args.command {
        return run_nested(command, config);
    }

    let parsed: Vec<ParsedAsset> = args
        .assets
        .iter()
        .map(|raw| parse_asset(raw))
        .collect::<Result<Vec<_>, _>>()?;

    // No `--asset`: preserve the historical placeholder acknowledgement so
    // the CLI contract (`every placeholder subcommand acknowledges and exits
    // 0`) continues to hold.
    if parsed.is_empty() {
        println!("obolargus-cli: sync — placeholder acknowledged");
        return Ok(());
    }

    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| format!("failed to start async runtime: {e}"))?;
    runtime.block_on(run_refresh_async(parsed, config, None))
}

/// The runtime a `sync` invocation needs.
///
/// The production path builds this from the process environment. The
/// internal helpers accept an optional pre-built context so the
/// provider-backed paths can be exercised without a live upstream.
#[derive(Clone)]
struct SyncContext {
    /// The resolved application configuration.
    app_config: obolargus_server::config::AppConfig,
    /// The market-data registry built from the configuration.
    registry: std::sync::Arc<obolargus_core::market_data::registry::DataSourceRegistry>,
}

fn run_nested(command: SyncCommand, config: SyncConfig) -> Result<(), String> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| format!("failed to start async runtime: {e}"))?;
    match command {
        SyncCommand::Lookup(args) => runtime.block_on(run_lookup_async(args, None)),
        SyncCommand::Reconcile(args) => runtime.block_on(run_reconcile_async(args, config, None)),
        SyncCommand::Load(args) => runtime.block_on(run_load_async(args, config, None)),
    }
}

/// Resolves the context to use: the injected one when the caller supplied
/// it, otherwise one built from the process environment.
fn resolve_context(context: Option<&SyncContext>) -> Result<SyncContext, String> {
    match context {
        Some(context) => Ok(context.clone()),
        None => build_runtime_context(),
    }
}

async fn run_refresh_async(
    parsed: Vec<ParsedAsset>,
    config: SyncConfig,
    context: Option<&SyncContext>,
) -> Result<(), String> {
    let context = resolve_context(context)?;
    let asset_ids = resolve_asset_ids(parsed, context.app_config.database_url()).await?;

    if config.dry_run {
        // Dry run: resolve the assets and report what would happen without
        // writing anything.
        let output = SyncOutput {
            dry_run: true,
            run_id: Uuid::nil(),
            assets_attempted: asset_ids.len() as i64,
            assets_refreshed: 0,
            assets_no_op: 0,
            assets_missing_identifier: 0,
            assets_source_unavailable: 0,
        };
        println!("{}", render(&output)?);
        return Ok(());
    }

    let pool = connect_pool(context.app_config.database_url()).await?;
    let summary = run_refresh(
        &pool,
        &context.registry,
        RefreshConfig {
            assets: Some(asset_ids),
            trigger: "cli".to_string(),
        },
    )
    .await
    .map_err(|e| format!("refresh error: {e}"))?;

    let mut output = SyncOutput::from(&summary);
    output.dry_run = false;
    println!("{}", render(&output)?);
    Ok(())
}

async fn run_lookup_async(args: LookupArgs, context: Option<&SyncContext>) -> Result<(), String> {
    let query = args.query.trim();
    if query.is_empty() {
        return Err("--query must not be empty".to_string());
    }
    if !matches!(args.format.as_str(), "json" | "table") {
        return Err(format!(
            "unknown --format '{}' (expected 'json' or 'table')",
            args.format
        ));
    }

    if args.offline {
        // Offline: print the same shape without contacting any provider.
        let output = LookupOutput {
            query: query.to_string(),
            hits: vec![],
        };
        print_lookup(&output, &args)?;
        return Ok(());
    }

    let context = resolve_context(context)?;
    let map = context
        .registry
        .every_source_lookup_ticker(TickerLookupRequest {
            query: query.to_string(),
        })
        .await;

    let mut hits: Vec<TickerLookupHit> = Vec::new();
    for (_source_id, outcome) in map {
        if let Ok(result) = outcome {
            hits.extend(result.hits);
        }
    }
    hits.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.symbol.cmp(&b.symbol))
    });
    hits.truncate(args.limit);

    let output = LookupOutput {
        query: query.to_string(),
        hits,
    };
    print_lookup(&output, &args)?;
    Ok(())
}

async fn run_reconcile_async(
    args: ReconcileArgs,
    config: SyncConfig,
    context: Option<&SyncContext>,
) -> Result<(), String> {
    let asset_uuid = Uuid::parse_str(args.asset.trim())
        .map_err(|_| format!("--asset '{}' is not a valid UUID", args.asset))?;
    let as_of = chrono::NaiveDate::parse_from_str(args.as_of.trim(), "%Y-%m-%d")
        .map_err(|_| format!("--as-of '{}' is not a valid YYYY-MM-DD date", args.as_of))?;
    let policy = match args.policy.as_str() {
        "primary_wins" => ReconciliationPolicy::PrimaryWins,
        "first_non_empty" => ReconciliationPolicy::FirstNonEmpty,
        "majority" => {
            return Err(
                "ReconciliationPolicy::Majority is deferred; use primary_wins or first_non_empty"
                    .to_string(),
            );
        }
        other => {
            return Err(format!(
                "unknown --policy '{other}' (expected 'primary_wins' or 'first_non_empty')"
            ));
        }
    };

    let context = resolve_context(context)?;
    if config.dry_run {
        println!(
            "{}",
            render(&ReconcileOutput {
                reconciliation_id: Uuid::nil(),
                canonical_value: String::new(),
                sources_tried: context.registry.enabled_count(),
            })?
        );
        return Ok(());
    }

    let pool = connect_pool(context.app_config.database_url()).await?;
    let summary = run_reconcile(
        &pool,
        &context.registry,
        ReconcileConfig {
            asset_id: AssetId::from_uuid(asset_uuid),
            fact_type: "market_price".to_string(),
            as_of_date: as_of,
            policy,
        },
    )
    .await
    .map_err(|e| format!("reconcile error: {e}"))?;

    println!("{}", render(&ReconcileOutput::from(&summary))?);
    Ok(())
}

/// Runs `sync load`: loads reference data for a batch of identifiers as of
/// one date and prints the execution report.
///
/// Implements FR-011 of `specs/018-load-reference-data/spec.md`.
async fn run_load_async(
    args: LoadArgs,
    config: SyncConfig,
    context: Option<&SyncContext>,
) -> Result<(), String> {
    if !matches!(args.format.as_str(), "json" | "table") {
        return Err(format!(
            "unknown --format '{}' (expected 'json' or 'table')",
            args.format
        ));
    }

    let identifiers: Vec<TypedIdentifier> = args
        .identifiers
        .iter()
        .map(|raw| parse_identifier(raw))
        .collect::<Result<Vec<_>, _>>()?;

    if identifiers.is_empty() {
        return Err("--identifier is required at least once".to_string());
    }

    let as_of_date = chrono::NaiveDate::parse_from_str(args.date.trim(), "%Y-%m-%d")
        .map_err(|_| format!("--date '{}' is not a valid YYYY-MM-DD date", args.date))?;

    let context = resolve_context(context)?;

    let pool = if config.dry_run {
        // A dry run persists nothing at all — no reference-data rows and no
        // run record — and reports an empty run id (FR-008). The core
        // operation owns that rule; the CLI only passes the flag through.
        None
    } else {
        Some(connect_pool(context.app_config.database_url()).await?)
    };

    let report = run_identifier_load(
        pool.as_ref(),
        &context.registry,
        IdentifierLoadConfig {
            identifiers,
            as_of_date,
            preferred_source: args.source.clone(),
            trigger: "cli".to_string(),
            dry_run: config.dry_run,
        },
    )
    .await
    .map_err(|e| format!("load error: {e}"))?;

    let mut output = LoadOutput::from(&report);
    output.dry_run = config.dry_run;
    print_load(&output, &args)?;
    Ok(())
}

/// Prints the load report in the requested format.
fn print_load(output: &LoadOutput, args: &LoadArgs) -> Result<(), String> {
    if args.format == "table" {
        println!(
            "{:<24} {:<38} {:<14} {:<20}",
            "IDENTIFIER", "ASSET_ID", "OUTCOME", "SOURCE"
        );
        for line in &output.identifiers {
            println!(
                "{:<24} {:<38} {:<14} {:<20}",
                format!("{}:{}", line.identifier.scheme, line.identifier.value),
                line.asset_id.map(|id| id.to_string()).unwrap_or_default(),
                serde_json::to_string(&line.outcome).unwrap_or_default(),
                line.source_id.clone().unwrap_or_default(),
            );
        }
        if output.identifiers.is_empty() {
            println!("no identifiers");
        }
        return Ok(());
    }
    println!("{}", render(output)?);
    Ok(())
}

/// Loads the app configuration and builds the market-data registry.
fn build_runtime_context() -> Result<SyncContext, String> {
    let app_config = obolargus_server::config::AppConfig::from_env()
        .map_err(|e| format!("config error: {e}"))?;
    let market_data = app_config
        .market_data_config()
        .map_err(|e| format!("market data config error: {e}"))?;
    let registry = obolargus_server::tasks::build_registry_from_config(&market_data)
        .map_err(|e| format!("registry build error: {e}"))?;
    Ok(SyncContext {
        app_config,
        registry,
    })
}

/// Connects to the configured PostgreSQL database.
async fn connect_pool(url: Option<&str>) -> Result<sqlx::PgPool, String> {
    let url = url
        .ok_or_else(|| "DATABASE_URL is not set; the sync refresh needs a database".to_string())?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .map_err(|e| format!("database connection error: {e}"))
}

/// Resolves the parsed `--asset` arguments into concrete asset ids.
///
/// A bare UUID resolves directly. A scheme-qualified identifier
/// (`TICKER:AAPL`) is looked up against `asset_identifiers`; the first
/// matching asset wins.
async fn resolve_asset_ids(
    parsed: Vec<ParsedAsset>,
    database_url: Option<&str>,
) -> Result<Vec<AssetId>, String> {
    let mut ids = Vec::with_capacity(parsed.len());
    for item in parsed {
        match item {
            ParsedAsset::Id(uuid) => ids.push(AssetId::from_uuid(uuid)),
            ParsedAsset::Typed { scheme, value } => {
                let resolved = resolve_by_identifier(database_url, &scheme, &value)
                    .await?
                    .ok_or_else(|| format!("no asset matches identifier '{scheme}:{value}'"))?;
                ids.push(resolved);
            }
        }
    }
    Ok(ids)
}

/// Looks up the first asset carrying the given `(scheme, value)` identifier.
async fn resolve_by_identifier(
    database_url: Option<&str>,
    scheme: &str,
    value: &str,
) -> Result<Option<AssetId>, String> {
    let url = database_url.ok_or_else(|| {
        "DATABASE_URL is not set; cannot resolve scheme-qualified identifiers".to_string()
    })?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .map_err(|e| format!("database connection error: {e}"))?;

    let row: Option<(Uuid,)> = sqlx::query_as(
        "SELECT asset_id FROM asset_identifiers WHERE scheme = $1 AND value = $2 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(scheme)
    .bind(value)
    .fetch_optional(&pool)
    .await
    .map_err(|e| format!("identifier lookup error: {e}"))?;

    Ok(row.map(|(id,)| AssetId::from_uuid(id)))
}

/// Prints the lookup output in the requested format.
fn print_lookup(output: &LookupOutput, args: &LookupArgs) -> Result<(), String> {
    if args.format == "table" {
        println!("{:<12} {:<10} {:<24}", "SYMBOL", "TYPE", "NAME");
        for hit in &output.hits {
            let name = hit
                .long_name
                .as_deref()
                .or(hit.short_name.as_deref())
                .unwrap_or("");
            println!(
                "{:<12} {:<10} {:<24}",
                hit.symbol,
                hit.quote_type.as_deref().unwrap_or(""),
                name
            );
        }
        if output.hits.is_empty() {
            println!("no hits");
        }
        return Ok(());
    }
    println!("{}", render(output)?);
    Ok(())
}

/// Renders a value as a single line of JSON.
fn render<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| format!("json encode error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- the `--identifier` grammar (spec 018 FR-002) ----------

    /// `TICKER:AAPL` is the type `TICKER` and the value `AAPL`.
    #[test]
    fn parse_identifier_accepts_scheme_and_value() {
        let parsed = parse_identifier("TICKER:AAPL").expect("parse");
        assert_eq!(parsed, TypedIdentifier::new("TICKER", "AAPL"));
    }

    /// The split happens at the **last** colon, so a venue-qualified type
    /// survives.
    #[test]
    fn parse_identifier_splits_at_the_last_colon() {
        let parsed = parse_identifier("TICKER:XNAS:AAPL").expect("parse");
        assert_eq!(parsed, TypedIdentifier::new("TICKER:XNAS", "AAPL"));
    }

    /// A bare value is rejected with a usage hint.
    #[test]
    fn parse_identifier_rejects_a_bare_value() {
        let error = parse_identifier("AAPL").expect_err("bare value");
        assert!(error.contains("<type>:<value>"), "got: {error}");
    }

    /// An empty value is rejected.
    #[test]
    fn parse_identifier_rejects_an_empty_value() {
        let error = parse_identifier("TICKER:").expect_err("empty value");
        assert!(error.contains("empty value"), "got: {error}");
    }

    /// An empty type is rejected.
    #[test]
    fn parse_identifier_rejects_an_empty_type() {
        let error = parse_identifier(":AAPL").expect_err("empty type");
        assert!(error.contains("empty type"), "got: {error}");
    }

    /// The type is upper-cased; the value keeps its case.
    #[test]
    fn parse_identifier_normalizes_the_type_but_not_the_value() {
        let parsed = parse_identifier("ticker:aapl").expect("parse");
        assert_eq!(parsed, TypedIdentifier::new("TICKER", "aapl"));
    }

    /// A whitespace-only value is rejected.
    #[test]
    fn parse_identifier_rejects_a_blank_value() {
        assert!(parse_identifier("TICKER:   ").is_err());
    }

    /// The load arguments accept a repeated `--identifier` flag.
    #[test]
    fn load_args_accept_repeated_identifiers() {
        use clap::Parser;
        #[derive(Parser)]
        struct Wrapper {
            #[command(flatten)]
            args: LoadArgs,
        }
        let parsed = Wrapper::parse_from([
            "test",
            "--identifier",
            "TICKER:AAPL",
            "--identifier",
            "TICKER:MSFT",
            "--date",
            "2026-10-24",
        ]);
        assert_eq!(parsed.args.identifiers.len(), 2);
        assert_eq!(parsed.args.date, "2026-10-24");
        assert!(parsed.args.source.is_none());
        assert_eq!(parsed.args.format, "json");
    }

    /// The `--source` flag names the preferred source.
    #[test]
    fn load_args_accept_a_source_flag() {
        use clap::Parser;
        #[derive(Parser)]
        struct Wrapper {
            #[command(flatten)]
            args: LoadArgs,
        }
        let parsed = Wrapper::parse_from([
            "test",
            "--identifier",
            "TICKER:AAPL",
            "--date",
            "2026-10-24",
            "--source",
            "yahoo_finance",
        ]);
        assert_eq!(parsed.args.source.as_deref(), Some("yahoo_finance"));
    }

    #[test]
    fn parse_asset_accepts_uuid() {
        let uuid = Uuid::new_v4();
        let parsed = parse_asset(&uuid.to_string()).expect("parse");
        assert_eq!(parsed, ParsedAsset::Id(uuid));
    }

    #[test]
    fn parse_asset_accepts_scheme_value() {
        let parsed = parse_asset("TICKER:AAPL").expect("parse");
        assert_eq!(
            parsed,
            ParsedAsset::Typed {
                scheme: "TICKER".to_string(),
                value: "AAPL".to_string(),
            }
        );
    }

    #[test]
    fn parse_asset_rejects_garbage() {
        assert!(parse_asset("").is_err());
        assert!(parse_asset("   ").is_err());
        assert!(parse_asset("not-an-id").is_err());
        assert!(parse_asset("TICKER:").is_err());
        assert!(parse_asset(":AAPL").is_err());
    }

    #[test]
    fn sync_output_serializes_with_all_fields() {
        let output = SyncOutput {
            dry_run: false,
            run_id: Uuid::nil(),
            assets_attempted: 3,
            assets_refreshed: 1,
            assets_no_op: 1,
            assets_missing_identifier: 1,
            assets_source_unavailable: 0,
        };
        let json = render(&output).expect("render");
        assert!(json.contains("\"assets_attempted\":3"));
        assert!(json.contains("\"assets_refreshed\":1"));
        assert!(json.contains("\"dry_run\":false"));
    }

    #[test]
    fn lookup_output_serializes_query_and_hits() {
        let output = LookupOutput {
            query: "MSFT".to_string(),
            hits: vec![],
        };
        let json = render(&output).expect("render");
        assert!(json.contains("\"query\":\"MSFT\""));
        assert!(json.contains("\"hits\":[]"));
    }

    #[test]
    fn reconcile_output_serializes_all_fields() {
        let output = ReconcileOutput {
            reconciliation_id: Uuid::nil(),
            canonical_value: "{\"close\":\"100\"}".to_string(),
            sources_tried: 2,
        };
        let json = render(&output).expect("render");
        assert!(json.contains("\"sources_tried\":2"));
        assert!(json.contains("canonical_value"));
    }

    /// A whitespace-padded UUID is still accepted — the parser trims
    /// before validating.
    #[test]
    fn parse_asset_trims_whitespace_before_validating() {
        let uuid = Uuid::new_v4();
        let padded = format!("  {uuid}  ");
        assert_eq!(parse_asset(&padded).expect("parse"), ParsedAsset::Id(uuid));
    }

    /// A scheme-qualified identifier is trimmed too.
    #[test]
    fn parse_asset_trims_whitespace_around_a_typed_pair() {
        assert_eq!(
            parse_asset(" TICKER:AAPL ").expect("parse"),
            ParsedAsset::Typed {
                scheme: "TICKER".to_string(),
                value: "AAPL".to_string(),
            }
        );
    }

    /// `SyncOutput` is derived from a refresh summary field by field
    /// (FR-032).
    #[test]
    fn sync_output_is_derived_from_a_refresh_summary() {
        let summary = RefreshSummary {
            run_id: Uuid::nil(),
            assets_attempted: 9,
            assets_refreshed: 4,
            assets_no_op: 3,
            assets_missing_identifier: 1,
            assets_source_unavailable: 1,
        };
        let output = SyncOutput::from(&summary);
        assert!(!output.dry_run, "a real run is not a dry run");
        assert_eq!(output.run_id, summary.run_id);
        assert_eq!(output.assets_attempted, 9);
        assert_eq!(output.assets_refreshed, 4);
        assert_eq!(output.assets_no_op, 3);
        assert_eq!(output.assets_missing_identifier, 1);
        assert_eq!(output.assets_source_unavailable, 1);
    }

    /// `ReconcileOutput` is derived from a reconciliation summary; the
    /// number of sources tried is the size of the per-source answer map
    /// (FR-022).
    #[test]
    fn reconcile_output_is_derived_from_a_reconcile_summary() {
        let mut per_source_answers = std::collections::BTreeMap::new();
        per_source_answers.insert(
            "a".to_string(),
            obolargus_core::market_data::reconcile::ReconcileEntry::Error(
                obolargus_core::market_data::error::MarketDataProviderError::NotFound(
                    "no hit".into(),
                ),
            ),
        );
        per_source_answers.insert(
            "b".to_string(),
            obolargus_core::market_data::reconcile::ReconcileEntry::Error(
                obolargus_core::market_data::error::MarketDataProviderError::NotFound(
                    "no hit".into(),
                ),
            ),
        );
        let summary = ReconcileSummary {
            reconciliation_id: Uuid::nil(),
            per_source_answers,
            canonical_value: "{\"close\":\"100\"}".to_string(),
        };
        let output = ReconcileOutput::from(&summary);
        assert_eq!(output.reconciliation_id, summary.reconciliation_id);
        assert_eq!(output.canonical_value, summary.canonical_value);
        assert_eq!(output.sources_tried, 2);
    }

    /// The table format prints one row per hit, preferring the long name
    /// and falling back to the short one (US2).
    #[test]
    fn print_lookup_table_renders_one_row_per_hit() {
        let output = LookupOutput {
            query: "MSFT".to_string(),
            hits: vec![
                obolargus_core::market_data::types::TickerLookupHit {
                    symbol: "MSFT".to_string(),
                    short_name: Some("Microsoft".to_string()),
                    long_name: Some("Microsoft Corporation".to_string()),
                    quote_type: Some("EQUITY".to_string()),
                    exchange: Some("NMS".to_string()),
                    currency: None,
                    schemes: vec![],
                    relevance: Some(0.9),
                },
                obolargus_core::market_data::types::TickerLookupHit {
                    symbol: "MSFT2".to_string(),
                    short_name: Some("Short Only".to_string()),
                    long_name: None,
                    quote_type: None,
                    exchange: None,
                    currency: None,
                    schemes: vec![],
                    relevance: Some(0.5),
                },
            ],
        };
        let args = LookupArgs {
            query: "MSFT".to_string(),
            format: "table".to_string(),
            limit: 10,
            offline: false,
        };
        assert!(print_lookup(&output, &args).is_ok());
    }

    /// A hit with neither a long nor a short name still renders a row.
    #[test]
    fn print_lookup_table_handles_a_hit_without_any_name() {
        let output = LookupOutput {
            query: "X".to_string(),
            hits: vec![obolargus_core::market_data::types::TickerLookupHit {
                symbol: "X".to_string(),
                short_name: None,
                long_name: None,
                quote_type: None,
                exchange: None,
                currency: None,
                schemes: vec![],
                relevance: None,
            }],
        };
        let args = LookupArgs {
            query: "X".to_string(),
            format: "table".to_string(),
            limit: 10,
            offline: false,
        };
        assert!(print_lookup(&output, &args).is_ok());
    }

    /// The table format says so when there are no hits.
    #[test]
    fn print_lookup_table_reports_no_hits() {
        let output = LookupOutput {
            query: "ZZZZ".to_string(),
            hits: vec![],
        };
        let args = LookupArgs {
            query: "ZZZZ".to_string(),
            format: "table".to_string(),
            limit: 10,
            offline: false,
        };
        assert!(print_lookup(&output, &args).is_ok());
    }

    /// The JSON format prints the serialized output.
    #[test]
    fn print_lookup_json_prints_the_serialized_output() {
        let output = LookupOutput {
            query: "MSFT".to_string(),
            hits: vec![],
        };
        let args = LookupArgs {
            query: "MSFT".to_string(),
            format: "json".to_string(),
            limit: 10,
            offline: false,
        };
        assert!(print_lookup(&output, &args).is_ok());
    }

    // ---------- the provider-backed paths ----------

    /// A provider that answers the ticker lookup with a fixed set of hits.
    struct StubProvider {
        id: String,
        priority: i32,
        hits: Vec<obolargus_core::market_data::types::TickerLookupHit>,
        bar: Option<obolargus_core::market_data::types::LatestBar>,
    }

    #[async_trait::async_trait]
    impl obolargus_core::market_data::trait_::MarketDataProvider for StubProvider {
        fn source_id(&self) -> &str {
            &self.id
        }

        fn display_name(&self) -> &str {
            "Stub Provider"
        }

        fn supported_schemes(&self) -> &[String] {
            static SCHEMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
            SCHEMES.get_or_init(|| vec!["TICKER".to_string()])
        }

        fn enabled(&self) -> bool {
            true
        }

        fn priority(&self) -> i32 {
            self.priority
        }

        async fn get_latest_bar(
            &self,
            _request: obolargus_core::market_data::types::LatestBarRequest,
        ) -> Result<
            obolargus_core::market_data::types::LatestBar,
            obolargus_core::market_data::error::MarketDataProviderError,
        > {
            match &self.bar {
                Some(bar) => Ok(bar.clone()),
                None => Err(
                    obolargus_core::market_data::error::MarketDataProviderError::NotFound(
                        "no bar".into(),
                    ),
                ),
            }
        }

        async fn get_historical_range(
            &self,
            _request: obolargus_core::market_data::types::HistoricalRangeRequest,
        ) -> Result<
            obolargus_core::market_data::types::HistoricalRange,
            obolargus_core::market_data::error::MarketDataProviderError,
        > {
            Err(
                obolargus_core::market_data::error::MarketDataProviderError::Unsupported(
                    "stub: not implemented".into(),
                ),
            )
        }

        async fn lookup_ticker(
            &self,
            _request: obolargus_core::market_data::types::TickerLookupRequest,
        ) -> Result<
            obolargus_core::market_data::types::TickerLookupResult,
            obolargus_core::market_data::error::MarketDataProviderError,
        > {
            Ok(obolargus_core::market_data::types::TickerLookupResult {
                hits: self.hits.clone(),
                source_id: self.id.clone(),
            })
        }
    }

    /// Builds a lookup hit.
    fn hit(
        symbol: &str,
        relevance: Option<f64>,
    ) -> obolargus_core::market_data::types::TickerLookupHit {
        obolargus_core::market_data::types::TickerLookupHit {
            symbol: symbol.to_string(),
            short_name: None,
            long_name: None,
            quote_type: None,
            exchange: None,
            currency: None,
            schemes: vec![],
            relevance,
        }
    }

    /// Builds a runtime context around the given providers.
    fn context_with(providers: Vec<StubProvider>) -> SyncContext {
        context_with_url(providers, "")
    }

    /// Builds a runtime context around the given providers, pointed at the
    /// given database URL.
    fn context_with_url(providers: Vec<StubProvider>, database_url: &str) -> SyncContext {
        let providers: Vec<
            std::sync::Arc<
                dyn obolargus_core::market_data::trait_::MarketDataProvider + Send + Sync,
            >,
        > = providers
            .into_iter()
            .map(|provider| std::sync::Arc::new(provider) as _)
            .collect();
        let registry = obolargus_core::market_data::registry::DataSourceRegistry::new(providers)
            .expect("registry");
        let app_config = obolargus_server::config::AppConfig {
            database_url: database_url.to_string(),
            ..obolargus_server::config::AppConfig::default()
        };
        SyncContext {
            app_config,
            registry: std::sync::Arc::new(registry),
        }
    }

    /// The lookup path collects the hits from every enabled source,
    /// sorts them by relevance (descending) and truncates to `--limit`.
    #[tokio::test]
    async fn lookup_collects_sorts_and_truncates_the_hits() {
        let context = context_with(vec![
            StubProvider {
                id: "a".to_string(),
                priority: 10,
                hits: vec![hit("ZZZ", Some(0.1)), hit("AAA", Some(0.9))],
                bar: None,
            },
            StubProvider {
                id: "b".to_string(),
                priority: 20,
                hits: vec![hit("MMM", Some(0.5))],
                bar: None,
            },
        ]);

        let args = LookupArgs {
            query: "A".to_string(),
            format: "json".to_string(),
            limit: 2,
            offline: false,
        };
        assert!(run_lookup_async(args, Some(&context)).await.is_ok());
    }

    /// The lookup path tolerates a hit without a relevance score.
    #[tokio::test]
    async fn lookup_handles_a_hit_without_a_relevance_score() {
        let context = context_with(vec![StubProvider {
            id: "a".to_string(),
            priority: 10,
            hits: vec![hit("AAA", None)],
            bar: None,
        }]);

        let args = LookupArgs {
            query: "A".to_string(),
            format: "table".to_string(),
            limit: 10,
            offline: false,
        };
        assert!(run_lookup_async(args, Some(&context)).await.is_ok());
    }

    /// The reconciliation path drives the orchestrator and prints the
    /// canonical value (FR-021).
    ///
    /// Skipped when no test database is configured.
    #[tokio::test]
    async fn reconcile_writes_the_canonical_value() {
        let Some(url) = std::env::var("TEST_DATABASE_URL")
            .ok()
            .or_else(|| std::env::var("DATABASE_URL").ok())
        else {
            eprintln!("SKIPPED: no test database configured");
            return;
        };
        let pool = match sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
        {
            Ok(pool) => pool,
            Err(error) => {
                eprintln!("SKIPPED: could not connect to the test database: {error}");
                return;
            }
        };
        if let Err(error) = sqlx::migrate!("../obolargus_core/migrations")
            .run(&pool)
            .await
        {
            eprintln!("SKIPPED: could not apply the migrations: {error}");
            return;
        }
        truncate(&pool).await;

        // An asset carrying a `TICKER` identifier.
        let asset_id: (uuid::Uuid,) = sqlx::query_as(
            "INSERT INTO assets (id, asset_type, display_name, currency) \
             VALUES (gen_random_uuid(), 'EQUITY', 'Reconciled', 'USD') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("insert asset");
        sqlx::query(
            "INSERT INTO asset_identifiers (id, asset_id, scheme, value) \
             VALUES (gen_random_uuid(), $1, 'TICKER', 'AAPL')",
        )
        .bind(asset_id.0)
        .execute(&pool)
        .await
        .expect("insert identifier");

        let bar = obolargus_core::market_data::types::LatestBar {
            bar: obolargus_core::market_data::types::PriceBar {
                as_of_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 2).expect("date"),
                open: rust_decimal::Decimal::new(100, 0),
                high: rust_decimal::Decimal::new(100, 0),
                low: rust_decimal::Decimal::new(100, 0),
                close: rust_decimal::Decimal::new(100, 0),
                adjclose: rust_decimal::Decimal::new(100, 0),
                volume: 1_000,
            },
            knowledge_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 2).expect("date"),
            source_id: "a".to_string(),
        };

        let context = context_with_url(
            vec![
                StubProvider {
                    id: "a".to_string(),
                    priority: 10,
                    hits: vec![],
                    bar: Some(bar.clone()),
                },
                StubProvider {
                    id: "b".to_string(),
                    priority: 20,
                    hits: vec![],
                    bar: Some(bar),
                },
            ],
            &url,
        );

        let args = ReconcileArgs {
            asset: asset_id.0.to_string(),
            as_of: "2026-01-02".to_string(),
            policy: "primary_wins".to_string(),
        };
        let result = run_reconcile_async(args, SyncConfig { dry_run: false }, Some(&context)).await;
        assert!(
            result.is_ok(),
            "the reconciliation should succeed: {result:?}"
        );

        // The reconciliation wrote its audit row.
        let audits: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM data_source_reconciliation_audits")
                .fetch_one(&pool)
                .await
                .expect("count audits");
        assert_eq!(audits, 1, "the reconciliation must write an audit row");

        truncate(&pool).await;
    }

    /// Truncates every table the reconciliation touches.
    async fn truncate(pool: &sqlx::PgPool) {
        for table in [
            "asset_reference_data",
            "asset_identifiers",
            "assets",
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
}
