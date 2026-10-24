# `system.default.executions` · el registro de lo que se ejecuta

> Diseño, sin código (2026-10-24). Viene de Query §3.7 («después de A5, por partes»). Primero se asienta qué
> es; luego se construye; luego se enlaza con la admisión, Jobs, Builds, Code repositories y el Assets Catalog.

## 1. Qué es

Una **tabla del sistema**, una por celda, en su lago, **de solo anexar**: **una fila por ejecución**, sea del tipo
que sea, con quién la lanzó, desde dónde, cuándo, dónde corrió, cuánto consumió, qué leyó y qué escribió, y
cómo acabó. Es inmutable: lo que pasó no se corrige, se anexa lo siguiente.

Es la respuesta a «¿qué se ha ejecutado aquí, quién, con qué y a qué coste?», con SQL, desde la propia Query.
El equivalente en Databricks son sus tablas del sistema (`system.query.history`, `system.lakeflow.*`,
`system.billing.usage`); aquí, una sola tabla para todas las ejecuciones, porque en ORE todas pasan por el
mismo sitio (`ore-serve`) y comparten la mayor parte de lo que se dice de ellas.

## 2. Por qué existe

Hoy lo que se ejecuta deja huella en cuatro sitios, cada uno con su forma, y en ninguno entero:

| dónde | qué guarda | qué falta |
|---|---|---|
| `trabajos/<id>.json` (el repositorio de trabajo de la celda) | el informe de un Build o un trabajo: estado, salida, filas | consumo, clase, lo leído; no se consulta con SQL |
| `copias/<base>_<tabla>.json` (el árbol) | por tabla copiada: filas, bytes, quién, cuándo | el resto de tipos; no es un historial (se sobrescribe) |
| `ore-iam`, lo vivo de Jobs (lo empuja el informador) | qué Jobs hay y en qué estado, ahora | lo de ayer |
| `ore-iam`, la actividad (0047 A6) | los **actos** que escriben (crear, borrar, proponer) | ejecutar en un puesto, a propósito (H12, A8) |
| — | **las consultas de Query, las celdas, las invocaciones** | todo |

Y cuatro cosas lo necesitan:

1. **La admisión** (Query §3.7): el optimizador se equivoca en los joins; lo que de verdad pesó la misma
   sentencia la vez anterior corrige el plan.
2. **El cliente**: Jobs / Usage — qué se ejecutó, cuánto tardó, cuánto consumió, qué falló y por qué.
3. **La auditoría y el consumo**: quién ejecutó qué sobre qué datos, y cuánto cómputo se gastó, por persona,
   por repositorio y por clase.
4. **El linaje operativo**: de la versión de un dataset a la ejecución que la escribió (E4 ya guarda
   `desde.ejecucion`), y de una ejecución a todo lo que escribió.

## 3. Qué es una ejecución

Una unidad de trabajo que `ore-serve` despacha y ve terminar. Los tipos:

| `tipo` | qué | dónde nace hoy |
|---|---|---|
| `consulta` | lo que se pulsa en una hoja de Query (una sentencia o un guion) | `POST /puestos/{id}/ejecutar` en una hoja |
| `celda` | una celda de una sesión de Code repositories o de un notebook | `POST /puestos/{id}/ejecutar` |
| `build` | un Build de un Transform | `builds.rs` |
| `trabajo` | un fichero corrido como Job que termina | `POST /trabajos` |
| `preview` | un Preview de un `@transform` (no escribe) | `POST /puestos/{id}/preview` |
| `copia` | la copia de un origen a una standard database (ingesta) | `48-la-copia`, el copiador |
| `catalogo` | catalogar una fuente | el Job de catálogo |
| `refresco` | la pasada que cumple un `freshness` (0063 S8) | el vigilante de `ore-serve` |
| `invocacion` | una Function invocada (0050) | `POST /functions/…` |

Una ejecución es **la cosa que se lanzó**, no sus piezas: un guion de tres sentencias es una ejecución de
tipo `consulta` con tres `pasos`; un Build es una, con las escrituras que haga.

## 4. El registro (una fila)

| grupo | campo | tipo | qué |
|---|---|---|---|
| **identidad** | `id` | string | el de la ejecución (el de su celda, build o Job), único en la celda |
| | `tipo` | string | §3 |
| | `padre` | string? | la ejecución que la lanzó (un refresco que encola una copia) |
| **quién** | `sujeto` | string | `user:<handle>` o `agente:<…>` |
| | `organizacion`, `celda` | string | |
| **desde dónde** | `desde` | struct | lo mismo que `desde` en la escritura (E1): `tipo` (query, sesion, notebook, build, api…), `id` (hoja, `.sql`, repositorio, documento del Transform), `commit`, `rama` |
| | `sentencia` | string? | el texto ejecutado (una consulta, el `.sql`), hasta un tope |
| | `huella` | string? | la de la sentencia normalizada: agrupa las repeticiones (la admisión la usa) |
| **cuándo** | `encolada`, `empezada`, `terminada` | timestamp | |
| | `espera_ms`, `duracion_ms` | long | de cola, de ejecución |
| **cómo acabó** | `estado` | string | `ok`, `fallida`, `cancelada`, `incompleta` (cortada por tope o por inactividad) |
| | `error` | struct? | código, mensaje, dónde (línea), clase (`usuario` / `sistema`) |
| **dónde corrió** | `clase` | string | `L`, `G` o `E` |
| | `puesto` | string | el puesto o el driver |
| | `nodo`, `pool` | string | |
| | `frio` | bool | si tuvo que esperar a que arrancara un nodo |
| **consumo** | `cpu_ms` | long | |
| | `memoria_pico_mb` | long | |
| | `derrame_mb` | long | lo que se fue a disco |
| | `ejecutores` | int? | en E |
| **datos** | `leido` | list<struct> | por cosa leída: nombre, filas, bytes, `en_vivo` (lo que E2 cuenta, con cantidades) |
| | `escrito` | list<struct> | por escritura: activo, versión (snapshot o transacción), filas, bytes, operación — el enlace con History (E4) |
| **la admisión** | `admision` | struct? | lo que estimó el plan (filas, bytes, clase) y por qué |
| **pasos** | `pasos` | list<struct> | en un guion o un Build con varias salidas: por paso, su sentencia, estado, duración y filas |

Lo que **no** guarda: los datos (ni filas ni resultados), ni credenciales, ni la salida de una celda.

## 5. Entradas: quién escribe

**Sólo `ore-serve`**, al ver terminar una ejecución. No lo escribe el código del cliente (no se puede
falsear: como la escritura de E2, lo cuenta el servidor). Cada tipo tiene un punto donde `ore-serve` ya sabe
que terminó:

| tipo | dónde se cierra |
|---|---|
| `consulta`, `celda`, `preview` | la salida de la última celda (`salida_del_puesto`) |
| `build`, `trabajo` | el informe (`informar_trabajo`, `trabajos/<id>.json`) |
| `copia`, `catalogo` | el informe del Job que recoge `ore-serve` |
| `refresco` | el vigilante, al cerrar la pasada |
| `invocacion` | la respuesta de la Function |

Lo que pone cada uno:
- **quién, desde dónde, cuándo, estado**: `ore-serve` ya lo tiene.
- **lo leído y lo escrito**: lo que el puesto apuntó (E2) y los commits que hizo (E1), con sus cantidades.
- **consumo**: el agente del puesto lo mide al terminar cada celda (DuckDB da su memoria pico y su derrame;
  el proceso, su CPU) y lo manda con la salida; en un Job, el informe.
- **la admisión**: la decisión que `ore-serve` tomó antes de correr.

Se escribe **a la vez que se cierra la ejecución, sin hacerla esperar**: `ore-serve` acumula y anexa por lotes
(cada pocos segundos o N filas), como un commit de Iceberg más.

## 6. Salidas: qué superficies sirve

| superficie | qué enseña | cómo lo lee |
|---|---|---|
| **Query** | el cliente consulta su historia con SQL: `select … from sistema.ejecuciones where …` | como cualquier dataset, con su gobierno |
| **Jobs / Usage** (consola) | la lista de ejecuciones con filtros (tipo, persona, estado, fecha), el detalle de una, y el consumo agregado por día, persona, repositorio y clase | `GET /ejecuciones?…` de `ore-serve`, que lee la tabla |
| **Assets Catalog** | desde la History de un dataset (E4), la ejecución que escribió cada versión; desde la ejecución, todo lo que escribió | el enlace `desde.ejecucion` ↔ `escrito` |
| **Builds y Code repositories** | la historia de un Build o de un repositorio: sus corridas, su duración, sus fallos | `GET /ejecuciones?desde.id=…` |
| **La admisión** | lo que pesó de verdad la última vez la misma `huella` | `ore-serve` en memoria (lo reciente), sin leer la tabla en cada *Run* |
| **Auditoría** | quién ejecutó qué sobre qué datos | la tabla, para quien tenga el permiso |

## 7. Lo que se decidió (2026-10-24)

| | qué | decidido |
|---|---|---|
| **DX1** | quién la lee | **cada persona, sus ejecuciones**; las de toda la organización, **sólo el admin** (ORGADMIN; ni SECURITYADMIN, a diferencia de la actividad) |
| **DX2** | qué tipos de ejecución entran primero | `query`, `cell`, `build` y `job` (los que ya pasan por el puesto o por `POST /trabajos`); `copy`, `catalog`, `refresh` e `invocation` después (X5) |
| **DX3** | retención | 13 meses; lo viejo se borra por mes y se expiran sus snapshots |
| **DX4** | nombre | **en inglés**: `system.default.executions` (base `system`, schema `default`), reservada, en el árbol de catálogo que ve cada usuario; las columnas, en inglés también |
| **DX5** | `statement` | hasta 64 KiB; más largo, su huella y el principio |
| **DX6** | cómo ve cada uno «la suya» con un solo nombre | **por decidir** (§8.1) |

## 8. El approach

### 8.1 Las tres cosas que hay que resolver antes de escribir código

**① Un nombre, dos lecturas (DX1 + DX4).** Todos escriben `select … from system.default.executions` y
cada uno ve sus filas; el admin ve las de todos. Con un filtro de filas no basta: el puesto lee los
ficheros del lago con la credencial que le presta `loadTable`, y con una tabla compartida esa credencial
alcanzaría las filas de los demás. Así que la separación tiene que ser **física**:

- **una tabla Iceberg por persona** (`system/executions/user=<handle>/`), más
- **una tabla de la organización** (`system/executions/_org/`), que sólo se presta al admin.

`ore-serve` anexa cada lote **a las dos**, y `loadTable` de `system.default.executions` resuelve, según
quién pregunta, a la suya o (para el admin) a la de la organización. Se duplica lo escrito, que son
filas pequeñas (unos KB por ejecución), a cambio de que la credencial no pueda saltarse nada.
**→ DX6** (recomendado): tabla por persona + tabla de la organización, escribiendo en las dos. La otra
opción, una tabla con filtro, sólo sería segura si toda lectura pasara por `ore-serve` y no por los
ficheros, y eso hoy no ocurre.

**② Fuera de la historia del árbol.** Los punteros de los datasets son ficheros del árbol
(`datasets/<base>/<schema>/<n>.json`), y cada commit de datos es un commit de git. Si un lote cada pocos
segundos entrara así, llenaría la historia del árbol, History (E4) y las propuestas. Por eso los
punteros de `system` **no van al árbol**: los guarda `ore-serve`, en el lago, en
`system/punteros/<tabla>.json`. Lo escribe un solo escritor (el `ore-serve` de la celda), con
reemplazo atómico. El catálogo los añade al resolver. En el árbol, `system` es un **nombre reservado**:
el compilador rechaza una base que se llame así (un código OOS nuevo; cambio de spec en `C:\oos`).

**③ Escribir sin hacer esperar.** `ore-serve` cierra la ejecución y deja la fila en una cola en
memoria. Un hilo junta las filas y las anexa cada **10 s o 500 filas**, por Arrow (`sellar-arrow` con
`anexar` de `ore-store`, como cualquier escritura del lago). Si el anexo falla, la fila espera al
siguiente lote. Si `ore-serve` se reinicia, se pierde como mucho el lote en vuelo. Las filas se
particionan **por mes** de `started_at`, para que la retención de 13 meses sea borrar particiones.

### 8.2 El registro, en inglés

| grupo | columnas |
|---|---|
| identidad | `id`, `kind` (`query`, `cell`, `build`, `job`…), `parent` |
| quién | `subject`, `organization`, `cell` |
| desde dónde | `source` struct(`kind`, `id`, `commit`, `branch`), `statement`, `fingerprint` |
| cuándo | `queued_at`, `started_at`, `ended_at`, `wait_ms`, `duration_ms` |
| cómo acabó | `status` (`ok`, `failed`, `cancelled`, `incomplete`), `error` struct(`code`, `message`, `line`, `origin`: `user`/`system`) |
| dónde corrió | `compute` (`L`/`G`/`E`), `worker`, `node`, `pool`, `cold_start` |
| consumo | `cpu_ms`, `peak_memory_mb`, `spill_mb`, `executors` |
| datos | `read` list<struct(`name`, `rows`, `bytes`, `live`)>, `written` list<struct(`asset`, `version`, `rows`, `bytes`, `operation`)> |
| admisión | `admission` struct(`rows`, `bytes`, `compute`, `reason`) |
| pasos | `steps` list<struct(`statement`, `status`, `duration_ms`, `rows`)> |

### 8.3 Los pasos

Cada paso va en local con su commit; se empuja al final, cuando lo digas.

| paso | qué | listo cuando |
|---|---|---|
| **X1** · la tabla y quien la escribe ✓ (§8.4) | `ore-serve/src/ejecuciones.rs`: la fila (§8.2), la cola y el hilo del lote, las dos tablas (①), los punteros fuera del árbol (②), la partición por mes. Las cuatro ejecuciones de DX2 se cierran donde `ore-serve` ya sabe que terminaron: `query` y `cell` en `salida_del_puesto` (lo leído de E2 y lo escrito de E1, el `desde` de la escritura, la admisión de A3 si la hubo, el `compute` de L/G); `build` y `job` en el informe del trabajo. `system` reservado en el compilador (spec + ore-core) | prueba de `ore-serve` (una hoja corre un guion de dos sentencias → una fila `query` con dos `steps`, en la tabla de Ana y en la de la organización, y no en la de Bea), y un nombre `system` en un árbol da su error |
| **X2** · leerla ✓ (§8.5) | `loadTable` / `datos` resuelven `system.default.executions` por quién pregunta (①); el permiso `executions:read-all` sólo para ORGADMIN; la base `system` sale en el árbol de catálogo de cada usuario (consola, de sólo lectura). Desde Query, `select … from system.default.executions` | en victor: Ana ve sus filas, Bea las suyas, el admin las de las dos; Bea con una credencial prestada no alcanza los ficheros de Ana |
| **X3** · el consumo | el agente mide por celda `cpu_ms` (el proceso), `peak_memory_mb` y `spill_mb` (DuckDB) y los manda con la salida; en un Job, con el informe | una consulta de 300M en G deja su pico y su derrame en la fila |
| **X4** · enlazarla | la admisión: lo que pesó de verdad la última vez la misma `fingerprint`, en memoria, corrige el plan; History del Assets Catalog → la ejecución que escribió cada versión (`desde.ejecucion` ↔ `written`); `GET /ejecuciones` para Jobs / Usage, Builds y Code repositories | desde una versión en History se llega a su ejecución, y al revés |
| **X5** · el resto y la retención | `copy`, `catalog`, `refresh`, `invocation`; la pasada diaria que borra lo de más de 13 meses y expira snapshots | cada tipo deja su fila; una partición de hace 14 meses desaparece |

X1 y X2 forman el producto mínimo: con ellos, cada uno ya consulta lo que ha ejecutado. X3 añade el
consumo, y X4 conecta la tabla con el resto de la plataforma.

### 8.4 Lo hecho

**X1 · hecho en local** (2026-10-24). Lo que quedó, y en qué se separa de §8.1–§8.3:

- **`ore-store anexar`** (`ciclo.rs`): la cabecera con `dataset` y, detrás, las filas. La primera vez la
  tabla nace; después, un snapshot `append` más. Su puntero vive al lado, en
  `ore/v2/<dataset>/puntero.json`, y no en el árbol. `ore-store puntero {dataset}` lo lee (X2 lo usará).
- **`ore-serve/src/ejecuciones.rs`**:
  - [`terminada`] decide, con la salida de cada celda, si ha terminado una ejecución: una celda suelta,
    o un guion con su última sentencia o con la que falla.
  - `kind`: `query` (hoja), `cell`, `build` o `job`.
  - `compute`: `G` si corrió en el puesto pesado; si no, `L`.
  - `read`: lo leído desde que empezó esa ejecución (`Puesto.leidas_ahora`), no lo acumulado de la sesión.
  - La fila va a una cola. Un hilo anexa cada 10 s (o al juntar 500 filas) en la tabla de la persona y en
    la de la organización, una llamada por tabla. Si una tabla falla, sus filas esperan al siguiente lote;
    el resto ya entró.
- **Un solo enganche**, en `salida_del_puesto`: un Build y un Job también terminan en una celda.
- **`--executions`**, apagado por defecto. Primero va el binario y luego la malla, cada cosa con su go.
- **`system` reservado en `ore-serve`**:
  - no nace como base (`POST /bases`, 422);
  - por `/v1` nadie escribe en ella (prefix, namespace o `commitTransaction`, 403);
  - leer, sí.
- **Las columnas, planas**: `ore-store` escribe filas de escalares. Por eso `source` y `error` van
  desplegados (`source_kind`…, `error_name`, `error_message`), y `read` y `steps` como JSON
  (`from_json(read, '[{"name":"VARCHAR","live":"BOOLEAN"}]')`). No hay `cell`: un `ore-serve` sirve una
  sola organización.
- **El handle** de cada persona sale de `pertenencia::handle_de`, con la regla de `ore-iam`. Es el de
  `subject` y el de la carpeta de su tabla.

Lo que X1 deja para después:

| qué | dónde |
|---|---|
| `written` (lo escrito, con su versión) y `admission` | X4: es el enlace con History y con la admisión |
| `cpu_ms`, `peak_memory_mb`, `spill_mb`, `node`, `pool`, `cold_start` | X3 |
| partición por mes y retención | X5 (la tabla nace sin particionar; añadir la partición es evolución de Iceberg) |
| `system` reservado en el **compilador** (un código OOS en la spec, con su puerta de versión) | deuda: hoy lo guarda `ore-serve`, que es por donde se crea y se escribe |
| si `ore-iam` desempata dos handles iguales, la tabla sigue la regla derivada | deuda, con X2 |

### 8.5 X2 · leerla (hecho en local, 2026-10-24)

- **Desde Query**: `select … from system.default.executions` (o `system.executions`).
  - `ore-core` reconoce el nombre aunque no esté en el árbol (`sistema::es_tabla_del_sistema`, en
    `nombres_de_celda`).
  - `datos` del puesto lo desvía a `ejecuciones::datos`. Esa respuesta lleva el `metadata.json` de la tabla
    de quien abrió el puesto y la credencial de `ore-store prestar` (`modo: leer`), acotada a esa tabla.
    Tiene la forma de la de un dataset, así que el SDK la lee igual.
  - Si nadie ha ejecutado nada todavía, la tabla nace vacía (`anexar` sin filas): existe desde el primer
    momento.
- **`system` en DuckDB**: `system` es un catálogo propio de DuckDB, de solo lectura, y en él no se crea un
  schema (medido, 1.5.4). Por eso:
  - el SDK registra el de ORE como `__ore_system`;
  - `ore-serve` devuelve la sentencia reescrita (`query`), con el tokenizador (`sistema::en_duckdb`); lo que
    dice un comentario o una cadena no se toca.
  - Quien escribe la consulta sigue poniendo `system.default.executions`.
- **El admin**: `ejecuciones:leer-todas` (migración `iam/052`), solo para ORGADMIN.
  - `ore-serve` lo pregunta al abrir el puesto, con el token de la persona, como `decision`, y lo guarda en
    el puesto (`lee_todas_las_ejecuciones`): desde el puesto solo habla el agente.
  - Si lo tiene, lee la tabla de la organización; si no, la suya.
  - Reabrir el puesto vuelve a preguntar.
- **En el árbol de catálogo**: con `--executions`, el índice (`GET /assets`) trae la base `system` y su
  `executions` como un `Package` y un `Dataset` generados (`sistema::documentos`), con las columnas y su
  descripción. La consola los pinta como cualquier base, sin tocarla.
- **Apagado** (sin `--executions`): ni el índice trae `system` ni `datos` la sirve (409).

Lo que X2 deja:

| qué | cuándo |
|---|---|
| la migración `052` en `ore-iam` y `--executions` en la malla | en victor, cada uno con su go; binario antes que malla |
| Preview, History y la ficha del dataset en la consola | leen por el árbol y `system` no está en él: con X4, o antes si estorba |
| `ore.over("system.executions")` y Spark | `sql()` es lo que usa Query; lo demás, cuando se pida |

