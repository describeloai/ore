# Distributed DataFusion: Ballista 55.x architecture, internals, production practice, and the alternatives (state as of 2026-10-30)

> Scope note on sources. The Ballista docs at `datafusion.apache.org/ballista/` are built from **`main`**. They already list an upgrade guide for **56.0.0**. So some features documented there (history server, Flight SQL frontend, result service, pipelined mode) may be post-55.0.0. Each item below says whether it is pinned to a release (release notes or blog) or only seen in the main-branch docs ("docs@main").
> The GitHub releases page shows "09 Aug" for releases 47–54.1. That looks like an artifact of how the page was captured, because the 54.0.0 blog is dated 2026-07-12. Trust the blog dates and the 55.0.0 date (08 Oct 2026) over those.

## 1. Architecture: scheduler, executors, job/stage/task, shuffle, scheduling, AQE, joins, slots

### Takeaway
Ballista is a standing cluster: one or more schedulers plus executors, connected over gRPC/protobuf, moving data as Arrow IPC/Flight. The physical plan is cut at repartition points into stages joined by `ShuffleWriterExec`/`ShuffleReaderExec`. Shuffle is **blocking and materialized to local disk**, and only sort-based shuffle remains. Since 55.0.0, AQE is on by default and push-staged scheduling is the default. Tasks pack several partitions up to the executor's free vcores.

### Cited Findings
**Roles and gRPC surface**
- The scheduler provides a gRPC service for submitting and managing jobs and a REST API for monitoring, and builds an execution graph per job. Executors connect to a scheduler and receive tasks as protobuf-serialized physical plans. Clients are the CLI, PyBallista and the Rust `ballista` crate — [Architecture](https://datafusion.apache.org/ballista/contributors-guide/architecture.html)
- Scheduler gRPC methods (`ballista.proto`): `ExecuteQuery`, `ExecuteQueryPush` (streams job status back to the client), `GetJobStatus`, `GetJobMetrics`, `CancelJob`, `RegisterExecutor`, `HeartBeatFromExecutor` (liveness and capacity), `PollWork` (pull-mode executors ask for the next task), `UpdateTaskStatus` — [Architecture](https://datafusion.apache.org/ballista/contributors-guide/architecture.html)
- 54.0.0 added a `submit_physical_plan` path, so a client can send an already-planned physical plan. `submit_job` is unchanged — [Ballista 54.0.0 blog, 2026-07-12](https://datafusion.apache.org/blog/2026/07/12/datafusion-ballista-54.0.0)
- 51.0.0: the scheduler accepts Substrait logical plans and the deprecated SQL submission path was removed (#1360) — [Releases](https://github.com/apache/datafusion-ballista/releases)

**Job, stage, task**
- A job is submitted as a logical plan or SQL. A stage is a set of operators that can run in isolation, with boundaries at pipeline breakers where partitioning changes. A task is a slice of a stage's output partitions bound to one executor. The scheduler packs up to the executor's free vcores into one task, capped by `ballista.scheduler.max_partitions_per_task` (default `0` = unbounded; `1` = one task per partition) — [Architecture](https://datafusion.apache.org/ballista/contributors-guide/architecture.html)
- The 55.0.0 upgrade guide confirms the change: `max_partitions_per_task` went from `1` to `0`, giving "fewer, larger tasks and higher per-task memory" — [Upgrading to 55.0.0](https://datafusion.apache.org/ballista/upgrading/55.0.0.html)
- Executors advertise a fixed number of `vcores` (task-slot capacity). `--vcores` defaults to all CPU cores — [Tuning guide](https://datafusion.apache.org/ballista/user-guide/tuning-guide.html)
- Example from the docs: a join plus aggregate becomes 4 stages. Stages 1–2 repartition inputs in parallel, stage 3 runs the join and partial aggregate, stage 4 runs the final aggregate — [Architecture](https://datafusion.apache.org/ballista/contributors-guide/architecture.html)

**Shuffle**
- Shuffle is blocking. Each stage materializes output to local storage, and downstream stages wait for the whole upstream stage. The docs call this "a property of the current scheduler, not the design" — [Architecture](https://datafusion.apache.org/ballista/contributors-guide/architecture.html)
- Writers are `ShuffleWriterExec` and `SortShuffleWriterExec`. Each partition writes Arrow IPC under `{work_dir}/{job_id}/{stage_id}/{partition_id}/`. The scheduler records a `ShuffleWritePartition` summary (rows, batches, bytes) when the file closes. `work_dir` can sit on a RAM-backed filesystem — [Shuffle design](https://datafusion.apache.org/ballista/contributors-guide/shuffle.html)
- The tuning guide describes only sort-based shuffle. Output is one consolidated Arrow IPC file per input partition plus an index file, and single-partition stages write one file. Settings: `ballista.shuffle.sort_based.batch_size` = 8192; `ballista.shuffle.sort_based.memory_limit_per_task_bytes` = 268435456 (256 MiB), worst case about vcores × that per executor. It spills when the memory pool rejects growth or the per-task budget is reached — [Tuning guide](https://datafusion.apache.org/ballista/user-guide/tuning-guide.html)
- 55.0.0 changes: one sort-shuffle file per task instead of per input (#2316); spilling when the memory pool rejects a reservation (#2061) — [Releases](https://github.com/apache/datafusion-ballista/releases)
- 55.0.0 removed the protobuf `is_sort_shuffle` field. `FetchPartition` now carries `layout: ShuffleLayout` (`Passthrough` | `Sort`) plus `file_kind` and `byte_ranges`, and there is a new `fetch_byte_ranges` — [Upgrading to 55.0.0](https://datafusion.apache.org/ballista/upgrading/55.0.0.html)
- History: 52.0.0 added sort-based shuffle (#1389); 53.0.0 deferred sort-shuffle materialization (#1598), made sort shuffle work in AQE (#1640), and moved shuffle disk I/O off Tokio worker threads (#1537) — [Releases](https://github.com/apache/datafusion-ballista/releases)
- Transport: `ShuffleReaderExec` reads partitions from the local FS directly, or from the producing executor's Arrow Flight service via `do_get` or the raw-block `IO_BLOCK_TRANSPORT` action — [Shuffle design](https://datafusion.apache.org/ballista/contributors-guide/shuffle.html)
- Shuffle config (docs@main): `ballista.shuffle.compression.codec` = `lz4` (none|lz4|zstd); `shuffle.force_remote_read` = false; `shuffle.remote_read_prefer_flight` = false; `shuffle.max_concurrent_read_requests` = 64; `shuffle.reader.max_bytes_in_flight` = 50331648 (48 MiB); `shuffle.reader.max_blocks_in_flight_per_address` = 128; `shuffle.reader.default_block_size_bytes` = 1048576; `shuffle.writer_channel_capacity` = 8 — [Configs](https://datafusion.apache.org/ballista/user-guide/configs.html)
- 54.0.0 added a reduce-side in-flight fetch governor (#1951) and deletion of intermediate shuffle files on job success (#1982) — [Releases](https://github.com/apache/datafusion-ballista/releases); [54 blog](https://datafusion.apache.org/blog/2026/07/12/datafusion-ballista-54.0.0)
- Experimental pipelined mode (docs@main): `ballista.execution.mode` = `blocking` | `pipelined`, with `pipelined.queue_capacity` = 2, `pipelined.registration_timeout_seconds` = 60, `pipelined.stream_window_bytes` = 524288. The docs say "the scheduler does not run pipelined jobs yet" — [Configs](https://datafusion.apache.org/ballista/user-guide/configs.html)
- Open shuffle directions, not implemented: partial-input early start (#2320, #408), hybrid exchange (#1151, #2003), in-memory shuffle (#2317, #2318), one indexed file per task (#660), shuffle affinity through a custom `TaskDistributionPolicy` (#2319), and external shuffle services Celeborn/Uniffle (#1539) — [Shuffle design](https://datafusion.apache.org/ballista/contributors-guide/shuffle.html)

**Push vs pull**
- 52.0.0 made push scheduling the default (#1461) and added push-based job status updates (#1478) — [Releases](https://github.com/apache/datafusion-ballista/releases)
- `--scheduler-policy` defaults to `push-staged` (also `pull-staged`), and scheduler and executors must use the same policy. "Push can have lower latency. Pull works more like Spark" — [Tuning guide](https://datafusion.apache.org/ballista/user-guide/tuning-guide.html)
- `task-distribution` = `bias` (or `round-robin`). In standalone mode, `standalone.task_scheduling_policy` = `pull-staged`. On the client side, `ballista.client.pull` = false (push job tracking) — [Configs](https://datafusion.apache.org/ballista/user-guide/configs.html)

**AQE**
- Planner fundamentals landed in 52.0.0 (#1372). 53.0.0 added lazy stage evaluation (#1649) and `CoalescePartitionsRule` (#1684). 54.0.0 added deferred join decisions with broadcast (#1752), empty-join handling (#1723) and chaos testing (#1802). **55.0.0 enabled AQE by default** (#2315) and removed redundant shuffles — [Releases](https://github.com/apache/datafusion-ballista/releases)
- In 54.0.0, AQE was "disabled by default and remains experimental". The scheduler re-runs the DataFusion physical optimizer between stages with statistics from completed stages. 54 also fixed AQE against remote object stores ("No suitable object store found") — [54 blog](https://datafusion.apache.org/blog/2026/07/12/datafusion-ballista-54.0.0)
- Mechanics: `StaticExecutionGraph` plans all stages up front with `DistributedPlanner`. `AdaptiveExecutionGraph` uses `AdaptivePlanner` and re-runs rules after each stage on exact row counts and byte sizes, so rules must be idempotent. Rules: `SelectJoinRule` and `DelayJoinSelectionRule` (hash, broadcast or SMJ), build-side staging, empty-stage elimination, shuffle-partition coalescing, parallel windows (bounded `RANGE` into a range shuffle) — [Architecture](https://datafusion.apache.org/ballista/contributors-guide/architecture.html)
- AQE keys (docs@main): `ballista.planner.adaptive.enabled` = true; `planner.adaptive_join.enabled` = true; `planner.coalesce.enabled` = false; `planner.coalesce.target_partition_bytes` = 67108864; `planner.coalesce.merged_partition_factor` = 1.2; `planner.coalesce.small_partition_factor` = 0.2; `planner.parallel_window.enabled` = false; `planner.propagate_empty.enabled` = true — [Configs](https://datafusion.apache.org/ballista/user-guide/configs.html)
- Known AQE gaps: executor failure handling (#1986), hash→SMJ switching (#1988), streaming→hash aggregation switching (#1989) — [Architecture](https://datafusion.apache.org/ballista/contributors-guide/architecture.html). The 54 blog also lists skew partition splitting and global LIMIT early-stop as open work under the umbrella issue #1359 — [54 blog](https://datafusion.apache.org/blog/2026/07/12/datafusion-ballista-54.0.0)

**Joins and broadcast**
- 53.0.0 made sort-merge join the default (#1651) and added broadcast-style hash join for small sides (#1647). 54.0.0 added broadcast through non-zero `CollectLeft` thresholds (#1900) and SMJ build-side broadcast (#1904). 54.1.0 disabled that SMJ conversion by default (#2130), and 55.0.0 removed it (#2072). Since 55.0.0, broadcast build sides get their own stage only when the probe side is partitioned (#2342). In 49.0.0, CollectLeft had been disabled as broken (#1301) — [Releases](https://github.com/apache/datafusion-ballista/releases)
- Defaults with AQE on: `broadcast_join_threshold_bytes` = 134217728 (128 MiB, up from 10 MiB in 55.0.0); `broadcast_join_threshold_rows` = 1000000; `hash_join_max_build_partition_bytes` = 67108864 (above it, SMJ); `stage_build_side` = true (min_probe_ratio 10, max_estimate_multiple 32). With AQE off, `datafusion.optimizer.prefer_hash_join` = false, so joins default to SMJ — [Tuning guide](https://datafusion.apache.org/ballista/user-guide/tuning-guide.html); [Upgrading to 55.0.0](https://datafusion.apache.org/ballista/upgrading/55.0.0.html)
- `ballista.optimizer.broadcast_sort_merge_join_enabled` is removed. Ballista **rejects unknown config keys**, so sessions that still set it fail with a "does not exist" error — [Upgrading to 55.0.0](https://datafusion.apache.org/ballista/upgrading/55.0.0.html)

**Partitions**
- `SessionConfig::new_with_ballista()` sets `target_partitions` = 16. A plain `SessionConfig::new()` passed through `*_with_state` uses the client's core count. Scans produce at most `target_partitions` file groups. `repartition_file_scans` = true splits files of at least 10 MB (`repartition_file_min_size`). Round-robin repartitioning is disabled in Ballista — [Tuning guide](https://datafusion.apache.org/ballista/user-guide/tuning-guide.html)

### Inferences
- For ORE's C7, Ballista's model is Spark-like: materialized blocking shuffle on executor-local disk. That gives natural retry points (completed stages persist), but shuffle data is tied to executor lifetime until an external shuffle service exists (#1539). That coupling matters for scale-to-zero and spot nodes.
- Because AQE is now the default and its rules are re-run, any custom physical optimizer rule ORE registers must be idempotent. AQE also has a known hole on executor failure (#1986).
- Strict unknown-key rejection means ORE's custom config namespaces must be registered as proper `ConfigExtension`s, not passed as free-form keys.

### Gaps
- `UnresolvedShuffleExec` is not mentioned in the current architecture docs. Whether it still exists in 55, or was subsumed by the adaptive graph, was not verified in source.
- Exact semantics of "bias" vs "round-robin" task distribution are not documented. A pluggable `TaskDistributionPolicy` is referenced only as a proposal (#2319).
- Default `--work-dir` is not stated in the docs.

## 2. Extensibility: config/session extension, codecs, object stores, plugin points, credentials

### Takeaway
Ballista's extension model is "override producers in `SchedulerConfig`/`ExecutorProcessConfig`". The hooks are config producer, runtime producer, session builder, logical and physical codec, and function registry. Session options defined as DataFusion `ConfigExtension`s, set by the client (including with `SET`), travel with the job to executors. The docs' own S3 example passes **access keys as session config**, which is cleartext unless TLS is enabled. No secure credential channel exists.

### Cited Findings
- Hooks: **config producer** builds a `SessionConfig` with extended options (example `session_config_with_s3_support`, set through `override_config_producer`). **Runtime producer** builds a `RuntimeEnv` from a `SessionConfig` (example `runtime_env_with_s3_support`, through `override_runtime_producer`, executor side). **Session builder** builds a `SessionState` per user session (example `session_state_with_s3_support`, through `override_session_builder`, scheduler side). **Function registry** override (executor). **Logical codec** and **physical codec** overrides in both `SchedulerConfig` and `ExecutorProcessConfig` — [Extending components](https://datafusion.apache.org/ballista/user-guide/extending-components.html)
- Object stores: `CustomObjectStoreRegistry` creates and caches `ObjectStore`s keyed by `S3Options`, so scans of the same bucket with the same options share one store. `S3Options` is a config extension registered with `with_option_extension` (both in `ballista_core::object_store`) — [Extending components](https://datafusion.apache.org/ballista/user-guide/extending-components.html)
- Client propagation: the client builds state with `state_with_s3_support()`, connects through `SessionContext::remote_with_state("df://localhost:50050", state)` (import `SessionContextExt` from `ballista::extension`), then runs `SET s3.endpoint …`, `SET s3.access_key_id …` so the options reach the cluster — [Extending components](https://datafusion.apache.org/ballista/user-guide/extending-components.html)
- 54.0.0 fixed client `datafusion.*` overrides so they are honored during scheduler planning — [54 blog](https://datafusion.apache.org/blog/2026/07/12/datafusion-ballista-54.0.0)
- `SessionConfigExt` carries Ballista setters such as `ballista_coalesce_target_partition_bytes` / `with_…` (type changed to `usize` in 55) — [Upgrading to 55.0.0](https://datafusion.apache.org/ballista/upgrading/55.0.0.html)
- `GET /api/job/{job_id}/config` (since 54.0.0, #1792) exposes a job's submitted session configuration — [54 blog](https://datafusion.apache.org/blog/2026/07/12/datafusion-ballista-54.0.0); [Scheduler](https://datafusion.apache.org/ballista/user-guide/scheduler.html)
- 52.0.0 added TLS and custom RPC headers (#1400) and configurable IPC read validation (#1374) — [Releases](https://github.com/apache/datafusion-ballista/releases)
- The Flight SQL frontend exposes an `Authenticator` trait passed through `SchedulerConfig` — [Flight SQL](https://datafusion.apache.org/ballista/user-guide/flightsql.html)
- Comparison: datafusion-distributed exposes "Passthrough headers", "Propagating config extensions", "Worker plan rewrite handlers", "Routing tasks to workers" and "Worker versioning" as explicit advanced hooks — [datafusion-distributed docs](https://datafusion-contrib.github.io/datafusion-distributed)

### Inferences
- **Security risk:** `GET /api/job/{id}/config` returns the submitted session config. If ORE ships credentials as session options (the docs' S3 pattern), they become readable through the REST API, and through event logs and the history server, which serve the same responses. ORE should resolve credentials on the executor (runtime producer or object-store registry pulling short-lived tokens from ORE's vault or IdP) and pass only an opaque reference (tenant, session id, signed token) in session config. This is an inference from the documented behavior, not a documented recommendation.
- Custom TableProviders (Iceberg, ORE catalog) and custom ExecutionPlans or UDFs must be registered on both sides: the logical codec on the scheduler, the physical codec plus function registry on executors. ORE's scheduler and executor binaries must be built from the same crate set, which the exact protocol-version check enforces anyway.

### Gaps
- Exact Rust field names for the codec overrides (e.g. `override_logical_codec` / `override_physical_codec`) are not named on the docs page. Verify in `ballista/scheduler/src/config.rs` and `ballista/executor/src/executor_process.rs`.
- No documented custom scheduler-policy plugin beyond push/pull and bias/round-robin.

## 3. State, HA, failure handling, retries, shutdown, protocol versioning

### Takeaway
The scheduler state is **in-memory only**. The etcd/sled backends of the 0.x era are gone, and `ClusterStorageConfig` was removed in 47.0.0. A scheduler crash loses its running jobs. Multiple schedulers can sit behind a Service, but each job's status lives only on the scheduler that ran it. Hardening in 54–55 covers cancellation, terminal-state persistence before eviction, failing jobs when all executors are lost, heartbeats while slots are busy, and graceful shutdown. 55.0.0 introduced strict protocol versioning.

### Cited Findings
- 47.0.0: `ClusterStorageConfig` removed (#1265), cluster state notifications exposed (#1263), `ClusterState` no longer caches session contexts (#1226) — [Releases](https://github.com/apache/datafusion-ballista/releases)
- Older (~0.12, 2023) scheduler config offered cluster storage `etcd`, `memory` or `sled` (sled default) — [scheduler_config_spec.toml 0.12.0 mirror](https://rust-digger.code-maven.com/src/ballista-scheduler-0.12.0/scheduler_config_spec.toml) (older info)
- K8s docs: "The scheduler keeps its state in memory, so jobs still running when it exits are lost. Multiple schedulers can run behind the client-facing Service. A job's status is kept only by the scheduler that ran it. Result fetches can be served by any scheduler or by the stateless result service" — [Kubernetes deployment](https://datafusion.apache.org/ballista/user-guide/deployment/kubernetes.html)
- Each scheduler has a unique `--id`, which matters when multiple schedulers write to shared history or cluster state. A duplicate active executor ID is rejected, and an ID can be reused after removal — [Scheduler](https://datafusion.apache.org/ballista/user-guide/scheduler.html)
- 55.0.0 fixes: terminal job state persisted before cache eviction (#2037); jobs fail when all executors are lost (#2212); executor heartbeats continue while all slots are busy (#2159); graceful shutdown fix (#2393) — [Releases](https://github.com/apache/datafusion-ballista/releases)
- 54.0.0: running stages and tasks are cancelled on job failure or cancel (#1903). Task errors and panics are handled instead of crashing the executor — [Releases](https://github.com/apache/datafusion-ballista/releases); [54 blog](https://datafusion.apache.org/blog/2026/07/12/datafusion-ballista-54.0.0)
- Chaos fault injection keys (testing only): `testing.chaos_execution.enabled` = false, `fault_type` = `transient`, `probability` = 0.25, `seed` — [Configs](https://datafusion.apache.org/ballista/user-guide/configs.html)
- Client IO retries: `io_retries_times` = 3, `io_retry_wait_time_ms` = 3000. gRPC: connect and request timeouts 20 s, HTTP/2 keepalive 300 s, TCP keepalive 3600 s, max message 16 MiB — [Configs](https://datafusion.apache.org/ballista/user-guide/configs.html)
- Cleanup: `finished-job-data-clean-up-interval-seconds` = 300 (0 disables); `finished-job-state-clean-up-interval-seconds` = 3600 — [Configs](https://datafusion.apache.org/ballista/user-guide/configs.html)
- Graceful shutdown flags: scheduler `--graceful-shutdown-timeout-seconds` (prose default 25, manifest uses 45), `--executor-termination-grace-period` (default 30); executor `--graceful-shutdown-timeout-seconds` (default 10). `/readyz` returns 503 during shutdown and requires `--min-ready-executors` — [Kubernetes deployment](https://datafusion.apache.org/ballista/user-guide/deployment/kubernetes.html); [Scheduler](https://datafusion.apache.org/ballista/user-guide/scheduler.html)
- **Protocol versioning (55.0.0):** `BALLISTA_PROTOCOL_VERSION` requires exact equality between scheduler and executor, and mismatches get `failed_precondition`. 54.x executors are rejected, so "upgrade schedulers and executors together". Clients must match the scheduler's **major** version and send a `ballista-version` gRPC header on `ExecuteQuery`/`ExecuteQueryPush`. 54.x clients, which send no header, are rejected. New `scheduler_id` and `executor_id` fields; proto `scheduler_id` renamed to `scheduler_endpoint` — [Upgrading to 55.0.0](https://datafusion.apache.org/ballista/upgrading/55.0.0.html)

### Inferences
- No scheduler HA with job failover exists. For C9, ORE either accepts "scheduler restart = resubmit" or builds job recovery itself: persist the job (plan plus session refs) in ORE's transactional catalog and resubmit, idempotently. Event logs (§6) give post-mortem data but are not a recovery log.
- Exact protocol equality means rolling executor upgrades across versions are impossible. Blue/green whole clusters, or scheduler and executors from the same image digest, is the safe pattern.

### Gaps
- Task and stage retry limits (max attempts), executor heartbeat timeout, and how lost shuffle outputs trigger stage recomputation on the static path are **not documented** on the docs pages fetched. Verify in `ballista/scheduler/src/state/execution_graph*.rs` and `executor_manager.rs`.
- **Speculative execution:** no evidence it exists in Ballista. Not mentioned anywhere fetched.

## 4. Deployment: Kubernetes, KEDA, scale-to-zero, sizing, memory, work dirs, cleanup

### Takeaway
The reference K8s deployment is one scheduler (in-memory), N executor Deployments on a shared PVC, and an optional stateless result service. KEDA scaling works through an **external scaler built into the scheduler** (Cargo feature `keda-scaler`, not in published images) and supports `minReplicaCount: 0`. Executor memory auto-sizes to 70% of host or cgroup memory, split per task as a `FairSpillPool` of pool/vcores.

### Cited Findings
- Manifests: `pv.yaml` (hostPath `/mnt` 10Gi PV, 3Gi PVC), `cluster.yaml` (Services `ballista-scheduler-registration` and `ballista-scheduler` :50050; scheduler Deployment with 1 replica, executor Deployment with 2), `result-service.yaml` (Service, Deployment of 2 replicas, PDB minAvailable 1, optional NetworkPolicy egress to executors :50051), `scale.yaml` (KEDA). Images: `apache/datafusion-ballista-scheduler`, `-executor`, `-result-service` — [Kubernetes deployment](https://datafusion.apache.org/ballista/user-guide/deployment/kubernetes.html)
- Ports: scheduler 50050 (gRPC, Flight and HTTP probes); executor 50051 (Flight) and 50053 (health, `--bind-health-port`); result service 50055 (h2c) — [Kubernetes deployment](https://datafusion.apache.org/ballista/user-guide/deployment/kubernetes.html)
- KEDA: v2.7.1. ScaledObject `ballista-executor`, `minReplicaCount: 0`, `maxReplicaCount: 5`, trigger `external` with `scalerAddress: ballista-scheduler.default.svc.cluster.local:50050`. The scheduler must be built with the `keda-scaler` feature ("published images don't include it"). Polling every 30 s — [Kubernetes deployment](https://datafusion.apache.org/ballista/user-guide/deployment/kubernetes.html)
- Prometheus metric `pending_task_queue_size` exists on the scheduler — [Metrics](https://datafusion.apache.org/ballista/user-guide/metrics.html)
- Release images are published from tag pushes as of 55.0.0 (#2526) — [Releases](https://github.com/apache/datafusion-ballista/releases)
- Memory: the pool auto-sizes from host or cgroup limit times `--memory-pool-fraction`, default **0.70**, range (0, 1]. `--memory-pool-size` overrides it (0 = unbounded). Each task gets a `FairSpillPool` of pool/vcores, and the executor refuses to start if that share is zero. `--max-temp-directory-size`: DataFusion's default spill cap is 100 GB per runtime, set on the executor (the client setting has no effect). `--work-dir` needs space for spills — [Tuning guide](https://datafusion.apache.org/ballista/user-guide/tuning-guide.html)
- `--memory-pool-size` was introduced in 53.0.0 (#1624) — [Releases](https://github.com/apache/datafusion-ballista/releases)
- Shuffle cleanup: files are removed on the `finished-job-data-clean-up-interval-seconds` interval (default 300 s), and intermediate files are deleted on job success (54.0.0) — [Shuffle design](https://datafusion.apache.org/ballista/contributors-guide/shuffle.html); [Configs](https://datafusion.apache.org/ballista/user-guide/configs.html)
- 50.0.0: the executor no longer panics on an unwritable work dir (#1332) — [Releases](https://github.com/apache/datafusion-ballista/releases)
- Executor rollout in the manifest is `maxSurge 25%`, `maxUnavailable 0`; the scheduler has `terminationGracePeriodSeconds: 60` with a 5 s preStop sleep — [Kubernetes deployment](https://datafusion.apache.org/ballista/user-guide/deployment/kubernetes.html)

### Inferences
- With scale-to-zero, the first query pays pod scheduling plus image pull plus executor registration, and readiness gates on `--min-ready-executors`. Shuffle outputs on executor-local disk also make scale-down risky mid-job. KEDA scale-in should be gated on "no running stages whose outputs live on this executor", which Ballista does not expose. That is a C7 design point.
- 70% of a cgroup limit leaves 30% for Tokio, Flight buffers, the shuffle reader in-flight (48 MiB default) and sort-shuffle budgets (256 MiB × vcores). With high vcores, the sort-shuffle worst case can exceed the pool, hence the "0 is safe only with a bounded pool" warning.

### Gaps
- The KEDA external-scaler metric name and formula (pending tasks vs. slots) are not documented on the page; check `ballista/scheduler/src/scheduler_server/external_scaler.rs`.
- No recommended CPU and memory requests in the docs. No published cold-start numbers.

## 5. Clients and the result path

### Takeaway
There are four clients: the Rust `SessionContext::remote`/`remote_with_state`, Python (`pip install ballista`, pre-alpha), the CLI/TUI, and a Flight SQL frontend on the scheduler port (ADBC and JDBC). Results can be fetched directly from executors (Flight tickets), proxied through the scheduler (Flight proxy since 52), or through a stateless result service.

### Cited Findings
- Rust: `SessionContext::remote_with_state("df://localhost:50050", state)` — [Extending components](https://datafusion.apache.org/ballista/user-guide/extending-components.html)
- Python: `pip install ballista==54.0.0` was the first release with PyPI wheels (macOS arm64; Linux x86-64 and ARM64, glibc 2.39+; Python 3.10+). `BallistaSessionContext("df://localhost:50050")`, with Jupyter extras. "Still pre-alpha" — [54 blog](https://datafusion.apache.org/blog/2026/07/12/datafusion-ballista-54.0.0)
- 52.0.0 added an Arrow Flight proxy to the scheduler (#1351) and a Python interface (#1338) — [Releases](https://github.com/apache/datafusion-ballista/releases)
- Flight SQL (docs@main; version not stated): non-default `flight-sql` feature plus scheduler flag `--flight-sql`, on port 50050. Clients: `adbc-driver-flightsql`, the Arrow Flight SQL JDBC driver (`jdbc:arrow-flight-sql://localhost:50050`, `useEncryption=false`). Results go back through the scheduler ("clients never need to reach executors"). Limitations: `GetFlightInfo` blocks until the query finishes, `PollFlightInfo` is not implemented, no bound parameters or cancel, and writes, CTAS, transactions and Substrait are rejected or not implemented — [Flight SQL](https://datafusion.apache.org/ballista/user-guide/flightsql.html)
- Result service: stateless, port 50055. Scheduler `--advertise-flight-endpoint` serves results outside the cluster. Client TLS for result fetch through a gateway uses `ballista.client.use_tls` — [Kubernetes deployment](https://datafusion.apache.org/ballista/user-guide/deployment/kubernetes.html); [Scheduler](https://datafusion.apache.org/ballista/user-guide/scheduler.html)

### Inferences
- For ORE's Query front door, the Flight SQL frontend is not production-ready: it blocks, has no cancel, no auth by default and no writes. ORE's own front door should use the Rust client API (`submit_physical_plan`/`ExecuteQueryPush`) and stream results through the result service or proxy, keeping executors unreachable from clients.

### Gaps
- Which release first shipped Flight SQL, the result service and the history server (55.0.0 or main toward 56) was not confirmed. The 55.0.0 changelog on the releases page was truncated.

## 6. Observability: REST/OpenAPI, event logs and history server, TUI, metrics, tracing

### Takeaway
Ballista has a REST API on by default (OpenAPI v3 at `/api/openapi.json`), optional Prometheus metrics at `/api/metrics` (7 scheduler metrics), a terminal TUI (53.0.0) and web TUI (54.0.0), and JSON-Lines per-job event logs replayed by a `ballista-history-server` (docs@main). Executor-level Prometheus metrics and OpenTelemetry tracing are not documented.

### Cited Findings
- REST (feature `rest-api`, default on; `--disable-rest-api`): `/api/openapi.json`, `/api/state`, `/api/version`, `/api/metrics`, `/api/executors`, `/api/executor/{id}`, `/api/jobs`, `/api/job/{id}`, `/api/job/{id}/config`, `/api/job/{id}/stages`, `PATCH /api/job/{id}` (cancel), DOT and SVG plan endpoints (SVG needs `graphviz-support`). Probes: `/healthz`, `/readyz` — [Scheduler](https://datafusion.apache.org/ballista/user-guide/scheduler.html)
- REST enabled by default since 53.0.0 (#1506), with job start and end times (#1511) and plan tree rendering (#1650). 53.0.0 also added executor system and process metrics reporting (#1547) and `EXPLAIN ANALYZE` (#1567). 51.0.0 added distributed `EXPLAIN` (#1309) — [Releases](https://github.com/apache/datafusion-ballista/releases)
- Prometheus (`prometheus-metrics` Cargo feature): `job_exec_time_seconds` (histogram), `planning_time_ms` (histogram), `job_submitted_total`, `job_completed_total`, `job_failed_total`, `job_cancelled_total`, `pending_task_queue_size`. Buckets are fixed; the workaround is a custom `SchedulerMetricsCollector` — [Metrics](https://datafusion.apache.org/ballista/user-guide/metrics.html)
- 54.0.0: shuffle-read fetch metrics, per-operator metrics in plan display, and `ShuffleReaderExec` shows its upstream stage ID. Failed tasks appear in the REST API and TUI. Task duration appears in the "Finished task" log line. The web TUI's nightly build is on nightlies.apache.org — [54 blog](https://datafusion.apache.org/blog/2026/07/12/datafusion-ballista-54.0.0)
- Event logs and history server (docs@main): off by default, enabled with scheduler `--event-log-dir`. One JSON-Lines file per job, `<job_id>.eventlog.running` renamed to `<job_id>.eventlog` on a terminal state. It records submission, stage start and end, and per-task completion (rows, compute time), with a final record holding the finished API responses (only the final record is served). Progress records can be dropped under backpressure, but the terminal record waits. `ballista-history-server --event-log-dir --bind-port 50060 --update-interval-seconds 10` serves the same `/api/*`. No disk cleanup, and orphan `.running` files remain after a crash. `/api/jobs` has no paging (#2270). Described as "Ballista's equivalent of the Spark History Server" — [History server](https://datafusion.apache.org/ballista/user-guide/history-server.html)
- Open observability epic #1426: more runtime metrics through REST, per-stage flame graphs — [54 blog](https://datafusion.apache.org/blog/2026/07/12/datafusion-ballista-54.0.0)

### Inferences
- For C10, ORE gets job, stage and task timing from the REST API and event logs, and coarse counters from Prometheus. Per-executor resource metrics, distributed traces and per-tenant attribution must be added in ORE's executor wrapper (for example `tracing` spans around task execution, using job and session IDs). This is not built in.
- The event log's progress records are best-effort (droppable), so ORE's `system.default.executions` style ledger should be fed from the terminal record or from ORE's own hooks, not from progress events.

### Gaps
- No OpenTelemetry or tracing exporter documentation found. No executor-side Prometheus endpoint documented.

## 7. Benchmarks, performance gaps, roadmap

### Takeaway
On TPC-H SF1000 (main@c76278cd, 2026-10-07, Ballista 55.0.0 on DataFusion 55.1.0), Ballista totals **502.43 s**. That is ahead of vanilla Spark 4.1.3 (898.17 s), behind Spark+Comet 1.1.0-rc2 (455.09 s), and behind Trino 483 (411.62 s, reference only). Weak spots: Q7, Q10, Q19 (slower than Spark) and Q21. The roadmap is parity with DataFusion, operability, AQE (skew, coalescing), SF1000 bottlenecks, Iceberg/Delta and external shuffle.

### Cited Findings
- SF1000 per engine, mean of 2 iterations including the cold one (Trino: 3 after warm-up). Ballista 502.43 s, Spark 898.17 s, Spark+Comet 455.09 s, Trino 411.62 s. Per query, Ballista beats Spark heavily on Q1 (14.07 vs 71.45), Q17 (20.74 vs 81.36) and Q18 (45.63 vs 134.11), but is slower on Q7 (37.26 vs 25.34), Q10 (43.36 vs 35.31) and Q19 (14.59 vs 13.27). Q21 is the slowest (75.74). The hardware is "the same cluster shape" (details on the Environment sub-page). The Spark, Comet and Trino numbers came from a harness with its own query phrasing. Open issues: #1359, #2025, #2063 — [Benchmarking](https://datafusion.apache.org/ballista/contributors-guide/benchmarking/index.html)
- The 54 blog lists the roadmap: close the DataFusion↔Ballista gap; predictable operation; land AQE; fix the bottlenecks blocking SF1000 (#1596); colocated joins for hash-bucketed tables (#1677); Celeborn/Uniffle (#1539); Iceberg and Delta (#1241, Iceberg #890). It says there is "no formal long-term plan" — [54 blog](https://datafusion.apache.org/blog/2026/07/12/datafusion-ballista-54.0.0)
- 55.0.0: 322 commits from 25 contributors (vs 171 from 11 in 54.0.0), with more TPC-DS queries enabled (#2064) — [Releases](https://github.com/apache/datafusion-ballista/releases)
- The datafusion-distributed self-reported benchmark (12 × c5n.2xlarge, S3 Parquet; sums of p50) shows Ballista far behind at small and medium scale. TPC-H SF1: df-dist 7 s vs Ballista 11 s, Spark 30 s, Trino 18 s. SF10: 10 / 42 / 51 / 33 s. SF100 (19 queries): 42 / 237 / 261 / 93 s. TPC-DS SF1: 29 / 72 / 101 / 85 s. The benchmark code is in `datafusion-distributed-dev-tools` — [datafusion-distributed README](https://github.com/datafusion-contrib/datafusion-distributed)

### Inferences
- The two benchmark sources measure different things. Ballista's own run is SF1000 throughput with materialized shuffle. Datadog's is interactive latency at SF1–100, where Ballista's per-stage materialization and scheduling overhead dominate. Neither is independent. The df-dist numbers may predate Ballista 55 (AQE default, fewer and larger tasks).

### Gaps
- Hardware for the Ballista SF1000 run (Environment sub-page) was not fetched. No independent third-party benchmark found.

## 8. Alternatives for distributing DataFusion

### Takeaway
The main alternative in 2026 is **datafusion-distributed** (datafusion-contrib, Datadog-led, crate 2.0.0, Datadog production). It is a library, not a cluster: it adds streaming network-boundary nodes to an ordinary DataFusion plan, with no coordinator/worker split, no retries and no persistence. **Sail** (LakeSail, 0.7.2) is Spark Connect on DataFusion with an actor-based driver and workers and Arrow Flight shuffle. **datafusion-ray** is no longer maintained.

### Cited Findings
**datafusion-distributed**
- "Not part of Apache DataFusion". Apache-2.0, about 397 commits and 171 stars at fetch time. Tenets: stay close to vanilla DataFusion, take no position on networking, no coordinator/worker split ("any node can act as either"), "tools for building engines, not be one" — [README](https://github.com/datafusion-contrib/datafusion-distributed)
- Published as crate `datafusion-distributed` (2.0.0 and 1.0.0 on docs.rs) — [docs.rs 2.0.0](https://docs.rs/crate/datafusion-distributed/2.0.0/source/README.md)
- Limitations: like vanilla DataFusion, "a node failure fails the whole query, with no retries". Intermediate results are not persisted (no checkpoint or resume). Contrasted with Ballista as a standing cluster that writes shuffle to disk — [User guide index](https://datafusion-contrib.github.io/datafusion-distributed)
- Concepts: stage = plan section between network boundaries; task = per-worker slice with a plan variant specialized at planning time. Leaf and head stages, with the head running as a single task on the coordinator. Transport is Arrow Flight on Tonic gRPC. Stages are sized bottom-up from data read and cardinality reduction, through `DesiredTaskCountHandler` and `ScaleUpLeafNodeHandler`. API: `SessionStateBuilderExt::with_distributed_planner()`, `Worker` (gRPC service), `WorkerResolver` (worker URLs; adaptable to K8s), `RouteTasksHandler`, `DistributedTaskContext` (task index and count in `execute()`), `DistributedLeafExec`, `ChannelResolver` — [Concepts](https://datafusion-contrib.github.io/datafusion-distributed/learn/01-concepts.html)
- Advanced features documented: passthrough headers, config-extension propagation, worker plan rewrite handlers, work-unit feeds, custom distributed plans, worker routing, **worker versioning**, AQE, and distributed dynamic filtering (blog) — [User guide index](https://datafusion-contrib.github.io/datafusion-distributed)
- Datadog blog (Gabriel Musat Mestre, 2026-10-01). It works only on the physical plan, with Substrait as the plan format. Nodes: `NetworkShuffleExec` (global repartition) and `NetworkCoalesceExec` (gather to one node), plus partitioned and broadcast joins, unions and tree-reduce aggregations. Settings `distributed.max_tasks_per_stage` and `distributed.file_scan_config_bytes_per_partition`. Small queries stay single-node. Described as "a zero-copy streaming solution". Ballista was rejected for intermediate materialization, Trino for poor Arrow/Substrait integration and cost, ClickHouse for extensibility. In Datadog production, "some long-running queries now complete up to 10× faster at the same total cost". Future work: AQE, GPU — [Datadog engineering blog](https://www.datadoghq.com/blog/engineering/distributed-datafusion/)

**Sail (LakeSail)**
- Cluster mode: a server hosts sessions, and each session's cluster job runner owns a driver actor that launches workers through a worker manager. Stages contain tasks (one partition each). The control plane is Sail gRPC with an actor model (lock-free state). Data plane: Arrow Flight for shuffle and results, with "workers don't communicate with each other here" on the control plane. Spark Connect server for PySpark. Docs version 0.7.2; fault tolerance not addressed — [Sail architecture](https://docs.lakesail.com/sail/latest/concepts/architecture/)

**datafusion-ray**
- The README states the project is "no longer maintained" (commit #88 by Andy Grove). It relied on Ray for scheduling, could run all of TPC-H, and batch execution was never implemented — [datafusion-ray mirror log](https://apache.googlesource.com/datafusion-ray/+log); [GitHub](https://github.com/apache/datafusion-ray)

### Inferences
- What ORE can learn without forking Ballista:
  1. datafusion-distributed's `WorkerResolver`/`ChannelResolver`/passthrough-headers pattern is the clean way to inject tenant identity and auth into every inter-node call. Ballista has only "custom RPC headers" (52.0.0) and TLS.
  2. Its streaming exchange suits interactive Query, while Ballista's materialized shuffle suits Build and Transforms. ORE could route by workload, since both are DataFusion physical plans and share codecs.
  3. Explicit worker versioning there vs. exact equality in Ballista 55.
- Sail validates the actor-model control plane plus Flight data plane, but it is Spark-API-first and its fault tolerance is undocumented.

### Gaps
- Not researched within the budget, with no sources gathered: Comet (Spark-executor-side DataFusion; only its benchmark appears above), Arroyo (streaming), InfluxDB 3/IOx, Coralogix, GlareDB, Daft. Their stage and shuffle designs are not covered here.
- datafusion-distributed release dates and the named maintainer list were not confirmed (docs.rs shows 1.0.0 and 2.0.0). Its AQE implementation details were not fetched.
- Sail's distributed shuffle persistence and K8s worker-manager details were not fetched.

## 9. Security: authn/authz between client, scheduler and executor

### Takeaway
Ballista has TLS (52.0.0) and custom RPC headers but **no built-in authentication or authorization** on any hop. The Flight SQL frontend ships no authenticator, and the result service checks no credentials and dials any executor address in a ticket. Every doc page says to keep it on a trusted network.

### Cited Findings
- TLS and custom RPC headers added in 52.0.0 (#1400) — [Releases](https://github.com/apache/datafusion-ballista/releases)
- `ballista.client.use_tls` = false by default — [Configs](https://datafusion.apache.org/ballista/user-guide/configs.html)
- Flight SQL: "Ballista ships none" (authentication). Without an authenticator, any reachable client runs queries, sharing one session. An `Authenticator` trait gives private sessions but "is not a security boundary for the cluster". JDBC requires `useEncryption=false` (plain gRPC) — [Flight SQL](https://datafusion.apache.org/ballista/user-guide/flightsql.html)
- The result service "has no authentication and connects to executors in plaintext", checks no credentials and dials any executor address named in a fetch ticket. An optional NetworkPolicy restricts its egress to executors :50051 — [Kubernetes deployment](https://datafusion.apache.org/ballista/user-guide/deployment/kubernetes.html); [Scheduler](https://datafusion.apache.org/ballista/user-guide/scheduler.html)
- datafusion-distributed takes "no position on networking" and offers `ChannelResolver` (custom Flight client construction) and passthrough headers as the hooks for auth — [README](https://github.com/datafusion-contrib/datafusion-distributed); [User guide index](https://datafusion-contrib.github.io/datafusion-distributed)

### Inferences
- An ORE posture that follows from the above: cluster-internal only, NetworkPolicies so only ORE's front door reaches scheduler :50050 and only scheduler, executors and result service reach executors :50051, and mTLS between components. Since Ballista exposes TLS settings, a service mesh is the non-fork alternative. Add a signed tenant or session token injected through custom RPC headers and verified in ORE-built interceptors. Keep `/api/*` (which exposes job configs) off any tenant-reachable path.
- The result-service ticket model ("dials any executor address in the ticket") is an SSRF-style risk if tickets are client-controllable. ORE should never forward client-supplied tickets unvalidated.

### Gaps
- Whether Ballista supports mTLS (client certs) or only server TLS is not stated in the pages fetched. Whether executors authenticate the scheduler on `RegisterExecutor` is undocumented; presumably they do not.
