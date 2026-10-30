# Parquet reading and writing performance in Apache DataFusion (state as of October 2026, DataFusion 55.x)

Scope note: defaults below are taken from the live DataFusion configs page on 2026-10-30, whose `created_by` default reads "datafusion version 55.1.0" (the page sidebar already lists an upgrade guide to 56.0.0, so the page tracks main, near 55.1/56). Release-specific facts come from the official release blogs for 50.0.0 (2025-09-29), 52.0.0 (2026-01-12), 53.0.0 (2026-04-02), 54.0.0 (2026-06-12) and 55.0.0 (2026-08-25). Anything older than 2026 is marked with its date.

## 1. Read path: pruning, pushdown, late materialization, dynamic filters, view types

### Takeaway
In 55.x almost every pruning layer is on by default (row-group statistics, page index, bloom filters on read, Utf8View/BinaryView, dynamic filters from TopK/joins/aggregates, metadata cache), but **row-level filter pushdown / late materialization (`datafusion.execution.parquet.pushdown_filters`) and `reorder_filters` are still `false` by default**. Turning them on is the single biggest opt-in lever (ClickBench Q23 up to 22x on one core when combined with dynamic filters), but it has known regressions on some query shapes, which is why the project has not flipped the default.

### Cited Findings

**Defaults in the current configs page (55.1.0-era)** — [DataFusion configs](https://datafusion.apache.org/user-guide/configs.html)
- `datafusion.execution.parquet.pruning` = `true` (row-group min/max pruning).
- `datafusion.execution.parquet.enable_page_index` = `true` (reads Column/Offset Index to prune pages and reduce decoded rows).
- `datafusion.execution.parquet.bloom_filter_on_read` = `true`.
- `datafusion.execution.parquet.pushdown_filters` = `false` ("late materialization").
- `datafusion.execution.parquet.reorder_filters` = `false` (only matters with pushdown_filters on).
- `datafusion.execution.parquet.force_filter_selections` = `false` (force RowSelections for filter results when pushdown is on).
- `datafusion.execution.parquet.max_predicate_cache_size` = NULL (use the parquet reader default; 0 disables).
- `datafusion.execution.parquet.schema_force_view_types` = `true` (read Utf8/Binary as Utf8View/BinaryView).
- `datafusion.execution.parquet.binary_as_string` = `false` (needed `true` for ClickBench `hits_partitioned`, whose strings are stored as Binary — see dynamic-filters blog below).
- `datafusion.execution.parquet.max_in_list_size` = 20 (max IN-list eligible for min/max pruning).
- `datafusion.execution.parquet.enable_rle_to_dictionary` = `false`.
- `datafusion.execution.parquet.row_group_range_assignment` = `start_offset` (alternative: `midpoint`; which byte-range split reads a row group).
- `datafusion.execution.parquet.skip_metadata` = `true` (ignore embedded schema metadata).
- `datafusion.execution.parquet.coerce_int96` / `coerce_int96_tz` = NULL.
- `datafusion.optimizer.enable_dynamic_filter_pushdown` = `true` (master switch), `enable_topk_dynamic_filter_pushdown` = `true`, `enable_join_dynamic_filter_pushdown` = `true`, `enable_aggregate_dynamic_filter_pushdown` = `true`.
- `datafusion.execution.target_partitions` = 0 (= CPU cores); `datafusion.execution.collect_statistics` = `true`.

**Pruning stack order (as of March 2026)** — [Turning LIMIT into an I/O Optimization (2026-03-20)](https://datafusion.apache.org/blog/2026/03/20/limit-pruning)
- Order: (1) partition pruning in ListingTable (directory partition columns) → (2) file-statistics pruning (`FilePruner`, file-level min/max/null counts) → (3) row-group statistics pruning, optionally bloom filters → (4) limit pruning (`prune_by_limit`) → (5) page index pruning → (6) late materialization (row filtering).
- Limit pruning (DataFusion 53, PR #18868): if "fully matched" row groups (whole row group satisfies the predicate per statistics) already contain enough rows for the LIMIT, partially matching row groups are skipped; only applies with LIMIT and no ORDER BY (`preserve_order` check). Metrics: `limit_pruned_row_groups`, `row_groups_pruned_statistics=... fully matched` — [DataFusion 53.0.0](https://datafusion.apache.org/blog/2026/04/02/datafusion-53.0.0)

**Late materialization / filter pushdown into the decoder**
- How it works: filter columns are decoded first, other projected columns are decoded only for surviving rows. Main cost: a column used by both filter and projection was decoded twice. ClickBench Q22 CPU breakdown: ~206 ms decompression, 117 ms decoding, 22 ms filter, 48 ms other — [Efficient Filter Pushdown in Parquet (2025-03-21)](https://datafusion.apache.org/blog/2025/03/21/parquet-pushdown)
- Fix ("predicate cache"): interleave filter evaluation and output decoding, caching only columns used in both filter and output, at most two pages per column (~2 MB; dictionary page pinned + current data page); arrow-rs PR #6921, <700 LOC. Measured on AMD 9900X ClickBench: total 38,344.79 ms → 32,800.50 ms (~15%); Q23 8104 → 3611 ms (2.24x), Q22 1.37x, Q37 1.26x, Q21 1.11x; 13 faster, 5 slower (Q40 1.21x slower, Q41 1.15x slower, attributed to noise), 25 unchanged — [Efficient Filter Pushdown in Parquet (2025-03-21)](https://datafusion.apache.org/blog/2025/03/21/parquet-pushdown)
- arrow-rs predicate cache API: `max_predicate_cache_size` on the async reader builder, default 100 MB ("per row group ... across all columns"; `usize::MAX` = unlimited). DataFusion exposes it as `max_predicate_cache_size` (None = parquet default, 0 = off) — [docs.rs DataFusion 55.0.0 ParquetOptions](https://docs.rs/datafusion/55.0.0/datafusion/config/struct.ParquetOptions.html); [arrow-rs ArrowReaderBuilder](https://arrow.apache.org/rust/parquet/arrow/arrow_reader/struct.ArrowReaderBuilder.html)
- arrow-rs late-materialization internals (Dec 2025): pipelined "read predicate column → RowSelection → read next column"; adaptive RowSelection representation — RLE `RowSelector` vs `BooleanBuffer` bitmask, switching to bitmask when average run length < **32** (PR #8733); `CachedArrayReader` with shared (evictable) + local (pinned) caches (PR #7850); page pruning via offset index `RowSelection::scan_ranges` (a page with even one selected row must be fully decompressed); when page pruning is active the bitmask policy falls back to RLE (open issue #8845). No end-to-end numbers given — [A Practical Dive Into Late Materialization in arrow-rs Parquet Reads (2025-12-11)](https://arrow.apache.org/blog/2025/12/11/parquet-late-materialization-deep-dive/)
- Default status: PR #16562 proposed flipping `pushdown_filters`/`reorder_filters` to true; PR #18873 ("TEST: enable pushdown_filters and reorder_filters by default", Nov 2025) was closed unmerged; an April 2026 PR (#21931) benchmarking pushdown on vs off showed mixed results (+7.4% in one run, −13.3% regression in another, attributed to an all-Map shape paying 128 probes per row). Umbrella ticket: apache/datafusion #3463 — [search results summary: PR #16562](https://github.com/apache/datafusion/pull/16562/files), [PR #18873](https://gh.ali360.cloudns.ch/apache/datafusion/pull/18873), [PR #21931](https://github.meowingcats01.workers.dev/apache/datafusion/pull/21931); configs page confirms still `false` — [configs](https://datafusion.apache.org/user-guide/configs.html)
- 54.0.0: when statistics show a filter matches every row in a row group, row filter and page-level pruning are skipped for that group (PR #21637, issue #19028); struct-field filters (`WHERE s['foo'] > 67`) pushed into the Parquet decoder plus leaf-level projection (PRs #20822, #20854, #20925) — [DataFusion 54.0.0](https://datafusion.apache.org/blog/2026/06/12/datafusion-54.0.0)
- 55.0.0: unread nested Parquet leaves are pruned (a Comet query went from 1.35 TB read to 30.9 GB, PR #24090); file-level Parquet row selections (PR #22940); virtual columns `file_row_index()` (#22604) and `input_file_name()` (#22978) — [DataFusion 55.0.0](https://datafusion.apache.org/blog/2026/08/25/datafusion-55.0.0)

**Dynamic filters**
- Mechanism: an operator publishes a runtime `Arc<dyn PhysicalExpr>` that the scan uses like any pushed predicate. Producers: TopK (`SortExec(TopK)` publishes the current Kth boundary), INNER hash joins (build-side min/max key range, later the build-side contents). Effect: prunes files and row groups via statistics; **row-level filtering by the dynamic filter requires `pushdown_filters=true`** — [Dynamic Filters (2025-09-10)](https://datafusion.apache.org/blog/2025/09/10/dynamic-filters)
- Numbers: ClickBench Q23 with dynamic filters + late materialization: 32.04 s → 1.42 s on one core (~22x); 5.04 s → 0.602 s on 12 cores (~8x). Hash-join example 2.5 s → 0.1 s (25x). Logfire "last K traces" >10x. Hash-join dynamic filters shipped in 50.0.0 enabled by default — [Dynamic Filters (2025-09-10)](https://datafusion.apache.org/blog/2025/09/10/dynamic-filters); [DataFusion 50.0.0](https://datafusion.apache.org/blog/2025/09/29/datafusion-50.0.0)
- Caveats: effective mainly when data is roughly ordered (e.g. append in time order); hash-join version initially min/max only — [Dynamic Filters](https://datafusion.apache.org/blog/2025/09/10/dynamic-filters)
- 52.0.0: MIN/MAX (no GROUP BY) aggregate dynamic filters (PR #18644); hash-join pushdown passes build-side hash map contents to the probe scan; with ≤20 build rows it becomes an `IN` list usable for statistics pruning (PR #18393 lineage) — [DataFusion 52.0.0](https://datafusion.apache.org/blog/2026/01/12/datafusion-52.0.0)
- 53.0.0: filters/dynamic filters pushed through more join types and through `UnionExec` (PRs #19918, #20145, #20192) — [DataFusion 53.0.0](https://datafusion.apache.org/blog/2026/04/02/datafusion-53.0.0)
- 55.0.0: runtime row-group pruning for TopK as the threshold tightens (PR #22450, tracking #23036): `topk_tpch` Q8 4.2x, ~43% total suite runtime cut, 5 of 11 queries 3.6–4.2x; dynamic filter propagation across stages (#24018, #24068) — [DataFusion 55.0.0](https://datafusion.apache.org/blog/2026/08/25/datafusion-55.0.0)

**Sort pushdown / statistics-ordered scans**
- 52.0.0: Parquet source reverses row-group and file scan order when the query asks for the opposite of the natural order (PR #19064); ~30x on benchmark queries over pre-sorted data — [DataFusion 52.0.0](https://datafusion.apache.org/blog/2026/01/12/datafusion-52.0.0)
- 54.0.0: files and row groups ordered by statistics to improve dynamic filtering and early stop for `ORDER BY ... LIMIT` (PRs #21182, #21426, #21956) — [DataFusion 54.0.0](https://datafusion.apache.org/blog/2026/06/12/datafusion-54.0.0)
- Exact mode: if each file is sorted and file min/max ranges do not overlap, files are reordered by min and `SortExec` is removed (a `BufferExec` keeps I/O parallel). Inexact mode: overlapping ranges keep the sort but read most-promising files/row groups first; TopK dynamic filter then prunes files, row groups and rows. Ordering declared via `WITH ORDER` or Parquet `sorting_columns`; on by default. Numbers: `sort_pushdown` single core 2.1x (full ORDER BY), 27x (`LIMIT 100`), 49x (`SELECT * ... LIMIT 100`); `sort_tpch` total 248.8 → 139.1 ms (−44%), no regressions — [Sort Pushdown (2026-07-20)](https://datafusion.apache.org/blog/2026/07/20/sort-pushdown)

**Scan parallelism**
- 54.0.0 morsel-driven scans (idle threads pull work): "up to ~2x faster for skewed scans such as ClickBench" (PRs #21327, #21342, #21351) — [DataFusion 54.0.0](https://datafusion.apache.org/blog/2026/06/12/datafusion-54.0.0)
- 55.0.0: `repartition_file_min_size` default lowered 10 MiB → 1 MiB (PR #22439); TPC-H Q22 68% faster — [DataFusion 55.0.0](https://datafusion.apache.org/blog/2026/08/25/datafusion-55.0.0)

**StringView / BinaryView**
- StringView landed in arrow-rs 52.2.0 / DataFusion 41.0.0 (Sep 2024, then opt-in). String-heavy ClickBench queries 20%–200% faster; loading BinaryView/StringView from Parquet ~2x faster than BinaryArray/StringArray in a microbenchmark; ClickBench Q20 ~24% faster (≈870 ms vs ≈1140 ms) — [StringView Part 1 (2024-09-13)](https://datafusion.apache.org/blog/2024/09/13/string-view-german-style-strings-part-1)
- Now default: `schema_force_view_types = true` in current configs — [configs](https://datafusion.apache.org/user-guide/configs.html)

### Inferences
- For ORE's C4 read path the recommended non-default settings are: `pushdown_filters=true` (+ consider `reorder_filters=true`) for selective queries, ideally per query/per table rather than globally, because the project's own benchmarks still show regressions on some shapes (wide Map columns, low-selectivity predicates). Keep `max_predicate_cache_size` at default unless memory-constrained.
- Dynamic filters (on by default) only reach their headline numbers (8–25x) together with `pushdown_filters=true`; without it they prune only files/row groups.
- Everything that relies on statistics (limit pruning, sort pushdown, TopK row-group pruning, dynamic filter file pruning) rewards writing data clustered/sorted on the hottest key with non-overlapping file ranges (see section 3).

### Gaps
- Exact arrow-rs/parquet crate version pinned by DataFusion 55.0/55.1 was not stated in the release blog; I did not verify whether it is 58.x or 59.x (52.0.0 referenced arrow 57.2.0 docs — [52.0.0](https://datafusion.apache.org/blog/2026/01/12/datafusion-52.0.0)). Check `Cargo.toml` of the 55.1.0 tag.
- No public end-to-end ClickBench/TPC-H numbers comparing `pushdown_filters` on vs off for 55.x were found beyond the April 2026 PR snippets.
- The dynamic-filters blog is internally inconsistent on whether dynamic filters skip rows without late materialization (footnote vs body) — treat "row-level requires pushdown_filters" as the safe assumption.

## 2. Metadata: footer caching, statistics/list caches, size hint, metadata decoder, object store I/O

### Takeaway
Since DataFusion 50 the parsed Parquet metadata (incl. page index) is cached by default (50 MB LRU), since 52 file statistics and list-files are cached too, and since arrow-rs 57 the footer thrift parser is ~3–9x faster. On object storage the dominant cost is request latency (tens–hundreds of ms each), so the best practice is: avoid footer reads entirely via an external catalog/index, prefetch the footer with a single ranged GET (`metadata_size_hint`, default 512 KiB), and let the reader coalesce nearby ranges (object_store: 1 MiB gap, 10 parallel).

### Cited Findings
- `datafusion.execution.parquet.metadata_size_hint` = 524288 bytes (512 KiB fetched optimistically from file end) — [configs](https://datafusion.apache.org/user-guide/configs.html)
- Runtime cache configs: `datafusion.runtime.metadata_cache_limit` = 50M; `datafusion.runtime.file_statistics_cache_limit` = 20M; `datafusion.runtime.list_files_cache_limit` = 1M; `datafusion.runtime.list_files_cache_ttl` = NULL. `datafusion.execution.meta_fetch_concurrency` = 32 (files read in parallel for schema/statistics inference) — [configs](https://datafusion.apache.org/user-guide/configs.html)
- 50.0.0 metadata cache: caches Parquet metadata (statistics, page indexes) for built-in ListingTable, LRU up to 50 MB, `metadata_cache_limit=0` disables, invalidated when the file changes (Epic #17000). ~12x on a point-read microbenchmark (PR #16971); EXPLAIN ANALYZE on a 100M-row file showed metadata load ~229 ms → 228 µs (~82x) — [DataFusion 50.0.0](https://datafusion.apache.org/blog/2025/09/29/datafusion-50.0.0)
- 52.0.0 statistics cache (inspect with `statistics_cache()` in datafusion-cli; columns path, file_modified, e_tag, num_rows, table_size_bytes, statistics_size_bytes; PRs #18971, #19054) and prefix-aware list-files cache that avoids extra LIST calls for Hive-partition predicates (`list_files_cache()`; PRs #18146, #18855, #19366, #19298) — [DataFusion 52.0.0](https://datafusion.apache.org/blog/2026/01/12/datafusion-52.0.0)
- 55.0.0: skip page-index load and `ParquetMetaData` clone when a file has none (PR #24150): ClickBench single-file Q1 38% faster — [DataFusion 55.0.0](https://datafusion.apache.org/blog/2026/08/25/datafusion-55.0.0)
- arrow-rs 57.0.0 (Oct 2025) custom thrift metadata parser: "almost 4x faster" than the `thrift`-crate parser; blog reports 3x–9x; benefits low latency and files with many row groups/columns — [arrow-rs 57.0.0](https://arrow.apache.org/blog/2025/10/30/arrow-rs-57.0.0/); [3x-9x Faster Parquet Footer Metadata (2025-10-23)](https://arrow.apache.org/blog/2025/10/23/rust-parquet-metadata/)
- Parquet dev list: ~7x from a more efficient thrift decoder that can also skip statistics; a FlatBuffers footer is still only a proposal — [dev@parquet thread](https://www.mail-archive.com/dev@parquet.apache.org/msg26675.html), [msg26769](https://mail-archive.com/dev@parquet.apache.org/msg26769.html)
- Object store request coalescing: `OBJECT_STORE_COALESCE_DEFAULT` = 1 MiB (ranges with gap ≤1 MiB merged into one GET), `OBJECT_STORE_COALESCE_PARALLEL` = 10 (buffered concurrent fetches); iceberg-rust's reader copies these defaults (gap 1 MiB, concurrency 10) — [object_store util.rs 0.14.2](https://docs.rs/crate/object_store/0.14.2/source/src/util.rs); [iceberg-rust arrow reader](https://rust.iceberg.apache.org/api/src/iceberg/arrow/reader/mod.rs.html)
- Each request to S3/GCS/Azure adds "tens to hundreds of milliseconds"; skipping per-file footer reads matters most on object storage; one contributor got >2x by rewriting files with Offset Indexes and no compression — [External indexes (2025-08-15)](https://datafusion.apache.org/blog/2025/08/15/external-parquet-indexes)
- 55.0.0: fewer object store requests for CSV (70% faster TPC-H CSV with simulated latency, PR #22962) — shows the project now benchmarks with simulated object-store latency — [DataFusion 55.0.0](https://datafusion.apache.org/blog/2026/08/25/datafusion-55.0.0)
- `datafusion.execution.objectstore_writer_buffer_size` = 10485760 (10 MiB multipart upload chunk) — [configs](https://datafusion.apache.org/user-guide/configs.html)

### Inferences
- The built-in caches only help a long-lived `SessionContext`/`RuntimeEnv`; ORE should share one `RuntimeEnv` (and its `CacheManager`) per process across queries, otherwise every query re-reads footers. 50 MB metadata cache is small for a multi-tenant lake; size `metadata_cache_limit` to the hot working set (page indexes of wide tables are large).
- With the default 512 KiB hint, most footers are fetched in one GET; for very wide tables/many row groups (footers >512 KiB) raise the hint or, better, store footer length/metadata in the ORE catalog.
- On GCS/R2, latency dominates: fewer, larger range requests (coalescing) and high concurrency across files (`target_partitions`, morsel scans) beat per-request tuning.

### Gaps
- Did not verify whether `ParquetObjectReader::get_ranges` (parquet crate async reader) delegates to `object_store::coalesce_ranges` in parquet 58/59, nor its concurrency value.
- No published GCS-vs-S3-vs-R2 latency/throughput measurements for DataFusion specifically were found.
- No source found on the arrow-rs `ParquetPushDecoder` / "push decoder" status in 58–59 (not mentioned in the 57.0.0 notes).

## 3. Write path: row groups, pages, compression, encodings, statistics, sorting, parallelism, Iceberg

### Takeaway
DataFusion 55's writer defaults are reasonable (zstd(3), dictionary on, page-level statistics, 1M-row row groups, 1 MiB pages, 20k-row page limit, parallel column serialization) but bloom filters on write are off and writer_version is 1.0. The highest-leverage write decisions for read performance are: sort/cluster on the hot predicate key with non-overlapping file ranges, keep page-level statistics (enables page index pruning), add bloom filters only for high-cardinality equality columns, and for Iceberg ensure `PARQUET:field_id` is set on every Arrow field.

### Cited Findings
- DataFusion write defaults — [configs](https://datafusion.apache.org/user-guide/configs.html):
  - `compression` = `zstd(3)`; `dictionary_enabled` = `true`; `dictionary_page_size_limit` = 1048576.
  - `statistics_enabled` = `page` (none/chunk/page); `column_index_truncate_length` = 64; `statistics_truncate_length` = 64.
  - `max_row_group_size` = 1048576 rows; `max_row_group_bytes` = NULL (new byte-based target).
  - `data_pagesize_limit` = 1048576; `data_page_row_count_limit` = 20000; `write_batch_size` = 1024.
  - `writer_version` = `1.0` ("1.0" or "2.0"); `encoding` = NULL.
  - `bloom_filter_on_write` = `false`; `bloom_filter_fpp` / `bloom_filter_ndv` = NULL.
  - `write_row_group_number_distinct_values` = `false` (write per-column NDV into row-group stats).
  - `skip_arrow_metadata` = `false`.
  - `allow_single_file_parallelism` = `true`; `maximum_parallel_row_group_writers` = 1; `maximum_buffered_record_batches_per_stream` = 2.
  - Experimental content-defined chunking: `content_defined_chunking.enabled` = `false`, `min_chunk_size` = 262144, `max_chunk_size` = 1048576, `norm_level` = 0 (added in 54.0.0, PR #21110, aligns page boundaries to content for dedup/incremental storage — [DataFusion 54.0.0](https://datafusion.apache.org/blog/2026/06/12/datafusion-54.0.0)).
- 54.0.0 extracts NDV from Parquet metadata for equality-filter selectivity (PRs #19957, #20789, #21077 etc.) — so writing NDV (`write_row_group_number_distinct_values`) now feeds the planner — [DataFusion 54.0.0](https://datafusion.apache.org/blog/2026/06/12/datafusion-54.0.0)
- Iceberg 1.12.0 table-property defaults (Java reference writer): `write.parquet.row-group-size-bytes` 134217728 (128 MB); `write.parquet.page-size-bytes` 1048576; `write.parquet.page-row-limit` 20000; `write.parquet.dict-size-bytes` 2097152; `write.parquet.compression-codec` zstd; `write.parquet.compression-level` null; `write.parquet.bloom-filter-enabled.column.<col>` not set; `write.parquet.bloom-filter-fpp.column.<col>` 0.01; `write.parquet.bloom-filter-max-bytes` 1048576; `write.target-file-size-bytes` 536870912 (512 MB); `write.metadata.metrics.default` truncate(16); `read.split.target-size` 134217728 — [Iceberg configuration](https://iceberg.apache.org/docs/latest/configuration/)
- Iceberg field ids: arrow-rs maps Arrow `Field` metadata key `PARQUET:field_id` (`parquet::arrow::PARQUET_FIELD_ID_META_KEY`) to the Parquet field id; without it the id is omitted; Iceberg requires field ids (issue #4702). Read-back loss of ids for files without embedded Arrow schema fixed in PR #4878. Open PR #9470 proposes a `Field::id()` API instead of the metadata key — [docs.rs PARQUET_FIELD_ID_META_KEY](https://docs.rs/parquet/latest/parquet/arrow/constant.PARQUET_FIELD_ID_META_KEY.html); [arrow-rs #4702](https://github.com/apache/arrow-rs/issues/4702); [arrow-rs #4877](https://github.com/apache/arrow-rs/issues/4877); [arrow-rs PR #9470](https://github.com/apache/arrow-rs/pull/9470)
- Layout guidance from the sort-pushdown post: write files sorted on the most-queried key and declare the ordering (`WITH ORDER` / `sorting_columns`); keep min/max ranges of files and row groups disjoint; make the sort key the leading column (row-group pruning depends on the first key); avoid low-cardinality or unsorted leading columns — [Sort Pushdown (2026-07-20)](https://datafusion.apache.org/blog/2026/07/20/sort-pushdown)
- Dynamic/TopK pruning works best on roughly time-ordered appends — [Dynamic Filters](https://datafusion.apache.org/blog/2025/09/10/dynamic-filters)
- ClickBench reference layout: partitioned 14 GB dataset in 100 files of ~140 MB — [ClickBench blog (2024-11-18)](https://datafusion.apache.org/blog/2024/11/18/datafusion-fastest-single-node-parquet-clickbench)

### Inferences
- DataFusion's 1,048,576-row row group vs Iceberg's 128 MB byte target: for wide/large rows a row-count cap can produce huge row groups (poor pruning granularity, high writer memory); for ORE use `max_row_group_bytes` (≈128 MB to match Iceberg readers/split planning) plus the row cap.
- Iceberg's 512 MB file target is tuned for Spark/HDFS throughput; for latency-sensitive DataFusion on GCS/R2 with the ClickBench reference of ~140 MB files, 128–512 MB is a reasonable band; smaller files multiply footer GETs unless the catalog serves metadata.
- `writer_version=2.0` (data page v2) is not the default; Vortex/DuckDB comparisons show Parquet V2 materially faster than V1 in DuckDB (see section 5), but no DataFusion-specific measurement was found — test before switching, and check Iceberg/Spark/Trino reader compatibility.
- zstd(3) default matches Iceberg's codec; lower levels (1) trade size for write speed; no DataFusion-specific level benchmark found.
- Bloom filters: enable per column only for high-cardinality equality/IN lookups (ids, keys); Iceberg's per-column property and 1 MB cap are a good template. DataFusion's `bloom_filter_on_write=true` applies to all columns (wasteful).
- Z-order/Hilbert: no DataFusion-native clustering writer was found; since the pruning stack is min/max based, a leading-key sort with disjoint ranges captures most gains; multi-dimensional curves help only for multi-column range predicates (relevant for geo: H3/Hilbert on geometry bbox).

### Gaps
- No measured DataFusion numbers found for zstd level vs snappy/lz4, data page v2, or `maximum_parallel_row_group_writers` > 1.
- No source on whether DataFusion's `COPY TO`/`DataSink` writes `PARQUET:field_id` automatically for Iceberg (it depends on the Arrow schema passed in; iceberg-rust's writer path should set them — not verified in this session).
- No source found on Z-order/Hilbert support in DataFusion or iceberg-rust compaction as of Oct 2026.

## 4. ClickBench / TPC-H: what made DataFusion fast on Parquet

### Takeaway
DataFusion topped single-node ClickBench on Parquet in Nov 2024 (v43) through StringView, a mature pruning reader and aggregation work; in Aug 2026 (v55) it is again top for partitioned Parquet on c7a.metal-48xlarge (combined metric) and second on c6a.4xlarge. Recent Parquet-relevant gains come from morsel scans (54), TopK/sort pushdown (52–55), metadata caching (50) and the faster footer parser (arrow-rs 57).

### Cited Findings
- Nov 2024: fastest single-node engine on Parquet in ClickBench "hot" (c6a.4xlarge, partitioned 14 GB, 100 files ~140 MB); DuckDB slightly faster on "cold". Credited: StringView ("performance improved for all queries"), Parquet reader features (projection pushdown, predicate pushdown via row-group metadata, page index, bloom filters, limit pushdown, parallel reading, interleaved I/O), skipping partial aggregation (PR 11627), columnar multi-column grouping (43.0.0). Late materialization was "coming soon"/not default. ClickBench improved >30% from v34 (Dec 2023) to v43 (Nov 2024) — [DataFusion fastest on Parquet (2024-11-18)](https://datafusion.apache.org/blog/2024/11/18/datafusion-fastest-single-node-parquet-clickbench)
- Aug 2026 (55.0.0): top of ClickBench for partitioned Parquet (combined metric) on c7a.metal-48xlarge; second on c6a.4xlarge — [DataFusion 55.0.0](https://datafusion.apache.org/blog/2026/08/25/datafusion-55.0.0)
- 54.0.0 morsel-driven scans: up to ~2x on skewed ClickBench scans — [54.0.0](https://datafusion.apache.org/blog/2026/06/12/datafusion-54.0.0)
- Related blog series: "Parquet Pruning in DataFusion: Read Only What Matters" (2025-03-20), "Efficient Filter Pushdown in Parquet" (2025-03-21), "Embedding User-Defined Indexes in Apache Parquet Files" (2025-07-14), "Using External Indexes, Metadata Stores, Catalogs and Caches..." (2025-08-15), "Dynamic Filters... 25x" (2025-09-10), "Turning LIMIT into an I/O Optimization" (2026-03-20), "Sort Pushdown" (2026-07-20) — [DataFusion blog index](https://datafusion.apache.org/blog/)

### Inferences
- ClickBench results are on local NVMe and "hot" runs, so they say little about object-storage latency; ORE should benchmark with object-store latency (the project itself now simulates latency, see PR #22962).

### Gaps
- Exact ClickBench timings for 55.x and the settings used in the official ClickBench DataFusion entry (e.g. whether `pushdown_filters` is enabled there) were not verified.
- I did not fetch the "Parquet Pruning in DataFusion" (2025-03-20) or "Querying Parquet with Millisecond Latency" (InfluxData/arrow blog, 2022) posts in this session; numbers from them are not included.

## 5. External indexes, catalogs and alternative formats

### Takeaway
DataFusion is explicitly designed for a catalog-driven read path: a custom `TableProvider` prunes files from an external index, a per-file `ParquetAccessPlan` (attached via `PartitionedFile::with_extensions`) skips row groups/rows, and a custom `ParquetFileReaderFactory` serves cached `ParquetMetaData` so no footer GET happens. This maps directly onto ORE's transactional catalog (0079). Newer formats (Vortex, Lance) beat Parquet only modestly (≈10–35% on TPC-H in DuckDB; <2x on ClickBench per the DataFusion team).

### Cited Findings
- APIs: custom `TableProvider` (`supports_filter_pushdown`, `scan`) returns only matching files; helpers `PruningPredicate`, `ExprSimplifier`, interval analysis; `ParquetAccessPlan::new_all(n)`, `.skip(i)`, `.scan_selection(i, RowSelection)` with `RowSelector::skip/select`; DataFusion still refines the plan with built-in statistics if the predicate is set on `ParquetSource`; metadata via `ParquetFileReaderFactory::create_reader` attached with `ParquetSource::with_parquet_file_reader_factory`. Examples: `examples/data_io/parquet_index.rs`, `examples/advanced_parquet_index.rs`. External stores suggested: separate JSON/Parquet files, PostgreSQL, Redis/Cassandra, in-memory maps — [External indexes (2025-08-15)](https://datafusion.apache.org/blog/2025/08/15/external-parquet-indexes)
- On ClickBench, custom formats differ from Parquet by less than 2x; recommendation is to use Parquet + DataFusion as building blocks — [External indexes](https://datafusion.apache.org/blog/2025/08/15/external-parquet-indexes)
- 52.0.0 `PhysicalExprAdapter` replaces `SchemaAdapter`, allowing per-file-schema predicate rewriting (relevant for schema evolution / Variant shredding; PRs #18998, #19345) — [52.0.0](https://datafusion.apache.org/blog/2026/01/12/datafusion-52.0.0)
- Vortex vs Parquet (DuckDB, TPC-H SF100, Jan 2026): Vortex 18% faster than Parquet V2 and 35% faster than Parquet V1 (geomean); historically 12–18% geomean, 8–14% total vs Parquet v2 — [DuckDB Vortex extension (2026-01-23)](https://duckdb.org/2026/01/23/duckdb-vortex-extension.html)
- ClickBench has a DataFusion-based Vortex entry (vortex-datafusion-cli tag 0.70.0-53.1.0) converting each partitioned Parquet file to one Vortex file — [ClickBench PR #749](https://github.com/ClickHouse/ClickBench/pull/749/changes); Vortex bench suite supports DataFusion/DuckDB across Parquet, Vortex, Lance — [Vortex benchmarking docs](https://docs.vortex.dev/developer-guide/benchmarking)

### Inferences
- For ORE: store per-file (and optionally per-row-group) min/max, row counts, footer offset/length and even serialized `ParquetMetaData` in the Postgres catalog; prune files in the catalog query, then hand DataFusion `PartitionedFile`s with `ParquetAccessPlan` extensions and a `ParquetFileReaderFactory` that returns cached metadata. This removes the per-file footer round-trip, which dominates on GCS/R2.
- Iceberg manifests already carry per-file column bounds (`write.metadata.metrics.default` truncate(16)); an ORE catalog can mirror those for SQL-side file pruning.
- Switching format is not justified by available numbers; the DuckDB data suggests Parquet V2 closes much of the gap.

### Gaps
- No published DataFusion-on-Vortex vs DataFusion-on-Parquet ClickBench table found; Lance+DataFusion numbers not found.
- Did not retrieve "An Empirical Evaluation of Columnar Storage Formats" (VLDB 2024) findings in this session.

## 6. Known pitfalls and open issues

### Takeaway
The main pitfalls are: forgetting that `pushdown_filters` is off (dynamic filters then only prune at file/row-group granularity), regressions when it is on for some shapes, caches that only help a shared long-lived runtime, row-count-based row groups that ignore row width, and missing Iceberg field ids.

### Cited Findings
- `pushdown_filters` default still false; enabling showed −13.3% on one benchmark shape (all-Map, 128 probes/row) in April 2026 — [PR #21931](https://github.meowingcats01.workers.dev/apache/datafusion/pull/21931); [configs](https://datafusion.apache.org/user-guide/configs.html)
- Predicate cache default 100 MB at the arrow-rs level; memory vs I/O tradeoff — [docs.rs ParquetOptions 55.0.0](https://docs.rs/datafusion/55.0.0/datafusion/config/struct.ParquetOptions.html)
- Bitmask selection + page pruning interaction still forces RLE fallback (arrow-rs #8845) — [Late materialization deep dive](https://arrow.apache.org/blog/2025/12/11/parquet-late-materialization-deep-dive/)
- Hash-join dynamic filters are weak for many distinct/skewed keys (min/max) and originally INNER only; TopK pruning needs roughly ordered data — [Dynamic Filters](https://datafusion.apache.org/blog/2025/09/10/dynamic-filters)
- ClickBench `hits_partitioned` stores strings as Binary and needs `binary_as_string=true` — a reminder that writers should emit UTF8 logical type for strings — [Dynamic Filters](https://datafusion.apache.org/blog/2025/09/10/dynamic-filters)
- `max_in_list_size` = 20 caps IN-list pruning — [configs](https://datafusion.apache.org/user-guide/configs.html)
- Field-id loss when rewriting files without embedded Arrow schema (fixed PR #4878) — [arrow-rs #4877](https://github.com/apache/arrow-rs/issues/4877)
- 53.0.0 post-release issues tracked in #21079 — [53.0.0](https://datafusion.apache.org/blog/2026/04/02/datafusion-53.0.0)

### Inferences
- Benchmark ORE queries with `pushdown_filters` on and off per workload before adopting; consider enabling it per session for selective point/range queries (Query/Preview) and leaving off for full scans (Builds).

### Gaps
- Did not survey the full open-issue list in apache/datafusion and arrow-rs for Parquet-on-object-store performance (e.g. GCS-specific throttling, `object_store` retry/backoff behaviour).
