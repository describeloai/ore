# ORE Federation Engine · los contratos

Lo que [ADR 0053](decisions/0053-ore-federation-engine.md) decide, escrito como contrato: lo que
un **conector** tiene que cumplir (§1) y cómo se comprueba (§2), lo que la **pasarela**
`ore-federation` expone (§3), lo que el **coordinador** `ore-serve` hace y en qué orden (§4), y los
**valores por defecto** (§5). La gramática y sus códigos son de
[OOS v1alpha24 `01-leer-el-origen`](../vendor/oos/spec/v1alpha24/01-leer-el-origen.md); aquí está
cómo los implementa ORE.

```
puesto / trabajo ──► ore-serve (coordinador) ──► ore-federation (pasarela, rol driver) ──► conector ──► origen
                     conducto · acceso · coste      cola por origen · presupuesto              ore-read-<tipo>
                     credencial · anotación         conexiones calientes
```

---

## 1. El conector v2

Un conector es el `ore-read-<tipo>` de su familia ([ADR 0008](decisions/0008-el-protocolo-del-driver.md)).
v2 conserva lo que hay —la petición es un **fragmento de plan**, no SQL; traducir al dialecto del
origen es suyo— y añade lo que F0 midió que falta.

### 1.1 · Los verbos

| verbo | qué hace | v1 | v2 |
|---|---|---|---|
| `leer` | una petición por stdin, las filas por stdout, y termina | ✓ | ✓ (Arrow siempre) |
| **`servir`** | **un bucle**: peticiones una tras otra por stdin, cada respuesta enmarcada por stdout; las conexiones al origen se quedan abiertas entre peticiones | — | **nuevo** |
| **`capacidades`** | qué sabe hacer este conector, y su versión (§1.4) | — | **nuevo** |
| **`estimar`** | lo que costaría una petición sin leerla, si el origen lo sabe (el *dry run* de BigQuery, el `EXPLAIN` de Postgres) | — | **nuevo, opcional** |
| `catalogo`, `testigo`, `explorar`, `check`, `versiones`, `bajar` | como hoy | ✓ | ✓ |

`servir` es lo que hace caliente a la pasarela: M2 midió ~890 ms por petición contra Neon, y casi
todo es abrir proceso, TLS y sesión, no la consulta.

### 1.2 · La petición

La de [ADR 0008](decisions/0008-el-protocolo-del-driver.md), con tres campos nuevos. Donde la
industria tiene nombre se usa el suyo.

```json
{
  "url": "<la conexión, con su credencial: nunca se escribe en ningún sitio>",
  "objeto": "olist.customers",
  "proyeccion": { "id": "customer_id", "estado": "customer_state" },
  "filtros": [
    { "columna": "customer_state", "operador": "in", "valor": ["SP", "RJ"] },
    { "columna": "created", "operador": "ge", "valor": "2026-01-01" }
  ],
  "limit": 1000,
  "orderBy": [{ "columna": "customer_id", "direccion": "asc" }],
  "timeoutMs": 30000,
  "formato": "arrow"
}
```

- **Operadores** (`filtros[].operador`): `eq`, `neq`, `in`, `lt`, `le`, `gt`, `ge`, `like`,
  `isNull`, `isNotNull`. Son el vocabulario de `reads.predicatePushdown` desplegado: `range` de la
  tabla es `lt/le/gt/ge` aquí. `in` lleva una lista; `isNull` e `isNotNull`, ningún valor. Un
  valor con la forma de otro operador (un `in` con un valor suelto, un `eq` con una lista) rechaza
  la petición; un `in` con la lista vacía no deja pasar ninguna fila.
- **El vocabulario sigue cerrado y la regla sigue siendo la de 0008**: un operador que el conector no
  sabe expresar **no se ignora: se rechaza la petición entera**. Quien pide sólo manda lo que el
  conector declaró (§1.4); un rechazo es un defecto de quien pidió.
- **`limit`** sólo llega si el coordinador puede empujarlo (v1alpha24 §3: nada que quede en el
  motor quita filas antes de él). **`orderBy`**, igual, y **con los nulos al final en los dos
  sentidos**: es lo que hace DuckDB, que recibe las filas, y un `ORDER BY … LIMIT n` empujado tiene
  que dar las mismas n que daría el motor. Los orígenes no coinciden por su cuenta (PostgreSQL pone
  los nulos primero en `DESC`; BigQuery, en `ASC`), así que el conector escribe `NULLS LAST` siempre.
- **`timeoutMs`**: el conector lo aplica en el origen (`statement_timeout` en Postgres,
  `timeoutMs` del job en BigQuery) y no sólo en su proceso.
- `start`, `end`, `cursor`, `claves`, `fichero`: como hoy.

### 1.3 · La respuesta

- **Siempre Arrow IPC en flujo** (stream format), con un `RecordBatch` cada pocos miles de filas
  **mientras se lee**. Nunca el resultado entero en memoria: M3 midió 551 MB de memoria y 9 s de
  parseo para 10⁶ filas en texto.
- **Los tipos son los del árbol**: la columna se tipa por su `type` de OOS (y su `physicalType`
  cuando trae precisión), con la tabla de [ADR 0032](decisions/0032-el-contrato-de-tipos.md) y `Decimal<p, s>`; un valor que no
  cabe en su tipo es un error que nombra columna y fila, no un nulo.
- **Una tabla vacía devuelve su esquema** sin filas (ADR 0045 A5).
- En `servir`, cada respuesta va enmarcada (`ore_driver::servir`):

  ```text
  {"estado":"ok","id":…}\n                         la cabecera
  <u32 big-endian n><n bytes> …  <u32 0>            el flujo Arrow, en trozos de hasta 64 KiB
  {"bytes":B,"filas":N,"fin":"ok"}\n                el cierre
    ó {"codigo":…,"fin":"error","mensaje":…,"reintentable":…}\n
  ```

  o, si falla antes de dar un solo byte, una sola línea
  `{"codigo":…,"estado":"error","id":…,"mensaje":…,"reintentable":…}`. **En trozos y no el flujo
  Arrow tal cual** (F2·0): en `leer`, un error a mitad deja el flujo sin su marca de fin y el
  proceso termina; en `servir` el proceso sigue y lo siguiente que escribe es otra respuesta, que
  sin longitudes se leería como el resto del flujo roto.
- **Los errores son tipados**, una línea JSON por stderr (en `leer`) o en la cabecera (en
  `servir`): `{"codigo": "…", "mensaje": "…", "reintentable": bool}` con estos códigos:

| `codigo` | cuándo | reintentable |
|---|---|---|
| `operador` | un operador o un campo que no sabe expresar | no |
| `objeto` | el objeto o una columna no existen en el origen | no |
| `credencial` | el origen rechaza la credencial | no |
| `tiempo` | se agotó `timeoutMs` | sí |
| `conexion` | no se llega al origen | sí |
| `origen` | el origen falló por su cuenta | según el origen |

- **Ni la URL ni la credencial salen nunca** por stdout, stderr ni un mensaje de error.

### 1.4 · `capacidades`

```json
{
  "conector": "ore-read-postgres", "version": "2.0.0", "protocolo": 2,
  "operadores": ["eq", "neq", "in", "lt", "le", "gt", "ge", "like", "isNull", "isNotNull"],
  "limit": true, "orderBy": true, "estimar": true, "servir": true,
  "agregados": false, "juntas": false
}
```

Lo que se empuja es **la intersección** de esto con el `reads` de la tabla (v1alpha24 §3). `agregados`
y `juntas` quedan para F9.

### 1.5 · La sesión

- **De sólo lectura pedida al origen**, no prometida (`SET SESSION CHARACTERISTICS AS TRANSACTION
  READ ONLY` en Postgres; en BigQuery, la cuenta del driver sin permisos de escritura).
- **Una conexión no se comparte entre credenciales**: en `servir`, el conector guarda conexiones
  por `url` y las cierra a los 60 s sin uso.
- **Cancelar** es cerrar su stdin o `SIGTERM`: el conector cancela en el origen (`pg_cancel_backend`,
  `jobs.cancel`) antes de salir.

## 2. El kit de conformidad de los conectores

Un conector entra en la pasarela cuando pasa el kit. Es **lo que hace posibles cientos de orígenes**:
cada familia nueva se mide contra el mismo contrato, no contra la memoria de quien la revisa.

El kit levanta un origen de prueba de su familia (un contenedor, o un emulador), siembra unas
tablas fijas y corre contra el binario:

| # | caso | afirma |
|---|---|---|
| 1 | proyección | salen exactamente las columnas pedidas, con los nombres de la proyección |
| 2 | cada operador declarado | devuelve las filas que su semántica dice, con nulos y bordes (`in` vacío, `like` con `%` y `_`, rangos en los límites) |
| 3 | un operador no declarado | se rechaza la petición entera (`operador`) |
| 4 | `limit` y `orderBy` | `limit n` da n filas; con `orderBy`, las primeras n en ese orden |
| 5 | tipos | cada `type` de OOS sale con su tipo Arrow: enteros, `Decimal<p, s>` exacto, fechas, `DateTimeTz` con zona, texto UTF-8, nulos |
| 6 | tabla vacía | esquema sin filas |
| 7 | flujo | 10⁶ filas salen en lotes y la memoria del conector se queda por debajo de un techo fijo |
| 8 | `timeoutMs` | una consulta larga se corta en el origen y sale `tiempo` |
| 9 | sólo lectura | una petición que intentara escribir no puede |
| 10 | cancelar | cerrar stdin cancela la consulta en el origen |
| 11 | `servir` | cien peticiones seguidas reutilizan la conexión; una credencial distinta abre otra |
| 12 | secretos | ni la URL ni la credencial aparecen en ninguna salida, tampoco en los errores |
| 13 | `capacidades` | lo que declara es lo que los casos 2–4 comprueban |
| 14 | `estimar` (si lo declara) | estima sin leer: en BigQuery, sin bytes facturados |

**Hecho código** (F2·1): `crates/ore-conector-kit`, binario `ore-kit`.

```text
ore-kit --conector <binario> --banco postgres|s3 [--casos 1,2,5] [--informe i.json] [--exige todos]
```

- **La semilla** (`semilla.rs`): `tipos` (una columna por tipo escalar de OOS —`Integer`,
  `Decimal<12, 2>`, `Float`, `String`, `Date`, `DateTime`, `DateTimeTz`, `Boolean`— con los bordes de
  un `int64`, la cadena vacía frente al nulo, `_` y `%` en un texto, microsegundos, el 29 de febrero
  y una fila entera nula), `grande` (10⁶ filas) y `vacia`. Lo que cada petición tiene que devolver
  **lo calcula el kit** con la semántica de SQL y los tipos de `ore_core::tipos` (0032): ningún
  conector es la referencia de otro.
- **El banco** (`bancos/`): lo que aporta cada origen —cargar la semilla y contestar lo que sólo él
  sabe (consultas vivas, sesiones, escrituras)—. Postgres: el esquema `kit`, un rol de lectura que
  *puede* insertar en `kit.marcas` (así el caso 9 prueba la sesión de sólo lectura y no un
  permiso), una vista que tarda 20 s en dar su fila (casos 8 y 10). S3: Parquet en un bucket del S3
  de mentira (`pruebas-de-fuego/de-mentira.py`) o de un MinIO. Lo que un origen no puede contestar
  sale «no aplica», con su motivo.
- **Sin `capacidades`** (los conectores v1), se prueba igual —lo que el conector hace, se mide— y
  los casos que dependen de lo declarado fallan por no declararlo.
- **El tipo de un instante** se compara con UTC dicho de cualquier forma: el lago lo escribe
  `+00:00` (Iceberg) y BigQuery `UTC`.
- En el CI, el trabajo `conectores`: línea de base en el resumen; un conector v2 lo pasa con
  `--exige todos`.

## 3. La pasarela `ore-federation`

Un servicio **por celda** con el rol `driver` —la red a los orígenes y la cuenta del driver—, sin
estado y sin hablar con el custodio. Sólo `ore-serve` puede llamarla (NetworkPolicy de entrada).

| | ruta | qué |
|---|---|---|
| leer | `POST /v1/read` | una lectura; responde un flujo Arrow |
| cancelar | `DELETE /v1/read/{id}` | corta una lectura en curso |
| estado | `GET /v1/read/{id}` | cómo terminó una lectura (para quien no lee los *trailers*) |
| conectores | `GET /v1/connectors` | `capacidades` de cada familia instalada |
| orígenes | `GET /v1/origins` | por origen: lecturas activas, en cola, latencias, errores |
| salud | `GET /v1/health` | vivo y con conectores |

**`POST /v1/read`**

```json
{
  "id": "<uuid de la lectura, lo pone ore-serve>",
  "origen": "postgresql_20260918_1920",
  "tipo": "postgres",
  "url": "<la credencial, que ore-serve trajo del custodio para ESTA lectura>",
  "peticion": { "…": "§1.2" },
  "presupuesto": { "filas": 100000, "bytes": 67108864, "ms": 30000 }
}
```

- **Responde un flujo Arrow** (`application/vnd.apache.arrow.stream`) y termina con *trailers*:
  `ore-estado: completo | cortado | error`, `ore-motivo`, `ore-filas`, `ore-bytes`, `ore-ms`.
- **Lo que pasa del presupuesto se corta**: la pasarela deja de leer, cancela en el origen y cierra
  el flujo con `ore-estado: cortado` y el motivo (`filas`, `bytes` o `tiempo`).
- **La cola por origen**: como mucho `concurrencia` lecturas a la vez por origen; las demás esperan
  en cola hasta `esperaMax`; si la cola está llena o se agota la espera, `503` con `Retry-After` y
  `{"codigo": "saturado"}`.
- **Conectores calientes**: la pasarela mantiene un `servir` por familia y por credencial, y lo
  recicla tras un tiempo sin uso. La credencial vive en memoria mientras dura su conexión; no se
  escribe a disco.
- **Errores antes del primer byte**: JSON `{"codigo", "mensaje", "reintentable"}` con el estado HTTP
  correspondiente (`400` operador, `404` objeto, `502` origen o credencial, `503` saturado, `504`
  tiempo).

## 4. El coordinador `ore-serve`

Es quien **decide**. Una lectura en vivo entra por `POST /federation/read` (desde un puesto, con
`x-ore-puesto`, o desde un trabajo) o la origina el planificador de una consulta SQL (F5), y pasa
por estos pasos **en este orden**; el primero que falla corta y nada se conecta:

| # | paso | si falla |
|---|---|---|
| 1 | **la tabla**: resolver `b.s.n` en el árbol de la rama (`x-ore-rama`) y que sea una `Table` con `reads` distinto de `none` | `404` / `422` (`OOS2020`) |
| 2 | **el conducto**: `federation.read` autorizado y que admita las columnas pedidas y las de los predicados; desde un puesto, además `contextSurface.workspace` | `403` `OOS4011` / `OOS4002` / `OOS4001` |
| 3 | **el acceso**: la acción `dato:leer` sobre `tabla/<b.s.n>` para el sujeto (0047; cierra A8 para esta vía) | `403` |
| 4 | **el coste**: lo empujado es la intersección de `reads` y `capacidades`; `forbidden` sin filtro empujado, o un `requiredFilter` sin empujar | `422` `OOS2044` / `OOS2045` |
| 5 | **el presupuesto**: el tope de la lectura en vivo y, si la tabla es `expensive`, su presupuesto; si el conector sabe `estimar` y la estimación lo pasa, no se empieza | `422` con la estimación |
| 6 | **la credencial**: `fuente-<origen>` del custodio, como el agente de la celda | `502` |
| 7 | **leer**: `POST /v1/read` a la pasarela y pasar el flujo a quien pidió, sin juntarlo en memoria | lo que diga la pasarela |
| 8 | **anotar**: un evento `federation:read` en la actividad, siempre, también si falló | — |

**El evento `federation:read`**: `sujeto`, `act` (el agente, si lo hay), `rama`, `tabla`, `origen`,
las columnas pedidas, **los predicados sin sus valores** (columna y operador: un valor de filtro
puede ser un dato personal), `limit`, `filas`, `bytes`, `ms`, `estado` (`completo`, `cortado`,
`negado`, `error`) y el `motivo`.

## 5. Los valores por defecto

Configurables por celda y, los del origen, por fuente (configuración de la celda, no del árbol: un
árbol publicable no dice cuánto aguanta la base de un cliente).

| | por defecto | de dónde sale |
|---|---|---|
| tope de una lectura en vivo | **100 000 filas o 64 MB** | ADR 0053 · M3: 10⁶ filas en texto = 128 MB |
| tiempo máximo | **30 s** | ADR 0053 · M2: 2·10⁶ filas de BigQuery, ~117 s sin freno |
| presupuesto de una tabla `expensive` | **10⁶ filas, 1 GB estimado, 30 s** | v1alpha24 §4; el GB por el *dry run* de BigQuery |
| concurrencia por origen | **4**, con cola de **16** y espera máxima de **10 s** | ADR 0053 · M3: 50 a la vez, 7 fallan |
| conexión caliente sin uso | se cierra a los **60 s** | |
| `maxRowsPerRequest` | el de la tabla; sin él, 50 000 por petición | v1alpha24 §3 |
