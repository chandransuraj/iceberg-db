//! Build Iceberg `Catalog` instances from `idb-config` entries.

pub mod demo_memory;

#[cfg(feature = "native")]
mod hadoop;
#[cfg(feature = "native")]
mod resolved_paths;
#[cfg(feature = "native")]
mod http_log;
#[cfg(feature = "native")]
mod rest_vended;
#[cfg(feature = "native")]
mod snowflake_auth;

#[cfg(feature = "native")]
use std::collections::HashMap;
#[cfg(feature = "native")]
use std::path::Path;
#[cfg(feature = "native")]
use std::sync::Arc;

#[cfg(feature = "native")]
use anyhow::{anyhow, bail, Context, Result};
#[cfg(feature = "native")]
use iceberg::io::{LocalFsStorageFactory, StorageFactory};
#[cfg(feature = "native")]
use iceberg::memory::{MemoryCatalogBuilder, MEMORY_CATALOG_WAREHOUSE};
#[cfg(feature = "native")]
use iceberg::{Catalog, CatalogBuilder};
#[cfg(feature = "native")]
use iceberg_catalog_rest::{RestCatalogBuilder, REST_CATALOG_PROP_URI, REST_CATALOG_PROP_WAREHOUSE};
#[cfg(feature = "native")]
use iceberg_storage_opendal::OpenDalStorageFactory;
#[cfg(feature = "native")]
use rest_vended::VendedRestCatalog;
#[cfg(feature = "native")]
use idb_config::{resolve_value, AppConfig, CatalogSpec};
#[cfg(feature = "native")]
use tracing::info;

#[cfg(feature = "native")]
pub struct CatalogRegistry {
    default_name: String,
    /// Schema (namespace) for unqualified table names and bare `SHOW TABLES`.
    default_schema: String,
    catalogs: HashMap<String, Arc<dyn Catalog>>,
}

#[cfg(feature = "native")]
impl CatalogRegistry {
    pub async fn from_config(config: &AppConfig) -> Result<Self> {
        if config.catalogs.is_empty() {
            bail!("no catalogs in config");
        }
        let default_name = config
            .default_catalog
            .clone()
            .or_else(|| config.catalogs.keys().next().cloned())
            .context("default catalog")?;
        let mut catalogs = HashMap::new();
        for (name, spec) in &config.catalogs {
            info!(catalog = %name, r#type = %spec.catalog_type, "opening catalog");
            let catalog = open_catalog(name, spec).await?;
            catalogs.insert(name.clone(), catalog);
        }
        if !catalogs.contains_key(&default_name) {
            bail!("unknown default catalog: {default_name}");
        }
        let default_schema = config
            .catalogs
            .get(&default_name)
            .and_then(default_schema_from_spec)
            .unwrap_or_else(|| "public".to_string());
        Ok(Self {
            default_name,
            default_schema,
            catalogs,
        })
    }

    pub async fn from_file_warehouse(catalog_name: &str, warehouse: &Path) -> Result<Self> {
        let warehouse = warehouse
            .canonicalize()
            .unwrap_or_else(|_| warehouse.to_path_buf());
        let catalog = open_file_warehouse_path(catalog_name, &warehouse).await?;
        Ok(Self {
            default_name: catalog_name.to_string(),
            default_schema: "public".to_string(),
            catalogs: HashMap::from([(catalog_name.to_string(), catalog)]),
        })
    }

    pub fn default_name(&self) -> &str {
        &self.default_name
    }

    pub fn default_schema(&self) -> &str {
        &self.default_schema
    }

    pub fn get(&self, name: &str) -> Result<Arc<dyn Catalog>> {
        self.catalogs
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow!("unknown catalog: {name}"))
    }

    pub fn default(&self) -> Arc<dyn Catalog> {
        self.catalogs
            .get(&self.default_name)
            .expect("default catalog")
            .clone()
    }
}

#[cfg(feature = "native")]
async fn open_catalog(name: &str, spec: &CatalogSpec) -> Result<Arc<dyn Catalog>> {
    match spec.catalog_type.to_ascii_lowercase().as_str() {
        "hadoop" | "file" => open_file_warehouse(name, spec).await,
        "rest" => open_rest(name, spec).await,
        other => bail!("unsupported catalog type: {other}"),
    }
}

#[cfg(feature = "native")]
async fn open_file_warehouse(name: &str, spec: &CatalogSpec) -> Result<Arc<dyn Catalog>> {
    let warehouse = spec
        .property("warehouse")
        .filter(|s| !s.is_empty())
        .map(|s| resolve_value(&s))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("catalog '{name}' requires warehouse"))?;
    let warehouse = Path::new(&warehouse);
    open_file_warehouse_path(name, warehouse).await
}

#[cfg(feature = "native")]
async fn open_file_warehouse_path(name: &str, warehouse: &Path) -> Result<Arc<dyn Catalog>> {
    if !warehouse.is_absolute() {
        bail!(
            "warehouse must be an absolute path for catalog '{name}': {}",
            warehouse.display()
        );
    }
    let props = HashMap::from([(
        MEMORY_CATALOG_WAREHOUSE.to_string(),
        warehouse.to_string_lossy().replace('\\', "/"),
    )]);
    let catalog = MemoryCatalogBuilder::default()
        .with_storage_factory(Arc::new(LocalFsStorageFactory))
        .load(name, props)
        .await
        .map_err(|e| anyhow!("memory catalog: {e}"))?;
    let catalog: Arc<dyn Catalog> = Arc::new(catalog);
    hadoop::bootstrap_hadoop_tables(catalog.clone(), warehouse)
        .await
        .context("bootstrap Hadoop-style warehouse tables")?;
    Ok(Arc::new(resolved_paths::ResolvedPathCatalog::new(catalog)))
}

#[cfg(feature = "native")]
async fn open_rest(name: &str, spec: &CatalogSpec) -> Result<Arc<dyn Catalog>> {
    let mut props = spec.rest_catalog_properties();
    if let Err(msg) =
        idb_config::profile::validate_rest_props(name, &props, spec.profile_name().as_deref())
    {
        bail!("{msg}");
    }
    let uri = props
        .remove("uri")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("REST catalog '{name}' requires uri"))?;
    props.entry(REST_CATALOG_PROP_URI.to_string()).or_insert(uri);
    if let Some(warehouse) = props.get("warehouse").cloned() {
        props
            .entry(REST_CATALOG_PROP_WAREHOUSE.to_string())
            .or_insert(warehouse);
    }
    let mut props: HashMap<String, String> = props.into_iter().collect();

    let storage_factory: Arc<dyn StorageFactory> = Arc::new(OpenDalStorageFactory::S3 {
        configured_scheme: "s3".to_string(),
        customized_credential_load: None,
    });

    let mut rest_props = props.clone();
    rest_props.remove("header.X-Iceberg-Access-Delegation");

    let is_snowflake = idb_config::profile::is_snowflake_horizon_profile(
        spec.profile_name().as_deref(),
        rest_props.get("uri").map(String::as_str),
    );

    if is_snowflake {
        let bearer = snowflake_auth::exchange_pat(&rest_props)
            .await
            .context(
                "Snowflake PAT → bearer exchange failed. Use username, scope session:role:<ROLE>, and PAT.",
            )?;
        rest_props.insert("token".to_string(), bearer.clone());
        rest_props.remove("credential");
        props.insert("token".to_string(), bearer);
        props.remove("credential");
    }

    http_log::log_catalog_bootstrap(&rest_props);

    let inner = RestCatalogBuilder::default()
        .with_storage_factory(storage_factory.clone())
        .load(name, rest_props)
        .await
        .map_err(|e| anyhow!("rest catalog: {e}"))?;

    let catalog = VendedRestCatalog::new(inner, props, storage_factory)
        .context("vended REST catalog")?;

    Ok(Arc::new(catalog))
}

#[cfg(feature = "native")]
fn default_schema_from_spec(spec: &CatalogSpec) -> Option<String> {
    spec.property("default-schema")
        .or_else(|| spec.property("schema"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
