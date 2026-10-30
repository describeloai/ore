# DataFusion in production on Parquet/Iceberg lakes: case studies and lessons learned (state as of October 2026)

Scope note: research done 2026-10-30, 17 tool calls. Vendor claims are marked as such. Items I could not verify this session are in each section's Gaps, not in Cited Findings.

## Case studies: who runs DataFusion at scale, and how they built it

### Takeaway
The best-documented production deployments on object storage in 2025–2026 are Datadog (Distributed DataFusion, interactive and low-latency), Cloudflare R2 SQL (serverless and distributed over Iceberg, with DataFusion on each worker), InfluxDB 3 (the FDAP stack, with querier caches), Spice.ai (federation plus local acceleration), ParadeDB (DataFusion inside Postgres custom scans) and Embucket (Snowflake-compatible, on Lambda plus Iceberg). The common pattern is to own the catalog, planning and pruning layer and use DataFusion as the per-node executor.

### Cited Findings

**The official "Known Users" list (current)**
- The active list includes Arroyo, ArkFlow, Auron, Ballista, CnosDB, Comet, Cube Store, dbt Fusion engine, delta-rs, EDB Postgres Lakehouse (built with Seafowl), Feldera, Funnel, GlareDB, GreptimeDB, HoraeDB, Hotdata, iceberg-rust, IceGate (an observability lake on Iceberg), InfluxDB, Kamu, LakeSoul, Lance, OpenObserve, ParadeDB, Parseable, Massive.com (stock market API), **R2 Query Engine (Cloudflare's distributed engine for querying Iceberg catalogs)**, rerun.io, Restate, ROAPI, Sail, **SedonaDB (single-node analytical DB with geospatial as a core feature)**, Sleeper, Spice.ai, Supermetal, Synnada, VegaFusion, Vortex, Xorq, KalamDB (multi-tenant SaaS state DB) and Infino (SQL, full-text and vector search over Parquet on object storage). The less-active list includes Dask SQL, Cloudfuse Buzz, Exon and Tensorbase. The subprojects are Ballista, Comet, DataFusion Java and DataFusion Python — [DataFusion user guide, Known Users](https://datafusion.apache.org/user-guide/introduction.html)
- Datadog, Coralogix, Apple (as a company) and Databend are not named in the Known Users list. Apple's involvement is through Comet, a subproject — [Known Users](https://datafusion.apache.org/user-guide/introduction.html)

**Datadog: Distributed DataFusion (blog post, 2026-10-01)**
- Datadog chose DataFusion as "one composable query engine" to replace several specialised query systems. It had already standardised on Arrow (in memory), Substrait (plan format) and DataFusion. Single-machine execution could not meet the latency needs of very large scans — [Datadog Engineering, Oct 1 2026](https://www.datadoghq.com/blog/engineering/distributed-datafusion/)
- Alternatives it rejected: ClickHouse (not extensible enough); Spark and **Ballista** ("rely on intermediate data materialization", a poor fit for interactive workloads); Trino (integrates poorly with Arrow and Substrait, and costly at Datadog scale) — [Datadog](https://www.datadoghq.com/blog/engineering/distributed-datafusion/)
- Architecture: it works only on the physical plan, leaving SQL/Substrait parsing and logical planning untouched. Stages are separated by network boundaries, and a task is a partition running on one worker. Operators are `DistributedLeafExec`, `NetworkCoalesceExec` and `NetworkShuffle`, and data moves over **Arrow Flight** streams. Knobs include `distributed.max_tasks_per_stage` and `distributed.file_scan_config_bytes_per_partition`. Aggregations run as partial, then shuffle by key, then final. Joins are partitioned or broadcast, with tree-reduce aggregations — [Datadog](https://www.datadoghq.com/blog/engineering/distributed-datafusion/)
- Numbers: a benchmark on 12 × c5n.2xlarge (8 vCPU, 21 GiB, 25 Gbps) reading Parquet from S3, TPC-H SF1–100 and TPC-DS, against Ballista, Spark and Trino (normalised results only, no absolute times). In production, "some long-running heavy queries complete up to 10× faster at the same total cost", and latency stays roughly flat as data grows — [Datadog](https://www.datadoghq.com/blog/engineering/distributed-datafusion/)
- Lessons: (1) **don't distribute unnecessarily**, because the planner keeps light queries on a single worker; (2) keep distribution thin (Flight, a minimal coordination layer, continuous benchmarking); (3) **"build a framework, not a service"**, meaning a library teams adapt. Extension points include custom sources and nodes, propagating config to workers, and **routing partitions to specific workers for cache affinity**. Next steps are adaptive query execution (some sources lack reliable statistics) and multi-GPU memory pooling — [Datadog](https://www.datadoghq.com/blog/engineering/distributed-datafusion/)
- Production users named in the governance discussion (issue #784, opened 2026-10-20): Datadog (originator, most contributors), **ParadeDB** ("quite committed"), **Basin SQL**, **Adobe**, with others "considering" — [datafusion-distributed #784](https://github.com/datafusion-contrib/datafusion-distributed/issues/784)
- Conflict: Datadog says it is in production, but the repo README has said "under construction / not yet ready for production use" (possibly stale) — [search summary of repo README](https://github.com/DataDog/datafusion-distributed); vs [Datadog blog](https://www.datadoghq.com/blog/engineering/distributed-datafusion/)

**Cloudflare R2 SQL (blog post, 2025-09-25; open beta then)**
- A serverless engine over Iceberg tables in R2, split into planner and distributed execution. The planner prunes top-down: catalog → snapshot → manifest list (partition stats) → manifests (column min/max, null counts) → Parquet footers (row groups) — [Cloudflare blog](https://blog.cloudflare.com/r2-sql-deep-dive/)
- **Streaming planning**: work units are emitted as they are found. Files are ordered by stats and the `ORDER BY`. For `ORDER BY … LIMIT` it keeps a top-K heap plus a high-water mark and stops once no remaining file can beat the heap. Ordering was limited to partition-key columns — [Cloudflare](https://blog.cloudflare.com/r2-sql-deep-dive/)
- Coordinator and worker: the receiving server plans and coordinates, and picks healthy workers through an internal API. Each worker gets a batch of row groups, the SQL and **serialized Parquet metadata (byte offsets), so workers never re-read footers from R2**. One row group maps to one DataFusion partition. Results return as Arrow IPC inside gRPC (not Flight). Complex distributed aggregations were future work — [Cloudflare](https://blog.cloudflare.com/r2-sql-deep-dive/)

**InfluxDB 3 (IOx), the "FDAP" stack (Flight, DataFusion, Arrow, Parquet)**
- InfluxData built an InfluxQL frontend and custom operators (deduplication of late-arriving data, gap filling, Parquet merge) on DataFusion's extension points — [InfluxData: why we built on DataFusion](https://www.influxdata.com/resources/why-we-built-influxdb-3s-query-engine-on-apache-datafusion/); [FDAP blog](https://influxdata.com/blog/flight-datafusion-arrow-parquet-fdap-architecture-influxdb)
- The querier reads Parquet from object storage through a **metadata cache plus a data cache**, prunes with DataFusion's `PruningPredicate`, and gets not-yet-persisted data from ingesters over Arrow Flight — [A. Lamb slides, "Building InfluxDB 3.0"](https://docs.google.com/presentation/d/12kdYHLyH79B5__9xs3de_hZyG9geW4jC3vUpiy39VA0/htmlpresent)
- It offers knobs for keeping data in a RAM/SSD cache versus object storage (with higher latency) — [Datanami 2024-03](https://www.datanami.com/2024/03/15/how-the-fdap-stack-gives-influxdb-3-0-real-time-speed-efficiency) (older, 2024)

**Spice.ai (vendor sources)**
- Acceleration is a DataFusion `TableProvider` wrapping two providers, a federated one (the source) and a local accelerator. Refresh runs DataFusion queries against the source and inserts the results locally. Iceberg is a first-class source (10-minute refresh cycle in one example), and hot datasets are accelerated locally. Accelerators include DuckDB and "Spice Cayenne" (Vortex-based, beta in v1.11). The Iceberg connector is labelled Beta, and writes (INSERT INTO) arrived in v1.8 — [Spice: how we use DataFusion](https://spice.ai/blog/how-we-use-apache-datafusion-at-spice-ai); [Spice: Iceberg at Spice](https://spice.ai/blog/apache-iceberg-at-spice-ai)

**ParadeDB**
- pg_search's JoinScan (0.25.x) "intercepts PostgreSQL join planning and replaces the standard executor with a DataFusion-based pipeline" over Tantivy columnar fast fields. Aggregate pushdown across joins was single-threaded in one version — [pg_search JoinScan README 0.25.5](https://pgxn.org/dist/pg_search/0.25.5/pg_search/src/postgres/customscan/joinscan/README.html); [ParadeDB joins docs](https://www.paradedb.com/docs/documentation/joins/overview.md)
- ParadeDB's pluggable spill backend spills into the Postgres buffer pool (cited in the DF 55 release) — [DataFusion 55.0.0 blog](https://datafusion.apache.org/blog/2026/08/25/datafusion-55.0.0)

**Embucket**
- A Snowflake-compatible engine (Snowflake v1 wire REST API and Snowflake SQL, plus a dbt adapter) on DataFusion and iceberg-rust. It runs as an **AWS Lambda function** with S3 Tables (Iceberg), uses DynamoDB for state, and exposes an Iceberg REST catalog. I found no evidence of its production maturity — [Embucket repo](https://github.com/embucket/embucket); [embucket-snowplow](https://github.com/Embucket/embucket-snowplow)

**Comet (Apple-originated Spark accelerator)**
- Comet 1.0.0 was released 2026-08-07 and 1.1.0 on 2026-10-01. AWS published a case study of running Comet on EKS (2026-06-03) — [DataFusion blog index](https://datafusion.apache.org/blog/)
- In a production Comet query, nested-Parquet pruning in DF 55 cut reads from 1.35 TB to 30.9 GB — [DF 55.0.0](https://datafusion.apache.org/blog/2026/08/25/datafusion-55.0.0)

**DuckLake on DataFusion (2026-07-29)**
- A DuckLake implementation for DataFusion supports several databases as catalog backends. The authors note the design suits multi-tenant deployments, or keeping many logical lakehouses in one DB — [ducklake.select](https://ducklake.select/2026/07/29/bringing-ducklake-to-datafusion/)

**Coralogix**
- I found no primary source tying Coralogix to DataFusion. What I did find: a metastore move from Postgres to ScyllaDB (30 s → 86 ms, for blocks, bloom filters and column metadata) and a Rust metrics engine — [HackerNoon/ScyllaDB case](https://hackernoon.com/lite/from-postgres-to-scylladb-how-coralogix-achieved-349x-faster-queries); [daily.dev summary](https://daily.dev/posts/introducing-the-new-coralogix-metrics-engine-52kopvnwy)

### Inferences
- Cloudflare R2 SQL is the closest public analogue to ORE (serverless, Iceberg on object storage, multi-tenant by nature). Its lessons are to own planning, stream it, ship pre-parsed Parquet metadata to workers and keep DataFusion as a per-worker executor. Note it is Arrow IPC over gRPC, not Flight or Ballista.
- ORE (an Iceberg catalog in Postgres) could use the DuckLake-on-DataFusion pattern (a catalog in a relational DB) as a template for a custom `TableProvider` / `CatalogProvider` backed by Postgres.
- Datadog's "framework, not a service" and "don't distribute unnecessarily" both argue for single-node-first in ORE, with a distributed path only for scans above a threshold.

### Gaps
- InfluxDB 3: no quantified lessons learned (cache hit ratios, querier memory limits) found this session. Only slides and overview blogs from 2023–2024.
- Not verified this session (no source fetched): Apple's Comet production numbers; Cube Store architecture; LanceDB/Lance; GreptimeDB; Polygon.io (the Known Users list now shows "Massive.com, Stock market API" — possibly a rebrand, unverified); LakeSail/Sail; GlareDB (status uncertain); Seafowl/Splitgraph (acquired by EDB per the "EDB Postgres Lakehouse, built with Seafowl" entry, details unverified); Arroyo; OpenObserve; Denormalized; Kamu; Vortex/SpiralDB; Synnada; Databend (does not appear in the list; historically it used arrow2 rather than DataFusion — unverified); Dremio (no DataFusion relationship found).
- Datadog does not name which products run on Distributed DataFusion, and the blog gives no absolute latencies.

## Multi-tenant serving patterns (sessions, caching, cold start, admission, memory limits, cancellation)

### Takeaway
DataFusion supplies the primitives: memory pools (Greedy/Fair/TrackConsumers), pluggable spill backends (55.0), a Parquet metadata cache in ListingTable (50.0), cooperative async cancellation and custom `ParquetFileReaderFactory` for caches. Multi-tenancy, admission and data caching are built by each embedder. The prominent data-cache project is LiquidCache, a pushdown cache from academic work published at VLDB 2025.

### Cited Findings
- **Metadata caching**: implement `ParquetFileReaderFactory::create_reader` returning a reader with cached metadata, wired through `ParquetSource::with_parquet_file_reader_factory` in `TableProvider::scan`. `ListingTable` caches Parquet metadata from DataFusion 50.0.0. Each S3/GCS/Azure request adds "tens to hundreds of milliseconds" — [DF blog: external indexes, metadata stores, catalogs and caches, 2025-08-15](https://datafusion.apache.org/blog/2025/08/15/external-parquet-indexes)
- **External indexes**: a custom `TableProvider` (`supports_filters_pushdown` plus `scan`) picks files from an index (in Postgres, Redis or JSON/Parquet sidecars) and builds a `ParquetAccessPlan` per file with a `RowSelection` for row-group and page pruning. Helpers are `PruningPredicate`, `ExprSimplifier` and interval analysis. Rewriting Parquet with offset indexes and no compression gave more than 2× faster reads in one user's case — [same blog](https://datafusion.apache.org/blog/2025/08/15/external-parquet-indexes)
- **LiquidCache**: a pushdown cache between DataFusion compute and S3. It evaluates filters, projections and aggregations on the cache server and stores data transcoded into its own "Liquid" format. The paper reports up to 10× less cache CPU and network traffic cut by two orders of magnitude. Many DataFusion nodes can share one cache server, and there is an in-process mode. It is "not a transparent cache — consider Foyer instead" — [VLDB vol 18 paper](https://www.vldb.org/pvldb/vol18/p5662-hao.pdf); [docs.rs liquid-cache](https://docs.rs/liquid-cache/latest/liquid_cache)
- **Cache affinity in distributed execution**: Distributed DataFusion supports "routing partitions to specific workers for cache affinity" — [Datadog](https://www.datadoghq.com/blog/engineering/distributed-datafusion/)
- **Avoiding footer re-reads**: R2 SQL ships serialized Parquet metadata from coordinator to workers — [Cloudflare](https://blog.cloudflare.com/r2-sql-deep-dive/)
- **Memory pools**: GreedyMemoryPool is the default. FairSpillPool divides the spillable share among spillable consumers. TrackConsumersPool wraps either and reports top consumers in the error message (secondary source) — [iThome write-up](https://ithelp.ithome.com.tw/articles/10407082)
- **Spill to custom storage (55.0)**: the `SpillFile` trait and `TempFileFactory` route spills to host storage (S3, the Postgres buffer pool). Larger-than-memory aggregation was refactored. On a failed spill-merge reservation, the largest stream is re-spilled in smaller batches. Merge fan-in is capped, and a lost-wakeup bug in the spill pool was fixed — [DF 55.0.0](https://datafusion.apache.org/blog/2026/08/25/datafusion-55.0.0)
- **Cancellation**: the DF blog post "Using Rust async for Query Execution and Cancelling Long-Running Queries" (2025-06-30) — [DF blog index](https://datafusion.apache.org/blog/)
- **Point queries**: "Best Practices for Single Row Lookups (Point Queries)" (2026-10-24) — [DF blog index](https://datafusion.apache.org/blog/)

### Inferences
- For ORE's per-tenant sessions: one `RuntimeEnv` per tenant (or a shared one with a per-query `MemoryPool` wrapped in TrackConsumers) gives memory isolation. Admission control must sit above DataFusion, since DataFusion has no admission layer of its own.
- ORE already holds Iceberg manifest and file stats in Postgres (the 0079 catalog). Following the external-index pattern, it could prune files from Postgres and pass `ParquetAccessPlan`s, skipping manifest reads from GCS/R2 entirely.
- With 55.0's pluggable spill backend, spill no longer has to go to local disk. That matters for serverless pods with small ephemeral storage.

### Gaps
- I found no public write-up of per-tenant session pooling or cold-start numbers for DataFusion in a multi-tenant SaaS (R2 SQL doesn't cover caching or multi-tenancy).
- Foyer usage in a DataFusion production stack: not verified this session.
- I did not fetch the cancellation blog post, so its details aren't summarised here.

## Distributed patterns in production (Ballista vs custom vs datafusion-distributed vs Ray)

### Takeaway
As of October 2026, production distributed DataFusion is mostly either custom-built (R2 SQL, InfluxDB, Spice) or on Datadog's datafusion-distributed library (Datadog, ParadeDB, Basin SQL, Adobe). Ballista is active (monthly-ish releases, now with a pipelined mode) and is positioned for batch ETL. datafusion-distributed is positioned for interactive, low-latency work. Governance of datafusion-distributed is under discussion (Oct 2026).

### Cited Findings
- Ballista releases: 53.0.0 (2026-05-24), 54.0.0 (2026-07-12), 54.1.0 (2026-08-09), 55.0.0 (2026-10-03). Ballista now tracks DataFusion's major version — [DF blog index](https://datafusion.apache.org/blog/)
- Positioning per the datafusion-distributed author: DFD is for interactive analytics (an alternative to Trino or ClickHouse), and Ballista is for batch ETL (an alternative to Spark SQL). Andy Grove: "Ballista is an engine, DFD is a library, not an engine". Ballista is adding a **pipelined mode**, while DFD supports only streaming exchanges — [issue #784](https://github.com/datafusion-contrib/datafusion-distributed/issues/784)
- **TPC-H SF1000 (reported by Andy Grove in #784, setup open to correction)**: pipelined Ballista 279 s, DFD 387 s, blocking Ballista 517 s — [issue #784](https://github.com/datafusion-contrib/datafusion-distributed/issues/784)
- Governance concerns: the top contributors are all from Datadog (alamb), and the "bus factor is effectively one" (Milenković). Options discussed are the ASF incubator, a subproject with its own release cycle, or a conditional donation. IP clearance is needed. No decision as of 2026-10-29 — [issue #784](https://github.com/datafusion-contrib/datafusion-distributed/issues/784)
- Datadog rejected Ballista because of intermediate materialization (before the pipelined mode) — [Datadog](https://www.datadoghq.com/blog/engineering/distributed-datafusion/)
- DF 55 added APIs aimed at distributed engines: dynamic-filter propagation, `FFI_QueryPlanner`, self-serializing plans and window accumulator state access — [DF 55.0.0](https://datafusion.apache.org/blog/2026/08/25/datafusion-55.0.0)
- R2 SQL uses a custom coordinator and workers over gRPC with Arrow IPC, and its first version targets filter queries, not distributed aggregations — [Cloudflare](https://blog.cloudflare.com/r2-sql-deep-dive/)
- A third-party 2026 overview still judged Spark or Trino safer for production distributed workloads (opinion) — [Medium, A. Madson](https://medium.com/@andymadson/apache-datafusion-what-data-engineers-need-to-know-in-2026-3ad96f7157f6)
- Iceberg integration moving: a vote to move the `iceberg-datafusion` crate from apache/iceberg-rust to a new **apache/datafusion-iceberg** repo (Sept 2026 dev-list roundup) — [Apache Data Lakehouse Weekly, Sept 9–17 2026](https://dev.to/alexmercedcoder/apache-data-lakehouse-weekly-september-9-to-17-2026-1509)

### Inferences
- For ORE's interactive Query path, DFD (streaming, a library you embed) fits better than Ballista. For Build/Transforms batch, Ballista's pipelined mode is competitive on the one reported benchmark. Choosing DFD carries governance risk until #784 resolves.
- Ray: I found no production DataFusion-on-Ray case in this session (DataFusion Ray is mentioned only as a less mature sibling in #784).

### Gaps
- Independent benchmarks of DFD against Ballista beyond Andy Grove's single SF1000 comment.
- How DFD discovers workers and ships plans (the Datadog blog doesn't cover it; docs not fetched).

## Upstream contributions, governance, release cadence, staying current

### Takeaway
DataFusion now ships a major version roughly every 2–3 months: 50 (Sept 2025), 51 (Nov 2025), 52 (Jan 2026), 53 (Apr 2026), 54 (Jun 2026) and 55 (Aug 2026, with 55.1 after). Each has breaking changes documented in the upgrade guide. Big users upstream generic work and keep their specific parts in extension points.

### Cited Findings
- Release dates: 50.0.0 2025-09-29, 51.0.0 2025-11-25, 52.0.0 2026-01-12, 53.0.0 2026-04-02, 54.0.0 2026-06-12, 55.0.0 2026-08-25 — [DF blog index](https://datafusion.apache.org/blog/)
- 55.0.0 had about 10 weeks of development, 877 commits and 175 contributors (a record). The post says "some breaking changes" and points to the upgrade guide — [DF 55.0.0](https://datafusion.apache.org/blog/2026/08/25/datafusion-55.0.0)
- The 55.1.0 vote passed with 9 +1 votes (6 binding), with Tim Saucer as release manager — [Apache Data Lakehouse Weekly, Aug 26–Sep 2 2026](https://dev.to/alexmercedcoder/apache-data-lakehouse-weekly-august-26-to-september-2-2026-40i1)
- InfluxData says its upstream contributions made DataFusion the fastest Parquet engine (vendor claim) — [InfluxData](https://www.influxdata.com/resources/why-we-built-influxdb-3s-query-engine-on-apache-datafusion/)
- DF 55 ranks first on ClickBench (partitioned Parquet) on c7a.metal-48xlarge and second on c6a.4xlarge — [DF 55.0.0](https://datafusion.apache.org/blog/2026/08/25/datafusion-55.0.0)
- Datadog maintains forks (`DataDog/datafusion`) alongside upstream work — [GitHub DataDog/datafusion](https://github.com/DataDog/datafusion)
- Embucket also maintains a fork with unmerged features (e.g. an `enable_hash_join_spill` option in a fork PR) — [Embucket/datafusion PR #45](https://github.com/Embucket/datafusion/pull/45/files)

### Inferences
- ORE should plan one upgrade every ~2–3 months. Pinning `datafusion`, `iceberg-datafusion` (soon `datafusion-iceberg`) and `datafusion-distributed` together is essential, because DFD and Ballista follow DataFusion's majors.
- Forks are common even among top users (Datadog, Embucket). The safer pattern is to rely on extension points (TableProvider, ExecutionPlan, optimizer rules, spill backends), not patches.

### Gaps
- I could not fetch the API health / deprecation policy page (404 at `library-user-guide/api-health.html`), so the exact deprecation window is unverified. The upgrade guide (`library-user-guide/upgrading.html`) was not fetched.
- PMC composition by company was not verified.

## Common pitfalls reported

### Takeaway
The recurring problems are memory blowups in non-spillable operators (above all the hash join build side), slow object-store requests per file, and single-machine limits on very large scans. 2025–2026 releases address spill (55.0), metadata caching (50.0) and pruning, but **hash join spilling is still not merged in mainline as far as the evidence shows**.

### Cited Findings
- Hash joins register as non-spillable (`can_spill: false`) and return "Resources exhausted". In one report, TPC-H Q14 failed with OOM at 8 GiB and passed at 16 GiB — [DataFusion PR/RFC thread #23979](https://github.com/apache/datafusion/pull/23979)
- Issue #17267 proposes hybrid hash-join spilling. The partitioned hash join's shared pool plus per-partition reservations makes coordinated spilling hard — [mail-archive of #17267](https://go.mail-archive.com/github@datafusion.apache.org/msg69132.html)
- An untracked join build side can make `SortExec` OOM, because the pool looks empty — [PR #13751](https://github.com/apache/datafusion/pull/13751/files)
- Comet users report OOM during native sort/merge — [Comet discussion #5001](https://github.com/apache/datafusion-comet/discussions/5001)
- Object-store latency: each request costs tens to hundreds of ms. Remedies are metadata caches, external indexes and ranged reads — [DF blog 2025-08-15](https://datafusion.apache.org/blog/2025/08/15/external-parquet-indexes); R2 SQL ships pre-parsed metadata to workers — [Cloudflare](https://blog.cloudflare.com/r2-sql-deep-dive/)
- Statistics quality: Datadog's next step is adaptive execution because "some data sources lack reliable statistics" — [Datadog](https://www.datadoghq.com/blog/engineering/distributed-datafusion/)
- Distribution overhead: light queries should stay single-node — [Datadog](https://www.datadoghq.com/blog/engineering/distributed-datafusion/)
- DF 55 lowered `repartition_file_min_size` from 10 MiB to 1 MiB, skips the page index when absent and stops reading unread nested leaves. It fixed a spill-pool lost-wakeup bug — [DF 55.0.0](https://datafusion.apache.org/blog/2026/08/25/datafusion-55.0.0)

### Inferences
- For ORE replacing DuckDB: DuckDB spills hash joins, but DataFusion (mainline, as far as the evidence shows) does not. Large join workloads therefore need generous per-query memory, sort-merge join, or a distributed partitioned join (DFD/Ballista) to cut per-node build size. Confirm the hash-join spill status in the current main before designing the Build path.
- Small files multiply per-file request latency. ORE's Iceberg compaction plus a footer/metadata cache (or stats in the Postgres catalog) are first-order concerns.

### Gaps
- No primary production write-up quantifies small-file overhead or plan-time costs for DataFusion on Iceberg.
- Whether the hash-join spill PR (#23979) or a hybrid join (#17267) merged after DF 55 is not confirmed.
