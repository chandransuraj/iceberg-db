//! Case-insensitive schema/table lookup for Iceberg catalogs (Snowflake returns uppercase).

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::catalog::{CatalogProvider, SchemaProvider};
use datafusion::datasource::TableProvider;
use datafusion::error::Result as DFResult;

pub fn wrap_catalog(provider: Arc<dyn CatalogProvider>) -> Arc<dyn CatalogProvider> {
    Arc::new(CaseInsensitiveCatalog { inner: provider })
}

struct CaseInsensitiveCatalog {
    inner: Arc<dyn CatalogProvider>,
}

impl std::fmt::Debug for CaseInsensitiveCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaseInsensitiveCatalog")
            .field("schema_names", &self.inner.schema_names())
            .finish_non_exhaustive()
    }
}

impl CaseInsensitiveCatalog {
    fn resolve_schema_name(&self, name: &str) -> Option<String> {
        if self.inner.schema(name).is_some() {
            return Some(name.to_string());
        }
        self.inner
            .schema_names()
            .into_iter()
            .find(|n| n.eq_ignore_ascii_case(name))
    }
}

impl CatalogProvider for CaseInsensitiveCatalog {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema_names(&self) -> Vec<String> {
        self.inner.schema_names()
    }

    fn schema(&self, name: &str) -> Option<Arc<dyn SchemaProvider>> {
        let resolved = self.resolve_schema_name(name)?;
        let inner = self.inner.schema(&resolved)?;
        Some(Arc::new(CaseInsensitiveSchema::new(inner)))
    }
}

struct CaseInsensitiveSchema {
    inner: Arc<dyn SchemaProvider>,
    table_names: HashMap<String, String>,
}

impl std::fmt::Debug for CaseInsensitiveSchema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaseInsensitiveSchema")
            .field("table_names", &self.table_names)
            .finish_non_exhaustive()
    }
}

impl CaseInsensitiveSchema {
    fn new(inner: Arc<dyn SchemaProvider>) -> Self {
        let table_names = inner
            .table_names()
            .into_iter()
            .map(|n| (n.to_ascii_lowercase(), n))
            .collect();
        Self { inner, table_names }
    }

    fn resolve_table_name(&self, name: &str) -> Option<String> {
        if self.inner.table_exist(name) {
            return Some(name.to_string());
        }
        self.table_names.get(&name.to_ascii_lowercase()).cloned()
    }
}

#[async_trait]
impl SchemaProvider for CaseInsensitiveSchema {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn table_names(&self) -> Vec<String> {
        self.inner.table_names()
    }

    fn table_exist(&self, name: &str) -> bool {
        self.resolve_table_name(name).is_some()
    }

    async fn table(&self, name: &str) -> DFResult<Option<Arc<dyn TableProvider>>> {
        let Some(resolved) = self.resolve_table_name(name) else {
            return Ok(None);
        };
        self.inner.table(&resolved).await
    }

    fn register_table(
        &self,
        name: String,
        table: Arc<dyn TableProvider>,
    ) -> DFResult<Option<Arc<dyn TableProvider>>> {
        self.inner.register_table(name, table)
    }

    fn deregister_table(&self, name: &str) -> DFResult<Option<Arc<dyn TableProvider>>> {
        let Some(resolved) = self.resolve_table_name(name) else {
            return Ok(None);
        };
        self.inner.deregister_table(&resolved)
    }
}
