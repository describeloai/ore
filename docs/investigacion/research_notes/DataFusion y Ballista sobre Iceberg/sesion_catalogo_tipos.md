# DataFusion 55.x embedding APIs for a platform session layer (session C1, catalogs C2, object stores C3, types C6, SQL, UDFs, testing, geo) — state Oct 2026

Version anchor: docs.rs serves **datafusion-common 55.2.0** and **datafusion-execution 55.2.0** as "latest" ([docs.rs extension.rs](https://docs.rs/datafusion-common/latest/src/datafusion_common/types/extension.rs.html), [docs.rs ObjectStoreRegistry](https://docs.rs/datafusion-execution/latest/datafusion_execution/object_store/trait.ObjectStoreRegistry.html)). **object_store latest is 0.13.2** ([docs.rs object_store 0.13.2](https://docs.rs/crate/object_store/latest/source/src/client/mod.rs)). The upgrade guide index lists entries from 46.0.0 to 55.0.0 ([upgrade guides](https://datafusion.apache.org/library-user-guide/upgrading/index.html)). I found no 55.0.0 release blog post. A GitHub issue names the 53.0.0 blog, dated 2026-04-02, as the latest post at the time it was written ([issue #22392](https://github.com/apache/datafusion/issues/22392)).

Notation: items marked **[PK]** come from prior knowledge and were *not* re-fetched in this session. The URL given is where to verify them. Treat them as leads.

---

## Session construction (C1): SessionStateBuilder, SessionConfig, RuntimeEnv, planners, factories, multi-tenant patterns

### Takeaway
In 55.x the session layer is split. The traits (`Session`, `CatalogProviderList`, `TableProvider`, `QueryPlanner`, `ExtensionPlanner`, `PhysicalOptimizerRule`…) now live in the `datafusion-session` crate, and planners take `&dyn Session` instead of `&SessionState`. That makes a custom platform session a supported first-class pattern. For a multi-tenant server, the usual approach is one shared `Arc<RuntimeEnv>` (memory pool, disk, object-store registry, caches) plus a cheap per-query or per-tenant `SessionState` cloned from a pre-built template.

### Cited Findings
- **55.0.0: traits moved to `datafusion-session`.** `CatalogProviderList`, `CatalogProvider`, `SchemaProvider`, `TableProvider`, `TableProviderFactory`, `TableFunctionImpl` and `TableFunction` moved there. So did the planner/optimizer traits `QueryPlanner`, `PhysicalPlanner`, `ExtensionPlanner`, `PhysicalOptimizerRule` and `PhysicalOptimizerContext`. Old paths still work via re-exports. — [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **55.0.0: planner session argument.** Planner methods now take `&dyn Session` instead of `&SessionState`. — [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **55.0.0: `Session` trait additions.**
  - `catalog_list` is now required; use `EmptyCatalogProviderList` if the session has no catalogs.
  - `query_planner`, `optimize`, `physical_optimizers` and `statistics_registry` have defaults. Custom sessions that plan physically must override them.
  - `QueryPlanner` now has `Any` as a supertrait, so `downcast_ref::<T>()` works on it.
  - [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **55.0.0: `PhysicalPlanningContext`.** It replaces `ExecutionProps::subquery_indexes`, `subquery_results`, `lambda_variable_qualifier` and related fields. `ExtensionPlanner` methods receive this context and must forward it. Trailing `planning_ctx` parameters were added to `create_physical_expr(s)`, `create_physical_sort_expr(s)` and `create_window_expr`. — [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **55.0.0: FFI.** `ForeignSession` plans on the owning library's side, and `datafusion_ffi::query_planner::FFI_QueryPlanner` lets a foreign planner be installed. This is relevant if a session is shared with datafusion-python or other FFI consumers. — [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **55.0.0: `CreateExternalTable`.** It now has `locations: Vec<String>`; build it with `CreateExternalTableBuilder::with_locations`. All locations must share one schema and one object store. — [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **55.0.0: `ListingOptions` cleanup.** `ListingOptions::target_partitions` and `collect_stat` were removed. Set `with_target_partitions` and `with_collect_statistics` on `SessionConfig` instead. — [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **55.0.0: one cache type.** `DefaultFileMetadataCache`, `DefaultListFilesCache` and `DefaultFileStatisticsCache` were merged into `DefaultCache`. The traits `FileStatisticsCache`, `ListFilesCache` and `FileMetadataCache` are now aliases of `dyn Cache<...>`. `CachedFileMetadata` now stores a `SchemaFingerprint`. — [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **55.0.0: spill.** `DiskManager::create_tmp_file` returns `Arc<dyn SpillFile>`, and `DiskManagerMode::Custom(_)` allows a custom spill backend. — [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **55.0.0: MSRV** is Rust 1.94.0. — [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **55.0.0: predicate reordering.** The logical optimizer may reorder `AND` predicates so cheap ones run first; volatile expressions keep their position. If evaluation order matters, guard fallible expressions with `CASE WHEN … THEN … ELSE false END`. — [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **54.0.0: `as_any` removed** from these traits, replaced by trait upcasting (`downcast_ref` / `is` directly):
  - `PhysicalExpr`, `ScalarUDFImpl`, `AggregateUDFImpl`, `WindowUDFImpl`, `ExecutionPlan`
  - `TableProvider`, `SchemaProvider`, `CatalogProvider`, `CatalogProviderList`, `TableSource`
  - `FileSource`, `FileFormat`, `FileFormatFactory`, `DataSource`, `DataSink`
  - [54.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/54.0.0.html)
- **54.0.0: `MemoryPool`** now has `Any` as a supertrait, which implies `'static`. — [54.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/54.0.0.html)
- **54.0.0: file statistics cache.** It is memory-limited and managed by `CacheManager`. The config is `datafusion.runtime.file_statistics_cache_limit` (e.g. `'10M'`; `'0'` disables it). Attach it with `ListingTable::try_new(config)?.with_cache(ctx.runtime_env().cache_manager.get_file_statistic_cache())`. — [54.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/54.0.0.html)
- **54.0.0: lambdas / higher-order functions.**
  - `Session::higher_order_functions()` returns `&HashMap<String, Arc<dyn HigherOrderUDF>>`.
  - `TaskContext::new` takes a `higher_order_functions` argument.
  - `RegisterFunction::HigherOrder(...)` was added.
  - `Expr` gained `HigherOrderFunction`, `Lambda` and `LambdaVariable`.
  - [54.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/54.0.0.html)
- **54.0.0: proto.** `from_bytes_with_registry` was renamed `from_bytes_with_ctx(&TaskContext)`, and `PhysicalProtoConverterExtension` takes a `PhysicalPlanDecodeContext`. This matters for Ballista-style plan shipping. — [54.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/54.0.0.html)
- **Type planner registration:** `SessionStateBuilder::new().with_default_features().with_type_planner(...).build()`.
- **Dialect selection:** `SessionConfig::new().set_str("datafusion.sql_parser.dialect", "postgres")`, then `SessionContext::new_with_config(config)`. — [Extending SQL blog, 2026-01-12](https://datafusion.apache.org/blog/2026/01/12/extending-sql/)
- **[PK] Builder surface to verify on docs.rs.** `SessionStateBuilder` has these methods:
  - `with_config`, `with_runtime_env`, `with_catalog_list`
  - `with_default_features`, `with_scalar_functions`, `with_aggregate_functions`, `with_window_functions`
  - `with_analyzer_rules`, `with_optimizer_rules`, `with_physical_optimizer_rules`
  - `with_query_planner`, `with_expr_planners`, `with_table_factories`, `with_function_factory`, `with_object_store`
  - `SessionContext::new_with_state(state)` wraps the result.
  - `RuntimeEnvBuilder` has `with_memory_pool`, `with_disk_manager_builder`, `with_object_store_registry`, `with_cache_manager`, `build_arc`.
  - [docs.rs SessionStateBuilder](https://docs.rs/datafusion/latest/datafusion/execution/session_state/struct.SessionStateBuilder.html)
- **[PK] Factories.** `TableProviderFactory` (registered per `STORED AS` keyword) backs `CREATE EXTERNAL TABLE`. `FunctionFactory` backs `CREATE FUNCTION`. — [docs.rs SessionStateBuilder](https://docs.rs/datafusion/latest/datafusion/execution/session_state/struct.SessionStateBuilder.html)

### Inferences
- **Template-and-clone design.** Build one fully configured template `SessionState` at server start. It holds the function sets, rules, planners, `TableProviderFactory`s and a shared `Arc<RuntimeEnv>`. For each query or tenant, clone it and attach the tenant's catalog list, config overrides and identity (e.g. via `SessionConfig` extensions). The `remote_catalog.rs` example already clones the context per query (see C2), so per-query clones are the idiomatic pattern.
- **Share the runtime, split the logic.** Sharing `RuntimeEnv` lets all tenants share one `MemoryPool` (fair or greedy), one disk manager and one `DefaultCache`. Per-tenant isolation then has to come from the catalog list and the object-store routing, not from separate runtimes.
- **55.0.0 lowers the cost of a custom `Session`.** Because planners now take `&dyn Session` and the traits sit in a small crate, a thin platform-owned `Session` implementation becomes viable. It would wrap a `SessionState` and add tenant context.

### Gaps
- No measured numbers found for `SessionState::new` vs clone cost, i.e. registering roughly 300+ default functions into HashMaps. Benchmark it locally before deciding on clone vs rebuild.
- I did not fetch the 52.0.0 and 53.0.0 upgrade guides, so changes in those releases are not covered: [52.0.0](https://datafusion.apache.org/library-user-guide/upgrading/52.0.0.html), [53.0.0](https://datafusion.apache.org/library-user-guide/upgrading/53.0.0.html).
- The exact 55.x Arrow, sqlparser and object_store pins are not stated in the upgrade guide. Check the `Cargo.toml` of datafusion 55.2.0 on docs.rs.

---

## Catalogs (C2): CatalogProviderList, async/remote catalog, information_schema, views, names, caching

### Takeaway
The sync catalog traits must not do I/O during planning. The supported pattern for remote catalogs (Polaris, Unity, Hive, or a platform's own catalog) is:
1. Parse the SQL.
2. Call `SessionState::resolve_table_references`.
3. Resolve those references asynchronously through `AsyncCatalogProviderList` / `AsyncCatalogProvider` / `AsyncSchemaProvider::resolve`, which returns a cached, query-scoped sync provider.
4. Register the result in a cloned context and plan with `statement_to_plan`.

### Cited Findings
- **The `remote_catalog.rs` flow**, in order:
  - `ctx.sql()` fails with "table … not found", because `SessionContext::sql` can't do async catalog access.
  - The example parses with `state.sql_to_statement(sql, &dialect)`, then calls `state.resolve_table_references(&statement)`.
  - It awaits `remote_catalog_adapter.resolve(&references, state.config(), "datafusion", "remote_schema")`.
  - It clones the context, registers the resolved schema in the clone, and plans with `query_state.statement_to_plan(statement).await`.
  - The resolved catalog is scoped to one query through the cloned context, so the shared context is never mutated.
  - The adapter implements `AsyncSchemaProvider`. `RemoteTable` implements `TableProvider`, and its `scan()` awaits remote data and builds `MemorySourceConfig::try_new_exec`.
  - The file is at `datafusion-examples/examples/data_io/remote_catalog.rs`, which has moved from the older flat `examples/` path.
  - [remote_catalog.rs (main)](https://raw.githubusercontent.com/apache/datafusion/main/datafusion-examples/examples/data_io/remote_catalog.rs)
  - Caveat: the example passes `state`, not `query_state`, to `DataFrame::new`. Don't copy that detail blindly.
- **The async traits.**
  - `AsyncSchemaProvider::resolve` returns an `Arc<dyn SchemaProvider>` holding a cached copy of the referenced tables. It runs once before planning and may be slow.
  - `AsyncCatalogProvider::resolve` does the same one level up. `CatalogProvider::schema` stays sync on purpose.
  - `AsyncCatalogProviderList` resolves into a private `ResolvedCatalogProviderList`, a by-name cache of catalogs.
  - The cache is intended to be short-lived, for the execution of a single query, and there is no refresh or eviction mechanism.
  - The traits are present from 50.x through 54.0.0 and latest.
  - [docs.rs AsyncCatalogProvider](https://docs.rs/datafusion-catalog/latest/datafusion_catalog/trait.AsyncCatalogProvider.html), [async.rs 54.0.0](https://docs.rs/datafusion-catalog/54.0.0/src/datafusion_catalog/async.rs.html), [async.rs 50.1.0](https://docs.rs/crate/datafusion-catalog/50.1.0/source/src/async.rs)
- **No DuckDB-style `ATTACH`.** DataFusion has no built-in equivalent of `ATTACH` for catalogs; only `CREATE EXTERNAL TABLE` exists. The blog shows how to add a custom `CREATE EXTERNAL CATALOG` statement by wrapping `DFParser`. — [Extending SQL blog](https://datafusion.apache.org/blog/2026/01/12/extending-sql/)
- **55.0.0:** `CatalogProviderList`, `CatalogProvider` and `SchemaProvider` now live in `datafusion-session` (re-exported). — [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **[PK] Information schema and naming:**
  - `information_schema` is enabled via `datafusion.catalog.information_schema = true` or `SessionConfig::with_information_schema(true)`.
  - Unquoted identifiers are lower-cased when `datafusion.sql_parser.enable_ident_normalization = true` (the default).
  - Three-part names `catalog.schema.table` resolve against `datafusion.catalog.default_catalog` / `default_schema`.
  - Views are `ViewTable` (a `TableProvider` wrapping a `LogicalPlan`).
  - [config docs](https://datafusion.apache.org/user-guide/configs.html)

### Inferences
- **Map the platform catalog onto the async traits.** A transactional catalog fits as `AsyncCatalogProviderList`: one round trip per query resolves every referenced table, its snapshot or pointer, and its vended credentials. Then planning stays sync. The per-query cache is exactly the snapshot-isolation boundary you want.
- **Cross-query caching is the platform's job.** Any metadata cache that should outlive a query (TTL, invalidated by catalog version) belongs in the platform's resolver, because DataFusion provides no refresh or eviction.
- **Fallback for remote listing.** `information_schema` over an async catalog only shows what was resolved. A full remote listing (`SHOW TABLES`) needs either a sync snapshot or a custom statement path.

### Gaps
- I did not verify whether 55.x added a first-class "async resolve then plan" entry point on `SessionContext`, to replace the manual parse, resolve and plan sequence.
- Not fetched: the `ViewTable` docs, and the exact identifier-normalization config names for 55.x.

---

## Object stores with credentials (C3): registry, per-prefix vended creds, retries, caching, GCS HTTP/2

### Takeaway
`ObjectStoreRegistry` keys stores by **scheme + host + port only**, never by path. So several short-lived, prefix-scoped vended credentials for the same bucket can't be routed by the `DefaultObjectStoreRegistry`. You need one of two approaches:
- **(a) A custom `ObjectStoreRegistry`** whose `get_store(&Url)` inspects the full URL and dispatches to a per-prefix store, possibly created lazily.
- **(b) Inject the store per table**, e.g. a `TableProvider` or `FileScanConfig` bound to a synthetic `ObjectStoreUrl` such as `gs://bucket--tenant-x` that maps to a store carrying that table's `CredentialProvider`.

### Cited Findings
- **The `ObjectStoreRegistry` trait** (datafusion-execution 55.2.0):
  - `register_store(&self, url: &Url, store: Arc<dyn ObjectStore>) -> Option<Arc<dyn ObjectStore>>`
  - `get_store(&self, url: &Url) -> Result<Arc<dyn ObjectStore>>`
  - `deregister_store` (provided)
  - The default registry keys stores "based on the scheme, host and port of the provided URL". Stores "may be lazily created and registered" by "providing a custom implementation of ObjectStoreRegistry", and `get_store` may do ad-hoc discovery depending on the URL.
  - [docs.rs ObjectStoreRegistry](https://docs.rs/datafusion-execution/latest/datafusion_execution/object_store/trait.ObjectStoreRegistry.html)
- **CLI precedent.** The DataFusion CLI (branch-37) wraps AWS SDK credentials in a custom object_store `CredentialProvider`. It builds `AmazonS3Builder` / `GoogleCloudStorageBuilder` from per-URL options, and older versions read the options from `CREATE EXTERNAL TABLE` options. — [datafusion-cli object_storage.rs (branch-37)](https://apache.googlesource.com/datafusion/+/refs/heads/branch-37/datafusion-cli/src/object_storage.rs), [branch-30](https://apache.googlesource.com/datafusion/+/refs/heads/branch-30/datafusion-cli/src/object_storage.rs)
- **GCS credential type.** object_store exposes `GcpCredentialProvider`, an `Arc<dyn CredentialProvider<Credential = GcpCredential>>`. — [object_store 0.12.0 gcp/mod.rs](https://docs.rs/crate/object_store/0.12.0/source/src/gcp/mod.rs)
- **`ClientOptions` knobs:**
  - `http2_only` uses `http2_prior_knowledge()`, skipping HTTP/1.1 negotiation.
  - `pool_max_idle_per_host` defaults to no limit.
  - There are HTTP/2 keep-alive options: `http2_keep_alive_interval`, `http2_keep_alive_timeout`, `http2_keep_alive_while_idle`.
  - Default timeouts: 30 s request, 5 s connect.
  - [object_store 0.5.4 client](https://docs.rs/crate/object_store/0.5.4/source/src/client/mod.rs), [0.12.0 client](https://docs.rs/crate/object_store/0.12.0/source/src/client/mod.rs), [arrow-rs source](https://apache.googlesource.com/arrow-rs/+/34e2ac2e71a2cf0e8276173eb8b82c474f785ca2/object_store/src/client/mod.rs), [obstore config docs](https://developmentseed.org/obstore/v0.1.0/api/store/config/)
- **55.0.0: `CreateExternalTable`.** All `locations` must share one object store. — [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **55.0.0: Parquet reader.** `ParquetObjectReader` / `ParquetObjectWriter` are deprecated upstream in the `parquet` crate; migrate to `ParquetFileReader` or a custom `AsyncFileReader`. `CachedParquetFileReader` was removed; use `ParquetFileReaderFactory::create_reader`. These are the hooks for a metadata or byte-range cache layer. — [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **[PK] Retries, concurrency and caching:**
  - object_store `RetryConfig` defaults to `max_retries = 10` and `retry_timeout = 3 min`, with exponential backoff.
  - `LimitStore` caps concurrent requests per store.
  - Built-in S3/GCS credential providers use a `TokenCache` that refreshes before expiry.
  - A custom `CredentialProvider::get_credential()` is called per request, so it must cache internally.
  - Since 2025, object_store lives in its own repo, apache/arrow-rs-object-store; the 0.13 line has API changes.
  - Caching layers to evaluate: foyer (hybrid memory+disk cache), liquid-cache (datafusion-contrib, pushdown cache server), and DataFusion's own `DefaultCache` for file metadata, list-files and statistics.
  - [object_store docs](https://docs.rs/object_store/latest/object_store/), [arrow-rs-object-store](https://github.com/apache/arrow-rs-object-store)

### Inferences
- **Recommended routing for vended credentials:**
  1. The async catalog resolve (C2) returns `{table → (prefix, short-lived token, expiry)}`.
  2. Build one `ObjectStore` per (bucket, prefix, credential) with a `CredentialProvider` that refreshes through the platform catalog. Optionally wrap it in `PrefixStore` and `LimitStore`.
  3. Register it under a synthetic per-table authority, or answer it from a custom registry's `get_store`.
  4. Point the table's `ObjectStoreUrl` at it.

  This keeps a token scoped to one table even when several tables share a bucket.
- **GCS HTTP/2.** A single multiplexed connection can cap throughput on GCS for many parallel range GETs. It is worth measuring HTTP/1.1 with a large pool vs `http2_only` against prod-like traffic before choosing.

### Gaps
- No primary source found on GCS-specific HTTP/2 vs HTTP/1.1 throughput with object_store, or on recommended `RetryConfig` tuning.
- `CredentialProvider` and `ClientOptions` were not re-verified on object_store 0.13.2; the searches mostly surfaced 0.5–0.12 sources.
- No official DataFusion example of prefix-scoped multi-credential routing was found. This is an open design area.

---

## Types (C6): Arrow types, view types, extension types / FieldMetadata, JSON, Iceberg v3

### Takeaway
Extension-type support in DataFusion is real but young:
- `FieldMetadata` propagates field metadata through plans (much better since 48.0.0).
- `DFExtensionType` / `DFJson` exist in datafusion-common 55.2.0, but the only customization point the docs show is pretty-printing via `create_array_formatter`. Casting and planning semantics per extension type are not covered.
- Geo (geoarrow.wkb) therefore has to be carried as Binary/BinaryView plus field metadata, with the semantics supplied by UDFs (as SedonaDB does).

### Cited Findings
- **`DFExtensionType` trait** (datafusion-common 55.2.0):
  - Bounds: `Debug + Send + Sync`.
  - `storage_type() -> DataType` (fixed storage type).
  - `serialize_metadata() -> Option<String>`.
  - `create_array_formatter(&dyn Array, &FormatOptions) -> Result<Option<ArrayFormatter>>`; the default `Ok(None)` uses default formatting.
  - Alias: `DFExtensionTypeRef = Arc<dyn DFExtensionType>`.
  - The source says extension-type support is young and that only pretty-printing is currently customizable.
  - `DFJson` delegates metadata to arrow-rs `arrow_schema::extension::Json`, with storage of `Utf8`, `LargeUtf8` or `Utf8View`.
  - [docs.rs extension.rs (55.2.0)](https://docs.rs/datafusion-common/latest/src/datafusion_common/types/extension.rs.html)
- **Two other type primitives:**
  - The `LogicalType` trait in `datafusion_common::types` covers logical-planning-level types; types with identical signatures are considered equal.
  - `FieldMetadata` in `datafusion_common::metadata` wraps an `Arc<BTreeMap<String,String>>` and converts to and from Arrow `Field`.
  - [logical.rs 54.0.0](https://docs.rs/crate/datafusion-common/54.0.0/source/src/types/logical.rs), [types index](https://docs.rs/datafusion-common/latest/datafusion_common/types/index.html), [metadata.rs](https://docs.rs/crate/datafusion-common/latest/source/src/metadata.rs)
- **Custom-types blog (2025-09-21).** It describes user-defined types via metadata. The work became easier after the metadata improvements in DataFusion 48.0.0, and canonical extension types such as UUID are supported. — [Implementing User Defined Types and Custom Metadata in DataFusion](https://datafusion.apache.org/blog/2025/09/21/custom-types-using-metadata)
- **54.0.0: alias metadata.** `Expr::unalias_nested()` no longer strips aliases that carry non-empty `FieldMetadata`, so metadata survives aliasing. — [54.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/54.0.0.html)
- **54.0.0: coercion changes:**
  - In string/numeric comparisons, the string side is now cast to numeric. For example, `5 > '100'` is now false, non-numeric strings raise cast errors, and mixed-type `IN` lists are rejected.
  - New `type_union_coercion` for `UNION`/`CASE`/`NVL2`, preferring strings.
  - Avro now reads via `arrow-avro`: `timestamp-*` maps to UTC-aware timestamps and `local-timestamp-*` stays naive.
  - [54.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/54.0.0.html)
- **55.0.0: type semantics:**
  - Typed `Coercion` materializes dictionaries by default; opt out with `Coercion::with_encoding_preservation(EncodingPreservation::dictionary())`.
  - Decimal scalars now display scaled, e.g. `Decimal128(0.1,1,1)`.
  - `time ± interval` returns a wrapped `time`.
  - Map casts with nested Structs match fields by case-sensitive name.
  - RANGE frames with offsets reject non-arithmetic `ORDER BY` types.
  - [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **Custom SQL type names.** `TypePlanner` maps them to Arrow types; for example, MySQL `DATETIME` is not supported out of the box, but a `TypePlanner` can map it. — [Extending SQL blog](https://datafusion.apache.org/blog/2026/01/12/extending-sql/)
- **JSON.** Postgres `->` / `->>` JSON operators parse only under the Postgres dialect and need an `ExprPlanner`; `datafusion-functions-json` is the complete implementation. — [Extending SQL blog](https://datafusion.apache.org/blog/2026/01/12/extending-sql/)
- **[PK] View types are the default:**
  - `datafusion.execution.parquet.schema_force_view_types = true` (Parquet strings and binaries are read as `Utf8View`/`BinaryView`).
  - SQL `VARCHAR`/`TEXT` maps to `Utf8View` through `datafusion.sql_parser.map_string_types_to_utf8view`, formerly `map_varchar_to_utf8view`.
  - Arrow canonical extension types include `arrow.uuid`, `arrow.json`, `arrow.bool8`, `arrow.opaque`, fixed/variable shape tensor and `arrow.parquet.variant`.
  - `geoarrow.wkb` is a GeoArrow-defined (community) extension, not an Arrow canonical one.
  - Iceberg v3 adds `variant`, `geometry`, `geography`, `timestamp_ns`, `timestamptz_ns`, `unknown` and default values.
  - [configs](https://datafusion.apache.org/user-guide/configs.html), [Arrow canonical extensions](https://arrow.apache.org/docs/format/CanonicalExtensions.html), [Iceberg spec](https://iceberg.apache.org/spec/)

### Inferences
- **Treat extension types as storage type plus metadata.** For a lakehouse, carry them as storage type + `FieldMetadata` (`ARROW:extension:name`/`metadata`) end to end. Make sure UDF return fields preserve metadata (`return_field_from_args`). Don't rely on DataFusion for casting or comparison semantics of extension types yet.
- **Expect test churn on upgrade.** 54.0.0's coercion change is a behaviour break that can silently change results in user SQL. Pin it with `.slt` tests in the platform's own suite (see Testing).

### Gaps
- I could not confirm the status of a 2026 "logical types" epic, or whether a `DFExtensionType` registry hooks into casting or planning in 55.x.
- Not verified: Variant support in DataFusion or arrow-rs (the `parquet-variant` crates), or Iceberg v3 type mapping in iceberg-rust's DataFusion integration.
- Not fetched: Decimal256, Timestamp-with-timezone semantics, `ListView`, `Interval` details.

---

## SQL dialect and DML

### Takeaway
DataFusion SQL (via sqlparser-rs, dialect set by `datafusion.sql_parser.dialect`) covers more DuckDB-isms than is often assumed:
- `QUALIFY`, `EXCLUDE`/`EXCEPT`/`REPLACE` wildcards (but not `RENAME`), `ASOF JOIN … MATCH_CONDITION`, `LATERAL`, `GROUP BY ALL`, `UNION BY NAME`, `DISTINCT ON`, BigQuery-style pipe `|>`, MySQL `LIMIT o,c`, and `ORDER BY ALL` under the DuckDB dialect.
- Missing natively: PIVOT/UNPIVOT, TABLESAMPLE, ATTACH and MERGE. The first three can be added via `RelationPlanner` / `DFParser` extensions.
- DML: `DELETE`/`UPDATE` route to `TableProvider::delete_from` / `update` hooks (from 52.0.0, per a 2026 PR thread), alongside `INSERT` and `COPY`.

### Cited Findings
- **The 55.x SELECT reference documents:**
  - `WITH [RECURSIVE]` (`datafusion.execution.enable_recursive_ctes`), `SELECT INTO`
  - `DISTINCT ON`, whose initial `ORDER BY` must match
  - Wildcards with `EXCLUDE`, `EXCEPT` and `REPLACE`; `RENAME` and `* AS alias` are explicitly unsupported
  - `UNNEST` / `unnest_outer`
  - Joins: `INNER/LEFT/RIGHT/FULL/CROSS/NATURAL`, `LEFT/RIGHT SEMI|ANTI`
  - `ASOF JOIN` with `MATCH_CONDITION` (both inputs bounded, right side must fit in memory)
  - `LATERAL`, excluding RIGHT/FULL lateral, HAVING and outer refs in the lateral select list
  - `GROUP BY` with `ROLLUP`/`CUBE`/`GROUPING SETS`/`ALL`, ordered aggregates
  - `WINDOW`, `QUALIFY` (requires a window function)
  - `UNION/INTERSECT/EXCEPT [ALL|DISTINCT]`, `UNION BY NAME`
  - Pipe operators: `WHERE, ORDER BY, LIMIT, SELECT, EXTEND, AS, UNION, INTERSECT, EXCEPT, AGGREGATE, JOIN`
  - `VALUES` as a FROM item
  - `ORDER BY ALL` (DuckDB dialect only)
  - [select.md](https://datafusion.apache.org/_sources/user-guide/sql/select.md.txt)
- **Extension points:**
  - `ExprPlanner` returns `Planned`/`Original` (PR #11180).
  - `TypePlanner` (PR #13294).
  - `RelationPlanner` intercepts FROM-clause constructs at any nesting level, available from **DataFusion 52** (PR #17843).
  - Custom statements are handled by wrapping `DFParser`.
  - Examples: `custom_sql_parser.rs`, `pivot_unpivot.rs` (PIVOT as GROUP BY + CASE, UNPIVOT as UNION ALL), `table_sample.rs` (custom logical node + `SampleExec`), `match_recognize.rs` (in progress).
  - Statement-level planning, JOIN planning and TOP/FETCH have limited hooks.
  - [Extending SQL blog](https://datafusion.apache.org/blog/2026/01/12/extending-sql/)
- **No native PIVOT.** A third-party crate (krishiv-sql) rewrites PIVOT/UNPIVOT to standard SQL because DataFusion doesn't parse them natively. This is a secondary source. — [krishiv-sql pivot_sql](https://docs.rs/krishiv-sql/latest/krishiv_sql/pivot_sql/)
- **DELETE/UPDATE hooks:**
  - `TableProvider` gained `delete_from` and `update`, plus `dml_capabilities()` (default NONE) and a `DmlCapabilities` struct.
  - Before this, DELETE/UPDATE parsed but returned NotImplemented at physical planning.
  - The `WHERE` clause reaches the provider only as `filters`; an empty vec means all rows. A follow-up (PR #24657) aims to reject plans whose row restriction can't travel through `filters`.
  - The provider returns affected rows in a `count` column. `MemTable` implements both.
  - Original PR #19142; docs PR #24567 states the support landed in 52.0.0.
  - Sources: [PR #19142](https://github.com/apache/datafusion/pull/19142), [PR #24567 review](https://www.mail-archive.com/github@datafusion.apache.org/msg167392.html), [PR #24567 open](https://mail-archive.com/github@datafusion.apache.org/msg156092.html)
  - These come from mailing-list mirrors of GitHub; the exact signatures were not seen.
- **Version-dependent subquery behaviour:**
  - 54.0.0: uncorrelated scalar subqueries run in `ScalarSubqueryExec` and fail at runtime if they return more than one row.
  - 55.0.0: new `Dialect::Spark` variant and `datafusion.spark.map_key_dedup_policy`.
  - [54.0.0](https://datafusion.apache.org/library-user-guide/upgrading/54.0.0.html), [55.0.0](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)

### Inferences
- **Iceberg row-level DML.** Iceberg DELETE/UPDATE can plug into `delete_from`/`update`, but only for predicates expressible as pushed `filters`. `MERGE INTO` would need a custom statement or `RelationPlanner` path.

### Gaps
- No source found confirming MERGE support status, `::` cast / struct dot-access docs (both believed supported **[PK]**), DuckDB `COLUMNS()`, FROM-first syntax, or the sqlparser-rs version pinned by 55.x.

---

## UDF / UDAF / UDWF / UDTF, async UDFs, Python UDFs

### Takeaway
The `*Impl` traits (`ScalarUDFImpl`, `AggregateUDFImpl`, `WindowUDFImpl`, `TableFunctionImpl`) are the extension surface. 54.0.0 removed `as_any`, and 54.0.0 added higher-order functions and lambdas as a new UDF kind. 55.0.0 changed `GroupsAccumulator` (breaks the FFI ABI).

### Cited Findings
- **54.0.0:** `as_any` was removed from `ScalarUDFImpl`/`AggregateUDFImpl`/`WindowUDFImpl`, and `HigherOrderUDF` was added, registered via `RegisterFunction::HigherOrder`. `FunctionRegistry` gained `higher_order_function(name)` (EPIC #21172). — [54.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/54.0.0.html)
- **55.0.0:**
  - `GroupsAccumulator::merge_batch` dropped `opt_filter`.
  - `convert_to_state` is now required and `supports_convert_to_state` was removed. This changes the `FFI_GroupsAccumulator` ABI, so FFI providers and consumers must be rebuilt against 55.
  - `WindowExpr::evaluate_stateful` takes `&WindowEvalContext`.
  - `TableFunctionImpl` / `TableFunction` moved to `datafusion-session`.
  - [55.0.0 upgrade guide](https://datafusion.apache.org/library-user-guide/upgrading/55.0.0.html)
- **SedonaDB Python UDFs.** SedonaDB 0.2.0 added Python UDF support on top of DataFusion. — [SedonaDB 0.2.0 release](https://sedona.apache.org/1.9.1/blog/2025/12/01/sedonadb-020-release/)
- **[PK] Async UDFs and datafusion-python:**
  - `AsyncScalarUDFImpl` (async scalar UDFs, executed by `AsyncFuncExec`) landed around 48.0.
  - datafusion-python exposes `udf`, `udaf`, `udwf` and `udtf`. It can import Rust providers via `datafusion-ffi` (`__datafusion_table_provider__` capsules).
  - [docs.rs datafusion](https://docs.rs/datafusion/latest/datafusion/), [datafusion-python docs](https://datafusion.apache.org/python/)

### Gaps
- Not verified in this session: the async UDF API name or signature in 55.x, or the datafusion-python version that matches 55.

---

## Testing standard: sqllogictest, datafusion-testing, fuzz, benchmarks

### Takeaway
DataFusion's conformance standard is `.slt` files run by `sqllogictest-rs`, one isolated `SessionContext` per file. It adds Postgres-compat, SQLite (millions of queries from the `datafusion-testing` submodule), TPC-H and Substrait round-trip modes. A platform should reuse this harness against its own session and keep its own `.slt` corpus.

### Cited Findings
- **Running the suite:**
  - `cargo test --test sqllogictests [-- <filter>|file:line]`
  - `--complete` rewrites expected output; it is refused for files that declare `configMatrix`.
  - `# configMatrix: key=v1,v2` runs a file once per combination of config values.
  - Each `.slt` file gets its own isolated `SessionContext`. Scratch dir: `test_files/scratch/<file>`.
  - Record types use one character per column (`B D I P R T ?`); sort modes are `nosort`, `rowsort` and `valuesort`.
  - [sqllogictest README](https://raw.githubusercontent.com/apache/datafusion/main/datafusion/sqllogictest/README.md)
- **Modes:**
  - `PG_COMPAT=true` + `PG_URI` with `--features=postgres`; the runner starts a temporary Docker container if `PG_URI` is unset, and the collation must be `C`. `pg_compat_*` files run against both engines.
  - `INCLUDE_TPCH=true` (SF 0.1 data).
  - `INCLUDE_SQLITE=true` uses the `datafusion-testing` submodule's `data/sqlite`; raise `RUST_MIN_STACK` and use the `release-nonlto` profile.
  - Substrait: `--features substrait -- --substrait-round-trip [--substrait-optimize]`. The full suite still has failures (issue #16248), so CI runs only a subset.
  - Diagnostics: `--timing-summary`, plus memory-drift checking (on by default).
  - [sqllogictest README](https://raw.githubusercontent.com/apache/datafusion/main/datafusion/sqllogictest/README.md)

### Inferences / [PK]
- **Custom-session testing [PK].** The `datafusion-sqllogictest` crate is published on crates.io. Its engine type `DataFusion` implements `sqllogictest::AsyncDB` around a `SessionContext`, so a platform test binary can construct its own context (catalogs, object stores, UDFs) and run DataFusion's `.slt` files plus its own. Verify the constructor in `datafusion/sqllogictest/src/engines/datafusion_engine`.
- **What to run in CI.** Run DataFusion's `.slt` corpus against the platform session to detect semantic drift on every DataFusion bump. 54.0.0's coercion change and 55.0.0's predicate reordering are the kind of thing this catches.

### Gaps
- The README does not describe the engine internals for plugging in a custom `SessionContext`.
- Not covered in this session: fuzz tests (`datafusion/core/tests/fuzz_cases`), `benchmarks/` (TPC-H, ClickBench, `bench.sh`) and the extended-tests CI workflow.

---

## Geo on DataFusion: SedonaDB, geodatafusion, geoarrow-rs

### Takeaway
SedonaDB (an Apache Sedona subproject) is the mature DataFusion-based spatial engine. It went from 0.1 in September 2025 to 0.4.0 on 2026-06-19:
- 176 ST_/RS_ functions tested against PostGIS/BigQuery
- larger-than-memory spatial joins (0.3)
- row-level CRS (0.3)
- geography via s2geography
- an optional GPU spatial join
- GeoParquet 1.1/2.0-draft and GeoArrow 1.0/1.1/2.0 metadata

The raster type and R dplyr support are still explicitly immature.

### Cited Findings
- **SedonaDB origins.** Released September 2025: a single-node engine on DataFusion with Python/R interfaces. 0.2.0 added:
  - GDAL/OGR read
  - GeoParquet 1.1 write
  - Python UDFs
  - an initial raster type
  - a crates.io release

  0.3.0 brought larger-than-memory spatial joins and row-level CRS.
  - [SedonaDB 0.2.0 release](https://sedona.apache.org/1.9.1/blog/2025/12/01/sedonadb-020-release/), [Sedona 2025 Year in Review](https://sedona.apache.org/1.8.1/blog/2026/01/11/apache-sedona-2025-year-in-review/), [Wherobots SpatialBench blog](https://wherobots.com/blog/sedonadb-spatialbench-spatial-query-engine-apache-sedona/)
- **SedonaDB 0.4.0 (2026-06-19):**
  - 187 issues resolved, 26 new functions, 176 ST_ and RS_ functions total
  - conda-forge packaging
  - Python DataFrame API (`.geo.centroid()` accessors)
  - R `sdplyr` (WIP)
  - geography rewritten on s2geography; `d_within` takes meters
  - GPU spatial join: off by default, not in pip packages, Docker image only, compute capability 7.5/8.6/8.9, enabled with `SET gpu.enable = true`
  - Parquet write for geometry/geography with statistics, as plain Parquet, GeoParquet 2.0 (draft) or GeoArrow 1.0/1.1/2.0 metadata
  - N-dimensional raster type plus a `sedonadb-zarr` extension
  - The authors say they are "not ready to announce that SedonaDB fully supports raster data".
  - [SedonaDB 0.4.0 release](https://sedona.apache.org/1.9.1/blog/2026/06/19/sedonadb-040-release/)
- **Version labels.** Sedona doc versions 1.8.1 and 1.9.1 both host SedonaDB posts; the mapping is not clear. — search results above
- **Geometry operators.** The DataFusion blog lists geometry operators as an `ExprPlanner` extension candidate. — [Extending SQL blog](https://datafusion.apache.org/blog/2026/01/12/extending-sql/)

### Inferences / [PK]
- **Embedding SedonaDB.** Its Rust crates (`sedona-*`, published from 0.2) register spatial UDFs, a spatial-join physical planner/optimizer rule, and GeoArrow-aware type handling into a DataFusion `SessionStateBuilder`. Embedding it in a platform session means pinning to the DataFusion major that SedonaDB pins, which typically lags DataFusion main by about one release. Verify in the sedona-db `Cargo.toml` (github.com/apache/sedona-db).
- **Lighter alternatives.** geodatafusion (datafusion-contrib, by the geoarrow-rs author) is a lighter ST_ UDF set on geoarrow-rs, with no spatial-join planner.

### Gaps
- Not obtained: SedonaDB's DataFusion version pin, its crate layout, a Rust embedding API example, its spatial-join algorithm (R-tree build side / KNN), its CRS representation (PROJJSON vs SRID in the type metadata), and its Iceberg v3 geometry support.
- No current geodatafusion maturity data.
