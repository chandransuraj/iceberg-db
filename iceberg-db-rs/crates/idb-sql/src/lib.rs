//! DataFusion SQL session over Iceberg catalogs.

#[cfg(target_arch = "wasm32")]
mod wasm_demo;

use std::sync::Arc;

use anyhow::{Context, Result};
use datafusion::arrow::array::{Array, RecordBatch, StringArray};
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
        let iceberg_catalog = registry.default();
        Self::from_iceberg_catalog(default_catalog, iceberg_catalog).await
    }

    pub async fn from_iceberg_catalog(
        catalog_name: String,
        iceberg_catalog: Arc<dyn Catalog>,
    ) -> Result<Self> {
        let config = SessionConfig::new()
            .with_information_schema(true)
            .with_create_default_catalog_and_schema(false)
            .with_default_catalog_and_schema(&catalog_name, "public");
        let ctx = SessionContext::new_with_config(config);
        let provider = IcebergCatalogProvider::try_new(iceberg_catalog)
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
        ctx.register_catalog(&catalog_name, Arc::new(provider));
        ctx.catalog(&catalog_name)
            .with_context(|| format!("catalog '{catalog_name}' not registered"))?;
        Ok(Self {
            ctx,
            default_catalog: catalog_name,
        })
    }

    pub fn session_context(&self) -> &SessionContext {
        &self.ctx
    }

    pub fn default_catalog(&self) -> &str {
        &self.default_catalog
    }

    pub async fn query(&self, sql: &str) -> Result<QueryResult> {
        #[cfg(not(target_arch = "wasm32"))]
        let started = std::time::Instant::now();
        #[cfg(target_arch = "wasm32")]
        let started = web_time::Instant::now();
        let df = self.ctx.sql(sql).await.context("plan sql")?;
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
            elapsed_ms: started.elapsed().as_millis(),
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
