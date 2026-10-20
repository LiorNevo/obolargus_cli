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
use obolargus_core::market_data::reconcile::{
    ReconcileConfig, ReconcileSummary, ReconciliationPolicy, run_reconcile,
};
use obolargus_core::market_data::refresh::{RefreshConfig, RefreshSummary, run_refresh};
use obolargus_core::market_data::types::{TickerLookupHit, TickerLookupRequest};

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
    runtime.block_on(run_refresh_async(parsed, config))
}

fn run_nested(command: SyncCommand, config: SyncConfig) -> Result<(), String> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| format!("failed to start async runtime: {e}"))?;
    match command {
        SyncCommand::Lookup(args) => runtime.block_on(run_lookup_async(args)),
        SyncCommand::Reconcile(args) => runtime.block_on(run_reconcile_async(args, config)),
    }
}

async fn run_refresh_async(parsed: Vec<ParsedAsset>, config: SyncConfig) -> Result<(), String> {
    let (app_config, registry) = build_runtime_context()?;
    let asset_ids = resolve_asset_ids(parsed, app_config.database_url()).await?;

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

    let pool = connect_pool(app_config.database_url()).await?;
    let summary = run_refresh(
        &pool,
        &registry,
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

async fn run_lookup_async(args: LookupArgs) -> Result<(), String> {
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

    let (_app_config, registry) = build_runtime_context()?;
    let map = registry
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

async fn run_reconcile_async(args: ReconcileArgs, config: SyncConfig) -> Result<(), String> {
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

    let (app_config, registry) = build_runtime_context()?;
    if config.dry_run {
        println!(
            "{}",
            render(&ReconcileOutput {
                reconciliation_id: Uuid::nil(),
                canonical_value: String::new(),
                sources_tried: registry.enabled_count(),
            })?
        );
        return Ok(());
    }

    let pool = connect_pool(app_config.database_url()).await?;
    let summary = run_reconcile(
        &pool,
        &registry,
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

/// Loads the app configuration and builds the market-data registry.
fn build_runtime_context() -> Result<
    (
        obolargus_server::config::AppConfig,
        std::sync::Arc<obolargus_core::market_data::registry::DataSourceRegistry>,
    ),
    String,
> {
    let app_config = obolargus_server::config::AppConfig::from_env()
        .map_err(|e| format!("config error: {e}"))?;
    let market_data = app_config
        .market_data_config()
        .map_err(|e| format!("market data config error: {e}"))?;
    let registry = obolargus_server::tasks::build_registry_from_config(&market_data)
        .map_err(|e| format!("registry build error: {e}"))?;
    Ok((app_config, registry))
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
}
