# idb-bench

`idb-bench` compares `iceberg-db-rs` with DuckDB over the same SQL files.
It is intended for TPC-H and TPC-DS style suites where each query lives in a
separate `.sql` file, for example `q01.sql`.

The harness deliberately does not vendor TPC-H/TPC-DS query text or data
generators. Generate those from your preferred TPC kit, adapt table names to
your Iceberg catalog, and point `--queries` at the resulting directory.

## Example

```bash
cargo run -p idb-bench -- \
  --suite tpch \
  --queries ./bench/tpch-queries \
  --warehouse /abs/path/to/warehouse \
  --duckdb-setup ./bench/duckdb-iceberg.sql \
  --warmup 1 \
  --iterations 5 \
  --output-json ./bench/tpch-results.json \
  --output-csv ./bench/tpch-results.csv
```

The DuckDB setup file should contain the Iceberg extension setup and any table
or view mapping required for the benchmark queries. For example, depending on
your DuckDB/Iceberg extension version and table layout, it may load the
extension and create views with the TPC table names used by the query suite.

```sql
INSTALL iceberg;
LOAD iceberg;

-- Example only; adjust to your table metadata locations/catalog support.
CREATE OR REPLACE VIEW lineitem AS
SELECT * FROM iceberg_scan('/warehouse/tpch/lineitem/metadata/v1.metadata.json');
```

## Notes

- `iceberg-db-rs` runs in-process through `idb-core`.
- DuckDB is invoked through the `duckdb` CLI so the Rust workspace does not need
  to compile or link DuckDB.
- DuckDB timings include CLI process startup and setup SQL execution for each
  measured run. Keep setup SQL lightweight and use `--warmup`/`--iterations` to
  reduce noise.
- By default, DuckDB queries are wrapped as `COPY (<query>) TO '/dev/null'` so
  large result sets do not benchmark terminal rendering.
- JSON output includes warmups, per-run timings, row counts for `iceberg-db-rs`,
  and min/max/average/median summaries.
