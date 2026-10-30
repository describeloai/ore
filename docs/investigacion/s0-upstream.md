# S0 · upstream: DataFusion 55.x, Ballista 55.0.0, iceberg-rust main

**Para qué.** Los hechos de upstream que necesita el crate que embebe DataFusion + Ballista + iceberg-rust,
leídos en la fuente primaria. Contesta A1–A3 y C1, C3–C7 de [s0-ore-runtime.md](s0-ore-runtime.md).

**Estado:** leído 2026-10-30. **Método:** los `.crate` publicados en `static.crates.io` (el `Cargo.toml`
normalizado y el código publicado, que es lo que compila `cargo`), la API de crates.io para versiones y
fechas, `gh api` para tags, PR y CI, y un clon de `apache/iceberg-rust` en `main`. Los números de línea son del
código **publicado** (para DataFusion, igual al del tag; para Ballista, `.cargo_vcs_info.json` da el commit
`d5379965`). Lo que no se pudo leer en la fuente va marcado **[sin verificar]**.

Commits de los tags:

| repo | tag | commit |
|---|---|---|
| apache/datafusion | 55.0.0 | `d5552342012888b7d1a3ab88d92e3d292fc0cde0` |
| apache/datafusion | 55.1.0 | `7d3835c71f30cbd3c3ae4041732267f1f453097a` |
| apache/datafusion | 55.2.0 | `500744d5c25499055e3b2ca46b022bc425989203` |
| apache/datafusion-ballista | 55.0.0 (= 55.0.0-rc1) | `d5379965528955a4afc5f41bac0c8c409de68a7d` |
| apache/iceberg-rust | merge de #3483 | `5c27157675a7babb611638786195dfe24ad41dab` (2026-10-22T09:21:55Z) |
| apache/iceberg-rust | `main` leído | `4dd8c946432f5e56a3ccc88e5b438a356ba28287` (2026-10-29, #3277), 31 commits por encima de #3483 |

---

## A1 · datafusion 55.x

Publicadas: 55.0.0 (2026-08-18), 55.1.0 (2026-09-11), 55.2.0 (2026-10-07). Ninguna retirada (yanked).
Fuente: `https://crates.io/api/v1/crates/datafusion/versions`, `Cargo.toml` de cada `.crate`.

**Las tres versiones tienen dependencias idénticas** (sólo cambia `datafusion-*` = la propia versión):

| dependencia | requisito | features |
|---|---|---|
| `arrow` | `59.2.0` | `prettyprint`, `chrono-tz` |
| `arrow-schema` | `59.2.0` | `canonical_extension_types`, sin default |
| `parquet` | `59.2.0` (opcional) | `arrow`, `async`, `object_store` |
| `object_store` | `0.13.2`, `default-features = false` | — |
| `sqlparser` | `0.62.0` (opcional) | `std`, `visitor` |
| `tokio` | `1.52` | `macros`, `rt`, `sync` |
| `chrono` | `0.4.45` | sin default |

- `rust-version = "1.94.0"`, `edition = "2024"` (en 54.x era 1.88.0).
- Requisitos con `^` implícito: `object_store = "0.13.2"` admite 0.13.x pero **no** 0.14 (en crates.io ya hay
  0.14.0–0.14.2; DataFusion 55 sigue en 0.13). Último 0.13.x: 0.13.2 (2026-03-24).
- `default = [nested_expressions, crypto_expressions, datetime_expressions, encoding_expressions,
  regex_expressions, string_expressions, unicode_expressions, compression, parquet, recursive_protection, sql]`.
- Features opcionales con interés:
  - `parquet_encryption = [parquet, parquet/encryption, datafusion-common/parquet_encryption, datafusion-datasource-parquet/parquet_encryption]`
  - `serde = [dep:serde, arrow-schema/serde]`
  - `avro = [datafusion-datasource-avro]`
  - `backtrace = [datafusion-common/backtrace]`
  - `recursive_protection` (en default) activa `recursive_protection` en common/expr/optimizer/physical-* y `sqlparser/recursive-protection`.
  - Resto: `array_expressions`, `math_expressions`, `extended_tests`, `force_hash_collisions`.
- No hay `num_cpus` en ningún `Cargo.toml` de los crates leídos (datafusion, -common, -execution, -physical-plan, ballista-*).

## A2 · Ballista 55.0.0

Publicado 2026-10-06 (ballista-cli 2026-10-08). `rust-version = "1.94.0"` en todos. Crates publicados en 55.0.0:
`ballista`, `ballista-core`, `ballista-scheduler`, `ballista-executor`, `ballista-cli`, y además
`ballista-api-types`, `ballista-flight-sql`, `ballista-history` (dependencias opcionales del scheduler/cli).

**Requisito de DataFusion:** `datafusion = "55"` (y `datafusion-proto = "55"`, `datafusion-proto-common = "55"`,
`datafusion-substrait = "55"` opcional, `datafusion-spark = "55"` opcional) — es decir `^55`, **no** un `=`:
casa con 55.0, 55.1 o 55.2. `arrow-flight = "59.2.0"` (feature `flight-sql-experimental`), `arrow = "59.2.0"`
(executor, `ipc_compression`), `object_store = "0.13.2"`, `tonic = "0.14"`, `prost = "0.14"`, `tokio = "1"`.

Features:

| crate | default | resto |
|---|---|---|
| `ballista` (path `ballista/client`) | `standalone` (= `ballista-executor` + `ballista-scheduler`) | `test_extended_stack` |
| `ballista-core` | `arrow-ipc-optimizations` | `build-binary` (= `clap`, `object_store` con `aws`+`http`, y el módulo `ballista_core::object_store`), `spark-compat`, `utoipa`, `force_hash_collisions`, `docsrs` |
| `ballista-scheduler` | `build-binary`, `rest-api` | `flight-sql` (dep `ballista-flight-sql`), `keda-scaler`, `prometheus-metrics`, `graphviz-support`, `substrait`, `spark-compat`, `disable-stage-plan-cache` |
| `ballista-executor` | `arrow-ipc-optimizations`, `build-binary` (= clap, tracing, **mimalloc**, axum) | `spark-compat` |
| `ballista-cli` | `cli`, `tui` | `web` |

Binarios: `ballista-scheduler`, `ballista-history-server`, `ballista-executor`, `ballista-cli`; `ballista-core`
trae un bin `update_config_docs`. Para embeber conviene `default-features = false` en executor (quita mimalloc y
axum) y elegir a mano en scheduler (`rest-api` arrastra `ballista-history`, `utoipa`).

### Scheduler como librería — sí

`ballista_scheduler::scheduler_process` (`ballista/scheduler/src/scheduler_process.rs`):

```rust
pub async fn create_scheduler<T: 'static + AsLogicalPlan, U: 'static + AsExecutionPlan>(
    cluster: BallistaCluster, config: Arc<SchedulerConfig>,
) -> ballista_core::error::Result<SchedulerServer<T, U>>;                       // l.50
pub async fn start_grpc_service<T, U>(address: SocketAddr, scheduler: SchedulerServer<T, U>) -> Result<()>;   // l.112
pub async fn start_grpc_service_with_listener<T, U>(listener: tokio::net::TcpListener, scheduler: SchedulerServer<T, U>) -> Result<()>; // l.131
pub async fn start_server(cluster: BallistaCluster, address: SocketAddr, config: Arc<SchedulerConfig>) -> Result<()>; // l.250
```

`start_server` = `config.validate()?` + `create_scheduler::<LogicalPlanNode, PhysicalPlanNode>` + `start_grpc_service`.
El cluster: `BallistaCluster::new_from_config(&SchedulerConfig).await` (`src/cluster/mod.rs:117`, sólo memoria:
`InMemoryClusterState` + `InMemoryJobState`), `BallistaCluster::new_memory(scheduler_id, SessionBuilder, ConfigProducer)` (l.101)
o `BallistaCluster::new(Arc<dyn ClusterState>, Arc<dyn JobState>)` (l.84) para estado propio.
El bin (`src/bin/main.rs:33-94`) crea un runtime multi-hilo con `thread_stack_size(32 MiB)`, convierte `Config`
(clap, sólo con `build-binary`) en `SchedulerConfig` vía `TryFrom<Config>` (`config.rs:851`) y fija
`with_override_config_producer(session_config_with_s3_support)` + `with_override_session_builder(session_state_with_s3_support)`.

`SchedulerConfig` (`src/config.rs:326`, `Default` en l.454, builders `with_*` l.587–800): `namespace`, `scheduler_id`
(por defecto `new_instance_id()`), `external_host` ("localhost"), `bind_host` ("127.0.0.1"), `bind_port` (50050),
`scheduling_policy`, `event_loop_buffer_size` (10000), `task_distribution`, `finished_job_data_clean_up_interval_seconds` (300),
`finished_job_state_clean_up_interval_seconds` (3600), `advertise_flight_endpoint`, `enable_embedded_flight_proxy`,
`job_resubmit_interval_ms`, `executor_termination_grace_period`, `scheduler_event_expected_processing_duration`,
`grpc_server_max_decoding_message_size`, `grpc_server_max_encoding_message_size`, `grpc_client_max_message_size`,
`executor_timeout_seconds`, `expire_dead_executor_interval_seconds`, `no_executors_grace_period_seconds`,
**`override_config_producer: Option<ConfigProducer>`** (l.399), **`override_session_builder: Option<SessionBuilder>`** (l.401),
**`override_logical_codec: Option<Arc<dyn LogicalExtensionCodec>>`** (l.403), **`override_physical_codec: Option<Arc<dyn PhysicalExtensionCodec>>`** (l.405),
`override_create_grpc_client_endpoint: Option<EndpointOverrideFn>`, `use_tls`, `min_ready_executors`, `task_max_failures`,
`stage_max_failures`, `flight_sql` (cfg `flight-sql`), `override_flight_sql_authenticator` (cfg `flight-sql`),
`disable_rest_api` / `cors_allowed_origins` / `cors_allowed_methods` (cfg `rest-api`), `event_log_dir`,
`web_tui_route` (cfg `rest-api`), `on_work_available: Option<OnWorkAvailableFn>`, `job_id_generator: Option<Arc<dyn JobIdGenerator>>`.
El scheduler **no** tiene campo de registro de funciones ni de object stores: ambos entran por el `SessionBuilder`
(el `SessionState` que construye lleva funciones y `RuntimeEnv`).

Tipos (`ballista-core/src/lib.rs`, `extension.rs`; `ballista-scheduler/src/scheduler_server/mod.rs`):

```rust
pub type SessionBuilder  = Arc<dyn Fn(SessionConfig) -> datafusion::common::Result<SessionState> + Send + Sync>; // scheduler_server/mod.rs:74
pub type ConfigProducer  = Arc<dyn Fn() -> SessionConfig + Send + Sync>;                                          // core lib.rs:126
pub type RuntimeProducer = Arc<dyn Fn(&SessionConfig) -> datafusion::error::Result<Arc<RuntimeEnv>> + Send + Sync>; // core lib.rs:117
pub type EndpointOverrideFn = Arc<dyn Fn(Endpoint) -> Result<Endpoint, Box<dyn Error + Send + Sync>> + Send + Sync>; // core extension.rs:57
```

Por defecto: `ballista_core::utils::default_session_builder` (`utils.rs:158`: `SessionStateBuilder::new().with_default_features()
.with_config(config).with_runtime_env(RuntimeEnvBuilder::new().build()?).with_optimizer_rules(ballista_default_optimizer_rules())
.with_scalar_functions(ballista_scalar_functions())…`) y `default_config_producer()` = `SessionConfig::new_with_ballista()` (l.180).

### Executor como librería — sí

`ballista_executor::executor_process::start_executor_process(opt: Arc<ExecutorProcessConfig>) -> ballista_core::error::Result<()>`
(`ballista/executor/src/executor_process.rs:424`); bloquea hasta SIGTERM / Ctrl+C / orden de parada del scheduler.
Otras entradas: `ballista_executor::new_standalone_executor`, `new_standalone_executor_from_state`,
`new_standalone_executor_from_builder` (`standalone.rs:159/51/88`). El bin (`src/bin/main.rs:40-97`) usa `#[tokio::main]`,
`opt.try_into()?` y fija `override_config_producer = session_config_with_s3_support`, `override_runtime_producer = runtime_env_with_s3_support`.

`ExecutorProcessConfig` (l.261, `Default` l.372): `executor_id`, `bind_host` ("127.0.0.1"), `external_host`, `port` (50051, Flight),
`grpc_port` (50052), `scheduler_host`, `scheduler_port` (50050), `scheduler_connect_timeout_seconds`, `vcores`, `task_scheduling_policy`,
`log_dir`, `work_dir`, `special_mod_log_level`, `print_thread_info`, `log_rotation_policy`, `job_data_ttl_seconds` (604800),
`job_data_clean_up_interval_seconds` (0), `grpc_max_decoding_message_size` / `grpc_max_encoding_message_size` (16 MiB),
`grpc_server_config`, `executor_heartbeat_interval_seconds` (60), `metric_collection_policy`, `memory_pool_size: Option<u64>`,
`memory_pool_fraction: f64` (0.70), `session_runtime_cache_capacity` (16), `override_execution_engine: Option<Arc<dyn ExecutionEngine>>`,
**`override_function_registry: Option<Arc<BallistaFunctionRegistry>>`** (l.323), **`override_runtime_producer: Option<RuntimeProducer>`** (l.325),
**`override_config_producer: Option<ConfigProducer>`** (l.327), **`override_logical_codec`** (l.329), **`override_physical_codec`** (l.331),
`override_arrow_flight_service: Option<Arc<ArrowFlightServerProvider>>`, `override_create_grpc_client_endpoint`, `client_ttl` (30), `health: ExecutorHealth`.

- **Registro de funciones:** `ballista_core::registry::BallistaFunctionRegistry` (`registry.rs:39`), campos públicos
  `scalar_functions`, `aggregate_functions`, `window_functions`, `higher_order_functions` (`HashMap<String, Arc<…>>`);
  `impl From<&SessionState>` (l.166).
- **Registro de object stores:** no hay campo propio; va dentro del `RuntimeEnv` que devuelve `override_runtime_producer`.
  La política de memoria del ejecutor reconstruye cada env con `RuntimeEnvBuilder::from_runtime_env(&base).with_memory_pool(FairSpillPool)`
  y conserva disk manager, cache manager y `object_store_registry` (test `preserves_base_object_store_registry`, l.1401).
  Ejemplo upstream: `ballista_core::object_store::{CustomObjectStoreRegistry, S3Options, runtime_env_with_s3_support, session_config_with_s3_support, session_state_with_s3_support}` (sólo con `build-binary`).

### Versiones de protocolo

- `ballista_core::BALLISTA_PROTOCOL_VERSION: u32 = 3` (`ballista/core/src/lib.rs:47`): scheduler↔executor,
  comparado en cada registro y latido; si no casa, `Status::failed_precondition`. Puede cambiar en cualquier release.
- Cliente↔scheduler: por **versión mayor** de `BALLISTA_VERSION`, cabecera gRPC (ver C6).

### SessionConfigExt / SessionStateExt (`ballista/core/src/extension.rs`)

- `SessionConfigExt` (l.121) para `SessionConfig`: `SessionConfig::new_with_ballista()` (l.457 = `SessionConfig::new()
  .with_option_extension(BallistaConfig::default()).with_information_schema(true).with_target_partitions(16)
  .ballista_restricted_configuration()`), `upgrade_for_ballista(self)` (idempotente: si ya lleva `BallistaConfig` no
  reaplica los defaults), `ballista_config()`, `with_ballista_logical_extension_codec`, `with_ballista_physical_extension_codec`,
  `with_ballista_query_planner`, `with_ballista_job_name`, `with_ballista_grpc_metadata`, `with_ballista_use_tls`,
  `with_ballista_standalone_parallelism`, broadcast join / coalesce / shuffle reader getters y setters.
- `ballista_restricted_configuration` (l.700): `with_round_robin_repartition(false)` (lo impone también el
  `session_manager.rs:74` del scheduler) y `datafusion.execution.parquet.schema_force_view_types = false`, entre otros.
- `SessionStateExt` (l.103) para `SessionState`: `SessionState::new_ballista_state(scheduler_url: String) -> Result<SessionState>`
  y `state.upgrade_for_ballista(scheduler_url: String) -> Result<SessionState>`. **No existe** `SessionStateBuilder::with_ballista`;
  `new_ballista_state` usa `SessionStateBuilder::new().with_default_features().with_config(..).with_cache_factory(BallistaCacheFactory)
  .with_query_planner(BallistaQueryPlanner)…`.
- Cliente: `ballista::prelude::{SessionConfigExt, SessionContextExt}`; `SessionContextExt` (`ballista/client/src/extension.rs:64`):
  `SessionContext::standalone()`, `standalone_with_state(state)` (cfg `standalone`), `remote(url)`, `remote_with_state(url, state)`.

### ConfigExtension: qué se rechaza y qué no

- `BallistaConfig` (`PREFIX = "ballista"`, `config.rs:1016`) rechaza claves desconocidas bajo `ballista.`:
  `ExtensionOptions::set` (l.987) → `config_err!("configuration key `{}` does not exist", key)`. `valid_entries()` (l.646).
- DataFusion `ConfigOptions::set` (`datafusion-common/src/config.rs:2018`) rechaza un espacio no registrado:
  `"Could not find config namespace \"{prefix}\""` (y una clave sin punto).
- **Pero** en el servidor los pares no se aplican con error: el cliente manda `to_key_value_pairs()` y el scheduler
  (`scheduler_server/grpc.rs:413`, `840`) y el ejecutor (`execution_loop.rs:338`) llaman `update_from_key_value_pair`,
  que ante un fallo de `options_mut().set` **sólo registra en `debug!` y sigue** (`extension.rs:675-697`).
  Consecuencia: una `ConfigExtension` propia hay que registrarla (`SessionConfig::with_option_extension(T)`) en el
  `ConfigProducer` del scheduler **y** del ejecutor; si no, sus claves desaparecen en silencio en el lado remoto.

## A3 · iceberg-rust `main` (4dd8c946, después de #3483)

Fuente: `Cargo.toml` raíz y de cada crate en el clon.

- `[workspace.package]`: `version = "0.10.1"`, `edition = "2024"`, **`rust-version = "1.95"`** (más alto que el 1.94.0 de
  DataFusion 55); `rust-toolchain.toml` = `nightly-2026-04-16`. Las dependencias internas dicen `version = "0.10.0"` + `path`.
- Crates del workspace (publish = true): `iceberg`, `iceberg-catalog-glue`, `iceberg-catalog-hms`, `iceberg-catalog-loader`,
  `iceberg-catalog-rest`, `iceberg-catalog-s3tables`, `iceberg-catalog-sql`, `iceberg-cache-moka` (`crates/integrations/cache-moka`),
  `iceberg-property-macro`, `iceberg-storage-opendal` (`crates/storage/opendal`). No publicados: `pyiceberg_core_rust`,
  `iceberg-examples`, `iceberg-integration-tests`, `iceberg_test_utils`.
- `arrow*` = `59.2`, `parquet` = `59.2`, `parquet-geospatial` = `59.2`, `opendal` = `0.58`,
  `tokio = { version = "1.47", default-features = false, features = ["sync", "rt-multi-thread"] }`,
  `reqwest = { version = "0.12.12", default-features = false, features = ["json"] }`. Casa con el arrow/parquet 59.2 de DataFusion 55.
- **El core `iceberg` no depende de DataFusion**, y ningún crate del workspace tampoco (`Cargo.lock` sin ningún paquete
  `datafusion*`). La integración se quitó en `a1de3a10` «chore: remove migrated DataFusion integration crates (#3228)»
  (2026-09-17): se movió a **apache/datafusion-iceberg** (crate `iceberg-datafusion`). Ese repo en `main` (`8979482`,
  2026-10-30) declara `datafusion = "55.0.0"`, `datafusion-sqllogictest = "55.0.0"` y fija iceberg por git en
  `rev = "5b82ab99aeaf50bf4f5b3b776ac9cd6380a5f3e3"` (2026-10-09), **anterior** a #3483.
- Ojo duplicado: en el `Cargo.lock` de iceberg-rust conviven `reqwest 0.12.28` (iceberg, catalog-rest, storage-opendal)
  y `reqwest 0.13.4` (`opendal-http-transport-reqwest 0.58.1`, xet-*). object_store 0.13.2 usa `reqwest 0.12`.
- `features` de `iceberg-storage-opendal`: `default = ["opendal-memory", "opendal-fs", "opendal-s3"]`, `opendal-all` añade gcs, oss, azdls, hf.
- Runtime propio de iceberg (relevante para C3): `iceberg::runtime::Runtime` (`crates/iceberg/src/runtime/mod.rs:136`) con
  `Runtime::new(&tokio::runtime::Runtime)`, **`Runtime::new_with_split(io_runtime, cpu_runtime)`** (l.158), `current()`, `try_current()`,
  `io()`, `cpu()`; `TableBuilder::runtime(Runtime)` (`table.rs:109`), `Table::with_runtime(Runtime)` (l.226). La punta de `main`
  (#3277) hace que las aperturas de ficheros de datos se lancen en el runtime de IO.

---

## C1 · APIs de sesión en DataFusion 55

**`datafusion-session` 55.2.0** (`src/`: `catalog.rs`, `physical_optimizer.rs`, `planner.rs`, `schema.rs`, `session.rs`, `table.rs`).
Re-exporta: `CatalogProvider`, `CatalogProviderList`, `EmptyCatalogProviderList`, `PhysicalOptimizerContext`, `PhysicalOptimizerRule`,
`ExtensionPlanner`, `PhysicalPlanner`, `QueryPlanner`, `UnsupportedQueryPlanner`, `SchemaProvider`, `Session`, `SessionStore`,
`ScanArgs`, `ScanResult`, `TableFunction`, `TableFunctionArgs`, `TableFunctionImpl`, `TableProvider`, `TableProviderFactory`.
En 54.1.0 el crate sólo tenía `lib.rs` + `session.rs`: en 55 los traits de catálogo/tabla/planificador viven aquí.

**Trait `Session`** (`datafusion-session/src/session.rs:80`, `#[async_trait] pub trait Session: Send + Sync`).
Obligatorios: `session_id`, `config`, **`catalog_list(&self) -> Arc<dyn CatalogProviderList>` (nuevo y obligatorio en 55)**,
`create_physical_plan` (async), `create_physical_expr`, `scalar_functions`, `higher_order_functions`, `aggregate_functions`,
`window_functions`, `extension_type_registry`, `runtime_env`, `execution_props`, `as_any`, `table_options`, `table_options_mut`, `task_ctx`.
Con defecto, **nuevos en 55**: `query_planner()` (defecto `UnsupportedQueryPlanner`: `create_physical_plan` falla), `optimize(&LogicalPlan)`
(defecto: plan **sin optimizar**), `physical_optimizers()` (defecto: **ninguna regla**, lo que rompe planes multi-partición),
`statistics_registry()` (defecto `None`). Ya existían `config_options`, `default_table_options`. Comparado contra 54.1.0.
Quien implemente `Session` a mano tiene que sobrescribir los cuatro nuevos delegando en `SessionState`.

**`SessionStateBuilder`** (`datafusion/core/src/execution/session_state.rs:1053`, `#[derive(Clone)]`):
`new()` (l.1095), `new_from_existing(SessionState)` (l.1137), `with_default_features()` (l.1195), `new_with_default_features()` (l.1252),
`with_session_id`, `with_analyzer_rules/_rule`, `with_optimizer_rules/_rule`, `with_expr_planners`, `with_relation_planners`, `with_type_planner`,
`with_physical_optimizer_rules/_rule`, `with_query_planner`, `with_catalog_list`, `with_table_functions`, `with_table_function_list`,
`with_scalar_functions`, `with_higher_order_functions`, `with_aggregate_functions`, `with_window_functions`, `with_extension_type_registry`,
`with_serializer_registry`, `with_file_formats`, `with_config`, `with_table_options`, `with_execution_props`, `with_table_factory/_factories`,
`with_runtime_env`, `with_function_factory`, `with_cache_factory`, `with_statistics_registry` (l.1523), `with_object_store` (l.1548), `build()` (l.1568),
y accesores `&mut Option<…>` (l.1765–1900). `SessionContext::into_state_builder()` (context/mod.rs:456).

**Coste de clonar `SessionState`:** `#[derive(Clone)]` (l.140) sobre un struct con `HashMap<String, Arc<…>>` de funciones
escalares/agregadas/ventana/tabla, `Vec`s de reglas, `Analyzer`, `Optimizer`, `PhysicalOptimizer`, `SessionConfig`, `TableOptions`…;
clonar copia los mapas y vectores (los valores son `Arc`). `SessionContext::state()` (context/mod.rs:2054) hace
`self.state.read().clone()` + `mark_start_execution()` en **cada** llamada; `state_ref()` da el `Arc<RwLock<SessionState>>` sin copia.
**[sin verificar]** No hay nota de coste documentada ni medida; medirlo si se clona por consulta.

**`RuntimeEnv` / `RuntimeEnvBuilder`** (`datafusion-execution/src/runtime_env.rs`):
- `RuntimeEnv` (l.73) campos públicos: `memory_pool: Arc<dyn MemoryPool>`, `disk_manager: Arc<DiskManager>`, `cache_manager: Arc<CacheManager>`,
  `object_store_registry: Arc<dyn ObjectStoreRegistry>`, `parquet_encryption_factory_registry` (cfg `parquet_encryption`).
  Métodos: `register_object_store`, `deregister_object_store`, `object_store(url)`, `spilling_progress`, `register_parquet_encryption_factory`, `config_entries`.
- `RuntimeEnvBuilder` (l.358): `new()`, `with_disk_manager_builder(DiskManagerBuilder)`, `with_memory_pool(Arc<dyn MemoryPool>)`,
  `with_cache_manager(CacheManagerConfig)`, `with_object_store_registry(..)`, `with_memory_limit(max_memory, fraction)` (=
  `TrackConsumersPool::new(GreedyMemoryPool::new(..), 5)`), `with_temp_file_path`, `with_max_temp_directory_size`,
  `with_max_spill_merge_fan_in`, `with_metadata_cache_limit`, `with_object_list_cache_limit`, `with_object_list_cache_ttl`,
  `with_file_statistics_cache_limit`, `build()`, `build_arc()`, `from_runtime_env(&RuntimeEnv)` (l.519), `entries()`.
  Sin pool explícito, `build()` usa `UnboundedMemoryPool`.
- Pools (`memory_pool/pool.rs`): `UnboundedMemoryPool`, `GreedyMemoryPool`, `FairSpillPool`, `TrackConsumersPool<I>`; además
  `PeakRecordingPool` (`peak_recording.rs`) y `ArrowMemoryPool` (`arrow.rs`).
- `DiskManagerMode` (`disk_manager.rs:161`): `OsTmpDirectory` (defecto), `Directories(Vec<PathBuf>)`, `Custom(Arc<dyn TempFileFactory>)`, `Disabled`.
- `CacheManagerConfig` (`cache/cache_manager.rs:413`): `file_statistics_cache`, `file_statistics_cache_limit`, `list_files_cache`,
  `list_files_cache_limit`, `list_files_cache_ttl`, `file_metadata_cache`, `metadata_cache_limit`.

## C3 · IO de object store en otro runtime de tokio

- Ejemplo oficial: **`datafusion-examples/examples/query_planning/thread_pools.rs`** en el tag 55.2.0
  (<https://github.com/apache/datafusion/blob/500744d5c25499055e3b2ca46b022bc425989203/datafusion-examples/examples/query_planning/thread_pools.rs>).
  Patrón: el runtime de `#[tokio::main]` hace el IO; un `CpuRuntime` propio (l.288–354: `tokio::runtime::Builder::new_multi_thread().enable_time().build()`,
  **sin `enable_io`**, que corre en un `std::thread` y se apaga con un `Notify`) ejecuta plan y stream. El object store se construye con
  `HttpBuilder::new().with_url(..).with_http_connector(SpawnedReqwestConnector::new(io_handle)).build()?` (l.211–217) y se registra con
  `ctx.register_object_store(&url, Arc::new(store))`; la consulta se lanza con `join_set.spawn_on(driver_task, cpu_runtime.handle())` y los
  lotes vuelven por un `mpsc::channel(2)`. Sin el conector, el IO falla con «A Tokio 1.x context was found, but IO is disabled.»
  Lo justifica la doc de crate, sección «Thread Scheduling, CPU / IO Thread Pools, and Tokio Runtimes» (`datafusion/core/src/lib.rs:520`).
- object_store 0.13.2 (feature `cloud`, módulo `object_store::client`, re-exportado desde `client::http`):
  - `pub struct SpawnedReqwestConnector` + `SpawnedReqwestConnector::new(runtime: tokio::runtime::Handle)` (`src/client/http/connection.rs:340/350`),
    `impl HttpConnector`: `connect()` envuelve el cliente en `SpawnService`. Se pasa a cualquier builder con `.with_http_connector(..)` (S3, GCS, Azure, HTTP).
  - `pub struct SpawnService<T: HttpService + Clone>` + `SpawnService::new(inner: T, runtime: Handle)` (`src/client/http/spawn.rs:49/56`):
    hace `runtime.spawn` de la petición y devuelve el cuerpo por un `mpsc::unbounded_channel` (sin contrapresión entre runtimes, a propósito).
  - `cfg(not(target_arch = "wasm32"))`.
- **DataFusion 55 no trae helper propio** para esto: ni `datafusion`, ni `datafusion-execution`, ni `datafusion-common-runtime` contienen
  `SpawnedReqwestConnector`/`DedicatedExecutor`/«io_runtime» (grep en el código publicado de 55.2.0).
- Ballista 55 sí separa CPU dentro del ejecutor: `ballista-executor/src/cpu_bound_executor.rs` (`DedicatedExecutor`, privado), creado con
  `DedicatedExecutor::new("task_runner", vcores)` en `execution_loop.rs:135` y `executor_server.rs:839`. No es API pública.
- iceberg-rust trae su propia separación (`Runtime::new_with_split`, ver A3); su IO va por OpenDAL, no por object_store.

## C4 · `target_partitions` por defecto, cgroups y Ballista

- `datafusion-common/src/config.rs:853`: `target_partitions` y `planning_concurrency` (l.868) tienen `default = get_available_parallelism()`
  y `transform = ExecutionOptions::normalized_parallelism` (`"0"` → el valor detectado, l.1816).
- `get_available_parallelism()` (`datafusion-common/src/utils/mod.rs:1151`) = `std::thread::available_parallelism()` con 1 de reserva,
  **cacheado en un `LazyLock` en la primera llamada** (no ve cambios posteriores de límites). Sin `num_cpus`.
- Rust std 1.94.0, Linux (`library/std/src/sys/thread/unix.rs:156`, <https://github.com/rust-lang/rust/blob/1.94.0/library/std/src/sys/thread/unix.rs>):
  `min(CPU_COUNT(sched_getaffinity), cgroups::quota())`, respaldo `sysconf(_SC_NPROCESSORS_ONLN)`. `cgroups::quota()` (l.675) lee
  `/proc/self/cgroup`; para v2 (`quota_v2`, l.722) exige `/sys/fs/cgroup/<path>/cgroup.controllers` y sube por la jerarquía leyendo
  `cpu.max` y quedándose con el mínimo de **`limit / period` (división entera, redondeo hacia abajo)**; `max` o no numérico = sin cuota; v1 lee `cfs_quota_us`/`cfs_period_us`.
  `quota().max(1)`. Así que **sí respeta `cpu.max` de cgroup v2**, pero: una cuota fraccionaria se trunca (1.5 CPU → 1; 0.5 → 1),
  no cubre montajes v2 no estándar ni rutas con espacios/caracteres de control (comentario l.653–656), y la doc avisa que puede
  sobrecontar si el fs de cgroup no se puede leer (sandbox).
- tokio 1.52 (`tokio/src/loom/std/mod.rs:85`): `worker_threads` por defecto = `TOKIO_WORKER_THREADS` o `std::thread::available_parallelism()` → misma lógica.
- **Ballista 55 ejecutor — vcores:** `ExecutorProcessConfig::default().vcores = std::thread::available_parallelism().unwrap().get()`
  (`executor_process.rs:383`); si `vcores == 0`, igual en `start_executor_process` (l.451). Por tanto hereda el comportamiento de cgroup de std.
  `SessionConfig::new_with_ballista()` fija `target_partitions = 16` sin mirar la máquina.
- **Ballista 55 ejecutor — memoria:** con `memory_pool_size = None`, auto = `memory_pool_fraction` (defecto `0.70`, l.87) ×
  `min(host_total, cgroup_limit)` (`resolve_pool`, l.138–163). `read_cgroup_memory_limit(Path::new("/sys/fs/cgroup"))` (l.170, ruta fija, l.210):
  v2 `memory.max` (`"max"` → sin límite), si no v1 `memory/memory.limit_in_bytes`. Host vía `sysinfo` (`read_host_total_memory`, l.192).
  Ningún dato → pool **no acotado** con `warn!`. El total se reparte en `FairSpillPool` de `total / vcores` por vcore, multiplicado por los
  vcores que consume la tarea (`memory_pool_policy`, l.224). `memory_pool_size = Some(0)` → no acotado explícito.

## C5 · sqllogictest

- **`datafusion-sqllogictest` sí está en crates.io**: 55.0.0, 55.1.0, 55.2.0 (mismas fechas que datafusion), `rust-version 1.94.0`.
  Depende de `datafusion = "55.2.0"`, `datafusion-spark`, `datafusion-substrait`, **`sqllogictest = "0.29.1"`** (hay que usar la misma
  versión para el trait `AsyncDB`). Features: `avro`, `backtrace`, `parquet_encryption`, `postgres`, `substrait`.
  El `.crate` publicado **incluye `test_files/` (504 ficheros `.slt`, 7,2 MB)**; no incluye los datos de los submódulos.
- API pública (`src/lib.rs`): `DataFusion`, `DataFusionSubstraitRoundTrip`, `Postgres`, `DFColumnType`, `DFOutput`, `DFSqlLogicTestError`,
  `CurrentlyExecutingSqlTracker`, `convert_batches`, `convert_schema_to_types`, `TestContext`, `TestFile`, filtros (`should_skip_file`, `should_skip_record`),
  utilidades (`setup_scratch_dir`, `value_normalizer`, `read_dir_recursive`, `df_value_validator`, `is_spark_path`).
  - `DataFusion::new(ctx: SessionContext, relative_path: PathBuf, pb: indicatif::ProgressBar) -> Self`
    (`src/engines/datafusion_engine/runner.rs:47`), `.with_currently_executing_sql_tracker(..)`, `.with_config_change_errors(Arc<Mutex<Vec<String>>>)`,
    `impl sqllogictest::AsyncDB for DataFusion` (l.141). Guarda la config inicial y puede validar que el fichero no la cambió.
  - `TestContext::new(SessionContext)` (`src/test_context.rs:95`), `TestContext::try_new_for_test_file(&Path)` (l.107), `session_ctx()`.
- Cómo construye el contexto el runner propio (`bin/sqllogictests.rs`, `run_test_file`, l.492–543): `TestContext::try_new_for_test_file`
  = `SessionStateBuilder::new().with_config(SessionConfig::new().with_target_partitions(4)).with_runtime_env(Arc::new(RuntimeEnv::default()))
  .with_default_features()` (+ `with_spark_features()` en rutas spark, type planner para `cast_extension_type_metadata.slt`) y registra
  tablas/UDF ad hoc por nombre de fichero (`joins.slt`, `map.slt`, `cte.slt`, `aggregate.slt`, `async_udf.slt`, …). Después
  `sqllogictest::Runner::new(|| async { Ok(DataFusion::new(test_ctx.session_ctx().clone(), relative_path, pb).with_…) })`,
  `add_label("Datafusion")`, `with_column_validator(strict_column_validator)`, `with_normalizer(value_normalizer)`, `with_validator(validator)`.
  `run_test_file`/`run_file_in_runner` están en el **bin**, no en la lib: un runner propio tiene que copiarlos.
  Ojo: para correr el corpus contra nuestra sesión hay que reproducir también esos registros por fichero.
- Corpus: `datafusion/sqllogictest/test_files/` (en el `.crate`: `test_files/`). Rutas relativas a datos en los `.slt`
  (conteo de apariciones): `../core/tests/data` (115), `../../testing/data` (65, submódulo **apache/arrow-testing** en `testing/`),
  `../../parquet-testing/data` (21, submódulo **apache/parquet-testing**), `../datasource-arrow/tests/data` (10), `../core/tests/tpch-csv` (6).
  Constante `DATAFUSION_TESTING_TEST_DIRECTORY = "../../datafusion-testing/data/"` (l.60, submódulo **datafusion-testing**, para los ficheros sqlite/pg_compat).
  README: `git submodule update --init --remote --recursive`, `cargo test --test sqllogictests`, filtro por nombre, `--complete`, `--timing-summary`.
  Es decir: correr el corpus exige el árbol de apache/datafusion en el tag, no sólo el `.crate`.
- Duración: en el CI de apache/datafusion (`rust.yml`, run 41034365266 de `main`, 2026-10-30), el paso «Run SQLLogicTests with coverage»
  (`cargo xtask ci step test slt-coverage`) tardó **≈3 min** (09:42:28→09:45:26) en un runner de 16 vCPU (`runs-on … cpu=16`), con la
  compilación ya hecha en el paso anterior. **[sin verificar]** el tiempo en nuestra máquina y sin cobertura.

## C6 · Versiones en tiempo de ejecución

- `datafusion::DATAFUSION_VERSION: &str = env!("CARGO_PKG_VERSION")` (`datafusion/core/src/lib.rs:770`).
- `ballista_core::BALLISTA_VERSION: &str = env!("CARGO_PKG_VERSION")` (`ballista/core/src/lib.rs:30`), `ballista_core::print_version()`.
- `ballista_core::BALLISTA_PROTOCOL_VERSION: u32 = 3` (l.47).
- `ballista_core::version` (`ballista/core/src/version.rs`): `BALLISTA_VERSION_HEADER = "ballista-version"` (l.49, metadato gRPC),
  `FIRST_MAJOR_WITH_VERSION_HEADER: u64 = 55`, `check_compatibility(client: Option<&str>, scheduler: Option<&str>) -> Result<(), String>`
  (misma mayor = compatible; un lado sin cabecera = anterior a 55), `insert_version_header(&mut MetadataMap)`, `version_from_metadata(&MetadataMap)`.
- HTTP del scheduler (`scheduler_process.rs`, `with_version_headers`): `Server: ballista/{BALLISTA_VERSION}` y `x-powered-by: datafusion/{DATAFUSION_VERSION}`.
  Scheduler y ejecutor registran al arrancar «Ballista … v{BALLISTA_VERSION} (DataFusion v{DATAFUSION_VERSION})».
- `SessionState::version()` (`session_state.rs:992`) devuelve la versión de DataFusion.

## C7 · Cancelación y pánicos en DataFusion 55

- `datafusion_common_runtime::SpawnedTask<R>` (`datafusion-common-runtime/src/common.rs:35`): envuelve un `tokio::task::JoinHandle`;
  **`Drop` llama a `abort()`** (l.108) — cancelar = soltar el stream/futuro. `spawn` / `spawn_blocking` (este último sólo se aborta si
  no empezó). `join()` → `Result<R, JoinError>`; `join_unwind()` / `join_unwind_mut()` **relanzan el pánico** con
  `std::panic::resume_unwind` (l.87–96); cancelación → `JoinError` + `warn!`.
- `datafusion_common_runtime::JoinSet<T>` (`join_set.rs:32`): envoltorio de `tokio::task::JoinSet` con trazado (`set_join_set_tracer`,
  `trace_future`, `trace_block`); `spawn`, `spawn_on(task, &Handle)`, `spawn_blocking(_on)`, `join_next`, `abort_all`, `shutdown`, `join_all`.
  El de tokio aborta todo al soltarse.
- Pánico dentro de un operador — **en general NO se convierte en `DataFusionError`, se propaga como pánico** al hilo que consume:
  - `RecordBatchReceiverStreamBuilder::build` (`datafusion-physical-plan/src/stream.rs:133-160`): si una tarea del `JoinSet` termina con
    `JoinError::is_panic()`, `std::panic::resume_unwind`; si es cancelación, `exec_err!("Non Panic Task error: {e}")`.
  - `collect_partitioned` (`execution_plan.rs:1800-1829`): igual, `resume_unwind`.
  - Excepción: `BufferExec` (`buffer.rs:405-433`) hace `catch_unwind` y lo devuelve como `internal_err!("BufferExec input stream panicked: {msg}")`.
  - `DataFusionError::ExecutionJoin(Box<JoinError>)` (`datafusion-common/src/error.rs:136`) existe para errores de join que sí se mapean.
  Consecuencia para nosotros: el servidor debe envolver la ejecución en `catch_unwind` (o en una tarea propia y leer `JoinError`).
- Ballista ya lo hace por tarea: `ballista-executor/src/executor.rs:249` — `AssertUnwindSafe(task).catch_unwind().await`;
  pánico → `BallistaError::Internal(msg)`, aborto → `BallistaError::Cancelled`. También `execution_loop.rs:385-398`.

---

## Lo que decide (resumen)

- Trío compatible en arrow/parquet: DataFusion 55.x = 59.2.0, Ballista 55.0.0 = 59.2.0 y `datafusion ^55`, iceberg-rust main = 59.2.
- **MSRV del trío = 1.95** (lo impone iceberg-rust; DataFusion y Ballista piden 1.94.0).
- object_store se queda en 0.13.x (DataFusion 55 y Ballista 55 piden `0.13.2`), aunque exista 0.14.
- iceberg-rust ya no trae integración con DataFusion: o se usa apache/datafusion-iceberg (fijado a una rev anterior a #3483) o se escribe la nuestra.
- Scheduler y ejecutor de Ballista se arrancan como librería (`start_server`, `start_executor_process`); las extensiones de config hay que
  registrarlas en ambos `ConfigProducer` porque el lado remoto descarta en silencio las claves que no conoce.
