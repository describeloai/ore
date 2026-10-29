# 0080 · Origen CockroachDB

**Leer CockroachDB** —servidor propio y CockroachDB Cloud— como origen de ORE. Es un **SQL distribuido**
que habla el protocolo de PostgreSQL: el transporte probablemente ya lo tenemos, y lo propio es la
consistencia (`AS OF SYSTEM TIME`), los reintentos (`40001`) y el cambio (changefeeds).

**Estado:** **X0 medido en un nodo, en 22.2, 24.3 y 25.4** (2026-10-29). Rama `0080-origen-cockroachdb`,
worktree `C:\ORE-cockroach`. §2 es lo publicado por Airbyte y Fivetran; §3.1 son nuestras mediciones y §1 lo
que de ellas se decide. **Sin medir:** la 26.x (no arranca en esta CPU), TLS y SCRAM, varios nodos, Cloud,
tablas particionadas y varios esquemas (§3.1, «Lo que X0 no midió»).

Sigue a [`0069`](0069-origin-mongodb.md), [`0070`](0070-origin-cassandra.md), [`0075`](0075-origen-oracle.md) y
[`0077`](0077-origin-clickhouse.md): medir primero, construir después; esqueleto `ore_driver::origen`; el kit
de conformidad como puerta.

## 1. Qué se decide

Cada punto cita la medición de §3.1 que lo sostiene.

1. **El transporte es PostgreSQL, con el cliente que ya tenemos** (`rust-postgres` 0.19, el del origen
   Postgres). Consulta extendida, resultados en binario, `ENUM` y arrays resueltos solos y `DECLARE`/`FETCH`
   funcionan sin tocar nada en las tres versiones (A1). **Binario propio** (decidido 2026-10-29): crates
   `ore-cockroach` (librería) y `ore-read-cockroach` (binario fino sobre `ore_driver::origen`), como Oracle,
   ClickHouse y Cassandra. `ore-read-postgres` es un binario monolítico de 3.815 líneas, sin librería y sin el
   esqueleto `origen`, y lo específico de Postgres no sirve aquí (reparto por páginas `ctid`, estimación por
   `pg_class`). La conversión de tipos se copia y adapta, sin extraer una crate común hasta que haya dos
   usuarios reales. **TLS entra desde el paso 1**, con la convención del origen Postgres (`sslmode` y
   `sslrootcert` dentro de la URL); el túnel SSH y el IAM de Cloud SQL, que `ore-read-postgres` tiene
   mezclados con el TLS en `tls::abrir`, quedan fuera. **Texto, no binario** (decidido en el paso 2): el origen Postgres lee en texto y Cockroach contesta con los mismos nombres de tipo y un
   `TIMESTAMPTZ` idéntico en las tres versiones; `TIMETZ` sale como texto y cambia entre 22.2 y 24.3.
2. **La versión se lee de `crdb_internal.node_build_info`** (campo `Version`) o de `version()`. `SHOW
   server_version` dice siempre `13.0.0` y no sirve (A2). Suelo provisional: **22.2** (la más antigua medida).
3. **Lectura: un instante fijo con `AS OF SYSTEM TIME`**, tomado una vez y compartido por todas las
   conexiones de la carga. Costó lo mismo que leer normal (B3). La foto coherente entre conexiones **se
   comprobó en el paso 3**, con un escritor concurrente y su control.
4. **`leer` no necesita bucle de reintento por `40001`.** En lecturas, con escritor concurrente sobre lo
   leído, no salió ni una vez en 36 intentos (B4). Un tope de reintentos defensivo sigue siendo barato.
5. **Repartir por rangos de clave primaria escala** hasta 8 conexiones en un nodo: 0,64 → 3,14 millones de
   filas/s (B5). El reparto sale de los extremos de la clave, **no de `SHOW RANGES`**, que exige
   `ZONECONFIG`/`VIEWACTIVITY` (E12).
6. **El cambio, por capas.** Primera: cursor sobre la columna oculta `crdb_internal_mvcc_timestamp`; no pide
   changefeed ni ajustes del cluster ni más permiso que `SELECT`, pero **no ve los borrados** (C8). Segunda:
   huella `fnv64` de la tabla para el verbo `cambio`: ve todo, cuesta una lectura entera (C8). Tercera, para
   borrados y baja latencia: changefeed sinkless, que exige activar `kv.rangefeed.enabled` (viene en
   `false`) y es *at-least-once* (C6, C9). El changefeed no entra en la primera entrega.
   Si el cursor cae por debajo del GC, el servidor contesta `XXUUU` (C7): el conector lo traduce a «reiniciar
   con carga completa», no a un fallo genérico.
7. **Catálogo: `information_schema` y `pg_indexes` bastan** para columnas, claves e índices únicos. Hay que
   descartar las columnas con `is_hidden = 'YES'` (la de *shard* de una clave hash-sharded) (E12).
8. **Tipos:** ver la tabla de §3.1 D. Los espaciales (`GEOGRAPHY`, `GEOMETRY`, `BOX2D`) llegan con OIDs
   propios (90000+) y bytes EWKB; se tratan a mano.

## 2. Lo que hacen los demás (publicado, sin medir)

**Airbyte `source-cockroachdb` 0.2.5** (Marketplace; JDBC, variante del de Postgres): full refresh e
incremental por cursor, **sin CDC**; mínimo v1.15; `ssl` como interruptor; `numeric`/`enum` → number y
`double precision` → string; arrays sí. No documenta cursor, borrados ni changefeeds.

**Fivetran** (GA septiembre de 2025; versiones 22.1.0 a 26.2.0; TLS 1.3): carga inicial con `SELECT` por
lotes reanudables (follower reads en Enterprise); incremental con **changefeeds sinkless**
(`CREATE CHANGEFEED FOR … WITH nobackfill`); borrados en `_fivetran_deleted`; history mode.
- **Cluster:** `kv.rangefeed.enabled` y `feature.changefeed.enabled` en `true`; `gc.ttlseconds` de 24 h
  (también en `system`); sincronizar al menos 4 veces más a menudo que el GC, o resync completo.
- **Usuario:** v22 `CONTROLCHANGEFEED VIEWCLUSTERSETTING`; v23+ `VIEWCLUSTERSETTING`; más `USAGE`, `SELECT` y
  default privileges.
- **Casos que les costaron una versión del conector:** hash-sharded PK (columna `crdb_internal_pk_shard_N`),
  columnas virtuales en la PK, `BYTES` como PK, familias de columnas múltiples, `BIT/VARBIT`, `ENUM`,
  arrays, `GEOGRAPHY/GEOMETRY` (GeoJSON), `DECIMAL` sin precisión (→ float), cambios de esquema con
  `nobackfill`, orden para borrar un tipo (`ALTER TABLE`, sync, `DROP TYPE`).
- **Sin soporte:** `BOX2D`, `INT2VECTOR`, `OIDVECTOR`, `REG*`, `TSQUERY`, `TSVECTOR`.
- **Por confirmar:** un resultado de búsqueda dice que las tablas particionadas no se soportan; no está en
  la página.
- Los logs del servidor muestran el changefeed sinkless como `EXPERIMENTAL CHANGEFEED FOR` (implementación
  deprecada desde 25.2).

## 3. Lo que hay que medir (X0)

Cada pregunta con su prueba. Contra un solo nodo (`start-single-node`) en varias versiones; el cluster de
varios nodos **no** entra en X0 (ver «Aplazado»).

### A. Transporte y dialecto
1. ¿Basta el cliente de PostgreSQL que ya usamos? Mirar extended query, SCRAM, TLS, cursores `DECLARE`.
2. ¿Cómo se detecta la versión real? `version()` se presenta como PostgreSQL; probar `SHOW server_version`
   y `crdb_internal.node_build_info`, de la 22 a la 26.

### B. Lectura y consistencia
3. Coste de `AS OF SYSTEM TIME` frente a lectura normal, con `follower_read_timestamp()` y con un instante
   fijo; el error cuando el instante cae por debajo del GC.
4. Cómo se ve el error `40001` (restart) en una lectura larga con escrituras concurrentes; qué reintento
   exige `leer`.
5. ¿Se puede repartir una tabla grande por rangos de PK? Comparar con `SHOW RANGES`; cuántas conexiones
   aguanta.

### C. El cambio (el paradigma propio)
6. Qué trae un changefeed sinkless con `nobackfill`, `diff`, `resolved` y `updated`: borrados, orden y qué
   pasa al reconectar.
7. Qué pasa si el cliente cae más que `gc.ttlseconds`: ¿error claro o resync?
8. Alternativa sin changefeed: `AS OF SYSTEM TIME` con huellas (como el `cambio` de ClickHouse) y lo que
   ofrezca `crdb_internal`.
9. Qué exige el cluster (`kv.rangefeed.enabled`, `feature.changefeed.enabled`, licencia) y en qué modo
   (self-hosted, Dedicated, Serverless).

### D. Tipos y trampas (las que Fivetran listó)
10. Para cada uno: qué manda el servidor en el protocolo y qué haríamos. Hash-sharded PK, columnas
    virtuales, familias de columnas, `DECIMAL` sin escala, `BYTES` como PK, arrays, `JSONB`,
    `GEOGRAPHY/GEOMETRY`, `INTERVAL`, `TIMETZ`, `BIT/VARBIT`, `OID`, `ENUM` y los tipos sin soporte.
11. Tablas particionadas y esquemas múltiples (la afirmación sin confirmar de §2).

### E. Catálogo y explorar
12. ¿Sirve `information_schema` o hace falta `crdb_internal` y `SHOW`? Claves, índices únicos, estimación
    de filas (`SHOW STATISTICS`) y permisos mínimos del usuario.

### 3.1 Resultados (2026-10-29)

Sondas fuera del repo: `psycopg` 3 (libpq) para el servidor y una sonda Rust con `postgres` 0.19 para el
cliente. Tabla `grande`: 10⁷ filas en 25.4, 2,5·10⁶ en 24.3 y 2·10⁶ en 22.2; las comparaciones entre
versiones usan `id <= 2000000`.

**A. Transporte y versión.**
- A1: `rust-postgres` funciona sin cambios en las tres versiones. `DECLARE`/`FETCH` de 10⁶ filas en 4,4–5,1 s.
  **Sin probar:** SCRAM y TLS (los nodos van con `--insecure`).
- A2: `SHOW server_version` = `13.0.0` siempre. `version()` y `crdb_internal.node_build_info` dan la real y
  `node_build_info` la puede leer un usuario con solo `SELECT`.

**B. Lectura.**
- B3: `AS OF SYSTEM TIME` (`follower_read_timestamp()` y `'-10s'`) no cuesta nada en un nodo: 22.2 ≈2,3 s,
  24.3 ≈1,9 s, 25.4 ≈1,5 s con y sin él. **El error por debajo del GC no se reprodujo:** un instante anterior
  a la base da `3D000 database "defaultdb" does not exist`, que engaña. El error de verdad está en C7.
- B4: ningún `40001` en lecturas (36 intentos entre las tres versiones, con otro cliente escribiendo sobre
  las filas leídas). Sí sale con escritura contra escritura: `WriteTooOldError`, `40001`.
- B5, 25.4, 10⁷ filas, cliente Rust: 1 conexión 15,5 s (644k filas/s), 2 → 6,0 s (1,65M), 4 → 4,0 s (2,47M),
  8 → 3,2 s (3,14M). La sonda de Python dio plano por el GIL y se descartó. La tabla ocupa 14 rangos, pero un
  usuario sin `ZONECONFIG`/`VIEWACTIVITY` no puede listarlos.

**C. Cambio.**
- C6: el changefeed sinkless devuelve `(tabla, clave, valor)` con `after`, `before` (con `diff`), `updated` y
  mensajes `resolved`. Un borrado llega como `after: null`. Reconectar con `cursor=` repite alguna fila
  (*at-least-once*), en 22.2, 24.3 y 25.4. En 22.2 se escribe `EXPERIMENTAL CHANGEFEED FOR`.
- C7: con `gc.ttlseconds = 5` en una tabla, el GC tardó **entre 7 y 12 minutos** en actuar (no a los 25 s).
  Después, `AS OF SYSTEM TIME <instante viejo>` y un changefeed con `cursor=` viejo fallan, en 22.2, 24.3 y
  25.4, con **SQLSTATE `XXUUU`**: `batch timestamp … must be after replica GC threshold …`. En 25.4 el
  changefeed lo dice mejor: `could not create changefeed: cursor … is older than the GC threshold`. Es un
  **error claro, no un resync silencioso**: el cliente lo reconoce por `XXUUU` y debe reiniciar con carga
  completa. Esto resuelve el B3 que quedó abierto: así se ve el error por debajo del GC.
- C8: `crdb_internal_mvcc_timestamp` existe en las tres versiones y `WHERE … > cursor` devuelve los inserts y
  updates, no los borrados. `max()` de esa columna sobre 2·10⁶ filas tarda ≈3 s (lectura entera). La huella
  `sum(fnv64(…))` sobre 2·10⁶ filas tarda ≈5 s.
- C9: `kv.rangefeed.enabled` = `false` por defecto; `feature.changefeed.enabled` = `true`; la organización va
  vacía y los changefeeds funcionaron sin licencia en las tres versiones. Un usuario con solo `SELECT` pudo
  abrir un changefeed acotado (`initial_scan='only'`) pero no puede ver los ajustes del cluster
  (`VIEWCLUSTERSETTING`).

**D. Tipos.** Lo que manda el servidor (binario del protocolo, `kind` de `rust-postgres`):

| Tipo | OID | Qué llega | Qué haríamos |
|---|---|---|---|
| `INT2/4/8`, `FLOAT`, `BOOL`, `TEXT`, `BYTES`, `UUID`, `JSONB`, `INET`, `DATE`, `TIME`, `TIMESTAMP(TZ)`, `INTERVAL`, `OID` | los de Postgres | binario estándar | igual que el origen Postgres |
| `DECIMAL` con y sin precisión | 1700 | `numeric` estándar | sin tope de escala: decidir como en Postgres |
| `BIT`, `VARBIT` | 1560, 1562 | `bit`/`varbit` | texto |
| `ENUM` | propio (100104) | `Kind::Enum` con sus valores | texto |
| Arrays 1-D | `_int8`, `_text` | `Kind::Array` | lista |
| `GEOGRAPHY`, `GEOMETRY`, `BOX2D` | 90002, 90000, 90004 | tipo desconocido, EWKB | a mano: GeoArrow o texto |
| `TSQUERY`, `TSVECTOR`, `PG_LSN` | 3615, 3614, 3220 | **no existen en 22.2** | texto; error claro en 22.2 |
| `INT2VECTOR`, `OIDVECTOR` | — | no se pueden crear como columna | no aplica |
| `INT8[][]` (2-D) | — | no se puede crear | no aplica |
| `TIMETZ`, `TIMESTAMPTZ` en texto | — | el offset cambia de `+02:00:00` (22.2) a `+02` (24.3+) | leer en binario, no en texto |
| Columna virtual y calculada | 20 | sale en `select` y se lee normal | leer; no usar como clave sin medir |
| Clave hash-sharded | — | `information_schema` muestra una columna oculta `crdb_internal_<c>_shard_16` en la clave | filtrar `is_hidden = 'YES'` |
| `unique_rowid()` / `SERIAL` | 20 | enteros ≈1,2·10¹⁸ | `Int64`; ojo a quien los lea como `Number` de JS |

**E. Catálogo.**
- E12: `information_schema.columns`, `key_column_usage`, `table_constraints` y `pg_indexes` dan columnas,
  clave primaria, únicos e índices. `SHOW STATISTICS` queda vacío hasta que haya estadísticas;
  `crdb_internal.table_row_statistics` dio la cuenta exacta de `grande` en las tres. Un usuario con solo
  `SELECT` puede leer `information_schema`, `node_build_info` y `SHOW STATISTICS`, pero no `SHOW RANGES` ni
  los ajustes del cluster. **Sin comprobar** que ese usuario pueda leer `table_row_statistics`.
- E11 (particionadas, varios esquemas): **no medido.**

**Lo que X0 no midió:** la 26.x (la CPU no tiene AVX2 y el binario exige `x86-64-v3`; ni 26.1 ni 26.2
arrancan), SCRAM y TLS, cluster de varios nodos y follower reads reales, Cloud, tablas particionadas, varios
esquemas, familias de columnas, `BYTES` como clave y columna virtual dentro de la clave.

### Aplazado (no entra en X0)
- Cluster de varios nodos: follower reads reales y qué hace el cliente si cae un nodo a mitad de lectura o
  de changefeed. Se hace al final, en Docker, y solo si X0 deja abiertas las preguntas 5 o la de caída.
- CockroachDB Cloud (Serverless/Standard/Dedicated): sin medir.

## 4. El laboratorio

Docker Desktop (hay que arrancarlo a mano). Un nodo por versión, `start-single-node --insecure
--accept-sql-without-tls`; imágenes elegidas por Docker Hub el 2026-10-29 (parches más recientes):

| Contenedor | Imagen | Puerto SQL |
|---|---|---|
| `ore-crdb-222` | `cockroachdb/cockroach:v22.2.19` | 26257 |
| `ore-crdb-243` | `cockroachdb/cockroach:v24.3.37` | 26258 |
| `ore-crdb-254` | `cockroachdb/cockroach:v25.4.18` | 26259 |
| `ore-crdb-262` | `cockroachdb/cockroach:v26.2.9` | 26260 — **no arranca aquí** |

**La 26.x no corre en esta máquina:** `Fatal glibc error: CPU does not support x86-64-v3` (Xeon sin AVX2),
igual que ClickHouse `latest`. Probadas 26.1.0, 26.1.3 y 26.2.9. Va a la VM `ore-labs` o queda sin medir.
Otras trampas: `kv.rangefeed.enabled` viene en `false`; sembrar 10⁷ filas con tres nodos a la vez tardó 24 min
y saturó la CPU, así que sembrar de uno en uno; `SHOW RANGES` cambia de columnas entre versiones (leer por
nombre); en `psycopg`, un changefeed se lee con `cursor.stream()`, no con `execute()`.

Pendiente: uno con TLS y usuarios sin contraseña para el kit. Datos: `grande` (10⁷ filas en 25.4) y las
tablas `t_<tipo>` de la sonda; falta la tabla `tipos` única para el kit. **Licencia:** desde la 24.3 los binarios usan la CockroachDB Software
License (gratuita para uso individual y pequeñas empresas); anotar qué funciones piden licencia.

## 5. Incremental y CDC

Primera entrega: cursor sobre `crdb_internal_mvcc_timestamp` (inserts y updates, sin borrados) más la
huella `fnv64` para el verbo `cambio`. Los borrados incrementales y la baja latencia esperan al changefeed
sinkless (§1.6), que exige `kv.rangefeed.enabled` y es *at-least-once*: el consumidor deduplica por clave y
`updated`. La deprecación de la forma sinkless desde 25.2 (§2) **no se midió**: hay que mirar qué ofrece la
26.x antes de construirlo.

## 6. La hoja de ruta

Propuesta, por capas como en 0077; cada paso cierra con el kit en verde.

1. **Esqueleto y `conectar`.** *(hecho 2026-10-29)* Crates `ore-cockroach` y `ore-read-cockroach` sobre `origen` (§1.1). Detectar
   la versión con `node_build_info`, suelo 22.2. Banco `cockroach` en `ore-conector-kit`.
2. **`leer` lo mínimo.** *(hecho 2026-10-29, en texto)* Texto → Arrow, tipos de la tabla de §3.1 D; los espaciales como texto EWKB.
3. **Instante fijo y reparto por rangos.** *(hecho 2026-10-29)* `AS OF SYSTEM TIME` compartido entre conexiones (foto
   coherente comprobada) y partición por extremos de la primera columna entera de la clave.
4. **Filtros, orden y límite**, reutilizando el traductor de Postgres. *(hecho 2026-10-29; el `ORDER BY` va también fuera de la subconsulta)*
5. **Tiempo y cancelar.** `statement_timeout` y `CANCEL QUERY`; medir en Linux.
6. **Verbos propios:** `check`, `catalogo`, `explorar`, `estimar` (`table_row_statistics`), `testigo` y
   `cambio` (huella).
7. **Incremental** con la columna `crdb_internal_mvcc_timestamp`.
8. **La puerta completa en el CI** (grupo `cockroach` en la matriz de `conectores`) y la hoja de la consola.
9. **Después:** changefeed, la 26.x en la VM, TLS y SCRAM, cluster de varios nodos.

### Paso 1 · diseño (2026-10-29): esqueleto, conectar, banco y laboratorio con TLS

**Entrega:** `ore-read-cockroach capacidades` contesta el protocolo v2; `conectar` abre sesión, comprueba la
versión y rechaza lo que no sirve; el kit tiene su banco `cockroach`; el laboratorio incluye un nodo seguro.
**No entrega:** filas, filtros, cancelar ni verbos (pasos 2 a 6).

**1 · Ficheros.**

| Fichero | Qué lleva |
|---|---|
| `crates/ore-cockroach/src/lib.rs` | `Cockroach` implementa `Origen`: `Sesion` = `postgres::Client` + versión + ajustes; `Token` = `postgres::CancelToken`; `CAPACIDADES` con `conector: "ore-read-cockroach"`, sin operadores, `limit` ni `orderBy` (se declaran al pasar, como en ClickHouse) |
| `…/url.rs` | análisis de la URL (apartado 2) |
| `…/tls.rs` | `planear`, `conector` y `explicar` de `ore-read-postgres/src/tls.rs` (líneas 52–250), **copiados** sin el túnel ni el IAM. Es código de seguridad duplicado: si se arregla uno hay que arreglar el otro. Anotado en §7; se extrae a una crate común cuando se haga el túnel o Cloud |
| `…/lectura.rs` | `SUELO` (22.2), `conectar` y la lectura de la versión |
| `…/verbos.rs` | `otros`: contesta un error claro para `catalogo`, `check`, `explorar`, `testigo` y `cambio` hasta el paso 6 |
| `crates/ore-read-cockroach/src/main.rs` | `principal::<Cockroach>(verbos::otros)` |
| `crates/ore-conector-kit/src/bancos/cockroach.rs` | el banco (apartado 5), detrás de la feature `cockroach` |
| `pruebas-de-fuego/cockroach/laboratorio.sh` | el laboratorio (apartado 4) |

Cableado que ClickHouse olvidó hasta el paso 9 y aquí va **ya**: miembros del workspace, `ci/compilar-binarios.sh`,
el `Dockerfile` y `ci/prueba-huella-de-los-binarios.py`. Las dependencias nuevas son las que ya trae
`ore-read-postgres` (`postgres` 0.19, `native-tls`, `postgres-native-tls`), así que no crecen el cierre del
`Cargo.lock` que vigila `ore-cli/tests/dependencias.rs`.

**2 · La URL.** Es la de PostgreSQL, porque es la que CockroachDB y sus guías ya dan:
`cockroachdb://usuario[:clave]@host[:26257][/base]?sslmode=verify-full&sslrootcert=<PEM codificado>&timeoutMs=…`
(también `postgresql://`).
- Mismas reglas que el origen Postgres: sin `sslmode`, `require` y `verify-full` verifican la cadena y el
  nombre; `verify-ca` solo la cadena; `disable`, `allow` y `prefer` se **niegan**; la CA va **en la URL**
  (el Job no tiene tus ficheros) y `sslrootcert=system` es el defecto.
- Laboratorio: `ORE_COCKROACH_LABORATORIO=1` deja pasar `disable` en los nodos `--insecure`. El CI lo pone.
- **Una contraseña sin TLS no se admite**, y un usuario sin contraseña, solo con esa variable.
- Para **Serverless/Standard** la guía de CockroachDB pone el clúster en la cadena (`options=--cluster=<nombre>`).
  **No medido**: queda para Cloud (§7). El conector pasa `options` tal cual si viene, sin interpretarlo.

**3 · `conectar`.** Tres consultas tras abrir, la primera ya es la prueba de la credencial (un usuario
inexistente sale como `credencial`):
1. `SELECT version()`, con la versión que sale de `crdb_internal.node_build_info` (campo `Version`).
   Por debajo de 22.2, o una que no se entiende (p. ej. un servidor que es PostgreSQL de verdad), sale
   `origen` con el nombre de la versión. **Comprobar al montar el paso** que `node_build_info` lo puede leer
   un usuario con solo `SELECT`: X0 lo midió con `ore_r` sobre una tabla, no con el usuario `kit`.
2. `SHOW transaction_read_only` y `SHOW default_transaction_read_only`: si la sesión no es de solo lectura,
   se fija (`SET default_transaction_read_only = true`) y se avisa.
3. `SHOW CLUSTER SETTING sql.defaults.…`: **no** se hace; hace falta `VIEWCLUSTERSETTING` y el conector no
   debe exigir permisos de más (X0, E12). `kv.rangefeed.enabled` se mira solo cuando llegue el changefeed.

La sesión lleva un `application_name` (`ore-read-cockroach`) para que se vea en `SHOW SESSIONS`, y el plazo de
conexión sale de `timeoutMs`.

**4 · El laboratorio** (`laboratorio.sh <nombre> <versión> <puerto> [seguro]`):

| Contenedor | Imagen | Puerto | Modo |
|---|---|---|---|
| `ore-crdb-222` | v22.2.19 | 26257 | `--insecure` |
| `ore-crdb-243` | v24.3.37 | 26258 | `--insecure` |
| `ore-crdb-254` | v25.4.18 | 26259 | `--insecure` |
| `ore-crdb-254-tls` | v25.4.18 | 26270 | **seguro**: CA y certificado propios (`localhost`, `127.0.0.1`), TLS y SCRAM |

- En los `--insecure` el usuario `kit` va sin contraseña, con `GRANT ALL ON DATABASE kit`, y se activa
  `kv.rangefeed.enabled`. En el seguro hay `kit` con contraseña (SCRAM, el defecto desde la 21.2) y un `kit_ro`
  con solo `SELECT`, y la CA se exporta para dársela al banco por `sslrootcert=`.
- Se prueban explícitamente: contraseña mala → `credencial`; contraseña sin TLS → rechazada por el conector;
  CA equivocada → error de certificado claro; nombre de host distinto → rechazado con `verify-full`.
- Trampas conocidas del propio laboratorio: Docker crea un **directorio** si el fichero montado no existe;
  Git Bash reescribe `/CN=` (usar `MSYS_NO_PATHCONV=1`); la 26.x no arranca aquí (§4 arriba).

**5 · El banco `cockroach` del kit.** `COCKROACH_KIT_URL=cockroachdb://kit@localhost:26259/kit` (con
`ORE_COCKROACH_LABORATORIO=1`) o el seguro con su `sslrootcert`. Carga `tipos`, `vacia` y `grande` (10⁶ filas con
`generate_series`) y la vista `lenta` (`pg_sleep`), con el mismo molde que `bancos/clickhouse.rs` y los tipos de
la semilla: `INT8`, `DECIMAL(12,2)`, `STRING`, `DATE`, `TIMESTAMPTZ`. La carga es idempotente. El banco declara lo
que Cockroach pierde o cambia: la cadena vacía **no** es nula; `INT8` es el entero entero; las fechas van en UTC.
`sesiones` no aplica en este paso (lo decide el paso 7).

**6 · Qué pasa el kit al cerrar el paso.** `--casos 13` (capacidades), `3` (no hay nada que probar sin operadores)
y `12` (secretos: usuario inexistente = `credencial`). Fallan con «todavía no sabe leer» el 1, 5, 6, 7, 8, 10 y
11; el 9 y el 14 no aplican hasta el paso 7. Esta es la lista exacta que debe salir, ni más ni menos.

**7 · Cómo se prueba.**

```text
cargo test -p ore-cockroach --lib                                  # url, plan de TLS, versión: sin red
COCKROACH_LAB_URLS=cockroachdb://kit@localhost:26257/kit,…:26258…,…:26259…  ORE_COCKROACH_LABORATORIO=1 \
  cargo test -p ore-cockroach --test conectar                       # × 3 nodos inseguros
COCKROACH_LAB_SEGURO=…  cargo test -p ore-cockroach --test tls      # el nodo seguro
ore-kit --conector target/debug/ore-read-cockroach --banco cockroach --casos 12,3,13
```

Sin las variables, las pruebas de laboratorio se saltan solas (así corre el CI hasta el paso 8).

**8 · Riesgos que el paso comprueba** (si alguno falla cambia el diseño): que `rust-postgres` haga SCRAM con
Cockroach sin ajustes; que `node_build_info` lo lea un usuario sin privilegios; que un `postgres://` normal no
se confunda con un Cockroach (la versión lo distingue); y que compile en Windows con `schannel`, como el origen
Postgres.

### Paso 1 · hecho (2026-10-29): esqueleto, conectar, banco y laboratorio con TLS

Como en el diseño: `ore-cockroach`, `ore-read-cockroach`, el banco `cockroach` del kit y el laboratorio con un nodo seguro. **`ore-kit
--banco cockroach --casos 12,3,13` pasa** contra 22.2, 24.3 y 25.4 sin TLS, y contra el nodo seguro (25.4 con TLS, SCRAM y la CA en la
URL). El mapa entero del kit en 25.4 es el prometido: **pasan** 2, 3, 4, 12 y 13; **fallan con «todavía no sabe leer»** 1, 5, 6, 7, 8, 10 y 11;
**no aplican** 9 y 14. `fmt` y `clippy -D warnings` limpios; compila en Windows con `schannel`.

**Cómo se prueba** (todo verde el 2026-10-29):

```text
cargo test -p ore-cockroach --lib                                  # 19 pruebas, sin red
COCKROACH_LAB_URLS=cockroachdb://kit@localhost:26257/kit,…:26258/kit,…:26259/kit  ORE_COCKROACH_LABORATORIO=1 \
  COCKROACH_LAB_POSTGRES=postgresql://postgres:pg@localhost:55432/postgres?sslmode=disable \
  cargo test -p ore-cockroach --test conectar                       # 7 pruebas × 3 nodos inseguros + un PostgreSQL de verdad
COCKROACH_LAB_SEGURO=cockroachdb://kit:kit-secreto@localhost:26270/kit  COCKROACH_LAB_CA=C:/ore-lab/ore-crdb-254-tls-ca.crt \
  cargo test -p ore-cockroach --test tls                            # 7 pruebas contra el nodo seguro (sin ORE_COCKROACH_LABORATORIO)
COCKROACH_KIT_URL=cockroachdb://kit@localhost:26259/kit  ORE_COCKROACH_LABORATORIO=1 \
  ore-kit --conector target/debug/ore-read-cockroach --banco cockroach --casos 12,3,13
```

Con el nodo seguro, `COCKROACH_KIT_URL` lleva `?sslmode=verify-full&sslrootcert=<PEM con %0A y %20>`.

**Los riesgos del diseño (apartado 8), comprobados:**
1. **SCRAM funciona sin ajustes** con `rust-postgres` 0.19: el hash guardado de `kit` y `kit_ro` empieza por `SCRAM-` (`system.users`).
2. **`node_build_info` lo lee un usuario sin privilegios**: `ore_sin_nada` (sin ningún permiso) y `kit_ro` (solo `SELECT`).
3. **Un PostgreSQL de verdad no se toma por CockroachDB**: `conectar` contra `postgres:16` dice «el servidor no es CockroachDB».
4. **Compila en Windows con `schannel`**, como el origen Postgres.

**Lo que construir enseñó, y que la medición no podía decir:**
1. **`explicar` solo reconocía mensajes de OpenSSL en inglés.** `schannel` (Windows) escribe la causa en el idioma del sistema («El nombre CN del
   certificado no coincide…») y la pista no salía nunca. Se añadió una pista genérica para cualquier `TLS handshake` roto, que nombra las dos
   cosas que se arreglan en la URL (`sslrootcert`, `sslmode=verify-ca`). **El mismo defecto está en la copia de `ore-read-postgres/src/tls.rs`**
   (en un Linux con OpenSSL no se ve) y no se ha tocado.
2. **Un lote de varias sentencias ya arranca en solo lectura.** `SET default_transaction_read_only = false; CREATE USER …` falla con `25006`
   porque el lote empieza la transacción antes de que el `SET` valga: el banco y las pruebas lo mandan en dos lotes.
3. **Un usuario inexistente en un nodo `--insecure` da `28P01` (`credencial`)**, igual que una contraseña mala; por eso el banco usa una u otra
   para el caso 12 según la URL lleve contraseña.
4. **Una base que no existe es `3D000` → `objeto`**, y un puerto cerrado, `conexion` reintentable.
5. **Los objetos creados a mano como `root` estorban a `kit`**: la vista `lenta` que dejó una prueba manual impedía su `DROP VIEW`. El
   laboratorio no crea nada como `root` más allá de la base y el usuario.
6. **`lenta` con `pg_sleep(9)` funciona en Cockroach** (`SELECT … FROM generate_series(1,1) WHERE pg_sleep(9)`). El caso 9 no aplica: una
   función que escribe dentro de una vista pide funciones definidas por el usuario, que no están en 22.2.
7. **El nodo seguro necesita `start-single-node` sin `--listen-addr`**: solo admite `localhost` ahí, y Docker no publica el puerto.

**Entorno.** Contenedores y puertos de §4; el seguro es `ore-crdb-254-tls` (26270), usuarios `kit` / `kit-secreto` y `kit_ro` /
`kit-ro-secreto`, CA en `C:\ore-lab\ore-crdb-254-tls-ca.crt` (la deja el script; el volumen de certificados es `ore-crdb-254-tls-certs`).
`ore-pg-x0` (`postgres:16`, 55432, clave `pg`) es solo para la prueba 3. Compilar y probar: `. C:\ORE\scripts\dev-windows.ps1` desde
`C:\ORE-cockroach`.

**Pendiente de este paso, a propósito:** la entrada de `ci/prueba-huella-de-las-pruebas.py` y el grupo `cockroach` de la matriz de `conectores`
del CI, que el guardián exige a la vez (paso 8); `consultas_vivas` del banco (`SHOW CLUSTER QUERIES`) está escrito pero no se ha ejercitado
hasta el caso 10 (paso 5).

### Paso 2 · hecho (2026-10-29): leer lo mínimo

`leer` lee la proyección de una tabla entera, con todos los tipos de X0, **en texto** y sin filtros, orden ni límite (pasos 3 a 5). Nuevos:
`src/tipos.rs` (traducción de tipos y paso de texto a Arrow), `src/leer.rs` (catálogo, consulta, transacción y lotes de 8192 filas) y
`tests/tipos.rs`. Es una adaptación de `ore-read-postgres` sin las lecturas en tramos por `ctid`; el código está **duplicado a propósito** (§1.1).

**Decisión: texto, no binario** (la que dejó abierta §1.1). Cada columna sale de la consulta como `col::STRING` y llega como los bytes UTF-8 de su
texto; `ore_base::tipos` lo analiza. Razones medidas: (1) el catálogo de CockroachDB (`pg_attribute`, `format_type`) contesta con los **mismos
nombres de tipo que PostgreSQL** en 22.2, 24.3 y 25.4, así que la traducción del origen Postgres sirve casi tal cual; (2) lo que no es exacto sale
como su texto sin escribir un decodificador por tipo; (3) `TIMESTAMPTZ` llega **igual** en las tres versiones (el instante, en UTC), que era el
temor de §3.1 D. Coste: ≈25 % frente al cliente crudo (abajo).

**Cómo se prueba** (verde el 2026-10-29 en 22.2, 24.3, 25.4 y, el kit, en el nodo seguro; `fmt` y `clippy -D warnings` limpios):

```text
cargo test -p ore-cockroach --lib                                   # 29 pruebas, sin red
COCKROACH_LAB_URLS=…:26257,…:26258,…:26259  ORE_COCKROACH_LABORATORIO=1  cargo test -p ore-cockroach --test tipos   # 3 pruebas × 3 nodos
ore-kit --conector target/release/ore-read-cockroach --banco cockroach          # el mapa entero
```

**El mapa del kit hoy** (25.4): **pasan** 1, 2, 3, 4, 5, 6, 7, 10, 12 y 13; **fallan** el 8 (`timeoutMs`: sin `statement_timeout` el kit lo mata a
los 6 s, paso 5) y el 11 (no declara `servir`, paso 7); **no aplican** 9 y 14. Los casos 6, 1, 5 y 7 pasan en las cuatro (22.2, 24.3, 25.4 y el
nodo seguro con TLS y SCRAM).

**Rendimiento** (`release`, una conexión, 25.4): 10⁶ filas en 1,7–1,8 s por el kit; **10⁷ filas de 4 columnas en 20,7 s = 484k filas/s**, frente a
los 644k del cliente Rust crudo de X0 (misma tabla, mismas 4 columnas). Un build de depuración es diez veces más lento (19,6 s las 10⁶ filas):
el kit en el CI debe usar `release`. El pico de memoria **no se midió** (el kit no lo ve en Windows).

**Los tipos, leídos de verdad** (`tests/tipos.rs`, igual en las tres versiones salvo lo que se dice):

| Sale como | Tipos |
|---|---|
| tipado | `INT2/4/8` → `Int64`; `FLOAT4/8` → `Float64`; `BOOL`; `DATE`; `TIME` → `Time64(µs)`; `TIMESTAMP`; `TIMESTAMPTZ` → instante UTC; `BYTES` → `Binary`; `DECIMAL(p,s)` ≤ 38 → `Decimal128`; columna virtual |
| lista | `INT8[]` → `List<Int64>`, `STRING[]` → `List<Utf8>`, con sus nulos y el array vacío |
| texto | `DECIMAL` sin precisión y de más de 38 cifras, `UUID`, `ENUM`, `INTERVAL`, `JSONB`, `INET`, `BIT`, `TIMETZ`, `GEOGRAPHY`, `GEOMETRY` (EWKB en hexadecimal) |

- **`TIMETZ` como texto cambia entre versiones:** `12:34:56+02:00:00` en 22.2 y `12:34:56+02` desde 24.3. Un consumidor que lo compare entre
  versiones ve dos textos para el mismo valor. Se deja así (es `Opaque` en el catálogo) y queda dicho.
- **La cadena vacía no es nulo** y un array vacío no es nulo (`vacio`, `arr_vacio`).
- **La geometría no es nativa todavía:** sale como EWKB en texto. GeoArrow es de un paso posterior (§3.1 D).
- Una columna que ya no está es `esquema`; una tabla que no está, `objeto`; y un fallo no deja la sesión en medio de una transacción.

**Dos cosas que construir enseñó:**
1. **El caso 10 (cancelar) pasa en Windows, pero por el cierre de la conexión, no por `SIGTERM`.** El kit mata el proceso (Windows no tiene
   `SIGTERM`) y CockroachDB corta la consulta en 13 ms al caerse el socket. El camino real, `cancel_query` con el `CancelToken` desde la señal, **no
   está probado** hasta correrlo en Linux. Antes de `leer` este caso fallaba con «no había consulta en el origen», de modo que `consultas_vivas`
   del banco (`SHOW CLUSTER QUERIES`) **sí ve la consulta mientras corre**: queda ejercitado.
2. **Las pruebas de laboratorio no pueden compartir una tabla** si corren en paralelo: se preparan una sola vez por proceso.

### Paso 3 · hecho (2026-10-29): instante fijo y reparto por rangos de la clave

Una tabla grande se lee con **varias conexiones que ven lo mismo**: se captura un instante con `cluster_logical_timestamp()`, cada conexión
abre `BEGIN AS OF SYSTEM TIME <ts>` y la tabla se parte por rangos de la primera columna entera de su clave primaria. `src/leer.rs`
(`reparto`, `trozos`, `en_tramos`), `tests/reparto.rs`; la URL gana `tramos=N` (1 a 16; 4 por defecto; `1` lo apaga) y el `Token` guarda todas las
conexiones en vuelo.

**Cuándo se reparte** (`reparto`): hay más de una conexión; la petición es una copia (sin `limit`, orden ni claves); el objeto tiene una o dos partes;
su clave primaria empieza —saltando las columnas ocultas— por `smallint`, `integer` o `bigint`; y entre su mínimo y su máximo, **medidos en el
mismo instante que luego se lee**, caben al menos 200 000 valores. Si no, una conexión y una transacción, que ya es una foto de un instante.
Si no se puede decidir (un error del catálogo), se lee con una y se avisa por stderr.

**El desequilibrio:** los rangos son de ancho igual sobre `[mínimo, máximo]`; una clave muy desigual dejaría a una conexión con casi todo. Se
cortan **4 trozos por conexión** y cada conexión va tomando el siguiente que queda. El primero no tiene cota de abajo y el último no la tiene de
arriba. `lab_escasa` (5 filas con claves de 10⁶ a 5·10⁶) sale entera.

**Cómo se prueba** (verde el 2026-10-29 en 22.2, 24.3 y 25.4; `fmt` y `clippy -D warnings` limpios):

```text
cargo test -p ore-cockroach --lib                                   # 33 pruebas, 4 de ellas sobre los trozos (sin huecos ni solapes)
COCKROACH_LAB_URLS=…:26257,…:26258,…:26259  ORE_COCKROACH_LABORATORIO=1  cargo test -p ore-cockroach --test reparto   # 5 pruebas × 3 nodos
ore-kit --conector target/release/ore-read-cockroach --banco cockroach
```

`tests/reparto.rs` crea en la base `kit` de cada nodo `lab_reparto` (300 000 filas), `lab_hash` (250 000, clave *hash-sharded*), `lab_escasa`,
`lab_uuid` y `lab_pequena`, y comprueba:
1. **Dónde se reparte y dónde no:** `lab_reparto` y `lab_hash` sí (la segunda por `id`, **no** por la columna de *shard* oculta); no mil filas, ni una
   clave UUID, ni una vista, ni tres partes, ni con `limit`; sí con un esquema omitido (`public`).
2. **Las mismas filas que con una conexión:** cada `id` una vez, la columna de al lado es la de su fila, y el servidor ve **≥ 3 sesiones a la
   vez** con 4 conexiones y **≤ 1** con `tramos=1` (se mira `SHOW CLUSTER SESSIONS` mientras lee: es lo que demuestra que de verdad se reparte).
3. **Un mismo instante:** un escritor actualiza en una transacción las filas `id = 1` e `id = N` (extremos opuestos de la clave) sin parar; el
   conector lee 8 veces y las dos filas **coinciden siempre**. El **control**: dos lecturas sueltas separadas 300 ms **sí** las ven distintas, y el
   escritor se ve entre vuelta y vuelta. Sin ese control la prueba no demostraría nada.
4. **Cancelar corta todas las conexiones**: sobre `grande` (10⁶ filas), a los 250 ms; ninguna consulta suya queda viva en el servidor.
5. Una clave escasa se lee entera.

**Rendimiento** (`release`, 25.4, `grande`: 10⁷ filas × 4 columnas, 404 MB idénticos, mejor de 2 vueltas):

| `tramos` | 1 | 2 | 4 (defecto) | 8 | 16 |
|---|---|---|---|---|---|
| segundos | 17,3 | 8,7 | 6,0 | 5,0 | 4,7 |
| miles de filas/s | 577 | 1.151 | 1.669 | 2.013 | 2.134 |

Con 4 conexiones, ×2,9 sobre una. El cliente Rust crudo de X0 llegó a 3,1 M con 8; aquí el techo está en el hilo único que escribe el flujo Arrow
por stdout, no en el servidor. El caso 7 del kit (10⁶ filas de `grande`, que ahora se reparte) baja de 1.697 ms a 665 ms. **Se deja en 4**: más
conexiones suman poco y cuestan en el servidor; es una decisión por defecto, no una medición de un cluster.

**El mapa del kit** (release): 25.4 y 24.3, 10 pasan, fallan 8 y 11, no aplican 9 y 14; 22.2, igual desde que el laboratorio da `VIEWACTIVITY` a `kit`
(abajo).

**Lo que construir enseñó:**
1. **`SHOW QUERIES` exige `VIEWACTIVITY` en 22.2 aun para las consultas propias**; desde 24.3 basta ser el mismo usuario. Sin él, el banco no podía
   comprobar el caso 10 y salía «no aplica» solo en 22.2. Es del **laboratorio** (el conector no lo necesita): `laboratorio.sh` lo da con
   `ALTER ROLE … VIEWACTIVITY` o `GRANT SYSTEM VIEWACTIVITY`, según la versión.
2. **El camino `CancelToken` ya está probado, en el proceso** (`Cockroach::cancelar` desde otro hilo, con el cancel de `rust-postgres`), en las tres
   versiones y con varias conexiones. Lo que sigue sin probar es la señal `SIGTERM` de verdad (Linux) y `cancel_query` **por TLS** (el nodo seguro).
3. **Una prueba que mira el servidor desde fuera necesita su propio control**: el contador de sesiones y el desgarro entre extremos de la clave se
   comprueban en el sentido contrario (`tramos=1`, lecturas sueltas) antes de fiarse del sentido bueno.

**Lo que este paso no cubre:** una clave primaria que no empieza por un entero (UUID, texto, fecha) se lee con **una** conexión; un cluster de varios
nodos, donde el instante fijo cobraría sentido para *follower reads*; y tablas con una clave muy desigual más allá de lo que 4 trozos por conexión
compensan.

### Paso 4 · hecho (2026-10-29): filtros, orden y límite

El conector declara los **diez operadores** (`eq`, `neq`, `in`, `lt`, `le`, `gt`, `ge`, `like`, `isNull`, `isNotNull`), `limit` y `orderBy`. La consulta
la arma `ore-sql` en su dialecto de Postgres (marcas `$n`, identificadores entre comillas, `NULLS LAST` en los dos sentidos) y los valores se escriben
como literales con la comilla doblada; CockroachDB los coacciona al tipo de la columna. Cambios en `src/leer.rs` y `src/lectura.rs`, y `tests/filtros.rs`.

**El defecto que el kit no vio (y que la prueba propia sí):** un `orderBy` **sin `limit`** salía en el orden de la clave, no en el pedido. La
consulta es `SELECT col::STRING … FROM (<consulta de ore-sql>) AS ore_q`, y el `ORDER BY` de la consulta de dentro no sobrevive a la de fuera:
SQL no lo garantiza y **CockroachDB no lo conserva** (PostgreSQL sí, en la práctica; por eso el origen Postgres, del que se copió, no lo
necesita). El kit solo prueba el orden **con** `limit`, donde el top-k lo conserva por casualidad. Arreglo: la consulta interna pide también las
columnas de orden (aunque no salgan) y la de fuera repite el `ORDER BY … NULLS LAST`. Dos pruebas unitarias lo fijan y `tests/filtros.rs` lo ejerce
con y sin `limit`, ascendente y descendente, con una columna de orden fuera de la proyección.

**Además se copia, del origen Postgres:** que las columnas de los filtros y del orden existan (si no, `esquema`, no un error del servidor a mitad) y
que un filtro por `BYTES` (que llega en base64) pase a `\x…` hexadecimal; un valor que no es base64 es `operador`.

**Cómo se prueba** (verde el 2026-10-29 en 22.2, 24.3 y 25.4; `fmt` y `clippy -D warnings` limpios):

```text
cargo test -p ore-cockroach --lib                                   # 35 pruebas
COCKROACH_LAB_URLS=…:26257,…:26258,…:26259  ORE_COCKROACH_LABORATORIO=1  cargo test -p ore-cockroach --test filtros   # 7 pruebas × 3 nodos
ore-kit --conector target/release/ore-read-cockroach --banco cockroach
```

`tests/filtros.rs` crea `lab_filtros` (9 filas, una toda nula) y comprueba:
1. **Un valor con comillas, barras o SQL es un valor**: `o'brien`, `a\b`, `x'; DROP TABLE lab_filtros; --`, `éñü`, la cadena vacía (que **no** es
   nulo), `like` con `%` y `_`, y que `neq` no trae los nulos. Tras todo eso la tabla sigue entera.
2. **Los tipos que el kit no toca se comparan como son**: `BYTES` (base64 → bytes, también en `in`), `UUID`, `ENUM`, `BOOL`, `DECIMAL` (`1.5` es `1.50`),
   `FLOAT`, `DATE` y `TIMESTAMPTZ` (`12:00+02` son las 10:00 UTC), nulos, `in` con 1 000 valores y con ninguno.
3. **El orden y el límite**: nulos al final en `asc` y en `desc`, orden por dos columnas, `limit 0`, `limit` mayor que la tabla, y el orden de **bytes**
   en el texto (`''` < `100%` < `Ana`), sin colación.
4. **Columna que no existe** en un filtro o en el orden: `esquema`.
5. **Con la lectura en tramos**: un filtro por rango de la clave da las mismas 100 001 filas en tramos y con `tramos=1`; un filtro que no es de la
   clave lo lleva cada trozo; con `limit` y orden se lee en orden con una conexión.

**El mapa del kit** (release, 22.2, 24.3, 25.4): **pasan** 1 a 7, 10, 12 y 13 (**10 pasan**); el caso 2 ya prueba 30 de 30 y no «0 de 0»; **fallan** el
8 (`timeoutMs`, paso 5) y el 11 (`servir`, paso 7); **no aplican** 9 y 14. En el nodo seguro, los 8 casos que se corrieron pasan.

**Lo que este paso no cubre:** la colación es la de bytes (la de CockroachDB por defecto); una base creada con otra colación daría otro orden de texto,
distinto del que espera el motor. No se midió. Y un `like` con un patrón que el usuario no escapó se lleva tal cual, como en Postgres.

## 7. Lo que se aceptó a cambio

- Sin changefeed en la primera entrega: los borrados solo se ven con la huella (una lectura entera).
- El suelo 22.2 y las 24.3 y 25.4 son lo único medido; **la 26.x se acepta sin medir** hasta tener la VM.
- TLS y SCRAM se miden en el paso 1, pero **varios nodos no**: el comportamiento ante la caída de un nodo,
  los follower reads y la latencia de red quedan sin saber.
- **Código de TLS duplicado** entre `ore-read-postgres` y `ore-cockroach` (apartado 1 del paso 1). Se extrae a
  una crate común cuando se haga el túnel SSH o Cloud.
- CockroachDB Cloud sin medir: `options=--cluster=…` de Serverless/Standard, lista de IPs permitidas y la CA
  que da Dedicated.

## Fuentes

- Airbyte: <https://docs.airbyte.com/integrations/sources/cockroachdb>
- Fivetran: <https://fivetran.com/docs/connectors/databases/cockroachdb>, `/setup-guide`, `/changelog`
- Versiones y licencia: <https://docs.cockroachlabs.com/docs/releases>
