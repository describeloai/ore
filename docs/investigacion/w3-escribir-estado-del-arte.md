# W3.6c · el estado del arte de escribir en un lago (2026-09-20)

Antes de medir y de escribir la spec del verbo **escribir** (0031 §9, el segundo de los cuatro
verbos del puesto) miramos cómo escribe hoy la industria en un entorno como el nuestro: tablas
Iceberg en un bucket, un catálogo que decide qué versión es la vigente, varios lenguajes
escribiendo, identidad por pod y ramas. Lo que ya está asentado —§10 «todo es un dataset»,
la copia como tabla Iceberg (W3.6a), el swap del puntero por `ore-serve` con su CAS (W3.6b)—
no se pone en cuestión; lo que se coteja es **el verbo**: quién escribe qué, con qué identidad,
cómo se confirma, qué pasa cuando falla a medias, cómo se declara la retención, y cómo se
ramifica.

Fuentes primarias (la spec del catálogo REST de Iceberg, las notas de Delta 4.1, la documentación
de Foundry, Polaris, BigLake, DuckDB, PyIceberg, Icebird), enlazadas al final. Donde la fuente da
un detalle concreto, va el detalle.

## 1 · Cómo escribe cada uno

| | quién decide la versión vigente | cómo se confirma | identidad al escribir | idempotencia | ramas | retención |
|---|---|---|---|---|---|---|
| **Iceberg · catálogo REST** (la *lingua franca*: Polaris, Lakekeeper, BigLake, S3 Tables, Unity) | el catálogo: `metadata_location` por tabla | `POST …/tables/{t}` con **`requirements` + `updates`**: `assert-ref-snapshot-id` («`main` sigue apuntando al snapshot desde el que partí»), `assert-table-uuid`, `assert-create`, `assert-current-schema-id`…; `add-snapshot`, `set-snapshot-ref`, `add-schema`, `set-properties`…; el servidor valida y **escribe él el `metadata.json` siguiente**; 409 `CommitFailedException` = refresca y reintenta; **500/502/504 `CommitStateUnknownException` = no reintentes a ciegas: mira**; `POST /transactions/commit` = varias tablas en un commit | **credenciales prestadas** (`X-Iceberg-Access-Delegation: vended-credentials`): al cargar la tabla el catálogo devuelve un token corto acotado al prefijo de esa tabla (STS con política inline en S3, **token *downscoped* por *Credential Access Boundary* en GCS**, SAS en Azure) más un endpoint de refresco | cabecera **`Idempotency-Key`** (UUID v7): la misma clave = la misma operación lógica; el servidor «finaliza y repite» 2xx y 4xx deterministas y **no** finaliza 5xx | **refs** de la tabla (`branch`/`tag` en `set-snapshot-ref`) con retención por ref (`max-ref-age-ms`); y **WAP** (write-audit-publish): escribir en una rama, auditar, *fast-forward* a `main` (sin copiar: metadatos) | **propiedades de la tabla**: `history.expire.max-snapshot-age-ms`, `history.expire.min-snapshots-to-keep`, `history.expire.max-ref-age-ms`; y reintentos: `commit.retry.num-retries`, `commit.retry.min-wait-ms` |
| **Delta Lake 4.1 · *catalog-managed tables*** (Unity «catalog commits», GA 2026) | el catálogo: «*system of record for table identity, discovery and authorization*» | el escritor deja el commit en `_delta_log/_staged_commits/` y **pide al catálogo que lo ratifique** (o lo manda *inline*); lo ratificado se *publica* al bucket después; el lector pregunta al catálogo lo ratificado y completa con lo publicado. Motivo declarado: el *put-if-absent* del bucket trataba el commit como «*opaque blob*»: no podía validar, imponer políticas ni coordinar varias tablas | la del catálogo (Unity), con *credential vending* propio | por `txnAppId` + `txnVersion` en el log: el mismo par se ignora (el patrón *exactly-once* de Structured Streaming) | por catálogo | `delta.logRetentionDuration`, `delta.deletedFileRetentionDuration` |
| **Foundry · datasets** | el servicio: un dataset = ficheros + **transacciones** + ramas + esquema | **transacción abierta → commit / abort**; cuatro tipos: `SNAPSHOT` (sustituye la vista), `APPEND` (añade ficheros; **si toca uno existente, el commit falla**), `UPDATE`, `DELETE`; el **esquema es metadato de la transacción**; `Output.write_dataframe` en un *transform* abre y cierra una transacción; *replace* / *modify* (incremental) | la del *build*; el código nunca ve el bucket (el sidecar) | por *build*: la misma entrada, el mismo JobSpec | ramas del dataset **atadas a la rama del código**; ***fallback branch***: una rama lee `master` para lo que aún no tiene; ramas protegidas | políticas de retención por *transaction selectors* |
| **Nessie** | el catálogo, **con semántica de git**: un commit mueve N tablas, una rama es una lista de commits | commit contra el `hash` esperado de la rama (CAS) | la del motor | — | ramas y tags **del catálogo** (no de la tabla): dev/staging/prod sobre el mismo lago | por catálogo |
| **lakeFS** | versiona **los datos** (objetos), no el catálogo | commit/branch/merge sobre el bucket | — | — | ramas del bucket | — |
| **DuckLake** | una **base de datos SQL** (DuckDB, SQLite, Postgres): esquemas, snapshots, listas de ficheros, estadísticas | una transacción de la base de datos (multi-tabla nativa) | — | — | — | — |

**Los escritores por lenguaje** (lo que hay para escribir Iceberg desde un puesto):

| | escribe Iceberg | necesita | límites |
|---|---|---|---|
| **Java · `org.apache.iceberg`** | sí, completo (la referencia) | un `Catalog` (`RESTCatalog`, o uno propio: `TableOperations.commit(base, metadata)`) | peso de dependencias (ya medido en W3.4/W3.5b) |
| **Python · PyIceberg** | `append`, `overwrite`, `overwrite_filter`, `upsert`, `delete`, evolución de esquema, transacciones | **un `Catalog`** (`StaticTable` desde un `metadata.json` **es sólo lectura**); un catálogo propio implementa `commit_table(table, requirements, updates)` | — |
| **Rust · iceberg-rust 0.10** | sí (lo medido en W3.6a: 2,6 M filas/s, `TableUpdate::apply`, escribe el `metadata.json` siguiente) | nada más: **ya es nuestro `ore-store`** | v3 parcial |
| **DuckDB · extensión `iceberg`** (1.4 LTS) | `CREATE TABLE`, `INSERT`; `DELETE`/`UPDATE` desde 1.4.2 (sólo *merge-on-read*, tablas sin partición ni orden) | **sólo por catálogo REST** (`ATTACH … TYPE iceberg`); la documentación sólo enseña S3; **sin evolución de esquema** («planned») | no escribe por `metadata.json` suelto; no hay GCS documentado |
| **Node · `iceberg-js`** (Supabase, 1.0) | **no**: sólo el catálogo REST (namespaces, tablas, commits); «*cannot read or write Parquet*» | un catálogo REST | — |
| **Node · Icebird** (hyparam) | **experimental**: `icebergCreateTable`, `icebergAppend`, `icebergDelete`, `icebergUpdateSchema`; v2 (+ *deletion vectors* de v3) | catálogo de ficheros o REST; S3 con su propia firma SigV4 | «*experimental*»; los *rewrites* no se reintentan en conflicto |

## 2 · Lo que se repite (las buenas prácticas)

1. **El catálogo decide, y decide con un CAS explícito.** En todos: `assert-ref-snapshot-id`
   (Iceberg REST), el `hash` esperado (Nessie), la ratificación (Delta), la transacción (Foundry).
   El escritor dice *desde dónde partió*; el catálogo compara y **escribe él** —o ratifica— la
   versión nueva. **Tenemos** exactamente eso en W3.6b (`esperado` → 409 con `actual`); la
   diferencia es que hoy el escritor escribe el `metadata.json` y `ore-serve` sólo mueve el
   puntero. Iceberg REST y Delta 4.1 han movido *también la escritura del metadato* al catálogo,
   y dan el motivo: **un commit opaco no se puede validar**; si el catálogo lo escribe, puede
   comprobar el esquema, imponer políticas y coordinar varias tablas.
2. **La escritura es una transacción con dos desenlaces, y un tercero que no es ninguno.**
   Commit / abort (Foundry, PyIceberg) y, en REST, **`CommitStateUnknown`** (5xx): el cliente
   *no* puede reintentar sin mirar antes qué versión hay. Nuestra medida de W3.6b lo tocó desde el
   otro lado (la carrera daba 502 y era 409); la spec del verbo tiene que decir qué hace `write()`
   tras un 5xx o un *timeout*: **`GET /datasets/{ns}/{n}` y comparar**.
3. **Idempotencia por una clave que viaja con el commit.** `Idempotency-Key` (REST), `txnAppId`
   + `txnVersion` (Delta), `flink.job-id` + `max-committed-checkpoint-id` en el resumen del
   snapshot con validación de ancestría (Flink). **Tenemos** el digest de la cabecera en el
   resumen (`ore.cabecera`) y `confirmar` idempotente para el mismo `metadata_location`; falta
   que `write()` lleve una **clave de operación** para que la misma escritura repetida (la celda
   que se reejecuta, el reintento tras un *timeout*) **no deje dos snapshots iguales**.
4. **El esquema nace con la primera escritura y evoluciona por id.** Foundry lo guarda como
   metadato de la transacción; Iceberg lo evoluciona con `add-schema` / `set-current-schema`;
   PyIceberg lo hace en la misma transacción que el `append`. **Tenemos** las dos cosas
   (`columnas` en `confirmar`, `esquema_deseado` por id en W3.6a).
5. **La retención se declara en la tabla, no en el cron.** `history.expire.*` son propiedades de
   la tabla en Iceberg (y por ref); Foundry las declara por dataset. Nuestro CronJob
   (`53-el-mantenimiento.yaml`) lleva la edad como variable de entorno: **el estándar es que
   `ore datasets --recoger` lea `history.expire.max-snapshot-age-ms` y
   `history.expire.min-snapshots-to-keep` de las propiedades de cada tabla** (que `write()` y la
   `Table` del árbol declaran), y que la variable de entorno sea sólo el valor por defecto.
6. **Nadie escribe con la identidad del bucket entero.** Polaris, BigLake, S3 Tables, Unity:
   una credencial **corta y acotada al prefijo de la tabla**, prestada por el catálogo al cargar
   la tabla. En GCS es un mecanismo nativo: **token *downscoped* por *Credential Access
   Boundary*** (un intercambio en `sts.googleapis.com` con el token del pod y una regla
   «`storage.objects.create` sólo bajo `ore/v2/datasets/<p>_<t>/`»). Hoy el pod escribe como
   `driver` (`objectAdmin` sobre todo el bucket); 0031 «W3.6b hecho» apuntaba a una cuenta con
   `objectCreator` sobre `datasets/` por el aprovisionador — **el estándar es más fino y no
   necesita al aprovisionador**: `ore-serve` presta el token acotado a *esa* tabla.
7. **Las ramas son del catálogo, y la publicación es *fast-forward*.** Nessie y Foundry
   ramifican **el catálogo** (todas las tablas a la vez, con el código); Iceberg ramifica *la
   tabla* (refs). WAP en los tres: escribir en una rama, auditar, publicar sin copiar. Nuestro
   catálogo es git, así que **una rama del árbol ya es una rama del catálogo** (Nessie): la
   sesión en una rama escribe punteros en su rama y leer cae a `main` (el *fallback* de Foundry,
   §4); publicar = el merge del puntero. **No hacen falta refs de Iceberg** para tener WAP, y
   evitarlas mantiene un solo modelo de rama.
8. **Un commit puede mover varias tablas.** `commitTransaction` (REST), Nessie, DuckLake. En git
   es gratis: **un commit con N punteros** (`ore datasets --confirmar` acepta uno; la spec debe
   decir si `write()` de varias salidas de una celda confirma en uno).
9. **Anexar no toca lo que hay; sobrescribir sustituye la vista.** `APPEND` de Foundry falla si
   pisa un fichero; `Sobrescribir` de Iceberg es un snapshot que retira y añade. **Tenemos** los
   dos (`Anexar`/`Sobrescribir`, `write` del `Storage` niega objetos existentes). *Upsert*
   (Foundry `UPDATE`, PyIceberg `upsert` con campos identificadores, *equality deletes* /
   *deletion vectors* v3) es el tercer modo y ya está aparcado con nombre.
10. **Se escribe por lo declarado, no por nombre suelto.** Dagster (*software-defined assets*
    + IO managers), Foundry (`Output` del *transform*), Iceberg REST (`stage-create`: la tabla se
    prepara y se crea en el commit). Coincide con §9 «declarar» y con 0031 §4 «escribir es
    publicar una salida con nombre».

## 3 · Lo que valida la tesis, lo que la corrige y lo que trae aire

**Valida.** «El árbol es el catálogo y el swap es el commit» es *exactamente* hacia donde ha ido
la industria en 2026: Iceberg REST (el catálogo valida `requirements` y escribe), Delta 4.1
(«*catalog-managed*», y el motivo: el *put-if-absent* del bucket es un commit opaco), DuckLake
(el catálogo es una base de datos), Nessie (el catálogo es git). Nuestro `confirmar
{metadata_location, esperado}` es un `updateTable {requirements: [assert-ref-snapshot-id],
updates: [add-snapshot, set-snapshot-ref]}` con otro nombre; el 409 con `actual` es
`CommitFailedException`; el puntero en git es el `metadata_location` del catálogo; y el
`git log` del puntero es lo que Nessie vende.

**Corrige (tres cosas concretas para la spec).** (a) **`CommitStateUnknown`**: el verbo tiene que
tratar el 5xx / *timeout* como «mira antes de reintentar», no como fallo. (b) **La clave de
operación**: sin ella, una celda reejecutada deja dos snapshots. (c) **La retención en la tabla**:
propiedades `history.expire.*`, no una variable del CronJob.

**Aire fresco (dos ideas que cambian el plan de W3.6c).**

- **`ore-serve` como catálogo REST de Iceberg, además de `confirmar`.** Con cuatro rutas
  (`GET /v1/config`, `GET …/namespaces/{ns}/tables/{t}` = el puntero + su `metadata.json`,
  `POST …/tables/{t}` = `requirements` + `updates` → `ore-store confirmar(tabla, updates,
  requirements)`, que **ya aplica `TableUpdate` y escribe el `metadata.json` siguiente**, y el
  swap de W3.6b) **todos los escritores del cuadro escriben sin SDK nuestro**: PyIceberg
  (`RestCatalog`), Java (`RESTCatalog`), DuckDB (`ATTACH`), Icebird, `iceberg-js`, y mañana
  Spark/Trino/Flink (el motor distribuido que se aparca). No sustituye a `write()` en los tres
  lenguajes —el verbo del puesto sigue siendo `write("p.salida", t)`— pero decide **cómo se
  implementa `write()`**: en Python y Java, *sobre* el catálogo REST del inquilino (PyIceberg e
  Iceberg Java hacen el resto); en Node, donde no hay escritor de producción, **`write()` manda
  la tabla Arrow (IPC) al agente y el agente escribe con `ore-store`** (el escritor de Rust que
  ya está medido). Una verdad común (Arrow) y un solo camino de commit (el catálogo).
- **El token acotado a la tabla**, prestado por el catálogo al cargar la tabla, como en Polaris
  y BigLake, con el mecanismo nativo de GCS (*Credential Access Boundary*). Quita la cuenta de
  puesto del aprovisionador y la regla «`objectCreator` sobre `datasets/`»: el puesto escribe
  sólo bajo el prefijo de la tabla que está escribiendo, y sólo mientras dura el token.

**Lo que no se adopta (con motivo).** Refs de Iceberg como ramas (dos modelos de rama; git ya
ramifica el catálogo). Escribir el `metadata.json` desde el puesto (el catálogo lo escribe, como
en REST y Delta 4.1: así `confirmar` puede validar el esquema *antes* de que exista y el puesto no
necesita permiso de escritura sobre `metadata/`). Iceberg v3 por ahora (iceberg-rust 0.10 lo
tiene parcial; 0032 no promete `ns`; se revisa cuando el lector del puesto —DuckDB— lo lea).

## 4 · Lo que hay que medir antes de la spec (`medida-w3-escribir.py`)

1. **El catálogo REST mínimo sobre `ore-serve`** (`config`, `loadTable`, `updateTable` con
   `assert-ref-snapshot-id`): ¿PyIceberg `append` e Iceberg Java escriben contra él **sin
   parches**? ¿DuckDB `ATTACH … TYPE iceberg` + `INSERT`? Cuántas peticiones por escritura y
   cuánto tarda el commit (clon + validar + push, que en W3.6b eran ~1,5 s por ronda).
2. **El token acotado en GCS**: intercambio STS con *Credential Access Boundary* desde el token del
   pod (`driver`), regla sobre `ore/v2/datasets/<p>_<t>/`; ¿escribe ahí y **niega** fuera? ¿lo
   aceptan PyIceberg (`gcs.oauth2.token`), Java (`gcs.oauth2.token`) y DuckDB (¿`gcs` por
   *bearer* o sólo HMAC/S3-interop?) como credencial prestada?
3. **Node sin escritor**: la tabla Arrow por IPC al agente y `ore-store sellar` (ya medido a
   2,6 M filas/s en Rust): coste del viaje (10 M filas) frente a PyIceberg en Python y al API de
   Java, leídas después por los otros dos (`over()`), con los diez físicos de 0032 exactos.
4. **La clave de operación**: la misma celda dos veces = **un** snapshot; y tras un 5xx simulado,
   `write()` mira y no duplica.
5. **La retención por propiedades**: `history.expire.*` en la tabla, `ore datasets --recoger`
   sin `--edad` las obedece; dos tablas con retenciones distintas en el mismo bucket.

## Fuentes

- Apache Iceberg · [REST Catalog Protocol](https://iceberg.apache.org/docs/nightly/rest-protocol/) ·
  [rest-catalog-open-api.yaml](https://github.com/apache/iceberg/blob/main/open-api/rest-catalog-open-api.yaml)
  (requirements/updates, `CommitStateUnknownException`, `commitTransaction`, `Idempotency-Key`,
  `X-Iceberg-Access-Delegation`, `stage-create`) · [Branching and Tagging](https://iceberg.apache.org/docs/latest/branching/) ·
  [Maintenance](https://iceberg.apache.org/docs/1.5.1/maintenance/) · [Flink writes](https://iceberg.apache.org/docs/1.10.0/flink-writes/) ·
  [Spec v3](https://iceberg.apache.org/spec/)
- Delta Lake · [Catalog-Managed Tables (4.1)](https://delta.io/blog/2026-02-02-delta-catalog-managed-tables/) ·
  [Catalog commits GA (Databricks)](https://www.databricks.com/blog/convergence-open-table-formats-and-open-catalogs-catalog-commits-generally-available) ·
  [idempotent writes (`txnAppId`/`txnVersion`)](https://docs.delta.io/delta-streaming/)
- Palantir Foundry · [Datasets · core concepts](https://www.palantir.com/docs/foundry/data-integration/datasets) ·
  [transactions API](https://www.palantir.com/docs/foundry/api/datasets-v2-resources/transactions/commit-transaction) ·
  [retention](https://www.palantir.com/docs/foundry/retention/transaction-selectors) ·
  [transforms API](https://www.palantir.com/docs/foundry/transforms-python/transforms-python-api-classes/index.html)
- Catálogos · [Apache Polaris · vended credentials](https://polaris.apache.org/in-dev/unreleased/vended-credentials/) ·
  [Google · BigLake metastore REST catalog](https://cloud.google.com/biglake/docs/blms-rest-catalog) ·
  [Google · credential vending](https://docs.cloud.google.com/lakehouse/docs/credential-vending) ·
  [Project Nessie](https://projectnessie.org/) · [Nessie vs git](https://projectnessie.org/guides/nessie_vs_git/) ·
  [Dremio · Nessie vs Iceberg vs lakeFS](https://www.dremio.com/blog/data-lakehouse-versioning-comparison-nessie-apache-iceberg-lakefs/)
- Escritores · [DuckDB · Iceberg writes](https://duckdb.org/2025/11/28/iceberg-writes-in-duckdb) ·
  [DuckDB · writing to Iceberg](https://duckdb.org/docs/current/core_extensions/iceberg/writing) ·
  [PyIceberg API](https://py.iceberg.apache.org/api/) · [iceberg-js](https://github.com/supabase/iceberg-js) ·
  [Icebird](https://github.com/hyparam/icebird)
- Panorama · [Lakehouse table formats in 2026](https://amdatalakehouse.substack.com/p/lakehouse-table-formats-in-2026-iceberg) ·
  [WAP con ramas (Dremio)](https://www.dremio.com/blog/streamlining-data-quality-in-apache-iceberg-with-write-audit-publish-branching/) ·
  [Dagster · software-defined assets](https://dagster.io/blog/software-defined-assets) ·
  [ADBC bulk ingestion](https://arrow.apache.org/adbc/current/format/specification.html)
