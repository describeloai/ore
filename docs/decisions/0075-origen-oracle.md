# 0075 · El origen Oracle

**Estado:** **aceptado** (2026-10-25) · capa 1 hecha (el conector, su banco del kit y el cableado) · el e2e en la
consola, el CDC y el segundo driver (OCI) **sin hacer** · **Decide:** Oracle Database es un origen más de ORE
—`oracle://`, `ore-read-oracle`— y se lee con **`oracledb`, el driver *thin* oficial de Oracle** (el protocolo nativo,
TNS/TTC, hablado desde Rust, **sin Instant Client**). Sirve **Oracle 12.1 o posterior**; **11g, el cifrado nativo de
red, Kerberos, RADIUS, LDAP y las wallets** quedan fuera, dichos con su porqué en la consola de credenciales. Los
tipos se traducen a los de OOS como lo hace PostgreSQL (`Opaque` para lo que el sistema de tipos no modela), los
parámetros se atan **tipados** y el catálogo es el de `ALL_TAB_COLUMNS`. La base se nombra por **servicio** (`/SERVICIO`) o
por **SID** (`?sid=ORCL`, con un descriptor, que el Easy Connect no sabe decir): como en Foundry, uno u otro.

La **imagen completa del origen** —cómo está hecho, lo medido en detalle, la hoja de ruta, cómo se prueba— está en la ficha
[`docs/products/0075-origin-oracle.md`](../products/0075-origin-oracle.md); este ADR es la decisión.

## Por qué un solo conector «Oracle»

Fivetran publica dos (`Oracle` y `HVA Oracle`, ambos de base de datos, más `Oracle E-Business Suite` aparte) y
Airbyte uno. Todos son **un conector de la base relacional**: tablas y vistas por SQL, y un segundo nivel, el CDC por
el redo log, que Airbyte no tiene. Lo demás del ecosistema (las aplicaciones sobre Oracle, NoSQL, Essbase) son
orígenes distintos. Este ADR es el **primer nivel** —snapshot e incremental por cursor—; el CDC con LogMiner es una
capa posterior y **no se declara hasta que se lea** (`cambia: none`).

## Cómo se nombra la base: servicio o SID

Foundry ofrece las dos (*Service name* | *SID*), y el thin las habla: medido el 2026-10-25 contra Oracle 26ai, un
descriptor con `(CONNECT_DATA=(SID=FREE))` conecta a la instancia (`FREE`, contenedor raíz) y un SID que no existe da
`ORA-12505` con su nombre. El **servicio** es lo normal (y lo único de una PDB, que no tiene SID); el **SID** es para una
base sin servicios o que solo se anuncia por él. Como el SID entra tal cual en el descriptor, solo se admiten los
caracteres que Oracle admite en uno (`A-Z a-z 0-9 _ $ # . -`): con un paréntesis se podría cerrar el nodo y añadir otro.

## Por qué el driver thin

Se midieron dos antes de elegir (2026-10-25, contra Oracle 26ai Free en Docker):

| | `oracle-rs` 0.1.7 (de un tercero) | `oracledb` 26.0.0-beta.4 (`oracle/rust-oracledb`) |
|---|---|---|
| tipos (24 de la sonda) | **corrompe** `BINARY_FLOAT/DOUBLE`, `NVARCHAR2`, los dos `INTERVAL` y `BOOLEAN` (bytes crudos decodificados como texto); `XMLTYPE` falla | los lee bien, salvo `XMLTYPE` |
| versiones | 12.1+ (protocolo ≥ 315) | 19c, 21c y 26ai declarados |
| API | asíncrona | **síncrona**, como el trait `Origen` |

`oracledb` es de los mantenedores de `python-oracledb`, con licencia UPL/Apache. Es una **beta**: se fija exacta
(`=26.0.0-beta.4`). El camino contrario —ODPI-C (`rust-oracle`) con el Instant Client— no se descarta, pero no entra
aquí: el cliente es una librería propietaria de glibc, y la imagen de los drivers es Alpine.

**Lo que el thin no cubre**, medido o leído del apéndice oficial: **Oracle 11g** (los dos drivers rechazan el
protocolo 314: *«connections to this database server are not supported»*), el cifrado nativo de red (NNE), Kerberos,
RADIUS, tokens, LDAP, las wallets SEPS, TAF y Application Continuity. Con una parte del parque sin actualizar, la
salida para 11g es un **segundo driver sobre OCI**, no un parche de este.

## Los tipos

El catálogo da `DATA_TYPE`, `DATA_PRECISION` y `DATA_SCALE`; el tipo de OOS sale de ahí (`tipos.rs`), con la regla
de PostgreSQL: lo que OOS no modela es `Opaque` y lo que no se reconoce se catalogará **sin tipo** y no se lee.

| Oracle | OOS |
|---|---|
| `NUMBER(p, 0)`, `p ≤ 18` | `Integer` |
| `NUMBER(p, s)`, `NUMBER(p, 0)` con `p > 18`, escala negativa, `NUMBER(*, 0)` | `Decimal<p, s>` (≤ 38 dígitos) |
| `NUMBER` sin precisión | `String` con el valor **exacto**; se declara `type: Decimal<p, s>` para tiparlo |
| `FLOAT`, `BINARY_FLOAT`, `BINARY_DOUBLE` | `Float` |
| `CHAR`, `VARCHAR2`, `NVARCHAR2`, `CLOB`, `NCLOB`, `LONG`, `ROWID` | `String` |
| `DATE` (lleva hora), `TIMESTAMP` | `DateTime` (a microsegundos; los ns se pierden y se avisa) |
| `TIMESTAMP WITH [LOCAL] TIME ZONE` | `DateTimeTz` (un instante, en UTC) |
| `BOOLEAN` (23ai) | `Boolean` |
| `RAW`, `LONG RAW`, `BLOB` | `Binary` (v1alpha35; sus bytes, en base64 por el cable) |
| `INTERVAL YEAR TO MONTH`, `INTERVAL DAY TO SECOND` | `Opaque` (ISO 8601: `P12Y3M`) |
| `JSON` | `Opaque` (su texto, con las claves ordenadas) |
| `XMLTYPE`, `VECTOR`, `BFILE`, objetos | se catalogan y **no se leen** |

Dos declaraciones que la tabla puede hacer y que el dato tiene que cumplir **valor a valor** (un valor que no cabe es
un error con su columna y su fila, nunca un nulo ni un redondeo; 0032 §4): `Date` sobre un `DATE`/`TIMESTAMP` (falla
si trae hora) e `Integer` sobre un `NUMBER(19, 0)`, `NUMBER(38, 0)` o `NUMBER` libre (falla si tiene decimales o
desborda un `i64`). **`''` es `NULL` en Oracle**: una cadena vacía no existe, y el kit lo mide (`vacio_es_nulo`).

**`Binary` (2026-10-26).** `RAW`, `LONG RAW` y `BLOB` se catalogan `Binary` y se leen como `binary` de Arrow; antes eran `Opaque`
(base64 como texto). Medido contra Oracle 23ai: un `RAW(16)`, un `RAW` de 4 bytes y un `BLOB` con acentos vuelven con sus bytes exactos, y
un filtro `guid = <base64>` se ata como `RAW` y encuentra la fila. Una tabla **ya declarada** `String` u `Opaque` sobre esas columnas sigue
leyendo lo mismo (base64 como texto): no se migra nada por detrás (B0), y el catálogo nunca propone esa forma. **Una limitación del driver**:
un `BLOB` vacío (`EMPTY_BLOB()`, no nulo en Oracle) llega **nulo**; en un `RAW` no existe el vacío (Oracle lo guarda `NULL`), así que el
contrato «nulo ≠ vacío» solo se rompe para el `BLOB`. Va al reporte a `oracle/rust-oracledb`.

**Los tipos que faltan** —`Map`, `Interval`, un semiestructurado (`Variant`)— se vieron aquí y **no se
resuelven aquí**: son de la especificación OOS, no de un origen, y Oracle, Mongo y Cassandra los piden a la vez. Hasta
entonces `Opaque` con el `physicalType` guardado, que es lo que PostgreSQL ya hace con `bytea`, `interval` y `json`. El análisis completo —qué tienen Foundry, Spark, Iceberg y Arrow, qué hace cada origen hoy, la
propuesta para `Binary` y `Map`, la compatibilidad con las copias ya hechas y las decisiones abiertas— está en la ficha, §12; y
lo que esto dice de `ore-arrow` y de `ore-store::carga` (dos sitios donde `Fisico` se vuelve Arrow), en su §11.

## Lo que la forma común tuvo que aprender

`ore-sql` extrajo la forma de una consulta de tres dialectos (PostgreSQL, MySQL, BigQuery) con la apuesta escrita:
*«si aparece un quinto eje, la forma estaba extraída de dos casos y no de la clase»*. Oracle es el cuarto y **trajo
dos ejes**: no tiene `LIMIT` (es `FETCH FIRST n ROWS ONLY`, desde 12c) ni `FALSE` (el booleano llega en 23ai; `1 = 0`
vale en todas). Entran como datos del dialecto (`Limite`, `falso`), como los tres primeros, y la forma aguanta. Además
`Consulta` devuelve ahora **la columna de cada parámetro** (`columnas_de_parametros`).

## Los parámetros se atan tipados

`ore-sql` entrega los valores como texto canónico de OOS (`2026-10-03T23:59:59`, `…Z`). Oracle convierte un
parámetro de texto según el `NLS` de la sesión, y ese formato no es el suyo (`ORA-01861`); fijar un `NLS` que lo
admita no basta (la `Z` no existe en ninguna máscara de zona) y dependería de cómo se abrió la sesión. Se sabe la
columna con que se compara cada parámetro y su tipo en el catálogo, así que **una fecha se ata como `OracleTimestamp`,
un instante como `OracleTimestamp` con zona y un lógico como `bool`**; el resto va como texto, que Oracle coacciona
sin ambigüedad. Antes de esto el kit fallaba 4 de las 30 pruebas de operadores (`eq`, `ge`, `lt` sobre fechas).

## Cancelar y el tiempo

`oracledb` **no tiene una llamada para interrumpir una sentencia** (solo `call_timeout`, que al vencer **cierra la
conexión**, y al doble del tiempo). Se hace con lo que hay, en tres escalones:

1. **la bandera**, que se mira en cada fila: mientras las filas fluyen, corta en milisegundos;
2. **`ALTER SYSTEM KILL SESSION`** desde otra conexión **si el usuario puede** (muere en 0,08 s): corta también una
   sentencia que aún no ha dado su primera fila;
3. **`timeoutMs`** de la petición, que se clasifica como `tiempo` y, al vencer, **mata la sesión propia** si puede.

El tercero no es un detalle: sin él, al vencer el tiempo el driver cierra *su* lado y Oracle sigue ejecutando la
consulta hasta el final, gastando la base de otro. **Sin el permiso `ALTER SYSTEM`** (lo normal en un cliente) la
lectura se corta igual, pero la sentencia termina sola en el servidor: queda dicho en la consola de credenciales.

## Lo medido

Contra Oracle 26ai Free en Docker, en la misma máquina (así que **cotas inferiores**):

- **10⁷ filas, 6 columnas:** el driver solo, 300 k filas/s (33 s) sin convertir y 200 k filas/s convirtiendo a texto;
  el conector completo, **139 k filas/s** (72 s, release) con Arrow IPC. Memoria de 8–29 MB: streaming real.
  `fetch_array_size` óptimo **5 000** (100 → 116 k filas/s; 1 000 → 266 k; 20 000 → 276 k con el triple de memoria).
- **Fiabilidad:** `KILL SESSION` desde otra conexión hace fallar la lectura con un error claro en milisegundos; un
  servidor que deja de contestar (`docker pause`) acaba en error a los 2× el `call_timeout`.
- **El kit de conformidad, en Linux:** **12 de 12** casos aplicables (el 9 y el 14 no aplican); **la pasarela** con
  Oracle detrás, 8 de 8 (2 no aplican). El caso 7: 10⁶ filas en 16 s con 15 MiB de pico.

## En la imagen de verdad (Alpine, musl)

Medido el 2026-10-25 con `rust:1.90-alpine`, los `apk` del `Dockerfile` **sin añadir nada** y el perfil `imagen`:
**compila** (5 min en frío, `aws-lc-sys` incluido: el `cc`/`gcc`/`clang` que la imagen ya trae basta; no hace falta
`cmake`, `perl` ni `go`), el binario pesa 6,5 MB, es estático (solo el cargador de musl) y arranca. **El kit pasa
12 de 12** con ese binario, incluidos `servir` con hilos, cancelar y el `timeoutMs`. El flujo de 10⁷ filas da
**127 000 filas/s** en musl frente a 139 000 en nativo (~9 % menos): el `malloc` de musl, que penaliza a HBase y a las
teselas por sus hilos, aquí no pesa, porque el driver es síncrono y lee con un solo hilo. **No lleva `mimalloc`** y no
hace falta; si la pasarela llegara a abrir muchas sesiones calientes a la vez, es lo primero que se mide.

## Un fallo del driver que queda abierto

`oracledb` 26.0.0-beta.4 falla con *«unknown TTC message type N»* en un caso concreto: una tabla de 3 filas, una de
ellas casi toda `NULL`, leída con `fetch_array_size ≥ 3` y una proyección que mezcla un `CLOB`/`BLOB`/`JSON` con
otras columnas (el mínimo hallado, reduciendo a mano: `T_TSLTZ`, `I_YM`, `I_DS`, `R_RAW`, `L_CLOB`, `L_BLOB`,
`J_JSON` y `B_BOOL`, más el `ID`). Con `fetch=1` o `2` funciona. **No se reproduce** con 15 columnas de tipos
comunes, ni con una tabla de 10⁵ filas con `CLOB`, `BLOB` y nulos en cualquier tamaño de lote. Es un fallo de decodificación del driver, no del conector. El repro está en
`pruebas-de-fuego/oracle/` (`LEEME.md`) y hay que **reportarlo a `oracle/rust-oracledb`**.

## Qué se aceptó a cambio

- **Una beta** como driver, fijada exacta, con el fallo de arriba.
- **Sin 11g ni NNE ni Kerberos.** Es un techo del protocolo, no de este crate; subirlo es un segundo driver.
- **Una pila TLS aparte.** `oracledb` trae `rustls` con `aws-lc-rs` (C compilado por `aws-lc-sys`) y las raíces de CA
  de Mozilla **incrustadas** (`webpki-roots`); los demás drivers usan el OpenSSL del sistema y su almacén. Son dos
  políticas de confianza en el mismo repositorio. No entra en el cierre de `ore` (`dependencias.rs`: 12 de 12 sin
  tocar un tope). **El TLS de Oracle (`tls=true`) no está probado**: el laboratorio no tiene un listener TCPS.
- **Una conversión por celda** (texto canónico y de ahí a Arrow) en vez de una directa: 139 k frente a 200–300 k
  filas/s. Hay margen y no se ha tomado.
- **`cambia: none`.** Oracle tiene redo log y SCN, pero este conector no los lee y no declara lo que no hace.

## `ore-arrow`: de tipo OOS a tipo Arrow, en un solo sitio

**Qué es hoy.** El crate que arma el lote de Arrow de un origen a partir de los valores de OOS (`Valor`): lo usan
Oracle, Phoenix, Cassandra, HBase y Mongo, que escribían cada uno lo mismo. **Conoce únicamente los 9 físicos
escalares** (`Fisico`): `Entero`, `Real`, `Decimal`, `Logico`, `Instante`, `Fecha`, `FechaHora`, `Hora` y `Texto`. No
conoce `Forma` (struct, lista, vector, geometría). Cada origen convierte *su* celda a un `Valor` y decide qué error dar
si no cabe; el crate nunca convierte, y **una fila entra entera o no entra**.

**Qué quiere ser.** El **único sitio del código Rust de la plataforma que sabe cómo se materializa un tipo de OOS en
Arrow**. Hoy ese mapa está en tres sitios: `ore-arrow` (solo `Fisico`), `ore-store::carga` (`Fisico` y `Forma`, y es
el contrato del lago) y `ore-read-postgres` (su propio `Col`, con geometría y listas). El nombre se mantiene y el alcance
se amplía **por fases, un PR por fase**, después de fusionar el PR #30:

| fase | qué | lista de tipos que conoce `ore-arrow` |
|---|---|---|
| **0** | la cabecera del crate dice su alcance | los 9 escalares |
| **1** | el constructor acepta `Forma` | + `Struct`, lista, `Vector<n>` y geometría |
| **2** | el mapa de tipos (`arrow_de_forma`, `campo`, `arrow_del_oos`) pasa de `ore-store::carga` a `ore-arrow` | igual; deja de estar duplicado |
| **3** | el lote tipado de `carga::lote` se construye con `ore-arrow` | igual |
| **4** | `ore-read-postgres` deja su `Col` | igual |
| **5** | una función `emitir` para el escritor IPC de los ocho lectores | igual |
| **6** | BigQuery y objetos, a la vista de lo anterior | igual |

**Qué no es, tampoco al final.**
- No es el Arrow de los SDK ni del agente del puesto: viven en Python, Node y JVM, y cumplen el mismo contrato (0032)
  por su cuenta. `over()`, `sql()` y `arrow()` leen el lago con DuckDB; `write()` manda Arrow por IPC al agente, que
  llama a `ore-store escribir`.
- No es el contrato del lago: validar y conformar un lote que *ya es* Arrow (`al_contrato`, `conformar`, `huella`) se
  queda en `ore-store`.
- Las **dos reglas de error se quedan separadas**: el origen da un error con columna y fila; el lago acepta una columna
  de texto «honesta» si un valor no analiza (`sin_estrechar`).
- No es una dependencia de `ore`: Arrow no puede entrar en el cierre del compilador (`dependencias.rs`).

**Salvaguardas.** El digest (`huella`) tiene que seguir siendo idéntico byte a byte (la fase 3 no se fusiona sin una
prueba que lo demuestre); cada PR conserva los recuentos de pruebas de los crates que toca; una abstracción que empeore
la escritura al lago (10⁷ filas en 2,1 s) se descarta.

**`Binary` y `Map`, que OOS aún no tiene.** Arrow sí los tiene (`Binary`, `Map`), así que `ore-arrow` no es el
obstáculo: lo que falta es **la especificación de OOS** (ficha §12), y de ahí salen `ore-base` y los demás.
`RAW` y `BLOB` ya son `Binary` en Oracle; `INTERVAL` y `JSON` siguen en `Opaque`. Orden recomendado:
1. **`Binary` antes de la fase 1.** Es un escalar (`Fisico::Binario`, `Valor::Bytes`), lo piden todos los orígenes,
   `carga` ya sabe escribirlo (la geometría es `Binary`) y no cambia nada en el cable (ya viaja en base64). Hacerlo
   antes evita extender el constructor dos veces. Lo que lo condiciona es la compatibilidad de las copias ya hechas
   (`string` → `binary` no es un ensanche, ficha §12.8), que hay que decidir antes de tocar un origen.
2. **`Map` con la fase 1**, como un compuesto más (`Forma::Mapa`). Solo lo pide Cassandra hoy y Oracle no lo
   necesita, así que no adelanta nada.
3. **`Interval` y `Variant` no urgen**; siguen en `Opaque` hasta que un cliente los pida.

## Cableado

`ore-oracle` y `ore-read-oracle` (fuera del cierre de `ore-cli`, como Phoenix, Cassandra y Mongo) · el dialecto
`ORACLE` en `ore-sql` · el banco `oracle` del kit (`ORACLE_KIT_URL`, feature `oracle`) · el binario en
`compilar-binarios.sh` y en la imagen de los drivers · el quinto grupo del job `conectores` del CI (`gvenzl/oracle-free`)
· el formulario de credenciales de la consola (`ore-serve`) con los cinco modos que **no** se admiten · la detección
de una URL con credencial. La `NetworkPolicy` de los drivers ya deja el 1521 (1024–65535).

## Lo que falta

El e2e en la consola (alta, descubrimiento, base *foreign* y *standard*) con 2–3 escenarios de cliente · el CDC con
LogMiner · `XMLTYPE` (envolver la columna en el `SELECT`) y `VECTOR` · el TLS contra un listener TCPS · afinar la conversión por celda ·
las wallets y TLS con CA propia (Autonomous) · el reporte del fallo del driver.
