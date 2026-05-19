//! DataFusion SQL session over Iceberg catalogs.

mod case_insensitive;

#[cfg(target_arch = "wasm32")]
mod wasm_demo;

use std::sync::Arc;

use anyhow::{Context, Result};
use datafusion::arrow::array::{Array, RecordBatch, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::execution::context::SessionContext;
use datafusion::prelude::SessionConfig;
use iceberg::Catalog;
use iceberg_datafusion::IcebergCatalogProvider;
#[cfg(feature = "native")]
use idb_catalog::CatalogRegistry;

#[derive(Debug, Clone)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug)]
pub struct QueryResult {
    pub columns: Vec<ColumnInfo>,
    pub batches: Vec<RecordBatch>,
    pub row_count: usize,
    pub elapsed_ms: u128,
}

pub struct SqlSession {
    ctx: SessionContext,
    default_catalog: String,
    default_schema: String,
    iceberg_catalog: Arc<dyn Catalog>,
}

impl SqlSession {
    /// Browser demo: `demo.customers` in a DataFusion memory catalog (no Iceberg/Moka).
    #[cfg(target_arch = "wasm32")]
    pub async fn from_wasm_demo() -> Result<Self> {
        wasm_demo::open_wasm_demo_session().await
    }

    #[cfg(feature = "native")]
    pub async fn from_registry(registry: &CatalogRegistry) -> Result<Self> {
        let default_catalog = registry.default_name().to_string();
        let default_schema = registry.default_schema().to_string();
        let iceberg_catalog = registry.default();
        Self::from_iceberg_catalog(default_catalog, default_schema, iceberg_catalog).await
    }

    pub async fn from_iceberg_catalog(
        catalog_name: String,
        default_schema: String,
        iceberg_catalog: Arc<dyn Catalog>,
    ) -> Result<Self> {
        let config = SessionConfig::new()
            .with_information_schema(true)
            .with_create_default_catalog_and_schema(false)
            .with_default_catalog_and_schema(&catalog_name, &default_schema);
        let ctx = SessionContext::new_with_config(config);
        let provider = IcebergCatalogProvider::try_new(iceberg_catalog.clone())
            .await
            .map_err(|e| {
                let msg = format!("{e}");
                let hint = if msg.contains("500") || msg.contains("status") {
                    "\nHorizon hint: use a Snowflake PAT (exchanged via OAuth), correct account URI, \
warehouse = database name, and scope session:role:<role> matching the PAT."
                } else {
                    ""
                };
                anyhow::anyhow!("iceberg catalog provider: {msg}{hint}")
            })?;
        let provider = case_insensitive::wrap_catalog(Arc::new(provider));
        ctx.register_catalog(&catalog_name, provider);
        ctx.catalog(&catalog_name)
            .with_context(|| format!("catalog '{catalog_name}' not registered"))?;
        Ok(Self {
            ctx,
            default_catalog: catalog_name,
            default_schema,
            iceberg_catalog,
        })
    }

    pub fn session_context(&self) -> &SessionContext {
        &self.ctx
    }

    pub fn default_catalog(&self) -> &str {
        &self.default_catalog
    }

    pub async fn query(&self, sql: &str) -> Result<QueryResult> {
        let started = QueryTimer::start();
        if let Some(schema) = parse_show_tables(sql) {
            let schema = schema.unwrap_or_else(|| self.default_schema.clone());
            return self.show_tables(&schema, started).await;
        }
        let df = self
            .ctx
            .sql(sql)
            .await
            .map_err(|e| plan_sql_error(&e, &self.default_catalog, &self.default_schema))?;
        let schema = Arc::new(df.schema().as_arrow().clone());
        let batches = df.collect().await.context("execute sql")?;
        let columns = schema
            .fields()
            .iter()
            .map(|f| ColumnInfo {
                name: f.name().clone(),
                data_type: format!("{:?}", f.data_type()),
            })
            .collect();
        let row_count: usize = batches.iter().map(|b| b.num_rows()).sum();
        Ok(QueryResult {
            columns,
            batches,
            row_count,
            elapsed_ms: started.elapsed_ms(),
        })
    }

    /// Flatten result batches into display strings for the WASM UI grid.
    pub fn rows_as_strings(batches: &[RecordBatch]) -> Vec<Vec<String>> {
        use datafusion::arrow::util::display::array_value_to_string;
        let mut rows = Vec::new();
        for batch in batches {
            for row in 0..batch.num_rows() {
                let mut cells = Vec::with_capacity(batch.num_columns());
                for col in 0..batch.num_columns() {
                    let value = array_value_to_string(batch.column(col), row)
                        .unwrap_or_else(|_| "NULL".to_string());
                    cells.push(value);
                }
                rows.push(cells);
            }
        }
        rows
    }

    pub fn format_batches_table(batches: &[RecordBatch]) -> String {
        use datafusion::arrow::util::pretty::pretty_format_batches;
        pretty_format_batches(batches)
            .map(|t| t.to_string())
            .unwrap_or_else(|e| format!("(could not format: {e})"))
    }

    async fn show_tables(&self, schema: &str, started: QueryTimer) -> Result<QueryResult> {
        use iceberg::NamespaceIdent;

        let namespace = NamespaceIdent::from_vec(vec![schema.to_string()])
            .map_err(|e| anyhow::anyhow!("invalid namespace '{schema}': {e}"))?;
        let tables = self
            .iceberg_catalog
            .list_tables(&namespace)
            .await
            .map_err(|e| anyhow::anyhow!("list tables in '{schema}': {e}"))?;
        let names: Vec<&str> = tables.iter().map(|t| t.name()).collect();
        let schema_arrow = Arc::new(Schema::new(vec![Field::new(
            "table_name",
            DataType::Utf8,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema_arrow.clone(),
            vec![Arc::new(StringArray::from(names))],
        )
        .context("build SHOW TABLES result")?;
        let columns = schema_arrow
            .fields()
            .iter()
            .map(|f| ColumnInfo {
                name: f.name().clone(),
                data_type: format!("{:?}", f.data_type()),
            })
            .collect();
        let row_count = batch.num_rows();
        Ok(QueryResult {
            columns,
            batches: vec![batch],
            row_count,
            elapsed_ms: started.elapsed_ms(),
        })
    }

    pub async fn explain(&self, sql: &str) -> Result<String> {
        let plan = self
            .ctx
            .sql(&format!("EXPLAIN {sql}"))
            .await
            .context("explain plan")?;
        let batches = plan.collect().await.context("collect explain")?;
        let mut lines = Vec::new();
        for batch in batches {
            let col = batch.column(0);
            push_utf8_column(col, &mut lines);
        }
        Ok(lines.join("\n"))
    }
}

struct QueryTimer {
    #[cfg(not(target_arch = "wasm32"))]
    inner: std::time::Instant,
    #[cfg(target_arch = "wasm32")]
    inner: web_time::Instant,
}

impl QueryTimer {
    fn start() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        return Self {
            inner: std::time::Instant::now(),
        };
        #[cfg(target_arch = "wasm32")]
        return Self {
            inner: web_time::Instant::now(),
        };
    }

    fn elapsed_ms(&self) -> u128 {
        self.inner.elapsed().as_millis()
    }
}

/// `None` = use session default schema; `Some(s)` = `SHOW TABLES IN s`.
fn parse_show_tables(sql: &str) -> Option<Option<String>> {
    let s = sql.trim().trim_end_matches(';').trim();
    if s.len() < 11 || !s[..11].eq_ignore_ascii_case("show tables") {
        return None;
    }
    let tail = s[11..].trim();
    if tail.is_empty() {
        return Some(None);
    }
    if tail.len() < 3 || !tail[..3].eq_ignore_ascii_case("in ") {
        return None;
    }
    let schema = tail[3..].trim();
    if schema.is_empty() {
        return None;
    }
    // `catalog.schema` → use schema segment for Iceberg namespace.
    let schema = schema
        .rsplit('.')
        .next()
        .unwrap_or(schema)
        .trim_matches('"')
        .trim_matches('`')
        .to_string();
    Some(Some(schema))
}

#[cfg(test)]
mod tests {
    use super::parse_show_tables;

    #[test]
    fn show_tables_bare() {
        assert_eq!(parse_show_tables("SHOW TABLES"), Some(None));
        assert_eq!(parse_show_tables("show tables;"), Some(None));
    }

    #[test]
    fn show_tables_in_schema() {
        assert_eq!(
            parse_show_tables("SHOW TABLES IN iceberg_test"),
            Some(Some("iceberg_test".into()))
        );
        assert_eq!(
            parse_show_tables("SHOW TABLES IN snowflake_horizon.iceberg_test"),
            Some(Some("iceberg_test".into()))
        );
    }

    #[test]
    fn not_show_tables() {
        assert_eq!(parse_show_tables("SELECT 1"), None);
    }
}

fn plan_sql_error(
    err: &datafusion::error::DataFusionError,
    default_catalog: &str,
    default_schema: &str,
) -> anyhow::Error {
    let msg = err.to_string();
    let hint = if msg.contains("not found") {
        format!(
            "\nNaming: `{default_catalog}` is the **catalog** name from config.yaml (not the Snowflake database). \
The database is `warehouse` (e.g. ICEBERG_TEST). \
Default schema is `{default_schema}`. \
Snowflake IRC identifiers are often uppercase in the catalog — try \
`SELECT * FROM {default_schema}.employee` or `ICEBERG_TEST.EMPLOYEE` if needed."
        )
    } else {
        String::new()
    };
    anyhow::anyhow!("plan sql: {msg}{hint}")
}

fn push_utf8_column(col: &dyn Array, lines: &mut Vec<String>) {
    use datafusion::arrow::array::LargeStringArray;

    if let Some(arr) = col.as_any().downcast_ref::<StringArray>() {
        for row in 0..arr.len() {
            if !arr.is_null(row) {
                lines.push(arr.value(row).to_string());
            }
        }
    } else if let Some(arr) = col.as_any().downcast_ref::<LargeStringArray>() {
        for row in 0..arr.len() {
            if !arr.is_null(row) {
                lines.push(arr.value(row).to_string());
            }
        }
    }
}
