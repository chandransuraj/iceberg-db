//! REST catalog wrapper: merges Snowflake / IRC `storage-credentials` into table FileIO.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use iceberg::io::{FileIO, FileIOBuilder, StorageFactory};
use iceberg::{
    Catalog, Namespace, NamespaceIdent, TableCommit, TableCreation, TableIdent,
};
use iceberg::table::Table;
use iceberg_catalog_rest::{LoadTableResult, RestCatalog, StorageCredential};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};
use reqwest::{Client, StatusCode};
use crate::http_log;
use crate::snowflake_auth;

/// REST catalog that applies vended S3 credentials from `loadTable` responses.
pub struct VendedRestCatalog {
    inner: RestCatalog,
    props: HashMap<String, String>,
    storage_factory: Arc<dyn StorageFactory>,
    http: Client,
}

impl VendedRestCatalog {
    pub fn new(
        inner: RestCatalog,
        props: HashMap<String, String>,
        storage_factory: Arc<dyn StorageFactory>,
    ) -> Result<Self> {
        let http = build_http_client(&props)?;
        Ok(Self {
            inner,
            props,
            storage_factory,
            http,
        })
    }

    async fn load_table_vended(&self, table_ident: &TableIdent) -> Result<Table> {
        let uri = self
            .props
            .get("uri")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("REST catalog missing uri"))?;
        let url = table_endpoint(uri, table_ident);

        let bearer = snowflake_auth::exchange_pat(&self.props).await?;

        let auth_header = format!("Bearer {}", http_log::redact_secret(&bearer));
        let mut req_headers = catalog_request_headers(&self.props);
        req_headers.push(("Authorization".into(), auth_header));

        http_log::log_outbound("GET", &url, &req_headers, None);

        let response = self
            .http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {bearer}"))
            .send()
            .await
            .context("load_table HTTP")?;

        if http_log::enabled() {
            eprintln!("--- idb HTTP response ---");
            eprintln!("GET {url}");
            eprintln!("status: {}", response.status());
            eprintln!("--- end ---");
        }
        let status = response.status();
        let body = response.bytes().await.context("load_table body")?;

        if status != StatusCode::OK {
            return Err(anyhow!(
                "load_table failed ({status}): {}\n\
Hint: verify PAT role/scope, warehouse database name, and account URI region",
                String::from_utf8_lossy(&body)
            ));
        }

        let load: LoadTableResult =
            serde_json::from_slice(&body).context("parse load_table JSON")?;

        let mut config: HashMap<String, String> = load
            .config
            .into_iter()
            .chain(self.props.clone())
            .collect();
        merge_storage_credentials(&mut config, load.storage_credentials.as_ref());

        let file_io = build_file_io(
            &self.storage_factory,
            load.metadata_location.as_deref(),
            &config,
        )?;

        let mut builder = Table::builder()
            .identifier(table_ident.clone())
            .file_io(file_io)
            .metadata(load.metadata);

        if let Some(metadata_location) = load.metadata_location {
            builder = builder.metadata_location(metadata_location);
        }

        builder.build().map_err(|e| anyhow!("build table: {e}"))
    }
}

fn merge_storage_credentials(
    config: &mut HashMap<String, String>,
    storage_credentials: Option<&Vec<StorageCredential>>,
) {
    let Some(creds) = storage_credentials else {
        return;
    };
    for cred in creds {
        config.extend(cred.config.clone());
    }
}

fn build_file_io(
    factory: &Arc<dyn StorageFactory>,
    metadata_location: Option<&str>,
    props: &HashMap<String, String>,
) -> Result<FileIO> {
    if metadata_location.is_none() && !props.contains_key("warehouse") {
        return Err(anyhow!(
            "cannot build FileIO: missing metadata_location and warehouse"
        ));
    }
    Ok(FileIOBuilder::new(factory.clone())
        .with_props(props.clone())
        .build())
}

fn catalog_request_headers(props: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (key, value) in props {
        let Some(name) = key.strip_prefix("header.") else {
            continue;
        };
        out.push((name.to_string(), value.clone()));
    }
    let (vended_name, vended_value) = idb_config::profile::snowflake_vended_credentials_header();
    out.push((vended_name.to_string(), vended_value.to_string()));
    out
}

fn build_http_client(props: &HashMap<String, String>) -> Result<Client> {
    let mut headers = HeaderMap::new();
    for (key, value) in props {
        let Some(name) = key.strip_prefix("header.") else {
            continue;
        };
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| anyhow!("invalid header name {name}: {e}"))?;
        let value = HeaderValue::from_str(value)
            .map_err(|e| anyhow!("invalid header value for {name}: {e}"))?;
        headers.insert(name, value);
    }
    let (vended_name, vended_value) = idb_config::profile::snowflake_vended_credentials_header();
    headers.insert(
        HeaderName::from_bytes(vended_name.as_bytes()).unwrap(),
        HeaderValue::from_str(vended_value).unwrap(),
    );
    Client::builder()
        .default_headers(headers)
        .build()
        .context("build HTTP client")
}

fn table_endpoint(uri: &str, table: &TableIdent) -> String {
    let base = uri.trim_end_matches('/');
    format!(
        "{base}/v1/namespaces/{}/tables/{}",
        table.namespace().to_url_string(),
        table.name()
    )
}

macro_rules! delegate {
    ($self:expr, $method:ident ( $($arg:expr),* $(,)? )) => {
        $self.inner.$method($($arg),*).await
    };
}

#[async_trait]
impl Catalog for VendedRestCatalog {
    async fn list_namespaces(
        &self,
        parent: Option<&NamespaceIdent>,
    ) -> iceberg::Result<Vec<NamespaceIdent>> {
        delegate!(self, list_namespaces(parent))
    }

    async fn create_namespace(
        &self,
        namespace: &NamespaceIdent,
        properties: HashMap<String, String>,
    ) -> iceberg::Result<Namespace> {
        delegate!(self, create_namespace(namespace, properties))
    }

    async fn get_namespace(&self, namespace: &NamespaceIdent) -> iceberg::Result<Namespace> {
        delegate!(self, get_namespace(namespace))
    }

    async fn namespace_exists(&self, namespace: &NamespaceIdent) -> iceberg::Result<bool> {
        delegate!(self, namespace_exists(namespace))
    }

    async fn update_namespace(
        &self,
        namespace: &NamespaceIdent,
        properties: HashMap<String, String>,
    ) -> iceberg::Result<()> {
        delegate!(self, update_namespace(namespace, properties))
    }

    async fn drop_namespace(&self, namespace: &NamespaceIdent) -> iceberg::Result<()> {
        delegate!(self, drop_namespace(namespace))
    }

    async fn list_tables(&self, namespace: &NamespaceIdent) -> iceberg::Result<Vec<TableIdent>> {
        delegate!(self, list_tables(namespace))
    }

    async fn create_table(
        &self,
        namespace: &NamespaceIdent,
        creation: TableCreation,
    ) -> iceberg::Result<Table> {
        delegate!(self, create_table(namespace, creation))
    }

    async fn load_table(&self, table: &TableIdent) -> iceberg::Result<Table> {
        self.load_table_vended(table)
            .await
            .map_err(|e| iceberg::Error::new(iceberg::ErrorKind::Unexpected, e.to_string()))
    }

    async fn drop_table(&self, table: &TableIdent) -> iceberg::Result<()> {
        delegate!(self, drop_table(table))
    }

    async fn table_exists(&self, table: &TableIdent) -> iceberg::Result<bool> {
        delegate!(self, table_exists(table))
    }

    async fn rename_table(&self, src: &TableIdent, dest: &TableIdent) -> iceberg::Result<()> {
        delegate!(self, rename_table(src, dest))
    }

    async fn register_table(
        &self,
        table: &TableIdent,
        metadata_location: String,
    ) -> iceberg::Result<Table> {
        delegate!(self, register_table(table, metadata_location))
    }

    async fn update_table(&self, commit: TableCommit) -> iceberg::Result<Table> {
        delegate!(self, update_table(commit))
    }
}

impl std::fmt::Debug for VendedRestCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VendedRestCatalog")
            .field("uri", &self.props.get("uri"))
            .field("warehouse", &self.props.get("warehouse"))
            .finish()
    }
}
