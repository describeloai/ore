# `sistema.ejecuciones` · el registro de lo que se ejecuta

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

## 7. Lo que hay que decidir

| | qué | recomendación |
|---|---|---|
| **DX1** | quién la lee | cada persona, sus ejecuciones; la de todos, con un permiso (como `actividad:leer-toda`). Lo ejecutado dice qué datos se tocaron: no es de todos |
| **DX2** | qué tipos entran en la primera versión | `consulta`, `celda`, `build` y `trabajo` (los que ya pasan por el puesto); `copia`, `catalogo`, `refresco` e `invocacion` después |
| **DX3** | retención | 13 meses, y lo viejo se compacta (Iceberg expira snapshots) |
| **DX4** | nombre | `sistema.ejecuciones` en la base `sistema` de cada celda (reservada: nadie más escribe en ella) |
| **DX5** | `sentencia` | guardarla hasta 64 KiB; más, su huella y el principio |

## 8. Cómo se construye (para después)

1. **X1** · la tabla y quién escribe: el esquema de §4, la base `sistema` reservada, el anexo por lotes en
   `ore-serve`, y los tipos de DX2.
2. **X2** · el consumo desde el agente: memoria pico, derrame y CPU por celda.
3. **X3** · leerla: `GET /ejecuciones` y desde Query.
4. **X4** · enlazarla: la admisión (la huella), History del Assets Catalog, Jobs / Usage, Builds.
5. **X5** · los demás tipos (copia, catálogo, refresco, invocación).
