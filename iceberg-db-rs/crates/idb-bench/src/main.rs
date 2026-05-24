use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::Parser;
use idb_config::default_config_path;
use idb_core::Engine;
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(
    name = "idb-bench",
    about = "Compare iceberg-db-rs and DuckDB on TPC-H/TPC-DS-style SQL suites",
    after_help = "\
Examples:
  idb-bench --suite tpch --queries ./tpch-queries --warehouse /data/warehouse --duckdb-setup duckdb-iceberg.sql
  idb-bench --suite tpcds --queries ./tpcds-queries --config config/snowflake-horizon.yaml --iterations 5

The query directory should contain one .sql file per query, such as q01.sql.
DuckDB setup SQL is intentionally user-provided because Iceberg table attachment
varies by catalog, extension version, and table layout."
)]
struct Args {
    /// Label written to output reports, for example tpch or tpcds.
    #[arg(long, default_value = "custom")]
    suite: String,

    /// Directory of .sql query files, or a single .sql file.
    #[arg(long)]
    queries: PathBuf,

    /// Path to iceberg-db YAML config. Defaults to ICEBERG_DB_CONFIG or ~/.iceberg-db/config.yaml.
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Local Hadoop-style warehouse path for iceberg-db; bypasses --config.
    #[arg(short, long)]
    warehouse: Option<PathBuf>,

    /// Catalog name used with --warehouse.
    #[arg(long, default_value = "local")]
    catalog: String,

    /// DuckDB executable to invoke.
    #[arg(long, default_value = "duckdb")]
    duckdb: PathBuf,

    /// DuckDB database file. Defaults to :memory:.
    #[arg(long)]
    duckdb_database: Option<PathBuf>,

    /// SQL file loaded before each DuckDB query, e.g. INSTALL/LOAD iceberg and CREATE VIEW statements.
    #[arg(long = "duckdb-setup")]
    duckdb_setup_files: Vec<PathBuf>,

    /// Inline SQL executed before each DuckDB query. Can be repeated.
    #[arg(long = "duckdb-setup-sql")]
    duckdb_setup_sql: Vec<String>,

    /// Force DuckDB query output to a sink via COPY (<query>) TO this path.
    #[arg(long, default_value = "/dev/null")]
    duckdb_output_path: PathBuf,

    /// Execute raw DuckDB query text instead of wrapping SELECTs in COPY TO /dev/null.
    #[arg(long)]
    duckdb_raw_output: bool,

    /// Number of warmup executions per query and engine.
    #[arg(long, default_value_t = 1)]
    warmup: usize,

    /// Number of measured executions per query and engine.
    #[arg(long, default_value_t = 3)]
    iterations: usize,

    /// Run only queries whose file name or stem matches one of these values.
    #[arg(long, value_delimiter = ',')]
    only: Vec<String>,

    /// Skip the iceberg-db-rs engine side.
    #[arg(long)]
    skip_idb: bool,

    /// Skip the DuckDB side.
    #[arg(long)]
    skip_duckdb: bool,

    /// Stop on the first query failure.
    #[arg(long)]
    fail_fast: bool,

    /// JSON report path.
    #[arg(long, default_value = "idb-bench-results.json")]
    output_json: PathBuf,

    /// Optional CSV report path with one row per measured run.
    #[arg(long)]
    output_csv: Option<PathBuf>,
}

#[derive(Debug)]
struct QuerySpec {
    name: String,
    file: PathBuf,
    sql: String,
}

#[derive(Debug, Serialize)]
struct BenchmarkReport {
    suite: String,
    generated_at_unix_ms: u128,
    query_count: usize,
    warmup: usize,
    iterations: usize,
    idb_input: Option<String>,
    duckdb_database: String,
    duckdb_output_mode: String,
    results: Vec<QueryReport>,
}

#[derive(Debug, Serialize)]
struct QueryReport {
    name: String,
    file: String,
    idb: EngineReport,
    duckdb: EngineReport,
}

#[derive(Debug, Serialize)]
struct EngineReport {
    status: EngineStatus,
    warmup_ms: Vec<u128>,
    runs: Vec<RunMeasurement>,
    stats: Option<Stats>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum EngineStatus {
    Ok,
    Error,
    Skipped,
}

#[derive(Debug, Serialize)]
struct RunMeasurement {
    iteration: usize,
    elapsed_ms: Option<u128>,
    row_count: Option<usize>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct Stats {
    min_ms: u128,
    max_ms: u128,
    avg_ms: f64,
    median_ms: f64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    if args.skip_idb && args.skip_duckdb {
        bail!("nothing to benchmark: both --skip-idb and --skip-duckdb were provided");
    }

    let queries = read_queries(&args.queries, &args.only)?;
    if queries.is_empty() {
        bail!("no .sql queries found in {}", args.queries.display());
    }

    let duckdb_setup = load_duckdb_setup(&args)?;
    let idb_engine = if args.skip_idb {
        None
    } else {
        Some(open_idb_engine(&args).await?)
    };

    let mut results = Vec::with_capacity(queries.len());
    for query in &queries {
        eprintln!("benchmarking {}", query.name);
        let idb = match idb_engine.as_ref() {
            Some(engine) => benchmark_idb(engine, query, &args).await,
            None => EngineReport::skipped(),
        };
        if args.fail_fast && matches!(idb.status, EngineStatus::Error) {
            bail!("iceberg-db-rs failed on {}", query.name);
        }

        let duckdb = if args.skip_duckdb {
            EngineReport::skipped()
        } else {
            benchmark_duckdb(query, &args, &duckdb_setup)
        };
        if args.fail_fast && matches!(duckdb.status, EngineStatus::Error) {
            bail!("DuckDB failed on {}", query.name);
        }

        results.push(QueryReport {
            name: query.name.clone(),
            file: query.file.display().to_string(),
            idb,
            duckdb,
        });
    }

    let report = BenchmarkReport {
        suite: args.suite.clone(),
        generated_at_unix_ms: unix_time_ms(),
        query_count: results.len(),
        warmup: args.warmup,
        iterations: args.iterations,
        idb_input: idb_input_label(&args),
        duckdb_database: args
            .duckdb_database
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| ":memory:".to_string()),
        duckdb_output_mode: if args.duckdb_raw_output {
            "raw".to_string()
        } else {
            format!("copy_to:{}", args.duckdb_output_path.display())
        },
        results,
    };

    write_json_report(&args.output_json, &report)?;
    if let Some(path) = &args.output_csv {
        write_csv_report(path, &report)?;
    }

    eprintln!("wrote {}", args.output_json.display());
    if let Some(path) = &args.output_csv {
        eprintln!("wrote {}", path.display());
    }

    Ok(())
}

impl EngineReport {
    fn skipped() -> Self {
        Self {
            status: EngineStatus::Skipped,
            warmup_ms: Vec::new(),
            runs: Vec::new(),
            stats: None,
            error: None,
        }
    }
}

async fn open_idb_engine(args: &Args) -> Result<Engine> {
    if let Some(warehouse) = &args.warehouse {
        eprintln!("iceberg-db-rs warehouse: {}", warehouse.display());
        return Engine::from_warehouse(warehouse, &args.catalog).await;
    }

    let config = args.config.clone().unwrap_or_else(default_config_path);
    eprintln!("iceberg-db-rs config: {}", config.display());
    Engine::from_config_file(&config).await
}

fn idb_input_label(args: &Args) -> Option<String> {
    if args.skip_idb {
        return None;
    }
    Some(if let Some(warehouse) = &args.warehouse {
        format!("warehouse:{} catalog:{}", warehouse.display(), args.catalog)
    } else {
        let config = args.config.clone().unwrap_or_else(default_config_path);
        format!("config:{}", config.display())
    })
}

async fn benchmark_idb(engine: &Engine, query: &QuerySpec, args: &Args) -> EngineReport {
    let mut warmup_ms = Vec::with_capacity(args.warmup);
    let mut runs = Vec::with_capacity(args.iterations);

    for _ in 0..args.warmup {
        match time_idb_query(engine, &query.sql).await {
            Ok((elapsed_ms, _rows)) => warmup_ms.push(elapsed_ms),
            Err(e) => {
                return EngineReport {
                    status: EngineStatus::Error,
                    warmup_ms,
                    runs,
                    stats: None,
                    error: Some(format!("{e:#}")),
                };
            }
        }
    }

    for iteration in 0..args.iterations {
        match time_idb_query(engine, &query.sql).await {
            Ok((elapsed_ms, row_count)) => runs.push(RunMeasurement {
                iteration: iteration + 1,
                elapsed_ms: Some(elapsed_ms),
                row_count: Some(row_count),
                error: None,
            }),
            Err(e) => {
                runs.push(RunMeasurement {
                    iteration: iteration + 1,
                    elapsed_ms: None,
                    row_count: None,
                    error: Some(format!("{e:#}")),
                });
                return EngineReport {
                    status: EngineStatus::Error,
                    warmup_ms,
                    stats: stats_from_runs(&runs),
                    runs,
                    error: Some(format!("{e:#}")),
                };
            }
        }
    }

    EngineReport {
        status: EngineStatus::Ok,
        warmup_ms,
        stats: stats_from_runs(&runs),
        runs,
        error: None,
    }
}

async fn time_idb_query(engine: &Engine, sql: &str) -> Result<(u128, usize)> {
    let started = Instant::now();
    let result = engine.query(sql).await?;
    Ok((started.elapsed().as_millis(), result.row_count))
}

fn benchmark_duckdb(query: &QuerySpec, args: &Args, setup_sql: &str) -> EngineReport {
    let mut warmup_ms = Vec::with_capacity(args.warmup);
    let mut runs = Vec::with_capacity(args.iterations);

    for _ in 0..args.warmup {
        match time_duckdb_query(query, args, setup_sql) {
            Ok(elapsed_ms) => warmup_ms.push(elapsed_ms),
            Err(e) => {
                return EngineReport {
                    status: EngineStatus::Error,
                    warmup_ms,
                    runs,
                    stats: None,
                    error: Some(format!("{e:#}")),
                };
            }
        }
    }

    for iteration in 0..args.iterations {
        match time_duckdb_query(query, args, setup_sql) {
            Ok(elapsed_ms) => runs.push(RunMeasurement {
                iteration: iteration + 1,
                elapsed_ms: Some(elapsed_ms),
                row_count: None,
                error: None,
            }),
            Err(e) => {
                runs.push(RunMeasurement {
                    iteration: iteration + 1,
                    elapsed_ms: None,
                    row_count: None,
                    error: Some(format!("{e:#}")),
                });
                return EngineReport {
                    status: EngineStatus::Error,
                    warmup_ms,
                    stats: stats_from_runs(&runs),
                    runs,
                    error: Some(format!("{e:#}")),
                };
            }
        }
    }

    EngineReport {
        status: EngineStatus::Ok,
        warmup_ms,
        stats: stats_from_runs(&runs),
        runs,
        error: None,
    }
}

fn time_duckdb_query(query: &QuerySpec, args: &Args, setup_sql: &str) -> Result<u128> {
    let query_sql = if args.duckdb_raw_output {
        format!("{};\n", trim_trailing_semicolons(&query.sql))
    } else {
        format!(
            "COPY (\n{}\n) TO {} (FORMAT CSV, HEADER false);\n",
            trim_trailing_semicolons(&query.sql),
            sql_string_literal(&args.duckdb_output_path.display().to_string())
        )
    };
    let script = format!("{setup_sql}\n{query_sql}");

    let started = Instant::now();
    let mut command = Command::new(&args.duckdb);
    command
        .arg(
            args.duckdb_database
                .as_ref()
                .map(|p| p.as_os_str())
                .unwrap_or_else(|| std::ffi::OsStr::new(":memory:")),
        )
        .arg("-c")
        .arg(script)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let output = command
        .output()
        .with_context(|| format!("spawn DuckDB executable {}", args.duckdb.display()))?;
    let elapsed_ms = started.elapsed().as_millis();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "duckdb exited with status {} on {}: {}",
            output.status,
            query.name,
            stderr.trim()
        );
    }
    Ok(elapsed_ms)
}

fn read_queries(path: &Path, only: &[String]) -> Result<Vec<QuerySpec>> {
    let mut files = Vec::new();
    if path.is_file() {
        files.push(path.to_path_buf());
    } else {
        for entry in fs::read_dir(path).with_context(|| format!("read {}", path.display()))? {
            let entry = entry?;
            let entry_path = entry.path();
            if entry_path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("sql"))
            {
                files.push(entry_path);
            }
        }
    }
    files.sort();

    let only: Vec<String> = only.iter().map(|s| s.to_ascii_lowercase()).collect();
    let mut queries = Vec::new();
    for file in files {
        let file_name = file
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&file_name)
            .to_string();
        if !only.is_empty() {
            let file_lower = file_name.to_ascii_lowercase();
            let stem_lower = stem.to_ascii_lowercase();
            if !only
                .iter()
                .any(|wanted| wanted == &file_lower || wanted == &stem_lower)
            {
                continue;
            }
        }
        let sql = fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
        queries.push(QuerySpec {
            name: stem,
            file,
            sql,
        });
    }
    Ok(queries)
}

fn load_duckdb_setup(args: &Args) -> Result<String> {
    let mut chunks = Vec::new();
    for file in &args.duckdb_setup_files {
        chunks.push(
            fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?,
        );
    }
    chunks.extend(args.duckdb_setup_sql.iter().cloned());
    Ok(chunks.join("\n"))
}

fn trim_trailing_semicolons(sql: &str) -> String {
    let mut trimmed = sql.trim();
    while let Some(rest) = trimmed.strip_suffix(';') {
        trimmed = rest.trim_end();
    }
    trimmed.to_string()
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn stats_from_runs(runs: &[RunMeasurement]) -> Option<Stats> {
    let mut values: Vec<u128> = runs.iter().filter_map(|run| run.elapsed_ms).collect();
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let min_ms = values[0];
    let max_ms = values[values.len() - 1];
    let avg_ms = values.iter().sum::<u128>() as f64 / values.len() as f64;
    let median_ms = if values.len() % 2 == 1 {
        values[values.len() / 2] as f64
    } else {
        let upper = values.len() / 2;
        (values[upper - 1] + values[upper]) as f64 / 2.0
    };
    Some(Stats {
        min_ms,
        max_ms,
        avg_ms,
        median_ms,
    })
}

fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn write_json_report(path: &Path, report: &BenchmarkReport) -> Result<()> {
    let text = serde_json::to_string_pretty(report)?;
    fs::write(path, text).with_context(|| format!("write {}", path.display()))
}

fn write_csv_report(path: &Path, report: &BenchmarkReport) -> Result<()> {
    let mut file = fs::File::create(path).with_context(|| format!("write {}", path.display()))?;
    writeln!(
        file,
        "suite,query,engine,iteration,elapsed_ms,row_count,error"
    )?;
    for query in &report.results {
        write_engine_csv(&mut file, &report.suite, &query.name, "idb", &query.idb)?;
        write_engine_csv(
            &mut file,
            &report.suite,
            &query.name,
            "duckdb",
            &query.duckdb,
        )?;
    }
    Ok(())
}

fn write_engine_csv(
    file: &mut fs::File,
    suite: &str,
    query: &str,
    engine: &str,
    report: &EngineReport,
) -> Result<()> {
    for run in &report.runs {
        writeln!(
            file,
            "{},{},{},{},{},{},{}",
            csv_escape(suite),
            csv_escape(query),
            csv_escape(engine),
            run.iteration,
            run.elapsed_ms
                .map(|v| v.to_string())
                .unwrap_or_else(String::new),
            run.row_count
                .map(|v| v.to_string())
                .unwrap_or_else(String::new),
            csv_escape(run.error.as_deref().unwrap_or_default())
        )?;
    }
    Ok(())
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_repeated_trailing_semicolons() {
        assert_eq!(trim_trailing_semicolons("SELECT 1;;\n"), "SELECT 1");
    }

    #[test]
    fn escapes_sql_string_literals() {
        assert_eq!(sql_string_literal("/tmp/o'clock.csv"), "'/tmp/o''clock.csv'");
    }

    #[test]
    fn computes_stats_from_successful_runs() {
        let runs = vec![
            RunMeasurement {
                iteration: 1,
                elapsed_ms: Some(30),
                row_count: None,
                error: None,
            },
            RunMeasurement {
                iteration: 2,
                elapsed_ms: Some(10),
                row_count: None,
                error: None,
            },
            RunMeasurement {
                iteration: 3,
                elapsed_ms: Some(20),
                row_count: None,
                error: None,
            },
        ];
        let stats = stats_from_runs(&runs).expect("stats");
        assert_eq!(stats.min_ms, 10);
        assert_eq!(stats.max_ms, 30);
        assert_eq!(stats.avg_ms, 20.0);
        assert_eq!(stats.median_ms, 20.0);
    }
}
