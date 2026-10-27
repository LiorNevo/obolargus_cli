//! The `temporal` subcommand family.
//!
//! Parses arguments and maps outcomes onto exit codes; every query delegates
//! to [`obolargus_core::temporal`]. See
//! `specs/021-bitemporal-market-data/contracts/cli-contract.md`.
//!
//! Delivered here: `facts` and `verify` (US1). The remaining verbs parse and
//! report that their user story has not landed yet, so the surface is stable
//! and the exit codes are already correct.

use std::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use obolargus_core::temporal::facts::read::{fact_as_known, fact_at, fact_history, fact_series};
use obolargus_core::temporal::facts::write::{append_fact, revoke_fact};
use obolargus_core::temporal::instant::TemporalWindow;
use obolargus_core::temporal::verify::{verify_lookahead, verify_parity};
use obolargus_core::temporal::{FactPoint, FactSubject, KnownAt, TemporalError, ValidAt};

use crate::CliError;

/// Arguments for `obolargus-cli temporal`.
#[derive(Debug, Args, Clone)]
pub struct TemporalArgs {
    /// Machine-readable output.
    #[arg(long, global = true)]
    pub json: bool,
    /// Override `DATABASE_URL`.
    #[arg(long, global = true, value_name = "URL")]
    pub database_url: Option<String>,
    /// Suppress the human summary.
    #[arg(long, global = true)]
    pub quiet: bool,
    /// The nested verb.
    #[command(subcommand)]
    pub command: TemporalCommand,
}

/// The `temporal` verbs.
#[derive(Debug, Subcommand, Clone)]
pub enum TemporalCommand {
    /// Read temporal facts and profiles.
    Facts(FactsArgs),
    /// Query intraday observations and run backfills.
    Observations(ForwardArgs),
    /// Read positions at an instant.
    Positions(ForwardArgs),
    /// Value a portfolio at an instant.
    Valuation(ForwardArgs),
    /// Inspect and apply retention.
    Retention(ForwardArgs),
    /// Run temporal integrity and parity checks.
    Verify(VerifyArgs),
    /// Show ingest health.
    Health(ForwardArgs),
}

/// A verb whose user story has not landed yet. Kept so the surface and the
/// help text are stable.
#[derive(Debug, Args, Clone)]
pub struct ForwardArgs {
    /// Trailing arguments are accepted and reported as unsupported.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub rest: Vec<String>,
}

/// The `temporal facts` verbs.
#[derive(Debug, Args, Clone)]
pub struct FactsArgs {
    /// The nested verb.
    #[command(subcommand)]
    pub command: FactsCommand,
}

/// The `temporal facts` verbs.
#[derive(Debug, Subcommand, Clone)]
pub enum FactsCommand {
    /// Read one value.
    Get(FactQuery),
    /// Read a resolved series over a window.
    Series(FactSeriesQuery),
    /// Read every revision of one fact.
    History(FactQuery),
    /// Read the complete profile as believed at an instant.
    Profile(ProfileQuery),
    /// Report coverage per fact type.
    Coverage(SubjectArgs),
    /// Append a belief.
    Append(AppendArgs),
    /// Append a retraction.
    Retract(RetractArgs),
}

/// `--subject` and the fact query coordinates.
#[derive(Debug, Args, Clone)]
pub struct FactQuery {
    /// `<kind>:<uuid>`, kind in `asset | issuer | portfolio`.
    #[arg(long, value_name = "KIND:UUID")]
    pub subject: String,
    /// The fact type (e.g. `market_price`).
    #[arg(long = "type", value_name = "FACT_TYPE")]
    pub fact_type: String,
    /// Valid-time instant (RFC 3339 or `YYYY-MM-DD`).
    #[arg(long, value_name = "INSTANT")]
    pub at: String,
    /// Transaction-time instant; when set, the read is as-known.
    #[arg(long = "known-at", value_name = "INSTANT")]
    pub known_at: Option<String>,
}

/// A series query.
#[derive(Debug, Args, Clone)]
pub struct FactSeriesQuery {
    /// `<kind>:<uuid>`.
    #[arg(long, value_name = "KIND:UUID")]
    pub subject: String,
    /// The fact type.
    #[arg(long = "type", value_name = "FACT_TYPE")]
    pub fact_type: String,
    /// Inclusive window start.
    #[arg(long, value_name = "INSTANT")]
    pub from: String,
    /// Exclusive window end.
    #[arg(long, value_name = "INSTANT")]
    pub to: String,
    /// Read as known at this instant.
    #[arg(long = "known-at", value_name = "INSTANT")]
    pub known_at: Option<String>,
}

/// A profile query.
#[derive(Debug, Args, Clone)]
pub struct ProfileQuery {
    /// `<kind>:<uuid>`.
    #[arg(long, value_name = "KIND:UUID")]
    pub subject: String,
    /// The transaction-time instant.
    #[arg(long = "known-at", value_name = "INSTANT")]
    pub known_at: String,
}

/// A subject-only query.
#[derive(Debug, Args, Clone)]
pub struct SubjectArgs {
    /// `<kind>:<uuid>`.
    #[arg(long, value_name = "KIND:UUID")]
    pub subject: String,
}

/// An append.
#[derive(Debug, Args, Clone)]
pub struct AppendArgs {
    /// `<kind>:<uuid>`.
    #[arg(long, value_name = "KIND:UUID")]
    pub subject: String,
    /// The fact type.
    #[arg(long = "type", value_name = "FACT_TYPE")]
    pub fact_type: String,
    /// Valid time.
    #[arg(long, value_name = "INSTANT")]
    pub at: String,
    /// Transaction time.
    #[arg(long = "known-at", value_name = "INSTANT")]
    pub known_at: String,
    /// The payload, taken verbatim.
    #[arg(long, value_name = "PAYLOAD")]
    pub value: String,
    /// The reporting source.
    #[arg(long, value_name = "ID")]
    pub source: String,
}

/// A retraction.
#[derive(Debug, Args, Clone)]
pub struct RetractArgs {
    /// `<kind>:<uuid>`.
    #[arg(long, value_name = "KIND:UUID")]
    pub subject: String,
    /// The fact type.
    #[arg(long = "type", value_name = "FACT_TYPE")]
    pub fact_type: String,
    /// Valid time.
    #[arg(long, value_name = "INSTANT")]
    pub at: String,
    /// Transaction time.
    #[arg(long = "known-at", value_name = "INSTANT")]
    pub known_at: String,
    /// Why the value is retracted.
    #[arg(long, value_name = "TEXT")]
    pub reason: String,
    /// The reporting source.
    #[arg(long, value_name = "ID")]
    pub source: String,
}

/// The `temporal verify` verbs.
#[derive(Debug, Args, Clone)]
pub struct VerifyArgs {
    /// The nested verb.
    #[command(subcommand)]
    pub command: VerifyCommand,
}

/// The `temporal verify` verbs.
#[derive(Debug, Subcommand, Clone)]
pub enum VerifyCommand {
    /// Legacy and temporal readers must agree.
    Parity(ParityArgs),
    /// As-known reads must not look ahead.
    Lookahead(LookaheadArgs),
    /// The position ledger must match a replay of events.
    Positions(PositionsVerifyArgs),
}

/// `verify parity` options.
#[derive(Debug, Args, Clone)]
pub struct ParityArgs {
    /// Only check facts valid on or after this date.
    #[arg(long, value_name = "DATE")]
    pub from: Option<NaiveDate>,
    /// Only check facts valid before this date.
    #[arg(long, value_name = "DATE")]
    pub to: Option<NaiveDate>,
}

/// `verify lookahead` options.
#[derive(Debug, Args, Clone)]
pub struct LookaheadArgs {
    /// How many stored beliefs to sample.
    #[arg(long, default_value_t = 100)]
    pub samples: usize,
}

/// `verify positions` options.
#[derive(Debug, Args, Clone)]
pub struct PositionsVerifyArgs {
    /// Restrict the check to one portfolio.
    #[arg(long, value_name = "UUID")]
    pub portfolio: Option<String>,
}

/// The output-shape selector, kept explicit so `--json` never changes the
/// exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputMode {
    /// Human-readable.
    Human,
    /// Machine-readable.
    Json,
}

/// Runs the `temporal` subcommand.
pub fn run(args: TemporalArgs) -> Result<(), CliError> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| CliError::infrastructure(format!("async runtime: {error}")))?;
    runtime.block_on(run_async(args))
}

async fn run_async(args: TemporalArgs) -> Result<(), CliError> {
    match &args.command {
        TemporalCommand::Facts(command) => {
            let pool = connect(&args).await?;
            run_facts(&pool, &command.command, &args).await
        }
        TemporalCommand::Verify(command) => {
            let pool = connect(&args).await?;
            run_verify(&pool, &command.command, &args).await
        }
        TemporalCommand::Observations(_)
        | TemporalCommand::Positions(_)
        | TemporalCommand::Valuation(_)
        | TemporalCommand::Retention(_)
        | TemporalCommand::Health(_) => Err(CliError::not_implemented(
            "this verb belongs to a later user story of spec 021",
        )),
    }
}

async fn run_facts(
    pool: &PgPool,
    command: &FactsCommand,
    args: &TemporalArgs,
) -> Result<(), CliError> {
    match command {
        FactsCommand::Get(query) => {
            let subject = parse_subject(&query.subject)?;
            let at = parse_valid_at(&query.at)?;
            let point = match &query.known_at {
                Some(raw) => {
                    fact_as_known(pool, subject, &query.fact_type, at, parse_known_at(raw)?).await?
                }
                None => fact_at(pool, subject, &query.fact_type, at).await?,
            };
            let point = point.ok_or_else(|| {
                CliError::not_found(format!(
                    "no value for {} {} at {}",
                    subject.subject_name(),
                    query.fact_type,
                    query.at
                ))
            })?;
            emit(args, &point, || render_point(&point));
            Ok(())
        }
        FactsCommand::Series(query) => {
            let subject = parse_subject(&query.subject)?;
            let window =
                TemporalWindow::new(parse_valid_at(&query.from)?, parse_valid_at(&query.to)?)?;
            let known_at = query.known_at.as_deref().map(parse_known_at).transpose()?;
            let points = fact_series(pool, subject, &query.fact_type, window, known_at).await?;
            emit(args, &points, || {
                for point in &points {
                    render_point(point);
                }
                if points.is_empty() {
                    println!("(no points in window)");
                }
            });
            Ok(())
        }
        FactsCommand::History(query) => {
            let subject = parse_subject(&query.subject)?;
            let points =
                fact_history(pool, subject, &query.fact_type, parse_valid_at(&query.at)?).await?;
            if points.is_empty() {
                return Err(CliError::not_found("no revisions for the requested fact"));
            }
            emit(args, &points, || {
                for point in &points {
                    render_point(point);
                }
            });
            Ok(())
        }
        FactsCommand::Profile(_) | FactsCommand::Coverage(_) => Err(CliError::not_implemented(
            "facts profile/coverage land with US2",
        )),
        FactsCommand::Append(append) => {
            let subject = parse_subject(&append.subject)?;
            let point = append_fact(
                pool,
                subject,
                &append.fact_type,
                parse_valid_at(&append.at)?,
                parse_known_at(&append.known_at)?,
                &append.value,
                &append.source,
            )
            .await?;
            emit(args, &point, || render_point(&point));
            Ok(())
        }
        FactsCommand::Retract(retract) => {
            let subject = parse_subject(&retract.subject)?;
            let point = revoke_fact(
                pool,
                subject,
                &retract.fact_type,
                parse_valid_at(&retract.at)?,
                parse_known_at(&retract.known_at)?,
                &retract.reason,
                &retract.source,
            )
            .await?;
            emit(args, &point, || render_point(&point));
            Ok(())
        }
    }
}

async fn run_verify(
    pool: &PgPool,
    command: &VerifyCommand,
    args: &TemporalArgs,
) -> Result<(), CliError> {
    match command {
        VerifyCommand::Parity(parity) => {
            let report = verify_parity(pool, parity.from, parity.to).await?;
            finish_verification(args, "parity", &report)
        }
        VerifyCommand::Lookahead(lookahead) => {
            let report = verify_lookahead(pool, lookahead.samples).await?;
            finish_verification(args, "lookahead", &report)
        }
        VerifyCommand::Positions(_) => {
            Err(CliError::not_implemented("verify positions lands with US4"))
        }
    }
}

fn finish_verification(
    args: &TemporalArgs,
    name: &str,
    report: &obolargus_core::temporal::verify::VerificationReport,
) -> Result<(), CliError> {
    emit(args, report, || {
        println!(
            "{name}: {} checks, {} failures",
            report.checks,
            report.failures.len()
        );
        for failure in &report.failures {
            println!("  FAIL {failure}");
        }
    });
    if report.is_ok() {
        Ok(())
    } else {
        Err(CliError::verification_failed(format!(
            "{name} failed with {} violation(s)",
            report.failures.len()
        )))
    }
}

fn render_point(point: &FactPoint) {
    println!(
        "valid_at={} known_at={} source={} retracted={} value={}",
        point.valid_at, point.known_at, point.source, point.retracted, point.value
    );
}

fn emit<T: Serialize>(args: &TemporalArgs, value: &T, human: impl FnOnce()) {
    if args.json {
        match serde_json::to_string_pretty(value) {
            Ok(rendered) => println!("{rendered}"),
            Err(error) => eprintln!("could not encode JSON: {error}"),
        }
    } else if !args.quiet {
        human();
    }
}

fn parse_subject(raw: &str) -> Result<FactSubject, CliError> {
    let (kind, id) = raw
        .split_once(':')
        .ok_or_else(|| CliError::usage("--subject must be <kind>:<uuid>"))?;
    let uuid = Uuid::parse_str(id)
        .map_err(|error| CliError::usage(format!("invalid subject id: {error}")))?;
    FactSubject::from_parts(kind, uuid).map_err(|error| CliError::usage(error.to_string()))
}

fn parse_instant(raw: &str) -> Result<DateTime<Utc>, CliError> {
    if let Ok(parsed) = raw.parse::<DateTime<Utc>>() {
        return Ok(parsed);
    }
    let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|error| CliError::usage(format!("invalid instant {raw:?}: {error}")))?;
    Ok(ValidAt::midnight_utc(date).as_datetime())
}

fn parse_valid_at(raw: &str) -> Result<ValidAt, CliError> {
    ValidAt::new(parse_instant(raw)?).map_err(CliError::from)
}

fn parse_known_at(raw: &str) -> Result<KnownAt, CliError> {
    KnownAt::new(parse_instant(raw)?).map_err(CliError::from)
}

async fn connect(args: &TemporalArgs) -> Result<PgPool, CliError> {
    let url = args
        .database_url
        .clone()
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .filter(|url| !url.trim().is_empty())
        .ok_or_else(|| CliError::infrastructure("DATABASE_URL is not set"))?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .map_err(|error| CliError::infrastructure(format!("database connection: {error}")))?;
    obolargus_core::bin_helpers::run_migrations(&pool)
        .await
        .map_err(|error| CliError::infrastructure(format!("database migration: {error}")))?;
    Ok(pool)
}

impl From<TemporalError> for CliError {
    fn from(error: TemporalError) -> Self {
        let code = match &error {
            TemporalError::ConflictingAppend { .. } => 4,
            TemporalError::NoValue { .. } => 3,
            TemporalError::OutsideRetention { .. } => 5,
            TemporalError::IngestUnavailable { .. } => 6,
            TemporalError::InvalidTime { .. } | TemporalError::UnsupportedInterval { .. } => 2,
            TemporalError::UnknownSubject { .. }
            | TemporalError::Retention { .. }
            | TemporalError::Infrastructure(_) => 1,
            // `TemporalError` is `#[non_exhaustive]`; an added variant is an
            // infrastructure failure until it is mapped explicitly.
            _ => 1,
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

/// Runs a future on a fresh runtime. Kept public so tests can drive a verb
/// without spawning the process.
pub fn block_on<F>(future: F) -> Result<(), CliError>
where
    F: Future<Output = Result<(), CliError>>,
{
    tokio::runtime::Runtime::new()
        .map_err(|error| CliError::infrastructure(format!("async runtime: {error}")))?
        .block_on(future)
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use crate::Cli;

    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    /// Builds `temporal` arguments for one verb.
    fn args_for(command: TemporalCommand, url: Option<&str>, json: bool) -> TemporalArgs {
        TemporalArgs {
            json,
            database_url: url.map(str::to_string),
            quiet: false,
            command,
        }
    }

    fn fact_get(subject: &str, at: &str, known_at: Option<&str>) -> FactQuery {
        FactQuery {
            subject: subject.to_string(),
            fact_type: "market_price".to_string(),
            at: at.to_string(),
            known_at: known_at.map(str::to_string),
        }
    }

    fn facts(command: FactsCommand) -> TemporalCommand {
        TemporalCommand::Facts(FactsArgs { command })
    }

    fn verify(command: VerifyCommand) -> TemporalCommand {
        TemporalCommand::Verify(VerifyArgs { command })
    }

    /// A pool that validates its URL but never connects, for verbs that
    /// reject before touching the database.
    fn lazy_pool() -> PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://user:password@127.0.0.1:1/obolargus")
            .expect("a syntactically valid URL builds a lazy pool")
    }

    fn database_url() -> Option<String> {
        std::env::var("TEST_DATABASE_URL")
            .ok()
            .or_else(|| std::env::var("DATABASE_URL").ok())
            .filter(|url| !url.trim().is_empty())
    }

    async fn migrated_pool(url: &str) -> Option<PgPool> {
        let pool = match sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(url)
            .await
        {
            Ok(pool) => pool,
            Err(error) => {
                eprintln!("SKIPPED: could not connect to the test database: {error}");
                return None;
            }
        };
        if let Err(error) = obolargus_core::bin_helpers::run_migrations(&pool).await {
            eprintln!("SKIPPED: could not apply the migrations: {error}");
            return None;
        }
        Some(pool)
    }

    async fn seed_asset(pool: &PgPool) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO assets (id, asset_type, display_name, currency) \
             VALUES (gen_random_uuid(), 'EQUITY', $1, 'USD') RETURNING id",
        )
        .bind(format!("temporal-coverage-{}", Uuid::new_v4()))
        .fetch_one(pool)
        .await
        .expect("seed an asset")
    }

    async fn drop_asset(pool: &PgPool, id: Uuid) {
        for statement in [
            "DELETE FROM asset_reference_data WHERE asset_id = $1",
            "DELETE FROM assets WHERE id = $1",
        ] {
            sqlx::query(statement)
                .bind(id)
                .execute(pool)
                .await
                .expect("clean up the seeded asset");
        }
    }

    #[test]
    fn every_temporal_verb_is_reachable() {
        let mut root = Cli::command();
        let temporal = root
            .find_subcommand_mut("temporal")
            .expect("temporal is registered");
        for verb in [
            "facts",
            "observations",
            "positions",
            "valuation",
            "retention",
            "verify",
            "health",
        ] {
            assert!(
                temporal.find_subcommand(verb).is_some(),
                "temporal {verb} is not reachable"
            );
        }
    }

    #[test]
    fn every_facts_and_verify_verb_is_reachable() {
        let mut root = Cli::command();
        let temporal = root
            .find_subcommand_mut("temporal")
            .expect("temporal is registered");
        let facts = temporal.find_subcommand_mut("facts").expect("facts");
        for verb in [
            "get", "series", "history", "profile", "coverage", "append", "retract",
        ] {
            assert!(
                facts.find_subcommand(verb).is_some(),
                "facts {verb} missing"
            );
        }
        let verify = temporal.find_subcommand_mut("verify").expect("verify");
        for verb in ["parity", "lookahead", "positions"] {
            assert!(
                verify.find_subcommand(verb).is_some(),
                "verify {verb} missing"
            );
        }
    }

    #[test]
    fn facts_subcommands_parse_with_their_options() {
        let cli = parse(&[
            "obolargus-cli",
            "temporal",
            "facts",
            "get",
            "--subject",
            "asset:02100000-0000-0000-0000-000000000001",
            "--type",
            "market_price",
            "--at",
            "2026-01-02T00:00:00Z",
            "--known-at",
            "2026-01-02T12:00:00Z",
            "--json",
        ])
        .expect("facts get parses");
        assert!(matches!(cli.command, crate::Command::Temporal(_)));
    }

    #[test]
    fn verify_subcommands_parse() {
        for verb in ["parity", "lookahead", "positions"] {
            parse(&["obolargus-cli", "temporal", "verify", verb])
                .unwrap_or_else(|error| panic!("{verb} should parse: {error}"));
        }
    }

    #[test]
    fn subject_parser_accepts_each_kind_and_rejects_garbage() {
        for kind in ["asset", "issuer", "portfolio"] {
            let parsed =
                super::parse_subject(&format!("{kind}:02100000-0000-0000-0000-000000000001"))
                    .expect("known subject");
            assert_eq!(parsed.subject_name(), kind);
        }
        assert!(super::parse_subject("asset:not-a-uuid").is_err());
        assert!(super::parse_subject("nonsense").is_err());
        assert!(super::parse_subject("instrument:02100000-0000-0000-0000-000000000001").is_err());
    }

    #[test]
    fn bare_dates_parse_as_midnight_utc() {
        let instant = super::parse_instant("2026-01-02").expect("bare date");
        assert_eq!(
            instant.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "2026-01-02T00:00:00Z"
        );
        assert!(super::parse_instant("not-a-date").is_err());
    }

    #[test]
    fn temporal_errors_map_onto_the_contract_exit_codes() {
        let cases = [
            (TemporalError::invalid_time("bad"), 2),
            (
                TemporalError::UnsupportedInterval {
                    requested: 1,
                    stored: vec![60],
                },
                2,
            ),
            (
                TemporalError::NoValue {
                    subject: "asset".to_string(),
                    at: "now".to_string(),
                },
                3,
            ),
            (TemporalError::conflict("identity"), 4),
            (
                TemporalError::OutsideRetention {
                    window: "w".to_string(),
                },
                5,
            ),
            (
                TemporalError::IngestUnavailable {
                    source_id: "yahoo".to_string(),
                    detail: "down".to_string(),
                },
                6,
            ),
            (
                TemporalError::UnknownSubject {
                    subject: "houses".to_string(),
                    id: "1".to_string(),
                },
                1,
            ),
            (
                TemporalError::Retention {
                    detail: "detach failed".to_string(),
                },
                1,
            ),
            (TemporalError::Infrastructure(sqlx::Error::RowNotFound), 1),
        ];
        for (error, code) in cases {
            let message = error.to_string();
            let converted = CliError::from(error);
            assert_eq!(converted.code, code, "wrong exit code for {message}");
            assert_eq!(converted.message, message);
        }
    }

    #[test]
    fn valid_and_known_instants_parse_from_rfc3339_and_dates() {
        let valid = super::parse_valid_at("2026-01-02T03:04:05Z").expect("rfc 3339");
        assert_eq!(
            valid.as_datetime().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "2026-01-02T03:04:05Z"
        );
        let known = super::parse_known_at("2026-01-03").expect("bare date");
        assert_eq!(
            known.as_datetime().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "2026-01-03T00:00:00Z"
        );
        assert!(super::parse_valid_at("nope").is_err());
        assert!(super::parse_known_at("nope").is_err());
    }

    #[test]
    fn render_point_and_emit_honor_the_output_mode() {
        let point = FactPoint {
            id: Uuid::new_v4(),
            subject: parse_subject("asset:02100000-0000-0000-0000-000000000001").expect("subject"),
            fact_type: "market_price".to_string(),
            valid_at: ValidAt::new(
                "2026-01-02T00:00:00Z"
                    .parse::<DateTime<Utc>>()
                    .expect("valid instant"),
            )
            .expect("valid"),
            known_at: KnownAt::new(
                "2026-01-03T00:00:00Z"
                    .parse::<DateTime<Utc>>()
                    .expect("known instant"),
            )
            .expect("known"),
            value: "{\"close\":\"100\"}".to_string(),
            source: "cli-test".to_string(),
            retracted: false,
        };

        render_point(&point);

        let emitted = std::cell::Cell::new(false);
        let mut json_args = args_for(
            facts(FactsCommand::History(fact_get(
                "asset:02100000-0000-0000-0000-000000000001",
                "2026-01-02",
                None,
            ))),
            None,
            true,
        );
        json_args.json = true;
        emit(&json_args, &point, || emitted.set(true));
        assert!(!emitted.get(), "--json must not run the human renderer");

        let mut quiet_args = json_args.clone();
        quiet_args.json = false;
        quiet_args.quiet = true;
        emit(&quiet_args, &point, || emitted.set(true));
        assert!(!emitted.get(), "--quiet must suppress the human renderer");

        let human_args = TemporalArgs {
            json: false,
            database_url: None,
            quiet: false,
            command: quiet_args.command.clone(),
        };
        emit(&human_args, &point, || emitted.set(true));
        assert!(emitted.get(), "the default mode renders for humans");

        // A value whose map key is not a string cannot be encoded; the error
        // is reported without panicking.
        let unencodable = std::collections::BTreeMap::from([((1_u8, 2_u8), 3_u8)]);
        emit(&json_args, &unencodable, || emitted.set(true));
    }

    #[test]
    fn later_user_stories_report_not_implemented() {
        for verb in [
            TemporalCommand::Observations(ForwardArgs { rest: vec![] }),
            TemporalCommand::Positions(ForwardArgs { rest: vec![] }),
            TemporalCommand::Valuation(ForwardArgs { rest: vec![] }),
            TemporalCommand::Retention(ForwardArgs { rest: vec![] }),
            TemporalCommand::Health(ForwardArgs { rest: vec![] }),
        ] {
            let error = run(args_for(verb, None, false)).expect_err("not delivered");
            assert_eq!(error.code, 1);
            assert!(error.message.contains("not implemented"));
        }

        let profile = run(args_for(
            facts(FactsCommand::Profile(ProfileQuery {
                subject: "asset:02100000-0000-0000-0000-000000000001".to_string(),
                known_at: "2026-01-02".to_string(),
            })),
            Some("not-a-postgres-url"),
            false,
        ));
        // `connect` is attempted before the verb dispatch, so the lazy URL
        // cannot be used through `run`; the dispatcher is exercised directly.
        assert!(profile.is_err());

        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let outer = args_for(
            facts(FactsCommand::Coverage(SubjectArgs {
                subject: "asset:02100000-0000-0000-0000-000000000001".to_string(),
            })),
            None,
            false,
        );
        runtime.block_on(async {
            let lazy = lazy_pool();
            let error = run_facts(
                &lazy,
                &FactsCommand::Profile(ProfileQuery {
                    subject: "asset:02100000-0000-0000-0000-000000000001".to_string(),
                    known_at: "2026-01-02".to_string(),
                }),
                &outer,
            )
            .await
            .expect_err("profile is not delivered");
            assert!(error.message.contains("profile/coverage"));

            let error = run_facts(
                &lazy,
                &FactsCommand::Coverage(SubjectArgs {
                    subject: "asset:02100000-0000-0000-0000-000000000001".to_string(),
                }),
                &outer,
            )
            .await
            .expect_err("coverage is not delivered");
            assert!(error.message.contains("profile/coverage"));

            let error = run_verify(
                &lazy,
                &VerifyCommand::Positions(PositionsVerifyArgs { portfolio: None }),
                &outer,
            )
            .await
            .expect_err("verify positions is not delivered");
            assert!(error.message.contains("US4"));
        });
    }

    #[test]
    fn fact_queries_validate_before_touching_the_database() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let outer = args_for(
            facts(FactsCommand::Get(fact_get(
                "asset:02100000-0000-0000-0000-000000000001",
                "2026-01-02",
                None,
            ))),
            None,
            false,
        );

        runtime.block_on(async {
            let lazy = lazy_pool();
            for subject in [
                "nonsense",
                "asset:not-a-uuid",
                "instrument:02100000-0000-0000-0000-000000000001",
            ] {
                let error = run_facts(
                    &lazy,
                    &FactsCommand::Get(fact_get(subject, "2026-01-02", None)),
                    &outer,
                )
                .await
                .expect_err("invalid subject");
                assert_eq!(error.code, 2, "usage errors exit 2 for {subject}");
            }

            let error = run_facts(
                &lazy,
                &FactsCommand::Get(fact_get(
                    "asset:02100000-0000-0000-0000-000000000001",
                    "not-a-date",
                    None,
                )),
                &outer,
            )
            .await
            .expect_err("invalid instant");
            assert_eq!(error.code, 2);

            let error = run_facts(
                &lazy,
                &FactsCommand::Series(FactSeriesQuery {
                    subject: "asset:02100000-0000-0000-0000-000000000001".to_string(),
                    fact_type: "market_price".to_string(),
                    from: "2026-02-01".to_string(),
                    to: "2026-01-01".to_string(),
                    known_at: None,
                }),
                &outer,
            )
            .await
            .expect_err("reversed window");
            assert_eq!(error.code, 2);
        });
    }

    #[test]
    fn verification_reports_success_and_failure() {
        use obolargus_core::temporal::verify::VerificationReport;

        let args = args_for(
            verify(VerifyCommand::Parity(ParityArgs {
                from: None,
                to: None,
            })),
            None,
            false,
        );
        let mut report = VerificationReport {
            checks: 2,
            failures: Vec::new(),
        };
        assert!(finish_verification(&args, "parity", &report).is_ok());

        report.failures.push("mismatch".to_string());
        let error = finish_verification(&args, "parity", &report).expect_err("failing report");
        assert_eq!(error.code, 5);
        assert!(error.message.contains("1 violation"));
    }

    #[test]
    fn connect_without_a_url_is_an_infrastructure_error() {
        let mut args = args_for(
            verify(VerifyCommand::Lookahead(LookaheadArgs { samples: 1 })),
            None,
            false,
        );
        // Only treat the process environment as absent when it really is.
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        if database_url().is_none() {
            let error = runtime.block_on(connect(&args)).expect_err("no URL");
            assert_eq!(error.code, 1);
            assert!(error.message.contains("DATABASE_URL is not set"));
        }
        args.database_url = Some("   ".to_string());
        let error = runtime.block_on(connect(&args)).expect_err("blank URL");
        assert_eq!(error.code, 1);
    }

    #[test]
    fn temporal_facts_round_trip_through_the_database() {
        let Some(url) = database_url() else {
            eprintln!("SKIPPED: no test database configured");
            return;
        };
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let Some(pool) = runtime.block_on(migrated_pool(&url)) else {
            return;
        };
        let asset_id = runtime.block_on(seed_asset(&pool));
        let subject = format!("asset:{asset_id}");
        let source = format!("cli-coverage-{}", Uuid::new_v4());

        // Append with `--json`.
        run(args_for(
            facts(FactsCommand::Append(AppendArgs {
                subject: subject.clone(),
                fact_type: "market_price".to_string(),
                at: "2026-02-01".to_string(),
                known_at: "2026-02-02T00:00:00Z".to_string(),
                value: "{\"close\":\"100\"}".to_string(),
                source: source.clone(),
            })),
            Some(&url),
            true,
        ))
        .expect("append");

        // An as-known read before the recording sees nothing (exit 3).
        let error = run(args_for(
            facts(FactsCommand::Get(fact_get(
                &subject,
                "2026-02-01",
                Some("2026-02-01T00:00:00Z"),
            ))),
            Some(&url),
            false,
        ))
        .expect_err("nothing was known yet");
        assert_eq!(error.code, 3, "{}", error.message);

        // The latest-knowledge read sees the value (human output).
        run(args_for(
            facts(FactsCommand::Get(fact_get(&subject, "2026-02-01", None))),
            Some(&url),
            false,
        ))
        .expect("latest read");

        // The as-known read at/after the recording sees the value.
        run(args_for(
            facts(FactsCommand::Get(fact_get(
                &subject,
                "2026-02-01",
                Some("2026-02-02T00:00:00Z"),
            ))),
            Some(&url),
            true,
        ))
        .expect("as-known read");

        // A series over a window with one point, then an empty window.
        run(args_for(
            facts(FactsCommand::Series(FactSeriesQuery {
                subject: subject.clone(),
                fact_type: "market_price".to_string(),
                from: "2026-02-01".to_string(),
                to: "2026-02-03".to_string(),
                known_at: Some("2026-02-02T00:00:00Z".to_string()),
            })),
            Some(&url),
            false,
        ))
        .expect("series");
        run(args_for(
            facts(FactsCommand::Series(FactSeriesQuery {
                subject: subject.clone(),
                fact_type: "market_price".to_string(),
                from: "2025-01-01".to_string(),
                to: "2025-02-01".to_string(),
                known_at: None,
            })),
            Some(&url),
            false,
        ))
        .expect("empty series");

        // History surfaces the recorded revision.
        run(args_for(
            facts(FactsCommand::History(fact_get(
                &subject,
                "2026-02-01",
                None,
            ))),
            Some(&url),
            true,
        ))
        .expect("history");

        // A retraction with a later known-at wins; the current read is empty.
        run(args_for(
            facts(FactsCommand::Retract(RetractArgs {
                subject: subject.clone(),
                fact_type: "market_price".to_string(),
                at: "2026-02-01".to_string(),
                known_at: "2026-02-03T00:00:00Z".to_string(),
                reason: "superseded".to_string(),
                source: source.clone(),
            })),
            Some(&url),
            true,
        ))
        .expect("retract");
        let error = run(args_for(
            facts(FactsCommand::Get(fact_get(&subject, "2026-02-01", None))),
            Some(&url),
            true,
        ))
        .expect_err("the winner is a retraction");
        assert_eq!(error.code, 3);

        // A conflicting append reuses the winning known-at (exit 4).
        let error = run(args_for(
            facts(FactsCommand::Append(AppendArgs {
                subject: subject.clone(),
                fact_type: "market_price".to_string(),
                at: "2026-02-01".to_string(),
                known_at: "2026-02-03T00:00:00Z".to_string(),
                value: "{\"close\":\"101\"}".to_string(),
                source: source.clone(),
            })),
            Some(&url),
            true,
        ))
        .expect_err("conflicting append");
        assert_eq!(error.code, 4, "{}", error.message);

        // `verify parity` and `verify lookahead` both pass on the fixture.
        run(args_for(
            verify(VerifyCommand::Parity(ParityArgs {
                from: Some(chrono::NaiveDate::from_ymd_opt(2026, 1, 1).expect("from date")),
                to: Some(chrono::NaiveDate::from_ymd_opt(2027, 1, 1).expect("to date")),
            })),
            Some(&url),
            false,
        ))
        .expect("parity");
        run(args_for(
            verify(VerifyCommand::Lookahead(LookaheadArgs { samples: 20 })),
            Some(&url),
            true,
        ))
        .expect("lookahead");

        // A missing fact is a `not_found` (exit 3).
        let error = run(args_for(
            facts(FactsCommand::History(fact_get(
                &subject,
                "2024-01-01",
                None,
            ))),
            Some(&url),
            false,
        ))
        .expect_err("no revisions");
        assert_eq!(error.code, 3);

        runtime.block_on(drop_asset(&pool, asset_id));
    }
}
