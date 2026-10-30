# Apache Iceberg + Apache DataFusion in Rust (state as of 2026-10-30)

Scope: reading and writing Iceberg from DataFusion with performance and spec correctness, to inform ORE's catalog (C2), credentials (C3), read path (C4) and write path (C5). Source-code facts were read from GitHub `main` on 2026-10-30 (iceberg-rust last commit 2026-10-29; datafusion-iceberg last commit 2026-10-30). Everything labelled "release notes" comes from the official release posts.

## 1. apache/iceberg-rust: version, maturity, read and write support

### Takeaway
iceberg-rust is pre-1.0. The latest release is **0.10.1 (2026-08-01)**, and `main` is still versioned 0.10.1 with MSRV 1.95. Read support is solid for v1/v2/v3: manifest and partition pruning, row-group and page-index pruning, position deletes, equality deletes and v3 deletion vectors all read. Writes remain **append-only (`fast_append`)**. Overwrite, RowDelta (MERGE/DELETE/UPDATE) and conflict-validating snapshot producers are open PRs. The biggest structural change of 2026: **the DataFusion integration left the repo on 2026-09-17 and now lives in `apache/datafusion-iceberg`**, which has no release yet (workspace version 0.12.0, DataFusion 55, iceberg-rust pinned by git rev).

### Cited Findings
**Release timeline (exact versions)**
- v0.6.0, 30 Jul 2025:
  - Transaction actions (fast append, update properties/location, sort order, format upgrade, statistics) with retry logic.
  - Position-delete parsing.
  - DataFusion metadata tables.
  - Azure ADLS storage.
  - OpenDAL 0.54.
  - Breaking: `Transaction` owns `base_table`.
  - Source: [GitHub releases](https://github.com/apache/iceberg-rust/releases)
- v0.7.0, 10 Oct 2025:
  - DataFusion 48.
  - `IcebergWriteExec`, `IcebergCommitExec` and `insert_into` on `IcebergTableProvider`.
  - `RollingFileWriter` and a partition-aware `LocationGenerator`.
  - Equality-delete parsing.
  - Bulk insertion into deletion vectors.
  - Catalog loaders for REST, Glue, S3 Tables, HMS and memory.
  - MSRV 1.87.
  - Source: [GitHub releases](https://github.com/apache/iceberg-rust/releases)
- v0.8.0, changelog dated 2026-01-06, blog post 2026-02-02 (144 PRs, 37 contributors):
  - DataFusion 48→51 and Arrow 53→57.
  - Native `INSERT INTO` for partitioned tables: partition column projection, a repartition operator, a `sort_by_partition` operator and a parallel writer interface.
  - Clustered and Fanout writers.
  - Position and equality deletes can apply to the same `FileScanTask`.
  - Delete files are cached and shared across readers.
  - V3 metadata support, including V3 manifests.
  - Position-based projection for Parquet files without field IDs.
  - A `_file` metadata column.
  - Drops smol in favour of tokio.
  - MSRV 1.88.
  - OpenDAL 0.55.
  - Source: [Iceberg Rust 0.8.0 blog](https://iceberg.apache.org/blog/apache-iceberg-rust-0.8.0-release/). The GitHub release page instead says "DataFusion v49, then v50" ([releases](https://github.com/apache/iceberg-rust/releases)); the blog's "51" is the final state.
- v0.9.0, blog 2026-03-10 (109 PRs):
  - DataFusion 52.2.
  - `INSERT INTO` with automatic sort-based clustering for partitioned writes.
  - `CREATE TABLE` / `DROP TABLE`.
  - New `Storage` trait integrated with `FileIO` (`LocalFsStorage`, `MemoryStorage`). OpenDAL moved to the optional `iceberg-storage-opendal` crate. `object_store` is only "being considered".
  - `rust_decimal` replaced by `fastnum` for 38-digit decimals.
  - Reader work: `get_byte_ranges` coalescing small reads, a fast path for single-concurrency `ArrowReader::read`, a Parquet metadata size hint, and file-size propagation.
  - MSRV 1.92.
  - Source: [Iceberg Rust 0.9.0 blog](https://iceberg.apache.org/blog/apache-iceberg-rust-0.9.0-release/)
- Comet says it "contributed a number of reader performance improvements to iceberg-rust 0.9.0" — [Comet 0.14.0 release](https://datafusion.apache.org/blog/output/2026/03/18/datafusion-comet-0.14.0/)
- v0.9.1, 06 May 2026: patch release; only the changelog link is published — [GitHub releases](https://github.com/apache/iceberg-rust/releases)
- v0.10.0, 2026-07-21 (254 PRs, 40 contributors):
  - DataFusion 53 (then 53.1) and Arrow 58.
  - MSRV 1.94.
  - `purge_table` on the `Catalog` trait, and `with_runtime` on `CatalogBuilder`.
  - Overwrite/replace `INSERT` is now **rejected** instead of silently appending.
  - `ExpireSnapshotsAction` and the `update_schema` action.
  - `delete_stream` on the Storage trait.
  - A timeout layer inside `RetryLayer`.
  - S3 defaults to virtual-host style.
  - Encryption groundwork (AES-GCM, KMS); writes to encrypted tables are blocked.
  - `read_with_metrics()` / bytes-read metrics.
  - Fixes for NaN pushdown, IsNaN pushdown and sub-row-group byte-range filtering.
  - Roadmap: "snapshot validation mechanism for detecting write conflicts", which lays groundwork for RewriteDataFiles, RowDelta and Overwrite.
  - Source: [Iceberg Rust 0.10 blog](https://iceberg.apache.org/blog/apache-iceberg-rust-0.10.0-release/)
- v0.10.1, 2026-08-01: patch to fix the `pyiceberg-core` PyPI package (0.10.0 was yanked) — [GitHub releases](https://github.com/apache/iceberg-rust/releases)
- After 0.10.1 on `main`:
  - Upgraded to DataFusion 55.0 and Arrow/Parquet 59.2 (PR #2958, 2026-09-03).
  - Then "remove migrated DataFusion integration crates" (PR #3228, merged 2026-09-17).
  - Source: [commit history](https://github.com/apache/iceberg-rust/commits/main/crates/integrations/datafusion), [PR #3228](https://github.com/apache/iceberg-rust/pull/3228)
- `main` workspace `Cargo.toml` says `version = "0.10.1"` and `rust-version = "1.95"` — [Cargo.toml](https://github.com/apache/iceberg-rust/blob/main/Cargo.toml)

**Crates on `main`**
- Core: `iceberg`.
- Catalogs: `catalog/{rest,glue,hms,s3tables,sql,loader}`.
- Storage: `storage/opendal`.
- Integrations: `integrations/cache-moka`, described as "Apache Iceberg Moka cache implementation". No DataFusion crate remains.
- Source: [repo tree](https://github.com/apache/iceberg-rust/tree/main/crates)
- Storage is OpenDAL-only. Comet's native build depends on `iceberg-storage-opendal` with the `opendal-s3`, `opendal-gcs`, `opendal-azdls`, `opendal-oss`, `opendal-fs` and `opendal-memory` features. Separately, it uses `object_store 0.13.2` for its own DataFusion Parquet scans — [Comet native/Cargo.toml](https://github.com/apache/datafusion-comet/blob/main/native/Cargo.toml)

**Read features (on `main`)**
- `TableScanBuilder` options:
  - Concurrency: `with_concurrency_limit`, `with_data_file_concurrency_limit`, `with_manifest_entry_concurrency_limit`. The limits for manifest files, manifest entries and data files default to `num_cpus`.
  - Pruning: `with_row_group_filtering_enabled` (default `true`), `with_row_selection_enabled` (default `false`; page-index pruning), `with_bloom_filter_enabled`.
  - Other: `with_batch_size`, `with_case_sensitive`, `with_filter`, `snapshot_id`.
  - Source: [scan/mod.rs](https://github.com/apache/iceberg-rust/blob/main/crates/iceberg/src/scan/mod.rs)
- Planning caches are per partition spec id: `PartitionFilterCache`, `ManifestEvaluatorCache` and `ExpressionEvaluatorCache`. A historical spec whose source column was dropped resolves to an always-true filter — [scan/cache.rs](https://github.com/apache/iceberg-rust/blob/main/crates/iceberg/src/scan/cache.rs)
- `ObjectCache` "Caches metadata objects deserialized from immutable files" (Manifest and ManifestList). It is a moka future cache weighted by size, with a `DEFAULT_CACHE_SIZE_BYTES` and a disabled mode — [io/object_cache.rs](https://github.com/apache/iceberg-rust/blob/main/crates/iceberg/src/io/object_cache.rs)
- Deletion vectors (v3):
  - `DeleteVector` wraps a `RoaringTreemap`. It parses the `deletion-vector-v1` Puffin blob (magic 0x6439D3D1).
  - The caching delete loader reads a DV "blob's byte range directly" using `content_offset` and `referenced_data_file`.
  - Sources: [delete_vector.rs](https://github.com/apache/iceberg-rust/blob/main/crates/iceberg/src/delete_vector.rs), [caching_delete_file_loader.rs](https://github.com/apache/iceberg-rust/blob/main/crates/iceberg/src/arrow/caching_delete_file_loader.rs)
- Related merged PRs: "[1/N] decode deletion-vector-v1 puffin blobs" (#2866, merged 2026-08-18), "validate deletion vector delete tasks" (#3145, 2026-10-09), "make the delete_vector module public" (#3461, 2026-10-15). DV *serialization* PR #3232 was closed — [PR list](https://github.com/apache/iceberg-rust/pulls?q=deletion+vector)
- The reader includes `row_lineage.rs`, `positional_deletes.rs`, `row_filter.rs`, `projection.rs` and `int96.rs` modules — [arrow/reader](https://github.com/apache/iceberg-rust/tree/main/crates/iceberg/src/arrow/reader)

**Write and transaction features (on `main`)**
- `Transaction` actions: `update_table_properties`, `update_schema`, `fast_append`, `replace_sort_order`, `update_location`, `update_statistics`, `expire_snapshots`, `upgrade_format_version` — [transaction/mod.rs](https://github.com/apache/iceberg-rust/blob/main/crates/iceberg/src/transaction/mod.rs)
- Commit retries:
  - `Transaction::commit` retries with `ExponentialBackoff` built from `commit.retry.min-wait-ms`, `commit.retry.max-wait-ms` and `commit_total_retry_timeout_ms`, only `when(|e| e.retryable())`.
  - Each attempt (`do_commit`) reloads the table. If the metadata or metadata location changed, it uses the refreshed table as the base and **re-applies the actions**.
  - Source: [transaction/mod.rs](https://github.com/apache/iceberg-rust/blob/main/crates/iceberg/src/transaction/mod.rs)
- Open PRs (not merged as of 2026-10-30):
  - RowDelta action for row-level modifications (#2203, opened 2026-03-03).
  - Overwrite action (#3300, 2026-09-29).
  - `MergingSnapshotProducer` and `RewriteFilesAction` (#3046).
  - Change-type splitter for delta writes (#3231).
  - Merged: "core COW rewrite primitive" (#2752, merged 2026-10-01).
  - Source: [PR list](https://github.com/apache/iceberg-rust/pulls)

### Inferences
- ORE's C5 cannot rely on iceberg-rust for row-level operations (MERGE/DELETE/UPDATE) or overwrite in October 2026. Only append is production-ready. Copy-on-write rewrite exists as a low-level primitive.
- On conflict, `Transaction::commit` re-applies `fast_append` blindly. Without validation (still on the roadmap), this is safe only for pure appends, which is exactly what Iceberg allows without validation.
- Pin by git rev: datafusion-iceberg, Comet and Lakekeeper all pin iceberg-rust by git rev rather than by crates.io version. That suggests the released 0.10.x already lags what engines need.

### Gaps
- No public 0.11 release date or plan was found.
- Exact `DEFAULT_CACHE_SIZE_BYTES` value not extracted.
- Whether DV *writing* (the deletion-vector-v1 serializer) exists on `main` is unclear: #3232 was closed, maybe superseded.

## 2. How the DataFusion integration exposes catalogs and tables; caching, refresh, snapshot pinning

### Takeaway
The integration (now `apache/datafusion-iceberg`, crate `iceberg-datafusion`) loads everything when the catalog is built, gives the scan a single partition and reports unknown statistics. It is functional but naive for a high-performance engine:
- `IcebergCatalogProvider::try_new` eagerly lists all namespaces and tables and loads every table.
- `IcebergCatalogTableProvider` reloads metadata from the catalog **on every scan and write**.
- `IcebergStaticTableProvider` pins a `Table` and optionally a snapshot id.
- The physical scan exposes **one DataFusion partition** with **unknown statistics**.

### Cited Findings
**Where the integration lives**
- The move was proposed in [issue #3029](https://github.com/apache/iceberg-rust/issues/3029) to decouple the release and review lifecycle. Ballista and datafusion-distributed had expressed interest in hosting it.
- It was ported to `apache/datafusion-iceberg` in its PR #1. iceberg-rust then removed the crates — [PR #3228](https://github.com/apache/iceberg-rust/pull/3228)
- The repo is described as "Apache DataFusion Iceberg Integration". Workspace `version = "0.12.0"`, `datafusion = "55.0.0"` (bumped to 55.2.0 on 2026-10-30, #78). `iceberg` and `iceberg-catalog-rest` are pinned to git rev `5b82ab99…`. The README warns applications to use the same rev. No GitHub releases or tags exist. Sources: [repo](https://github.com/apache/datafusion-iceberg), [Cargo.toml](https://github.com/apache/datafusion-iceberg/blob/main/Cargo.toml), [README](https://github.com/apache/datafusion-iceberg/blob/main/README.md)
- The last published crate is `iceberg_datafusion` 0.10.1 on docs.rs. Its public items are `IcebergCatalogProvider`, `IcebergTableProviderFactory`, the `table` and `physical_plan` modules, and `to_datafusion_error`/`from_datafusion_error` — [docs.rs](https://docs.rs/iceberg-datafusion/latest/iceberg_datafusion/)
- Recent changes on datafusion-iceberg `main`:
  - "Enable page-index pruning for Iceberg scans" (#65, 2026-10-26).
  - "unescape LIKE prefixes before pushing them down as STARTS WITH" (#46).
  - "project columns correctly when scanning iceberg table metadata" (#47).
  - "Use avro for datafiles collection from write-exec" (#54).
  - Source: [commits](https://github.com/apache/datafusion-iceberg/commits/main)

**Catalog and schema providers**
- `IcebergCatalogProvider::try_new(client: Arc<dyn Catalog>)` is async. It calls `list_namespaces(None)` and builds an `IcebergSchemaProvider` per namespace — [catalog/provider.rs](https://github.com/apache/datafusion-iceberg/blob/main/crates/datafusion/src/catalog/provider.rs)
- `IcebergSchemaProvider::try_new`:
  - Calls `list_tables` and builds an `IcebergCatalogTableProvider` for each table with `try_join_all`, storing them in a `DashMap`.
  - A source comment says: "TODO: Tables and providers should be cached based on table_name if we have a cache miss… As of right now; tables might become stale."
  - `table_names()` and `table_exist()` are sync and answer from the map.
  - `table()` is async but only reads the map. It also resolves metadata tables via the `table$<type>` syntax.
  - Source: [catalog/schema.rs](https://github.com/apache/datafusion-iceberg/blob/main/crates/datafusion/src/catalog/schema.rs)
- `register_table` only accepts empty tables (it checks with LIMIT 1) — [catalog/schema.rs](https://github.com/apache/datafusion-iceberg/blob/main/crates/datafusion/src/catalog/schema.rs)

**Table providers**
- `IcebergCatalogTableProvider` is documented as "Catalog-backed table provider with automatic metadata refresh. This provider loads fresh table metadata from the catalog on every scan and write operation".
  - Its Arrow schema is cached at construction.
  - `scan()` calls `load_table` and "always use[s] current snapshot".
  - `supports_filters_pushdown` returns `Inexact` for every filter.
  - `insert_into` rejects any `InsertOp` other than `Append`.
  - Source: [catalog_table_provider.rs](https://github.com/apache/datafusion-iceberg/blob/main/crates/datafusion/src/table_provider/catalog_table_provider.rs)
- `IcebergStaticTableProvider` is a "Static table provider for read-only snapshot access… holds a cached table instance and does not refresh metadata". Constructors are `try_new_from_table(table)` and `try_new_from_table_snapshot(table, snapshot_id)`; the second is for time travel — [static_table_provider.rs](https://github.com/apache/datafusion-iceberg/blob/main/crates/datafusion/src/table_provider/static_table_provider.rs)
- `IcebergTableProviderFactory` supports `CREATE EXTERNAL TABLE … STORED AS ICEBERG LOCATION '<metadata.json>'`. It is read-only; registering a table does not create one — [README](https://github.com/apache/datafusion-iceberg/blob/main/README.md)

**Physical scan (`IcebergDataSource`, a DataFusion `DataSource`)**
- `output_partitioning()` returns `Partitioning::UnknownPartitioning(1)`.
- `partition_statistics()` returns `Statistics::new_unknown`.
- `open()` runs `table.scan()[.snapshot_id()].select(cols).with_filter(pred).with_row_selection_enabled(enable_page_index).build().to_arrow()` as one stream. Limit is applied by slicing batches.
- `try_swapping_with_projection` returns `None`.
- Source: [read/scan.rs](https://github.com/apache/datafusion-iceberg/blob/main/crates/datafusion/src/read/scan.rs)
- Filters convert to an Iceberg `Predicate` through `convert_filters_to_predicate`. Unconvertible parts are dropped, so the scan may return rows the filters reject; DataFusion re-applies them because the pushdown is Inexact — [read/scan.rs](https://github.com/apache/datafusion-iceberg/blob/main/crates/datafusion/src/read/scan.rs)

**Write path**
- `insert_into` builds this pipeline:
  1. Project partition values.
  2. Repartition.
  3. A `SortExec` when not in fanout mode (fanout is the default).
  4. `IcebergWriteExec` writes the data files.
  5. `CoalescePartitionsExec` merges the outputs.
  6. `IcebergCommitExec` commits.
  - Source: [catalog_table_provider.rs](https://github.com/apache/datafusion-iceberg/blob/main/crates/datafusion/src/table_provider/catalog_table_provider.rs)
- `IcebergCommitExec` "collect[s] the files written and use[s] `Transaction::fast_append`". It requires a single input partition. It commits through `Transaction::new(&table).fast_append().add_data_files(..).commit(catalog)` — [write/commit.rs](https://github.com/apache/datafusion-iceberg/blob/main/crates/datafusion/src/write/commit.rs)

### Inferences
- **No per-query snapshot pinning exists in the catalog-backed provider.** A query that references the same table twice may load metadata twice and could see two snapshots. ORE should resolve one snapshot per table per query in its own `TableProvider` (static-style) at planning time, which also avoids a catalog round-trip per scan.
- One-partition output plus unknown statistics means DataFusion's CBO, join ordering and repartitioning get no help. All parallelism happens inside iceberg-rust's `to_arrow` (data-file concurrency = `num_cpus`).
- RisingWave's TPC-H 100 GB failures were caused by missing cardinality estimates (Section 4), which reinforces that ORE should populate `Statistics` (row counts, column min/max/null counts) from manifest metrics.
- Eager listing of every namespace and table is unsuitable for a multi-tenant catalog. ORE needs a lazy `SchemaProvider` that does an async `load_table` inside `table()` and caches with an ETag.

### Gaps
- No documented benchmark of iceberg-datafusion versus a custom provider was found.
- No published timeline for a `datafusion-iceberg` crates.io release was found.

## 3. REST catalog spec features (vended credentials, loadCredentials, server-side planning, multi-table commits, idempotency) and iceberg-rust support

### Takeaway
The REST OpenAPI on `apache/iceberg` `main` (last changed 2026-10-29) defines:
- Delegated access through the `X-Iceberg-Access-Delegation` header.
- Credential refresh through `loadCredentials`.
- Server-side scan planning through `planTableScan` and its companion endpoints.
- Multi-table commits through `commitTransaction`.
- An `Idempotency-Key` header and an `idempotency-key-lifetime` config field.

The iceberg-rust REST client (which reports `ICEBERG_REST_SPEC_VERSION = "0.14.1"`) implements multi-table commit as an endpoint constant, ETag-conditional `loadTable`, and the use of vended credentials returned in `loadTable`. It does **not** implement `loadCredentials`, scan planning or per-prefix credentials; those are open PRs. No automatic `Idempotency-Key` header was found in the client.

### Cited Findings
**What the spec defines**
- `X-Iceberg-Access-Delegation` is an "Optional signal to the server that the client supports delegated access via a comma-separated list". Values: `vended-credentials`, `remote-signing`, `none`. The `none` value was added 2026-10-29 (#18480) — [rest-catalog-open-api.yaml](https://github.com/apache/iceberg/blob/main/open-api/rest-catalog-open-api.yaml), [commit history](https://github.com/apache/iceberg/commits/main/open-api/rest-catalog-open-api.yaml)
- `GET /v1/{prefix}/namespaces/{namespace}/tables/{table}/credentials` (`loadCredentials`), summary "Load vended credentials for a table from the catalog". Optional `planId` query parameter: "The plan ID that has been used for server-side scan planning" — [OpenAPI](https://github.com/apache/iceberg/blob/main/open-api/rest-catalog-open-api.yaml)
- Server-side planning:
  - `planTableScan` is `POST …/tables/{table}/plan`.
  - `fetchPlanningResult` is `GET …/plan/{plan-id}`.
  - `cancelPlanning` cancels a plan.
  - `fetchScanTasks` returns the tasks for each plan task.
  - Plans may come back synchronously or as "submitted" with a `plan-id` to poll. "Clients should inform the service if planning results are no longer needed by calling cancelPlanning".
  - Source: [OpenAPI](https://github.com/apache/iceberg/blob/main/open-api/rest-catalog-open-api.yaml)
- `Idempotency-Key` header:
  - A UUIDv7, "globally unique (no reuse across different operations)".
  - Finalize and replay: 200/201/204 and "deterministic terminal 4xx (including 409…)". 5xx are not stored.
  - The response "may reflect a newer state of the catalog".
  - `GET /v1/config` advertises `"idempotency-key-lifetime": "PT30M"`.
  - The header applies to `planTableScan`, `cancelPlanning`, `fetchScanTasks`, `commitTransaction` and other mutations.
  - Source: [OpenAPI](https://github.com/apache/iceberg/blob/main/open-api/rest-catalog-open-api.yaml)
- Iceberg Java 1.11 (2026-05-19) added a REST scan planning client — [catalog-state article, June 2026](https://dev.to/alexmercedcoder/the-state-of-apache-iceberg-catalogs-in-june-2026-265e) (secondary source). Java 1.12.0 was released 2026-10-05 — [Iceberg blog, release category](https://iceberg.apache.org/blog/category/release/)

**What iceberg-rust implements**
- Endpoint constants include `V1_COMMIT_TRANSACTION => POST "/v1/{prefix}/transactions/commit"`, `V1_REPORT_METRICS` and `V1_REGISTER_TABLE`. There is **no** `/credentials` endpoint. The client negotiates capabilities from the `endpoints` field of `GET /v1/config`, and tests probe `POST …/plan` support — [endpoint.rs](https://github.com/apache/iceberg-rust/blob/main/crates/catalog/rest/src/endpoint.rs), [catalog.rs](https://github.com/apache/iceberg-rust/blob/main/crates/catalog/rest/src/catalog.rs)
- `load_table`:
  - Sends `If-None-Match` with a cached ETag and serves 304 from cache.
  - Treats the response as vending credentials when `storage-credentials` is non-empty or the config has short-lived keys (S3 session token, GCS token, ADLS SAS). **Tables with vended credentials are never cached.**
  - Builds the `FileIO` from response `config` chained with user props.
  - Source: [catalog.rs](https://github.com/apache/iceberg-rust/blob/main/crates/catalog/rest/src/catalog.rs)
- `update_table` maps HTTP 409 to `CatalogCommitConflicts` with `retryable=true`. 500, 502 and 504 map to "the commit state is unknown" (not retryable). The table cache is invalidated before the commit — [catalog.rs](https://github.com/apache/iceberg-rust/blob/main/crates/catalog/rest/src/catalog.rs)
- Extra headers are only added through `header.<name>` properties, and sensitive headers are redacted in logs. The client has no built-in `X-Iceberg-Access-Delegation` constant, so it must be set as `header.X-Iceberg-Access-Delegation=vended-credentials` — [catalog.rs](https://github.com/apache/iceberg-rust/blob/main/crates/catalog/rest/src/catalog.rs)
- Open PRs:
  - "support vended storage credentials with per-prefix FileIO" (#2651, opened 2026-06-15).
  - "add REST server-side scan planning client" (#3011, opened 2026-08-16).
  - "expose portable reader options for planned tasks" (#3514).
  - "scan plan extraction" (#3294).
  - Source: [PR list](https://github.com/apache/iceberg-rust/pulls)
- Comet's native reader "does not automatically use temporary credentials vended by a REST catalog". It needs a credential provider bridge (`s3.comet.credential.provider.class`) — [Comet Iceberg guide](https://datafusion.apache.org/comet/user-guide/latest/iceberg.html)

### Inferences
- Credentials (C3):
  - iceberg-rust has no credential refresh. Vended credentials are frozen into the `FileIO` at `load_table` time.
  - Long scans or writes outliving the token TTL need ORE-side handling: rebuild the `FileIO`/`Table` with fresh credentials from ORE's `/credentials` endpoint, or keep the TTL longer than the maximum query duration.
  - The "vended ⇒ never cached" rule means every query pays a full `loadTable`, unless ORE caches metadata itself and separates metadata from credentials.
- Commits (C5):
  - ORE's server can be stricter than the client. iceberg-rust treats 409 as retryable and re-applies actions.
  - ORE's Idempotency-Key semantics (replay of 409 included) match the spec. The Rust client will not send the key unless ORE's own client layer adds it.
- Planning (C4): server-side planning is the natural home for ORE's catalog to do manifest pruning centrally and vend plan-scoped credentials (`loadCredentials?planId=`). No Rust client implements it yet.

### Gaps
- Not verified whether `commitTransaction` is exposed as a public multi-table API in iceberg-rust (only the endpoint constant was seen).
- `remote-signing` support in iceberg-rust was not checked; there was no evidence of it.

## 4. Other projects (Comet, JanKaul iceberg-rust, Lakekeeper, RisingWave, Sail, Embucket, Polaris, Nessie) and what they learned

### Takeaway
- **Comet** is the most production-proven DataFusion+Iceberg reader. Java plans, iceberg-rust executes. It has been the default since 0.15.0 (Apr 2026); 1.0.0 shipped 2026-08-07 and 1.1.0 on 2026-10-06.
- **RisingWave** shows DataFusion beating its native engine about 2.7× on Iceberg TPC-H, but it hits out-of-memory failures due to missing statistics.
- **Lakekeeper** (a Rust REST catalog) runs on a fork of iceberg-rust.
- **JanKaul's** independent `iceberg-rust` / `datafusion_iceberg` remains an `object_store`-based alternative.

### Cited Findings
**Comet**
- Design: "unmodified Iceberg Java [handles] query planning (i.e., catalog access, partition pruning, etc.), then Comet serializes Iceberg FileScanTask objects directly to iceberg-rust" (0.12.0) — [Comet 0.12.0 release](https://datafusion.apache.org/blog/2025/12/04/datafusion-comet-0.12.0)
- In 0.15.0 (2026-04-18) the "fully-native Iceberg reader is now enabled by default" and the Java-based Iceberg path was removed — [Comet 0.15.0 release](https://datafusion.apache.org/blog/2026/04/18/datafusion-comet-0.15.0)
- Releases: 1.0.0 on 2026-08-07, 1.1.0 on 2026-10-06 — [Comet GitHub releases](https://github.com/apache/datafusion-comet/releases)
- Native deps: DataFusion 55.2.0 and iceberg-rust git rev `1f3bc340…` with `iceberg-storage-opendal` — [native/Cargo.toml](https://github.com/apache/datafusion-comet/blob/main/native/Cargo.toml)
- Comet docs, supported:
  - Spec v1, v2 and v3.
  - Position deletes, equality deletes, mixed deletes and deletion vectors.
  - UUID, schema evolution (add/drop), `VERSION AS OF` and branch reads.
  - Partition transforms: identity, days, hour, bucket and truncate.
  - Encrypted v3 tables with AES-GCM 128 and 256 (Java unwraps the keys).
  - Source: [Comet Iceberg guide](https://datafusion.apache.org/comet/user-guide/latest/iceberg.html)
- Comet docs, falls back to the JVM reader on:
  - Spec v4 or newer.
  - v3 columns with an initial default.
  - `geometry`, `geography`, `unknown` and `variant` types.
  - Delete files that are not Parquet or Puffin, and Avro or ORC data files.
  - Equality deletes on a nested field.
  - Data or delete files spanning more than one S3 bucket.
  - Nested reads after schema evolution, or on tables with `schema.name-mapping.default`.
  - Known silent differences exist, such as files with neither field IDs nor a name mapping.
  - Source: [Comet Iceberg guide](https://datafusion.apache.org/comet/user-guide/latest/iceberg.html)
- Comet tuning:
  - `spark.comet.scan.icebergNative.dataFileConcurrencyLimit` defaults to 2 (2–8 described as reasonable).
  - The limit helps with many small files or high-latency storage, but each file in flight holds buffers.
  - Values above 1 reorder rows.
  - `bytesRead` comes from iceberg-rust `ScanMetrics` and includes delete files.
  - Source: [Comet Iceberg guide](https://datafusion.apache.org/comet/user-guide/latest/iceberg.html)
- A conference abstract claims JVM overhead adds "50-70%" on scan-heavy workloads. No figures are given; treat as unverified — [Sessionize talk](https://sessionize.com/s/shekharrajak/native-iceberg-scans-at-rust-speed-how-datafusion-/170180)

**RisingWave** (post dated 2026-07-23; TPC-H 100 GB on Iceberg, RisingWave v3.0.1)
- RisingWave plans the query, converts the plan to DataFusion, and uses its own `IcebergScan`. That operator "groups Iceberg file tasks by available parallelism and any query limit. Each group becomes a DataFusion partition" — [RisingWave blog](https://www.risingwave.com/blog/how-risingwave-uses-apache-datafusion-for-faster-iceberg-analytics/)
- Results:
  - Completion: DataFusion 20/22, native engine 16/22.
  - On the 16 common queries: native 4,536 s vs DataFusion 1,669 s, a 2.72× speedup. Earlier runs showed 2.16× (Jan 2026) and 1.40× (Feb 2026).
  - DataFusion ran out of memory on Q5 and Q18, attributed to "inaccurate or missing cardinality estimates". The lesson: prioritise "join planning and statistics propagation", with spilling as a fallback.
  - Source: [RisingWave blog](https://www.risingwave.com/blog/how-risingwave-uses-apache-datafusion-for-faster-iceberg-analytics/)

**JanKaul/iceberg-rust**
- `datafusion_iceberg` 0.10.0 (2026-03-26) depends on DataFusion 53 and `object_store` 0.13 (AWS/GCP/Azure). It provides `TableProvider`, `SchemaProvider` and `CatalogProvider`. Releases: 0.9.0 (2025-11-04), 0.8.0 (2025-07-08) — [lib.rs datafusion_iceberg](https://lib.rs/crates/datafusion_iceberg)
- A separate `datafusion-iceberg-sql` crate references DataFusion 53 — [lib.rs datafusion-iceberg-sql](https://lib.rs/crates/datafusion-iceberg-sql)

**Lakekeeper**
- Releases v0.14.2 and v0.13.8 (2026-10-25), v0.14.1 (2026-10-16). Depends on `iceberg = { git = "https://github.com/lakekeeper/iceberg-rust.git", rev = "e8eea499…" }`, a fork — [Lakekeeper releases](https://github.com/lakekeeper/lakekeeper/releases), [Cargo.toml](https://github.com/lakekeeper/lakekeeper/blob/main/Cargo.toml)
- Docs mark vended credentials and remote signing as done for S3 (AWS, with role assumption and session tags), ADLS Gen2 and GCS — [docs.lakekeeper.io](https://docs.lakekeeper.io) (via search snippet)

**Sail**
- Docs describe Iceberg via the Spark DataFrame API and SQL, with REST, Glue and Unity catalogs — [Sail Iceberg docs](https://docs.lakesail.com/sail/main/guide/sources/iceberg/)

### Inferences
- Comet's split (planning outside, file-task execution in iceberg-rust) matches ORE's situation: ORE's catalog could plan centrally and hand `FileScanTask`s to workers. Comet's fallback list is a ready checklist of what the iceberg-rust reader does not yet do (v3 defaults, geo, variant, cross-bucket scans).
- RisingWave's grouping of file tasks into N DataFusion partitions is the missing piece in iceberg-datafusion's single-partition scan.

### Gaps
- No primary sources were fetched for Embucket, Databend, Polaris or Nessie on DataFusion+Iceberg specifics.
- No Iceberg Summit 2026 talk content was retrieved.
- Comet 1.0/1.1 release-note details were not fetched; only tags and dates.

## 5. Performance best practices

### Takeaway
The levers that matter, as evidenced by code and projects:
1. Cache immutable manifests and manifest lists (`ObjectCache`, moka) and avoid reloading metadata per scan. Use ETag/304.
2. Run manifest planning concurrently (num_cpus defaults).
3. Map file tasks to N DataFusion partitions instead of one stream.
4. Enable row-group, page-index and bloom pruning.
5. Coalesce small reads (`get_byte_ranges`) and give a Parquet metadata size hint.
6. Expose manifest column statistics to DataFusion for the CBO.

The upstream integration currently does only 1, 2 and 4.

### Cited Findings
- Planning concurrency: manifest-file, manifest-entry and data-file concurrency default to `num_cpus` and are tunable per scan — [scan/mod.rs](https://github.com/apache/iceberg-rust/blob/main/crates/iceberg/src/scan/mod.rs)
- `with_row_selection_enabled` (page index) defaults to `false` in iceberg-rust. datafusion-iceberg wires it to DataFusion's `execution.parquet.enable_page_index` (#65, 2026-10-26) — [scan/mod.rs](https://github.com/apache/iceberg-rust/blob/main/crates/iceberg/src/scan/mod.rs), [read/scan.rs](https://github.com/apache/datafusion-iceberg/blob/main/crates/datafusion/src/read/scan.rs)
- Byte-range splits:
  - 0.8.0 "improved row group filtering when a FileScanTask contains byte ranges" ([0.8.0 blog](https://iceberg.apache.org/blog/apache-iceberg-rust-0.8.0-release/)).
  - 0.10.0 fixed "row-group byte-range filtering (sub-row-group splits)" ([0.10 blog](https://iceberg.apache.org/blog/apache-iceberg-rust-0.10.0-release/)).
- Reader I/O (0.9.0): `get_byte_ranges` coalescing small reads, a metadata size hint, file-size propagation (avoids a HEAD), and fewer redundant stream-builder calls — [0.9.0 blog](https://iceberg.apache.org/blog/apache-iceberg-rust-0.9.0-release/)
- Delete files are cached and shared across readers (0.8.0) — [0.8.0 blog](https://iceberg.apache.org/blog/apache-iceberg-rust-0.8.0-release/)
- Open PR "optionally retain column statistics on FileScanTask" (#3424, opened 2026-10-12), plus a partition statistics RFC (#3305) and spec rows (#3335). Today stats are not carried to the execution side — [PR list](https://github.com/apache/iceberg-rust/pulls)
- Writes respect content-defined chunking for Parquet file boundaries (0.10) — [0.10 blog](https://iceberg.apache.org/blog/apache-iceberg-rust-0.10.0-release/)
- A Comet draft PR claims 1.5–1.8× faster compaction than Spark's default using native DataFusion Parquet read/write; unmerged — [Comet PR #3519](https://github.com/apache/datafusion-comet/pull/3519)
- Comet keeps per-task file concurrency low (default 2) for memory reasons — [Comet Iceberg guide](https://datafusion.apache.org/comet/user-guide/latest/iceberg.html)

### Inferences
For ORE's read path (C4):
- Plan once per query: catalog `loadTable` with ETag, then `plan_files()` with a shared `ObjectCache` that lives across queries. Manifests are immutable, so the cache key is the path.
- Bin-pack `FileScanTask`s by bytes into `target_partitions` groups. Split big files into byte-range tasks aligned to row groups.
- Report `Statistics` aggregated from manifest entries (record_count, null_value_counts, lower/upper bounds), then DataFusion's own Parquet pruning.
- Keep the vended-credentials-disable-cache behaviour in mind (Section 3).

### Gaps
- No quantitative benchmark was found of manifest caching or partition counts in iceberg-rust.
- Compaction guidance for Rust writers (target file size defaults) was not extracted.

## 6. Correctness pitfalls (field ids, timestamps, decimals, UUID, geo, nulls, defaults)

### Takeaway
iceberg-rust reads by **field id** and falls back to position or name mapping for migrated files. A string of 2025–2026 fixes shows where the bugs were:
- NaN pushdown.
- INT96 timestamps.
- Nested field-id fallback.
- 38-digit decimals (needed `fastnum`).
- Promoted-type partition bounds causing false pruning (fixed 2026-10-22).

v3 `geometry`/`geography`/`variant`/`unknown` types and initial-default columns are not handled natively; Comet falls back to Java for them.

### Cited Findings
- Fallback field-id mapping for nested schemas, INT96 timestamps, NaN pushdown correctness, v2 snapshots without sequence numbers, and `fixedbinary(n)` were fixed in 0.10.0 — [GitHub releases](https://github.com/apache/iceberg-rust/releases), [0.10 blog](https://iceberg.apache.org/blog/apache-iceberg-rust-0.10.0-release/)
- Position-based projection for Parquet files without field IDs ("helps with migrated tables") and a fix to schema-evolution handling for Parquet (0.8.0) — [0.8.0 blog](https://iceberg.apache.org/blog/apache-iceberg-rust-0.8.0-release/)
- Decimal: `rust_decimal` was replaced by `fastnum` "for 38-digit precision", with a fix to the decimal byte representation used in hashing (bucket transforms) — [0.9.0 blog](https://iceberg.apache.org/blog/apache-iceberg-rust-0.9.0-release/)
- "widen promoted partition values and bounds to prevent false pruning" (#3483, merged 2026-10-22). This is a silent wrong-results class of bug after type promotion — [PR #3483](https://github.com/apache/iceberg-rust/pull/3483)
- Open: "drop v3-only data file fields from the v2 manifest schema" (#3452) — [PR list](https://github.com/apache/iceberg-rust/pulls)
- Negative positional delete positions now error instead of wrapping. A lost-wakeup hang in `DeleteFileIndex` was fixed. Puffin magic is validated (0.10) — [0.10 blog](https://iceberg.apache.org/blog/apache-iceberg-rust-0.10.0-release/)
- 0.8.0 fixes: equality deletes with partial schemas, case-sensitive equality matching, and stack overflow and deadlocks with large deletes — [0.8.0 blog](https://iceberg.apache.org/blog/apache-iceberg-rust-0.8.0-release/), [GitHub releases](https://github.com/apache/iceberg-rust/releases)
- A historical partition spec whose source column was dropped resolves to an always-true filter, so data is not wrongly pruned — [scan/cache.rs](https://github.com/apache/iceberg-rust/blob/main/crates/iceberg/src/scan/cache.rs)
- Comet's fallbacks:
  - v3 initial-default columns.
  - `geometry`, `geography`, `unknown` and `variant` types.
  - `truncate` on decimal.
  - Nested reads after evolution or with `schema.name-mapping.default`.
  - Known silent mismatch for files with neither field IDs nor a name mapping.
  - Source: [Comet Iceberg guide](https://datafusion.apache.org/comet/user-guide/latest/iceberg.html)
- `NOT IN` is evaluated after the scan rather than pushed down (Comet) — [Comet Iceberg guide](https://datafusion.apache.org/comet/user-guide/latest/iceberg.html)
- In datafusion-iceberg, LIKE-prefix pushdown to `STARTS WITH` needed an unescaping fix (#46) — [commits](https://github.com/apache/datafusion-iceberg/commits/main)

### Inferences
- For ORE's geo vertical (ADRs 0066/0067), v3 native `geometry`/`geography` reading through iceberg-rust is not ready: Comet explicitly falls back. ORE's current WKB-in-binary codec approach avoids this.
- Filters should stay `Inexact` (re-evaluated by DataFusion). Several pushdown translations (NaN, LIKE, promoted types) have had wrong-pruning bugs.
- Pin iceberg-rust to a rev that includes #3483 (2026-10-22) if tables use type promotion.

### Gaps
- No explicit source was found on timestamp_ns/timestamptz_ns (v3) read and write support in iceberg-rust.
- No explicit source was found on v3 `write-default`/`initial-default` handling in the reader.
- No explicit source was found on UUID partition/bounds handling beyond Comet stating UUID is supported.
- The `row_lineage.rs` module exists in the reader, but its exact coverage (`_row_id`, `_last_updated_sequence_number` inheritance) was not verified.
