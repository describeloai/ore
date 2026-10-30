# S0 · el previo del ORE Runtime

**Para qué.** Saber, antes de escribir una línea del suelo (0081 §7.4), **qué** se construye, **cómo**,
**dónde** y **en qué orden**, sin sorpresas. Cada pregunta tiene su respuesta medida o leída en la fuente, y
lo que decide.

**Estado:** **hecho** en local (2026-10-30), salvo D (victor). Sobre `origin/main` 10092ad3, rama
`datafusion-fundamentos`. El spike está en `pruebas-de-fuego/s0-ore-runtime/` (fuera del workspace) y las
notas del upstream, con sus fuentes, en [`s0-upstream.md`](s0-upstream.md). El camino de construcción que
sale de aquí está en 0081 §7.4.

## Las preguntas

### A · Las versiones y el grafo

| | pregunta | cómo se contesta | decide |
|---|---|---|---|
| A1 | dependencias exactas de DataFusion 55.x (arrow, parquet, object_store, sqlparser, tokio) y su MSRV | el `Cargo.toml` publicado | el trío |
| A2 | Ballista 55.0: qué DataFusion fija, qué crates publica, qué features (flight-sql, keda-scaler, prometheus), y si el scheduler y el ejecutor se pueden usar como librería | el `Cargo.toml` y el código publicados | cómo nacen los binarios |
| A3 | iceberg-rust en una rev posterior a #3483: arrow, parquet, opendal y MSRV | el `Cargo.toml` de esa rev | la rev |
| A4 | ¿compila el trío junto, y sin duplicados? | un spike en Docker, `cargo tree -d` | si el camino es posible sin bifurcar |
| A5 | nuestro `rust-toolchain.toml` frente al MSRV del trío | el fichero | si sube la toolchain |

### B · El workspace de ORE

| | pregunta | cómo se contesta | decide |
|---|---|---|---|
| B1 | quién usa arrow, parquet, iceberg, sqlparser, object_store y tokio, con qué versión y qué features | `Cargo.toml` de cada crate | el alcance de la subida |
| B2 | qué rompe subir el workspace entero al trío | `cargo check` del workspace con las versiones nuevas | subir todo o vivir aparte un tiempo |
| B3 | qué comprueba el guardián de dependencias del CI y cómo se le añade la regla «un solo arrow-rs, un solo DataFusion» | `ci/` | la guarda |
| B4 | cómo entra un crate y un binario nuevos en el CI (`compilar-binarios.sh`), en las imágenes (`Dockerfile`) y en la prueba de la huella; musl o glibc; asignador | `ci/`, `Dockerfile` | dónde vive y cómo se construye |
| B5 | cuánto cuesta compilar el trío: tiempo y disco | el spike | el impacto en el CI y en la máquina |

### C · Las APIs del upstream

| | pregunta | cómo se contesta | decide |
|---|---|---|---|
| C1 | la sesión en 55: `SessionStateBuilder`, `datafusion-session`, y cuánto cuesta clonar una `SessionState` | código y el spike | la forma de la plantilla |
| C2 | Ballista: los nombres exactos de los ganchos (config, runtime, session builder, codecs), cómo se arranca un scheduler y un ejecutor con nuestro constructor, y la constante del protocolo | el código de 55.0 | los tres papeles |
| C3 | los dos runtimes: el patrón vigente en 55 para enrutar la E/S del object store a otro runtime | el ejemplo `thread_pools` y object_store | el zócalo de ejecución |
| C4 | ¿`target_partitions` por defecto y `available_parallelism` respetan la cuota de CPU del cgroup v2? ¿Cómo detecta Ballista la memoria? | el código y el spike con `--cpus` | el dimensionado |
| C5 | `sqllogictest`: ¿se publica `datafusion-sqllogictest`?, ¿cómo se enchufa una sesión propia?, ¿de dónde sale el corpus de la versión exacta y cuánto tarda? | el crate y el repositorio | el arnés |
| C6 | qué versión publica cada pieza en tiempo de ejecución (`DATAFUSION_VERSION`, la de Ballista) | el código | cómo se ve la versión |
| C7 | cancelación y pánicos: qué garantiza DataFusion 55 (`SpawnedTask`, `JoinSet`) | el código | el aislamiento por consulta |

### D · Lo que se mide en victor

Con el binario del spike, en un pod de un pool dedicado (nunca el nodo de sistema):

- el coste de clonar la sesión plantilla;
- `target_partitions` frente a la cuota del pod;
- la latencia de la salud con la CPU saturada.

## Las respuestas

### A · Las versiones y el grafo

- **A1 · DataFusion 55.0, 55.1 y 55.2 tienen las mismas dependencias:**
  - arrow y parquet 59.2;
  - object_store 0.13.2, sin features por defecto; la 0.14 ya existe, pero DataFusion 55 no la usa;
  - sqlparser **0.62.0**;
  - tokio 1.52;
  - MSRV 1.94 y edición 2024.
- **A2 · Ballista 55.0.0** (tag `d5379965`, MSRV 1.94):
  - pide `datafusion = "55"`, un rango, no `=`; el spike resolvió 55.2.0;
  - publica `ballista`, `-core`, `-scheduler`, `-executor`, `-cli`, `-api-types`, `-flight-sql` e `-history`;
  - **el scheduler y el ejecutor se arrancan como librería**:
    `ballista_scheduler::scheduler_process::start_server(BallistaCluster, SocketAddr, Arc<SchedulerConfig>)` y
    `ballista_executor::executor_process::start_executor_process(Arc<ExecutorProcessConfig>)`;
  - ganchos del scheduler: `override_config_producer`, `override_session_builder`,
    `override_logical_codec`, `override_physical_codec`;
  - ganchos del ejecutor: los mismos, más `override_runtime_producer` y `override_function_registry`;
  - el registro de almacenes no es un gancho: viaja dentro del `RuntimeEnv`;
  - protocolo `BALLISTA_PROTOCOL_VERSION = 3`;
  - la feature por defecto `build-binary` del ejecutor arrastra mimalloc y axum.
- **A3 · iceberg-rust `main` en `4dd8c946`**, que lleva #3483 (`5c271576`):
  - workspace 0.10.1, **MSRV 1.95**, arrow y parquet 59.2, opendal 0.58;
  - el crate `iceberg` **ya no depende de DataFusion**;
  - trae su propio reparto E/S/CPU: `Runtime::new_with_split`.
- **A4 · El trío compila junto** (medido):
  - un solo arrow-rs (59.3.2), un solo parquet (59.3.2), un solo DataFusion (55.2.0), un solo object_store
    (0.13.2) y un solo reqwest;
  - los duplicados que quedan son menores: hashbrown, rand, zstd 0.13/0.14, thiserror 1/2, syn 2/3;
  - **glibc:** release en 14 min con 16 núcleos, `target` de 2,0 GB, binario de 178 MB sin despojar;
  - **musl (Alpine, el target de las imágenes):** compila, en 36 min, y el binario pesa 166 MB.
- **A5 · La toolchain:** `rust-toolchain.toml` dice `stable`, que hoy es 1.99.1 (≥ 1.95). Pero la huella
  del CI hashea el texto «stable», así que una imagen `ore-deps` cacheada conserva el stable con que se
  construyó. Si esa es < 1.95, iceberg-rust no compila.

### B · El workspace de ORE

- **B1 · Quién usa qué:**
  - unos 18 crates usan arrow 58: ore-store, ore-read-objetos, ore-conector-kit, ore-piramide,
    ore-teselas, ore-arrow, ore-federation y los conectores;
  - parquet 58 lo usan 5;
  - `iceberg = "0.10.1"`, sólo ore-store;
  - sqlparser 0.63, sólo ore-core, por `[workspace.dependencies]`;
  - nadie usa object_store, opendal ni datafusion.
- **B2 · Subir el workspace entero al trío** (medido con `cargo check --workspace` sobre una copia). Rompe
  **tres crates**, y todo lo demás compila con arrow 59 sin cambios:
  - **ore-store**, 9 errores, por la API de iceberg-rust: `ManifestFile::load_manifest`,
    `MetadataLocation::new_with_metadata`/`with_new_metadata`, y `FileWrite::close`, que ahora devuelve
    `FileMetadata`;
  - **ore-read-objetos**, 1 error: `LogicalType::Geometry` de parquet 59 pasa a variante de tupla;
  - **ore-core**, 2 errores, pero sólo si se **baja** sqlparser a 0.62: le falta `OrderBySort`. Con 0.63 en
    ore-core y 0.62 en DataFusion compila, con dos sqlparser en el grafo.
- **B3 · El guardián de dependencias** es la prueba `crates/ore-cli/tests/dependencias.rs`, no cargo-deny.
  Lee `Cargo.lock` como texto. Con el trío **dentro** del workspace (775 paquetes en el lock, frente a
  671) **pasa 12/12**:
  - `CIERRE` sigue en 193;
  - `ore-serve` sigue sin poder alcanzar iceberg, hyper ni reqwest; el runtime tiene que ser otro binario.

  La regla «un solo arrow-rs, parquet, DataFusion y object_store» se añade como una prueba más en ese
  fichero, con sqlparser como excepción nombrada.
- **B4 · Cómo entra un binario:**
  - en el CI, por `ci/compilar-binarios.sh` (`PAQUETES` y `TODOS`, y la comprobación de arranque de 3 s);
  - en las imágenes, por el `Dockerfile`, que compila todo en `rust:1.90-alpine` para
    **x86_64-unknown-linux-musl estático**, con mold, cargo-chef y un `COPY --from=bin` por binario;
  - en las huellas, por `huella-de-los-binarios.py` y sus pruebas, más `huella-de-la-imagen.py` si hay
    imagen nueva;
  - el asignador es **mimalloc sólo con musl** (ore-teselas, ore-piramide, ore-read-hbase, ore-read-phoenix:
    67 s → 7,4 s);
  - **el perfil `release`, y con él `imagen`, lleva `panic = "abort"`.**
- **B5 · Coste:**
  - el `construir` del CI tarda ~3 min en un cambio normal;
  - un crate nuevo cambió la receta y costó una compilación en frío de 26 min (antes de K8);
  - un cocinado completo de dependencias, ~20 min con 4 vCPU;
  - el trío añade, en local con 16 núcleos, 14 min en glibc y 36 en musl.
- **Nombres:** `ore-runtime` está libre. `ore-motor` está ocupado: es el motor Python de `ore_motor.py`,
  con su Deployment y su destino en el CI.
- **Versión:** no hay `build.rs`. `ore` sella con `option_env!("ORE_COMMIT")`, y `ore-serve` publica
  `GET /version`.

### C · Las APIs del upstream

- **C1 · La sesión:**
  - **clonar una `SessionState` con las funciones por defecto cuesta 36,6 µs en glibc y 74,3 µs en musl sin
    mimalloc**; construirla de cero, 190 µs y 350 µs (medido);
  - `SessionContext::state()` clona entero; `state_ref()`, no;
  - el trait `Session` de 55 exige `catalog_list()` y trae cuatro métodos con valores por defecto inútiles
    (`query_planner`, `optimize`, `physical_optimizers`, `statistics_registry`). Por eso conviene usar
    `SessionState`, no escribir un `Session` a mano.
- **C2 · Ballista:**
  - los ganchos y los puntos de entrada están en A2;
  - **las claves desconocidas sólo se rechazan en el cliente.** En el scheduler y en el ejecutor,
    `update_from_key_value_pair` falla en `debug!` y la clave se pierde sin error. Una `ConfigExtension` de
    ORE tiene que registrarse en los dos `config_producer`, o se evapora;
  - no existe `SessionStateBuilder::with_ballista`: se usa `SessionConfig::new_with_ballista()` /
    `upgrade_for_ballista()`.
- **C3 · Los dos runtimes:**
  - el ejemplo `query_planning/thread_pools.rs` (tag 55.2.0) ejecuta el plan en un runtime de CPU sin
    `enable_io`, y construye el almacén con
    `.with_http_connector(SpawnedReqwestConnector::new(io_handle))` (object_store 0.13, feature `cloud`);
  - iceberg-rust trae su propio `Runtime::new_with_split(io, cpu)`;
  - DataFusion no trae ayuda propia.
- **C4 · El cgroup:**
  - `target_partitions` sale de `std::thread::available_parallelism()`, que **respeta `cpu.max` y redondea
    hacia abajo**. Medido con `--cpus`: 2 → 2, 4 → 4 y **1,5 → 1**;
  - Ballista toma los vcores igual, y su pool es el 70 % de min(host, `memory.max`).
- **C5 · `sqllogictest`:**
  - `datafusion-sqllogictest` 55.x está publicado, con su driver `DataFusion::new(SessionContext, PathBuf,
    ProgressBar)`;
  - el corredor de ficheros y la preparación por fichero (`TestContext`) no son reutilizables: hay que
    copiarlos;
  - el corpus son 504 `.slt` y necesita los submódulos `testing`, `parquet-testing` y `datafusion-testing`;
  - tarda ~3 min con 16 vCPU.
- **C6 · Versiones en tiempo de ejecución:** `datafusion::DATAFUSION_VERSION`,
  `ballista_core::BALLISTA_VERSION` y la cabecera `ballista-version`. iceberg-rust no publica su rev: la
  sella la compilación.
- **C7 · Pánicos:**
  - DataFusion **vuelve a lanzar** el pánico de un operador; no lo convierte en `DataFusionError`, salvo
    `BufferExec`;
  - `SpawnedTask` aborta al soltarse, así que cancelar es soltar el stream;
  - Ballista captura los pánicos por tarea.
  - **Con `panic = "abort"`, un pánico de un operador tumba el proceso entero.**

### D · Lo que se mide en victor

Pendiente. Se mide con el binario del suelo (S5 en 0081 §7.4), no con el spike, para medirlo ya con
mimalloc y con el perfil definitivo.

### Lo que el S0 decide

1. **El camino es posible sin bifurcar nada**, y con un solo Arrow en todo el árbol.
2. **El workspace entero sube al trío:** arrow y parquet 59 e iceberg-rust por rev. Cuesta arreglar 10
   errores en dos crates. sqlparser 0.63 se queda en ore-core, como excepción nombrada, hasta que los
   productos vuelvan a nacer.
3. **El runtime es un binario propio** (`ore-runtime`), nunca dentro de `ore-serve`, y lo exige el
   guardián.
4. **Necesita su propio perfil, con `panic = "unwind"`**, y aislar cada consulta en su tarea. Sin eso, la
   propiedad 6 del suelo es imposible.
5. **Con musl, mimalloc es obligatorio:** sin él, la sesión cuesta el doble.
6. **Una sola función construye la sesión**, y lo exige Ballista: una clave de ORE que no esté registrada
   en los tres papeles se pierde en silencio.
7. **Las CPU de un pod, en números enteros:** la cuota fraccionaria se redondea hacia abajo.
8. **La toolchain, fijada (≥ 1.95):** con «stable», una imagen cacheada puede compilar con una versión
   vieja.
