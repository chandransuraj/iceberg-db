//! iceberg-db in the browser via WebAssembly.
//!
//! Uses an in-memory DataFusion table `demo.customers` (3 rows). A browser tab cannot read
//! `C:\...\.iceberg-db\warehouse`; use the native `idb` CLI for that path.

use std::sync::{Mutex, OnceLock};

#[cfg(target_arch = "wasm32")]
use tokio::runtime::Runtime;

use idb_sql::SqlSession;
use serde::Serialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

static SESSION: OnceLock<Mutex<SqlSession>> = OnceLock::new();

#[cfg(target_arch = "wasm32")]
static TOKIO: OnceLock<Runtime> = OnceLock::new();

#[cfg(target_arch = "wasm32")]
fn tokio_runtime() -> &'static Runtime {
    TOKIO.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("tokio runtime")
    })
}

/// Run async Rust on WASM where DataFusion expects a Tokio reactor.
#[cfg(target_arch = "wasm32")]
fn run_async<F, T>(future: F) -> Result<T, JsValue>
where
    F: std::future::Future<Output = Result<T, JsValue>>,
{
    tokio_runtime().block_on(future)
}

#[cfg(target_arch = "wasm32")]
fn log_init(step: &str) {
    web_sys::console::log_1(&format!("idb_init: {step}").into());
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub fn idb_wasm_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Build in-memory demo tables and a SQL session. Must be awaited before `idb_query`.
#[wasm_bindgen]
pub fn idb_init() -> js_sys::Promise {
    future_to_promise(async {
        #[cfg(target_arch = "wasm32")]
        {
            return run_async(init_session());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            init_session().await
        }
    })
}

async fn init_session() -> Result<JsValue, JsValue> {
    #[cfg(target_arch = "wasm32")]
    {
        log_init("datafusion demo table");
        let session = SqlSession::from_wasm_demo().await.map_err(js_error)?;
        log_init("done");
        SESSION
            .set(Mutex::new(session))
            .map_err(|_| JsValue::from_str("idb_init already called"))?;
        return Ok(JsValue::from_str("ready"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        use idb_catalog::demo_memory;
        let catalog = demo_memory::open_memory_demo_catalog()
            .await
            .map_err(js_error)?;
        let session = SqlSession::from_iceberg_catalog("local".to_string(), "public".to_string(), catalog)
            .await
            .map_err(js_error)?;
        SESSION
            .set(Mutex::new(session))
            .map_err(|_| JsValue::from_str("idb_init already called"))?;
        Ok(JsValue::from_str("ready"))
    }
}

#[wasm_bindgen]
pub fn idb_query(sql: String) -> js_sys::Promise {
    future_to_promise(async move {
        #[cfg(target_arch = "wasm32")]
        {
            return run_async(run_query(sql));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            run_query(sql).await
        }
    })
}

async fn run_query(sql: String) -> Result<JsValue, JsValue> {
    let session = SESSION
        .get()
        .ok_or_else(|| JsValue::from_str("call idb_init() first"))?;
    let result = session
        .lock()
        .map_err(|_| JsValue::from_str("session lock poisoned"))?
        .query(&sql)
        .await
        .map_err(js_error)?;
    let response = QueryResponse::from_result(&result).map_err(js_error)?;
    serde_wasm_bindgen::to_value(&response).map_err(js_error)
}

#[derive(Serialize)]
struct QueryResponse {
    row_count: usize,
    elapsed_ms: u128,
    columns: Vec<ColumnDto>,
    rows: Vec<Vec<String>>,
    text: String,
}

#[derive(Serialize)]
struct ColumnDto {
    name: String,
    data_type: String,
}

impl QueryResponse {
    fn from_result(result: &idb_sql::QueryResult) -> anyhow::Result<Self> {
        let text = SqlSession::format_batches_table(&result.batches);
        Ok(Self {
            row_count: result.row_count,
            elapsed_ms: result.elapsed_ms,
            columns: result
                .columns
                .iter()
                .map(|c| ColumnDto {
                    name: c.name.clone(),
                    data_type: c.data_type.clone(),
                })
                .collect(),
            rows: SqlSession::rows_as_strings(&result.batches),
            text,
        })
    }
}

fn js_error(err: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&err.to_string())
}
