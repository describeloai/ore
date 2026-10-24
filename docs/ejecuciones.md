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
| **X3** · el consumo ✓ (§8.6) | el agente mide por celda `cpu_ms` (el proceso), `peak_memory_mb` y `spill_mb` (DuckDB) y los manda con la salida; en un Job, con el informe | una consulta de 300M en G deja su pico y su derrame en la fila |
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

### 8.6 X3 · el consumo (hecho en local, 2026-10-24)

- **El agente mide cada celda** (`agente.py`, `Consumo`), dentro del proceso, porque DuckDB corre en él:
  - **CPU**: `os.times`.
  - **Memoria residente más alta**: `/proc/self/statm`, cada 100 ms.
  - **Derrame**: el tamaño del directorio de derrame de DuckDB, menos lo que ya había.
  - Va en la salida como `consumo: {cpu_ms, memoria_pico_mb, derrame_mb}`, también cuando la celda falla.
- **`ore-serve`** lo lleva a tres columnas nuevas: `cpu_ms` (sumada entre las celdas de la ejecución),
  `peak_memory_mb` y `spill_mb` (lo más alto). Un agente de antes no lo manda, y la fila las deja nulas.
- **`ore-store anexar` evoluciona el esquema**: una columna que la tabla aún no tiene se añade con ids
  nuevos, y las filas de antes la tienen nula.
- Quedan sin medir `node`, `pool`, `cold_start` y `executors`: los sabe Kubernetes y no el agente. Van con
  Jobs / Usage (X4), si hacen falta.

### 8.7 X2b · una System table del Assets Catalog, que existe desde el principio (hecho en local)

Decidido el 2026-10-24:

- **Global**: la misma en todas las ramas, sin ramificarse ni fusionarse. La rama de cada ejecución va en
  `source_branch`.
- **Es una tabla del Assets Catalog con su vista detallada**, y lo dice: **System table**.
- **Existe desde el principio, vacía**: no espera a la primera ejecución de nadie.

Cómo:

1. **El índice la marca** (`system: true` en el `Package` y en el ítem). La consola:
   - en el árbol: el icono de tabla con la etiqueta *System table* y, en una rama, *Global* en lugar de
     «from main» o «built on this branch»;
   - en la ficha:
     - *System table* como tipo, y la descripción;
     - **Overview** con su esquema (ya viene en el índice);
     - **Preview** de las filas de quien mira (las suyas, o las de todos si es admin), por una ruta de
       `ore-serve` que lee su tabla, no por el árbol;
     - **Access**, que lo explica: «each person sees their own executions; admins see the organization's»;
   - sin History, Copy, Health ni las acciones de editar, renombrar o borrar: no tiene versiones en git
     ni receta, y la escribe ORE.
2. **Nace con la celda.** La semilla (el Job `el-arbol`, E5 de 0022) siembra el árbol una sola vez y no se
   vuelve a correr en las celdas que ya existen (`IfNotPresent`); además no tiene la identidad del lago.
   Por eso quien la crea es `ore-serve` **al arrancar** con `--executions`: comprueba que la tabla de la
   organización exista y, si no, la crea vacía (`ore-store anexar` sin filas), con reintentos si el lago
   aún no contesta.
   - En una celda nueva, eso pasa en su primer arranque, justo tras la semilla.
   - En las que ya existen, en el siguiente despliegue.
   - La tabla de cada persona sigue naciendo al leerla o con su primera fila. No se ve la diferencia: la
     base y la tabla salen en el árbol desde el principio, y leer la propia antes de ejecutar nada da 0
     filas.

Hecho (2026-10-24):
- **El índice** marca `system: true` en la base y en el ítem (`assets.rs`, `marcar_lo_del_sistema`).
- **`GET /preview/dataset/system/default/executions`** lee la tabla de quien mira (o la de la organización,
  si es admin: se le pregunta a `ore-iam` con su token) por `ore-store muestra`. Es lo que pide la vista
  detallada de la consola.
- **Al arrancar con `--executions`**, `ore-serve` asegura la tabla de la organización (`asegurar_al_arrancar`,
  con reintentos que se espacian hasta 5 min).
- **Consola** (`AssetDetail.tsx`):
  - el tipo *System table* y la etiqueta *Global*;
  - Overview con la descripción (sin editar), lo que la escribe, su alcance y quién ve qué;
  - Schema, sin History ni el último cambio de su documento;
  - pestañas Overview y Access.
- **Queda la etiqueta en el árbol** (`CatalogTreeSidebar.tsx`): ese fichero tiene cambios sin commit de
  otra sesión y no lo he tocado.


### 8.8 El puntero, sin reescribir (2026-10-24)

En victor no entraba ningún lote: `system/executions/org: 1 filas esperan: no se pudo borrar
ore/v2/system/executions/org/puntero.json … 403`. La cuenta del pod de `ore-serve` sólo crea y lee
(`objectCreator` + `objectViewer`), y en GCS sobrescribir un objeto es borrarlo. Las pruebas usaban un
almacén en memoria que sí borraba.

Ahora cada versión del puntero es un objeto nuevo, `ore/v2/<dataset>/punteros/<u64::MAX − n>.json`,
creado sólo si no estaba. Con el número invertido, el vigente es el **primero** en orden de bytes, y se
lee pidiendo una sola clave (`Almacen::primera`; en GCS, `maxResults=1`), por muchas versiones que se
acumulen. El `puntero.json` de las tablas que nacieron vacías (X2b) vale hasta su primer lote y no se toca.
Si dos escritores crean la misma versión, el segundo falla y su lote vuelve a esperar; su snapshot
queda huérfano. La cuenta del pod no gana ningún permiso.

La prueba `anexar_sin_poder_borrar_crea_un_puntero_por_version` usa un almacén que, como el pod, no borra.

### 8.9 X4 · lo que la tabla sirve (las superficies, 2026-10-24)

La tabla ya se escribe (X1–X3, en victor desde `726a8170`). X4 es que el resto de la plataforma la lea.
Todas salen de las mismas filas y casi todas de un solo `GET /ejecuciones` (filtros por `kind`, `source_*`,
`status`, intervalo; la de la organización sólo con `ejecuciones:leer-todas`).

| superficie | qué saca de la tabla | columnas |
|---|---|---|
| **La admisión** | lo que de verdad pesó la última vez la misma `fingerprint` corrige el plan del optimizador (que falla en los joins): L o G se acierta por lo medido, no por lo estimado | `fingerprint`, `peak_memory_mb`, `spill_mb`, `cpu_ms`, `compute`, `status` |
| **Jobs / Usage** | qué se ejecutó, cuánto tardó y consumió, qué falló y por qué; mío y, para el admin, de la organización | todas |
| **El historial de Query** | en la propia hoja: mis consultas recientes, reabrir y volver a correr, en qué motor fue cada una y por qué | `statement`, `compute`, `duration_ms`, `status`, `error_*` |
| **History del Assets Catalog** | cada versión de un activo → la ejecución que la escribió, y al revés | `id` ↔ `desde.ejecucion` de la escritura |
| **El uso de cada activo** | quién lo lee y cuánto: *más usado*, *quién depende de esto*, *sin lecturas en 90 días* (candidato a retirar) | `read`, `subject`, `started_at` |
| **Billing / coste** | CPU, memoria y motor (L/G/E) por persona, proyecto y mes: la base de una factura o de una cuota | `cpu_ms`, `peak_memory_mb`, `compute`, `subject`, `source_*` |
| **Fallos y alertas** | `error_name` agrupado: qué falla más, desde cuándo y a quién; alimenta avisos y Data Health | `error_name`, `error_message`, `status` |
| **El rendimiento de un activo** | cuánto tarda en construirse o refrescarse cada dataset a lo largo del tiempo: una regresión se ve | `duration_ms`, `kind`, `source_id` |
| **Builds** | cada Build con sus pasos, su consumo y su error | `kind = build`, `steps` |
| **Code repositories** | el historial de ejecuciones de cada celda o fichero, en su commit y su rama | `source_id`, `source_commit`, `source_branch` |
| **Auditoría** (admin) | quién ejecutó qué sentencia, sobre qué datos y cuándo | la tabla de la organización |
| **Agentes / MCP** | que un agente responda «¿por qué falló mi build de ayer?» consultándola | todas |

**Orden:**
1. **La admisión por `fingerprint`** — la que cambia el producto (L/G); se mide con la escalera 3M/30M/300M.
2. **Jobs / Usage y el historial de Query** — la misma lista filtrada, sobre `GET /ejecuciones`.
3. **History ↔ ejecución.**
4. **El uso de cada activo y Billing.**
5. **Builds, Code repositories, fallos, rendimiento, auditoría y agentes.**

**Antes de X4, dos arreglos (hechos el 2026-10-24):**
- **El `id` no se repite.** Era `<puesto>/<n>`, y el contador vuelve a empezar cuando el puesto se recrea (la
  hoja de las 12:10 y la de las 13:45 eran las dos `…/1`). Ahora es `<job>/<n>`: el Job de Kubernetes lleva
  otro sufijo en cada vida del puesto. La escritura deja el mismo en `desde.ejecucion` (antes, sólo el número
  de la celda), y por él se enlaza History.
- **Los huérfanos del 403.** Mientras el puntero se reescribía, cada reintento (cada 10 s) subía un fichero de
  datos que nadie apuntaba: ~300 en `system/executions/org/data/`. Borrados con otra credencial tras comprobar
  contra el `metadata.json` vigente qué ficheros apunta la tabla.

### 8.10 X4·1 · La admisión que aprende (diseño, 2026-10-24)

**Qué falla hoy.** La admisión (Query A3, `ore_core::admision`) decide L, G o E **antes de correr** con el
plan de DuckDB: recorre sus operadores (`Peso::visitar`) y se queda con el mayor volumen que pasa o que se
construye (el lado de construcción de un `HASH_JOIN`, lo que agrupa u ordena), a 8 bytes por columna, contra
umbrales fijos (L hasta 500 MB y 50 M filas). No sabe cuánto pesa de verdad cada operador ni lo que pesa
escribir, y no aprende: se equivoca igual la primera vez que la centésima.

⛔ **Lo que no es.** Nada se reintenta en otro motor tras fallar: lo que se corrige es la decisión de antes
de correr. Lo pulsado corre entero en una clase, el cliente no elige motor y no hay «preparando cómputo».

#### Cuatro capas

| capa | sirve a | de dónde sale |
|---|---|---|
| **1 · No morir** | todo, la primera vez incluida | el reparto de memoria del puesto (DuckDB / escritor / Python) y el disco de derrame: un error de estimación cuesta tiempo, no la sesión |
| **2 · El modelo de memoria del plan** | lo que nunca se corrió | la **memoria prevista** por operador: `HASH_JOIN` → su lado de construcción × factor; `WINDOW`/`ORDER_BY` → su entrada; `HASH_GROUP_BY` → sus grupos; lo espacial, sus factores; `CREATE_TABLE_AS`/`INSERT` → lo que escribe (tramo Hilbert incluido) |
| **3 · La calibración** | lo que nunca se corrió, cada vez mejor | los factores del modelo, ajustados con **todas** las ejecuciones (lo previsto contra el pico real), no sólo con las que se repiten |
| **4 · La repetición** | lo que ya se corrió | la huella exacta: su último pico real manda sobre cualquier previsión |

**La capa 1 ya existe a medias**: el puesto da el 50 % de su memoria a DuckDB (`_DUCKDB_POR_CIENTO`) y el
50 % de su disco al derrame (`_DERRAME_POR_CIENTO`), y desde `f94c874f` ni el SDK ni el escritor juntan la
tabla. Falta **medir** que el otro 50 % basta para el escritor (tramos de 512 MB: ~1 GB de pico) y Python, en
L (4 GiB) y en G (48 GiB). Las tres muertes del 2026-10-24 fueron del camino de escritura, ninguna de DuckDB.

#### Cómo se itera rápido y sin romper nada

**① El banco: la verdad, fuera de producción.** `pruebas-de-fuego/admision/banco/` guarda casos: el plan
(`EXPLAIN (FORMAT JSON)`, como los de `crates/ore-core/tests/planes/`) y lo que midió de verdad en L y en G
(pico con el escritor, derrame, duración, estado; `killed` si murió). Un caso vale para siempre: el modelo se
re-evalúa contra el banco entero sin volver a correr nada.

**② El evaluador, en segundos.** `ore admision banco [--modelo m.json]` corre la admisión sobre cada caso y
da tres números:
- **graves**: decidió L y en L murió, derramó más del X % del disco o pasó del 80 % de su memoria. **Objetivo 0**;
- **caros**: decidió G y en L cabía con holgura (pico ≤ 50 %, sin derrame). Objetivo < 20 %;
- **el error de la previsión**: pico real / previsto (p50 y p90), por tipo de operador.

En el CI es una prueba con umbral: un cambio que suba los graves no entra.

**③ El modelo es datos.** Los factores van en `modelo.json` (compilado en `ore-core` por defecto) y
`ore-serve` puede tomar otro de un ConfigMap (`--modelo-de-admision`). Ajustar un factor es editar un JSON,
pasar el banco y empujar la malla: sin compilar ni esperar una imagen.

**④ La sombra antes de decidir.** Un modelo nuevo corre primero **en sombra**: decide al lado del que manda,
y la fila de executions guarda las dos decisiones (`admission`, `admission_shadow`). Se pasa a mandar cuando
el banco y las filas reales dicen que acierta más. Volver atrás es cambiar el ConfigMap.

**⑤ Producción alimenta el banco.** Cada fila guarda `plan_summary`: los operadores con su cardinalidad
estimada y la memoria prevista (~1–2 KB). `pruebas-de-fuego/admision/exportar.py` saca filas de la tabla de
la organización y las convierte en casos. Las consultas reales del cliente son las que más enseñan.

**⑥ La calibración es un guion, revisado.** `pruebas-de-fuego/admision/calibrar.py` ajusta los factores con
el banco (lo previsto contra lo medido, por tipo de operador) y escribe un `modelo.json` nuevo, que se revisa
y pasa por la sombra antes de mandar. Que el ajuste corra solo en producción es un paso posterior, cuando el
banco demuestre que el ajuste es estable.

#### Los pasos

| paso | qué | listo cuando |
|---|---|---|
| **B0 · el banco** (previo en victor) | el corpus: la escalera (3M/30M/300M `create or replace`), las nueve consultas de `docs/query.md` §2.1 a 6M y 60M, y joins espaciales; cada una en L y en G, con el pico que ya incluye al escritor. `medir.py` las corre con el SDK y guarda plan y medida | ~40 casos en `banco/`, con L y G |
| **B1 · la capa 1** | el reparto de memoria del puesto, ajustado con lo que dice B0 | ningún caso del banco muere en L ni en G |
| **B2 · la fila** | `plan_summary`, `admission`, `admission_reason`, `admission_shadow`; `steps` con huella y consumo; la ejecución de un puesto que muere se cierra (`killed`, `OutOfMemory`, pico = el límite) | una celda de dos sentencias deja dos pasos medidos; un OOM provocado deja su fila |
| **B3 · el modelo y el evaluador** | la memoria prevista por operador en `admision`, sus factores en `modelo.json`, `ore admision banco`, la prueba del CI | el banco da 0 graves; los caros y el error, medidos |
| **B4 · la calibración** | `calibrar.py` y `exportar.py`; `--modelo-de-admision`; la sombra | un modelo calibrado, en sombra en victor, comparado con el de hoy en filas reales |
| **B5 · la repetición** | la memoria de lo medido (huella → últimas 3 medidas, en vivo y al arrancar con 30 días) y su regla | la escalera, la segunda vez, por lo medido |
| **B6 · mandar** | el modelo calibrado pasa de la sombra a decidir | las filas reales, una semana: graves 0 |

B0 es lo primero porque sin él no se puede medir ninguna de las capas.

**La regla de la repetición** (B5; vale con lo medido de hasta 30 días, manda lo más reciente):

| lo medido | decide |
|---|---|
| murió por memoria, o derramó, en L | **G**, aunque el modelo diga L |
| `ok` en L con pico ≥ 80 % de L | **G** (iba justa) |
| `ok` en L con pico ≤ 50 % y los datos no han crecido (bytes ≤ 1,2× los de entonces) | **L** |
| `ok` en G | su pico escalado por lo que crecieron los datos (entre 0,5× y 4×): ≤ 50 % de L y el modelo dice L ⇒ L; si no, G |
| falló por otra cosa (SQL, permisos) | no cuenta: el modelo |

E no cambia: lo sigue decidiendo el volumen que se lee.

**Decisiones abiertas:**
- **D-A2 · la forma, no el texto.** `where id = 5` y `where id = 6` dan hoy dos huellas. Propuesta: la huella
  exacta primero; quitar los literales cuando la tabla diga cuántas se repiten sólo en ellos.
- **D-A5 · la rama.** Propuesta: la clave no la lleva; el escalado por bytes cubre que otra rama tenga más o
  menos datos.
- **D-A6 · el objetivo.** Propuesta: graves = 0 manda sobre caros < 20 %. Ante la duda, G: un nodo grande
  cuesta dinero, una sesión muerta cuesta al cliente.

### 8.11 R0 · Medido en victor: el puesto puede crecer desde L (2026-10-24)

Guiones y resultados en `pruebas-de-fuego/estructura/` (`medir.sh r01|r02|r03-*`). Imagen de hoy
(`puesto-python:1`, DuckDB 1.5.6), GKE 1.35.8.

**R0·2 · Un puesto que crece mientras corre: sí.** Agrupar 120 M filas en 60 M grupos (L derrama):

| pasada | tiempo | derrame | pico |
|---|---|---|---|
| A · fija en L (cgroup 4 GiB, DuckDB 2 GB) | 45,5 s | 5,9 GB | 2,5 GB |
| **B · empieza en L; a los 4 s el pod crece en caliente a 16 GiB y DuckDB sube a 9,1 GiB** | **27,0 s** | **0** | **9,3 GB** |
| C · grande desde el principio (DuckDB 9,6 GB) | 26,6 s | 0 | 8,1 GB |

- `kubectl patch --subresource resize` (sólo memoria, *strategic merge*): el cgroup cambia en **2–4 s**, sin
  reiniciar el contenedor (`resizePolicy: NotRequired`). Un parche que toque otro recurso (p. ej. borra
  `ephemeral-storage`) es `Forbidden: only cpu and memory resources are mutable`.
- El puesto ve crecer su `memory.max` y sube `memory_limit` **desde otra conexión, con la consulta en marcha**:
  DuckDB la usa en el acto (deja de derramar). Crecer a mitad cuesta lo mismo que haber empezado grande.
- Si el nodo no tiene sitio: `PodResizePending · Infeasible: Node didn't have enough capacity` (60 GiB en un
  nodo de 57 GiB asignables) y el pod **sigue como estaba**; 40 GiB en el mismo nodo, en 2 s.
- La `ResourceQuota` del namespace (`cuota`, 112 GiB de `requests.memory`) rechaza el crecimiento al instante
  (`Forbidden: exceeded quota`): el crecimiento cuenta contra la cuota.

**R0·1 · Qué derrama** (un puesto como L: 4 GiB, DuckDB 1 GB, 40 M filas):
- ✅ terminan derramando: agrupar (20 M grupos), join 40 M × 20 M, ordenar, ventana, `count(distinct)`, join
  espacial, `create … as` con geometría (1,7 GB escritos en flujo, 0,9 GB de pico) y las tres juntas.
- ❌ `list()` y `string_agg()` por grupo: `OutOfMemoryException` (controlada: el proceso sigue vivo).
- ⚠️ `median` / `quantile`: terminan sin derramar, pero con **1,95 GB de pico con un límite de 1 GB**: se pasan
  del `memory_limit`, y en un cgroup justo eso es un OOM del kernel.

**R0·3 · El disco de derrame** (la agrupación de 40 M con 1 GB):

| disco | hasta correr | escribir / leer | derrame |
|---|---|---|---|
| el del nodo de las sesiones (hoy, `e2-standard-4`, 30 GB pd-balanced) | 4 s | 160 / 161 MB/s | 21,0 s |
| propio por pod, pd-balanced 200 GiB | 17 s | 214 / 217 MB/s | 20,8 s |
| propio por pod, pd-ssd 200 GiB | 12 s | 240 / 241 MB/s | 15,6 s |

El nodo de las sesiones tenía **7 GB libres** para todos sus pods: es lo que limita a L, no la velocidad.

**Lo que queda obsoleto** (de §8.10 y de `docs/query.md`):
- ~~L, G y E como clases fijas elegidas antes de correr, con lo pulsado entero en una~~ → una sesión nace
  pequeña y **crece en su nodo mientras corre**; E sólo para lo que no cabe en un nodo.
- ~~«Ante la duda, G»~~ → no hay duda que cubrir: crecer cuesta segundos y no se pierde lo hecho.
- ~~El techo de 10 GiB de derrame en el disco del nodo~~ → disco de derrame propio y grande.
- ~~El banco mide «graves» (sesiones muertas)~~ → mide cuánto y cuándo crecer (coste y latencia).
- ~~`_DUCKDB_POR_CIENTO = 50` fijo~~ → `memory_limit` sigue al cgroup; con margen para `median`/`quantile`.

**La estructura que sale** (diseñada en `docs/puesto-que-crece.md`):
1. **Nodos de sesión con sitio para crecer**: varias sesiones pequeñas por nodo grande (como `pesado`,
   64 GB y SSD local), en vez de nodos de 16 GB llenos. El SSD local da derrame grande y rápido sin los
   12–17 s de conectar un disco propio.
2. **Crecer antes de necesitarlo**: el puesto mira la presión de DuckDB (memoria usada cerca del límite,
   derrame que empieza) y pide crecer a `ore-serve`, que redimensiona su pod (memoria, y CPU si hace falta).
   El plan y lo medido (§8.10) dicen **cuánto** pedir de una vez; no deciden si la consulta sobrevive.
3. **Si el nodo no tiene sitio** (`Infeasible`): la consulta sigue derramando a su disco (termina, más lenta)
   y el siguiente arranque de esa sesión va a un nodo con sitio. Reservar holgura por nodo es D-E2.
4. **La cuota y Kueue** cuentan lo que crece (D-E3: Kueue admite por `requests`; ¿ve un redimensionado?).
5. **La lista cerrada** (R0·1): `list`/`string_agg` sobre muchos grupos y los cuantiles reservan memoria de
   antemano en el plan; el resto confía en el derrame.
