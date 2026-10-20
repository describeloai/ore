# 0069 · Origen MongoDB

**Leer MongoDB** —Atlas, un replica set propio, un clúster con shards o uno de los compatibles—
como origen de ORE. Es el primer origen **sin esquema** que entra como base de datos: lo que en
Postgres y BigQuery es un catálogo que se pregunta, aquí es un esquema que se **infiere**.

**Estado:** en diseño (2026-10-19). X0 casi cerrado: medido contra un Atlas M0 de desarrollo con el
usuario de solo lectura, y contra un `mongo:7` local (TLS, 10⁶ documentos, cancelación); el crate de
Rust, compilado y probado en Alpine/musl; **M1 a M6 hechos** (el crate, el binario, su cableado, los cambios, la hoja de la consola y la prueba de fuego contra un Atlas, §5); falta el caso 10 del kit en Linux, `estimar` y lo que la consola enseña del esquema inferido. **Hoja de ruta mientras se desarrolla; al cerrarse,
la imagen** de lo que es el origen MongoDB en ORE. Prioridad en [`0066`](0066-geo-paradigm.md) §2:
*nice-to-have*, subida por petición el 2026-10-19.

Sigue a [`0008`](0008-el-protocolo-del-driver.md) (el protocolo del driver),
[`0032`](0032-el-contrato-de-tipos.md) (el contrato de tipos), [`0042`](0042-origin-rest-bigquery.md),
[`0061`](0061-origenes-de-objetos.md) y [`0065`](0065-postgres-origen-de-verdad.md) (el patrón de
un origen nuevo: medir primero, construir después).

## 1. Qué es

Un conector `ore-read-mongodb` con el contrato de siempre (`docs/federation.md` §1): verbos
`catalogo`, `leer`, `testigo`, `check`, `explorar`, `capacidades`, `estimar` y `servir`; Arrow IPC
en flujo; errores tipados; la URL por stdin; solo lectura pedida al origen. Entra en la pasarela
cuando pasa el kit de conformidad (14 casos).

Lo que lo distingue de los otros orígenes de base de datos:

- **El esquema se infiere**, por muestreo. No hay un catálogo que preguntar.
- **Los tipos son BSON**, no SQL: `ObjectId`, `Decimal128`, fechas en milisegundos, binarios, UUID.
- **Los documentos se anidan**: la spec manda aplanarlos (§4).
- **La petición se traduce a MQL**, no a SQL, y `null`, `eq` sobre arrays y el orden no se
  comportan como en SQL (§5).
- **Los cambios son change streams**, y su límite es el oplog.

Funciona con Atlas y con un replica set propio, probados. DocumentDB, Cosmos DB (API Mongo) y
FerretDB se declaran en `capacidades` y quedan «de su documentación», como los perfiles de S3 en
0061.

## 2. Lo que hacen los demás

Leído el 2026-10-19. Databricks y Foundry **no se pudieron verificar** (abajo).

| | Fivetran | Airbyte (MongoDB V2) |
|---|---|---|
| carga inicial | `find()` por colección, en trozos | `find()` con puntos de control |
| incremental | change streams (4.0+, WiredTiger, pv1); `_id` es la clave; borrado lógico | change streams; modo `Lookup` (por defecto) o `Post Image` (6.0+) |
| cursor caducado | re-sync automático y aviso | falla (por defecto) o re-sync |
| esquema | *packed* (`_id` + JSON `data`, recomendado) o *unpacked* (un nivel) | muestreo `$sample` (10 000 por defecto) o *schemaless* (`_id` + `data`) |
| anidados | JSON, sin tablas hijas | parseados al esquema; en *schemaless* dentro de `data` |
| tipos | ObjectId→STRING, Date→INSTANT, Decimal128→BIGDECIMAL, Binary/UUID→STRING, arrays y docs→JSON; sin soporte: DBPointer, Symbol, MinKey, MaxKey | polimorfismo: envuelve un objeto donde se esperaba un array |
| despliegues | replica set y sharded 4.0+; **serverless no** | Atlas, replica set, sharded (2.1.0+) |
| permisos | `readAnyDatabase@admin` o `read@<db>` | `readAnyDatabase` para todo el clúster; `read` de una base |
| red | PrivateLink, Private Link de Azure, PSC, proxy agent; TLS 1.1–1.3 | connection string; respeta la read preference |
| límites | no se excluyen campos; sin vistas ni `system.*`; 3 conexiones por base | `BSONObjectTooLarge` en eventos de >16 MB; el muestreo en paralelo carga el clúster |

Coinciden en cinco cosas, y por eso son el punto de partida: (1) el esquema se infiere por
muestreo y existe una **salida sin esquema** (el documento entero como JSON); (2) los anidados no
se parten en tablas hijas; (3) la clave es `_id`; (4) lo incremental son change streams y su
límite es el oplog; (5) el permiso es `read`.

**No verificado:** *Databricks* no tiene, que se haya podido encontrar, un conector administrado
de MongoDB —se usa el conector de Spark, y su página dice que esas configuraciones «no están
respaldadas ni probadas» por ellos—. *Foundry*: la página del conector devolvió 404 y la búsqueda
no dio documentación oficial (solo un hilo de comunidad: URI de conexión y permitir sus IP en
Atlas). No se afirma nada de cómo tratan el esquema.

## 3. Lo medido

Contra un **Atlas M0** (tier gratuito), MongoDB **8.0.34**, replica set de 3 nodos, con el usuario
`readAnyDatabase` (2026-10-19). Los datos de prueba, en una base propia (`ore_prueba`: 30
documentos de tipos y bordes, 2 000 polimórficos, 100 anidados, ids de tipos distintos, una
colección vacía, una con validador, una vista y una *time series*); la de ejemplo de Atlas
(`sample_mflix`, 21 349 películas) no se tocó.

| qué | medido | lo que supone |
|---|---|---|
| conexión `mongodb+srv` | entra; `hello` da `setName`, 3 hosts | la URL es un connection string; SRV pide DNS |
| rol de lectura | `readAnyDatabase@admin`; un `insert` da `8000 not allowed to do action [insert]` | la solo lectura la da el **rol**; no hay un `SET … READ ONLY` que pedir |
| `{v: null}` | casa **3**: el `null` explícito, el campo **ausente**, y un **array que contiene `null`** | `isNull` no es el `eq` con null: es `$type:"null"` ∪ `$exists:false` |
| `{v: x}` sobre un array | busca **dentro** del array | un `eq` ingenuo devuelve filas que SQL no devolvería |
| orden ascendente de `edad` (null/ausente/int/str) | `[None, None, 1, 2, '14', '7']` | nulos **primero** en `ASC` y **últimos** en `DESC`; los tipos mezclados siguen el orden de BSON (null < números < texto) |
| `NULLS LAST` en ambos sentidos | no existe nativo | o se emula (pipeline con un campo calculado) o `capacidades` declara `orderBy: false` |
| `like` por `$regex` | `^100%_` casa lo esperado | `%`/`_` se traducen y los literales se escapan; el caso 2 del kit los prueba |
| `maxTimeMS=1` | `50 ExecutionTimeout` | se mapea al código `tiempo` (reintentable) |
| change stream, usuario de solo lectura | abre y da `resume_token` | el CDC no pide un usuario con más privilegios |
| oplog (`local.oplog.rs`) | **43 entradas**; la más antigua tiene ~3,9 h y la última, ~3,1 h (el clúster está en reposo). El tamaño máximo no se pudo leer: `collStats` sobre `local` da `Unauthorized` incluso con `atlasAdmin` | en reposo el oplog guarda todo lo escrito desde hace horas; cuánto aguanta **bajo carga** depende de su tamaño en bytes, que en un M0 no se puede consultar. *(La primera medida, «~48 min», era la edad de la entrada más antigua, no el límite de retención: se retira.)* |
| fecha 9999-12-31 23:59:59.999 | se lee entera | cabe en `timestamp[us]`; el límite inferior y superior de Arrow/Parquet falta medirlo |
| vista | se lee como una colección | se cataloga; sin testigo propio |
| *time series* | `type: timeseries` y una `system.buckets.*` | saltar `system.*`; la serie, tabla |
| validador | `options.validator.$jsonSchema` legible | si existe, **manda sobre el muestreo** |
| `explain` de un filtro sin índice | `COLLSCAN` | `estimar` posible (`explain`, `estimatedDocumentCount`) |
| `$sample` de 1 000 sobre `movies` (21 349 docs, 21 campos; el menos frecuente en el 33 %) | ve **los 21 campos**; 0,26 s. De 10 000: también 21/21, 1,99 s. Escaneo completo: 4,2 s (5 100 docs/s) | una muestra pequeña basta para el primer nivel; `$sample` cuesta ~1/16 del escaneo |
| `$sample` sobre documentos grandes (`embedded_movies`, ~24 KB) | 1 000 → 2,3 a 3,3 s; **10 000 → error 292** (`Sort exceeded memory limit of 33554432 bytes`), también con `allowDiskUse` | en M0 el límite de 32 MB no se levanta: el tamaño de la muestra se limita por `32 MB / avgObjSize` (el `collStats` de las colecciones de usuario sí se puede leer) |
| `NULLS LAST` emulado (`$project` de un marcador + `$sort`) | correcto en **los dos sentidos** sobre `polimorfica` (321 nulos/ausentes, todos al final); cuesta ~14 % más que el `sort` nativo (0,42 s frente a 0,37 s) | se puede declarar `orderBy`. La emulación **no usa índice**: con `limit` solo guarda las *k* primeras filas; sin `limit`, un orden grande choca con los 32 MB |
| crate `mongodb` 3.9.1, `openssl-tls` + `dns-resolver` | **223 crates** (Postgres: 114). Sin `dns-resolver`: 143, pero `mongodb+srv` (Atlas) no resuelve; con `rustls-tls` en vez de OpenSSL: 227; añadiendo `aws-auth` (MONGODB-AWS): 314 | el peso es asumible fuera de `ore-cli` (el guardián vigila el cierre de `ore-cli`, 190); `aws-auth` es una feature aparte |
| compilación en `rust:1.90-alpine` (la imagen de ORE) con `musl-dev openssl-dev openssl-libs-static` | compila en **4 min 40 s** en frío; el binario (LTO, `strip`) pesa **9,7 MB** y solo enlaza `ld-musl` | entra en la imagen de drivers como los demás |
| ese binario contra el Atlas, desde un contenedor Alpine | `mongodb+srv` resuelto en 121 ms; `ping` a los 0,75 s; `find` de 12 docs en 47 ms; `$sample` de 500 en 133 ms; change stream abierto con `resume_token` | TLS, SRV y change streams funcionan con el cliente de Rust |
| lectura de 10⁶ documentos (`mongo:7` local, 67 MB) | BSON crudo: **347 000 docs/s**, pico de memoria **7 MB**; decodificado a `Document`: 95 000 docs/s, 8 MB | el caso 7 del kit (flujo con techo de memoria) se cumple de sobra; conviene construir el Arrow desde BSON crudo, no desde `Document` |
| TLS con una CA propia (`mongo:7` local, certificado firmado por esa CA) | con `tlsCAFile`: conecta; con `tlsCAFile` + certificado de cliente (mTLS): conecta; **sin** la CA: `certificate verify failed`; con el servidor en una IP que no está en el certificado: `certificate verify failed` | la verificación está siempre activa; `tlsInsecure` no se ofrece |
| cancelar (`mongo:7` local) | `killOp` sobre la operación (la ve `$currentOp`): la consulta termina con el error **11601** a los 0,02 s; `maxTimeMS=1500` corta a los 1,50 s (**50**); `cursor.close()` libera el cursor del servidor | el caso 10 del kit se cumple: cerrar stdin cierra el cursor; `killOp` solo si el usuario puede verlo |

**Sin medir todavía:** la autenticación **x509** (`MONGODB-X509`, con un usuario en `$external`) y
**AWS IAM** (`MONGODB-AWS`, la de Atlas con IAM), que piden la feature `aws-auth`; un **clúster con
shards** (`mongos`); los compatibles —**DocumentDB**, **Cosmos DB** (API Mongo) y **FerretDB**: este
último se intentó y su imagen de evaluación pide una autenticación propia que no se configuró—;
el tamaño del oplog de un clúster de pago (en el M0 no se puede leer). **Medido después, en M4**
(§5): `currentOp` y `killOp` con un usuario de solo lectura en Atlas, y el banco del kit con `servir`.

## 4. Cómo está hecho

Lo que sigue es el diseño de partida. Lo que no está medido lo dice; el resto cambia si una
medida lo contradice.

1. **Un binario, `ore-read-mongodb`, con el connection string como URL.** `mongodb://…` y
   `mongodb+srv://…`. Los sabores no llevan código propio: lo que cambia entre ellos se declara
   en `capacidades` (si hay change streams, si `$sample`, etc.).
2. **El esquema se infiere por muestreo, y la salida segura es el documento.** El catálogo toma
   una muestra acotada con `$sample` (tamaño y tiempo máximo configurables, contra un secundario si
   lo hay; 1 000 documentos bastan para el primer nivel, y el tamaño se limita por `32 MB / avgObjSize`
   para no chocar con el límite de ordenación del M0) y, si la colección tiene `$jsonSchema`, **manda el validador**. Un campo que no se deja
   tipar —tipos mezclados— se queda **`String`** y el informe dice cuál y por qué (la tercera vía
   de 0032 §4: ni un nulo inventado ni un fallo por una fila). Una colección puede declararse **sin
   esquema**: `_id` más el documento entero como JSON. Pasar de un modo al otro rehace la copia.
3. **BSON → escalar de OOS** (0032):

   | BSON | OOS | nota |
   |---|---|---|
   | Int32, Int64 | `Integer` | `Int64` entra entero; el JSON de la consola lo cadena sobre 2⁵³ |
   | Double | `Float` | `NaN`/`±Infinity` viajan (Arrow los tiene) |
   | Decimal128 | `Decimal<p, s>`; con más de 38 cifras, **`String` exacto** | como BigQuery (0042) |
   | String | `String` | |
   | Boolean | `Boolean` | |
   | Date | `DateTimeTz` (UTC) | el rango se mide |
   | ObjectId | `String` (hex de 24) | como Fivetran |
   | Binary, UUID | `Opaque`; UUID subtipo 4 como `String` | a cerrar con la consola |
   | Timestamp (interno) | `Opaque`/`String`; no es un instante de negocio | |
   | Array | `list<T>` si es homogéneo; si no, JSON | |
   | Document | ver 4 | |
   | Regex, Code, Symbol, DBPointer, MinKey, MaxKey | **citado y no traducido**, dicho en el catálogo | como un `STRUCT` en 0042 |
4. **Anidados.** OOS manda aplanar (v1alpha1 §4.4) y la ruta física se conserva (`awards.wins`).
   De partida, **sin tocar la spec**: los **documentos** se aplanan hasta una profundidad
   configurable (por defecto 3), con la ruta física completa como `column`; los **arrays de
   documentos** y lo que pase de la profundidad quedan como JSON en una columna, no como tablas
   hijas (ni Fivetran ni Airbyte las hacen). Las structs de primera clase en OOS son cosa de la
   spec (`C:\oos`) y quedan fuera.
5. **La petición se traduce a MQL.** `eq/neq/in/lt/le/gt/ge` → operadores de MQL; `like` →
   `$regex` con escape; `isNull` → `$type:"null"` ∪ `$exists:false`; la proyección es **cerrada**
   (0008): el `find` solo pide lo proyectado. `limit` → el cursor; `timeoutMs` → `maxTimeMS`;
   `estimar` → `explain`. `orderBy` con los nulos al final se **emula** (medido, §3) y se declara; sin
   `limit` y sobre un conjunto grande el servidor puede negarse por memoria, y ese fallo sale como
   error tipado. Sobre una columna **escalar**, `eq`, `neq`, `in`, los
   cuatro de orden y `like` excluyen los arrays (`$not: {$type: "array"}`: el `eq` de Mongo busca
   dentro), `neq` deja fuera `null` y ausente, e `isNull` se escribe con `$type` agregado. Una columna
   de tipos mezclados o compuesta **no se empuja**: se rechaza con `operador` (medido contra `mongo:7`,
   §5 M1). Lo que el conector no declare se **rechaza**, nunca se ignora.
6. **Credencial y red.** La URL lleva el usuario y la contraseña, que van al **cofre** como
   `fuente-<n>` (como Postgres y S3 con clave): no se canjea nada, y una URL con secreto **no
   viaja en un Job** —se comprueba al catalogar, no antes del alta—. **Permiso mínimo**: `read`
   sobre las bases que se declaran; `readAnyDatabase` solo para explorar. En Atlas, el cliente
   pone la **IP de salida fija de la celda** (`ORE_IP_SALIDA`) en su lista de acceso; PrivateLink y
   PSC quedan fuera hasta que se pidan. TLS por defecto; `tlsCAFile` y x509 se miden en X0.
7. **Testigo y cambios.** El testigo es `log` con el `operationTime`/`clusterTime` (ordinal, como
   un LSN). Los **change streams** —abiertos con un usuario de solo lectura, medido— son el camino
   incremental (M4), con el `_id` como clave y el borrado como señal. Si el cursor caduca (el oplog
   de un clúster con carga puede ser corta; el tamaño del oplog de un M0 no se puede leer), **no se inventa continuidad**: la pasada dice «re-sync
   necesario» (el modo seguro de Airbyte) y la copia se rehace entera. Un origen sin change streams
   declara `changes: none`.
8. **Qué se cataloga.** Colecciones y vistas; **fuera** `system.*`, `admin`, `local` y `config`;
   una *time series* como tabla; los índices informan `primaryKey`/`uniqueKeys` solo cuando son
   únicos. `_id` siempre es la clave (`changes.key`).

## 5. La hoja de ruta

Cada hito se construye y se prueba **en local** contra un `mongo:7` en Docker y contra el Atlas de
desarrollo; la prueba contra otros sabores queda como deuda.

| hito | qué | laboratorio | estado |
|---|---|---|---|
| **M0 · medir (X0)** | lo que falta de «Sin medir todavía» | el Atlas M0 y `mongo:7` en Docker | **casi cerrado**: §3; queda x509/AWS IAM, shards, compatibles y el banco del kit |
| **M1 · el crate y el traductor** | `ore-mongodb` (transporte; tipos BSON→Arrow) y el traductor petición→MQL como **función pura comprobable sin base de datos** (la proyección cerrada es un aserto, 0008) | pruebas unitarias; `mongo:7` y el Atlas M0 | **hecho** (2026-10-19) |
| **M2 · `ore-read-mongodb`** | `catalogo` (muestreo y validador), `leer` en Arrow en flujo, `check` paso a paso (red, DNS/SRV, TLS, identidad, listar, leer), `explorar`, `capacidades`, `testigo` | `mongo:7`; los 14 casos del kit | **hecho en lo esencial** (2026-10-19): el kit da 7 pasan, 3 fallan y 2 no aplican; el 10 espera a Linux |
| **M3 · cableado** | los **tipos declarados** en la petición; `ore-cli` (el esquema `mongodb+srv`), `ore-serve` (asistente de credenciales), `ore-federation` (carga `mongodb`), imagen, `ci/compilar-binarios.sh`, CI con un MongoDB de servicio, banco del kit (`bancos/mongodb.rs`) | kit directo y por la pasarela | **hecho** (2026-10-19), salvo la malla (§ M3) |
| **M4 · cambios** | la cara `D` sondeada, el testigo ordinal, `cambio`, y la lectura por posición con change streams (provisional); la posición caducada | `mongo:7` en replica set, con el oplog truncado de verdad; el Atlas M0 con el usuario de solo lectura | **hecho** (2026-10-19) |
| **M5 · consola** | `MongoDbConnection.tsx` en el wizard (`rubix-platform`), lo que el driver admite y lo que no | tipado, la composición de direcciones y el render de la hoja | **hecho** (2026-10-19) la hoja; **no** la comprobación por pasos antes del alta ni el esquema inferido en el catálogo (abajo) |
| **M6 · prueba de fuego** | `pruebas-de-fuego/mongodb.sh`, de punta a punta; **cada protección se rompe a propósito** para ver que muerde | el Atlas de desarrollo, con el usuario de solo lectura; un replica set local | **hecho** (2026-10-19) |

### M1 · hecho (2026-10-19)

El crate **`ore-mongodb`** (`crates/ore-mongodb`), fuera del cierre de `ore-cli`. Siete módulos, 44 tests
unitarios sin servidor y 8 de laboratorio contra un `mongo:7` y el Atlas M0 de desarrollo: **52/52**.

| módulo | qué hace |
|---|---|
| `traductor` | `Peticion` → `Plan::Find` o `Plan::Agregado`, **función pura**. Los diez operadores, el recorte por clave, el rango del cursor (`start` exclusivo, `end` inclusivo), `limit`, `timeoutMs` → `maxTimeMS`. La proyección es cerrada, con `_id: 0` si no se pide, y una columna que es prefijo de otra se pide una vez. El orden con nulos al final es un pipeline de cinco pasos con un marcador por criterio y desempate por `_id` |
| `esquema` | `Observador` + `Esquema`: el tipo de OOS de cada hoja a partir de una muestra, con los avisos de lo que no se dejó tipar; documentos aplanados hasta una profundidad, arrays como JSON, tipos mezclados como texto |
| `lote` | documentos BSON crudos → `RecordBatch` de Arrow con el físico de 0032 (`Decimal128`, `Timestamp[us, UTC]`…); un valor que no cabe es un error que nombra **columna y fila** |
| `decimal` | `Decimal128` → `i128` escalado **sin pasar por `f64`**, o error que dice por qué no cabe |
| `transporte` | abrir, `ping`, bases, colecciones (sin `system.*`; vista y serie temporal; validador), muestra (`$sample` limitada por memoria), leer con cancelación, testigo (`operationTime`), y el mapa de errores a los siete códigos del conector |
| `url` | valida (`tlsInsecure` y compañía **no se admiten**) y enseña la URL sin la credencial |

**Lo que los tests contra un servidor de verdad confirmaron** (los mismos casos medidos en X0, ahora como
aserto): `eq` no casa dentro de un array; `neq` deja fuera `null` y ausente; `isNull` no confunde un array
que contiene `null`; `like` no tiene escape, el `.` es un punto y `_` cruza un salto de línea; los nulos
van al final en `ASC` y en `DESC`, con y sin `limit`; un instante con microsegundos sueltos se redondea
según el operador (`eq` no casa nada) y `10.50` es `10.5`; el `find` devuelve **solo** lo proyectado; un
`maxTimeMS` agotado sale como `tiempo`.

**Todavía no hay**: el binario `ore-read-mongodb` y el flujo Arrow IPC enmarcado (M2), `estimar`
(`explain`) y `servir`, el cableado (M3), los change streams (M4: un rango por la posición del origen se
rechaza diciéndolo) y una feature `aws-auth` para IAM.

### M2 · hecho en lo esencial (2026-10-19)

**El esqueleto se escribió una vez.** `ore_driver::origen` (`crates/ore-driver/src/origen.rs`) es lo que
repetían `ore-read-postgres` y `ore-read-bigquery`: despachar `capacidades`/`leer`/`estimar`/`servir`,
leer la petición, el `SIGTERM` que cancela en el origen y las sesiones de `servir`. Un origen nuevo
implementa el trait `Origen` (conectar, leer a Arrow, estimar, cancelar y qué declara) y su `main` es
una línea. El binario `ore-read-mongodb` (`crates/ore-read-mongodb`) es eso más `ore_mongodb::verbos`
(`catalogo`, `check`, `explorar`, `testigo`). **No se tocaron** los lectores existentes.

| verbo | estado |
|---|---|
| `capacidades`, `leer` (Arrow), `servir`, `catalogo`, `check`, `explorar`, `testigo` | hechos, medidos contra `mongo:7` |
| `estimar` | **no** (`explain` honesto con filtros es trabajo propio); se declara `estimar: false` |

**El kit** (`ore-kit --banco mongodb`, con `--features mongodb`; `MONGO_KIT_URL` apunta a un Mongo de
pruebas con permiso de escritura):

| caso | | |
|---|---|---|
| 1 proyección, 3 operador no declarado, 4 `limit` y `orderBy`, 6 tabla vacía | **pasan** | |
| 7 flujo | **pasa** | 10⁶ filas en 87 s con un binario de depuración, primer byte a los 790 ms |
| 8 `timeoutMs` | **pasa** | `tiempo` a los 1062 ms con `timeoutMs: 1000`, nada vivo en el origen |
| 11 `servir` | **pasa** | 100 peticiones por una sesión |
| 12 secretos | **pasa** | ninguna credencial en 45 salidas |
| 2 operadores, 5 tipos, 13 capacidades | **fallaban** | con los tipos declarados (M3) **pasan** |
| 9 sólo lectura, 14 `estimar` | no aplican | |
| 10 cancelar | **pendiente** | el caso usa `kill -TERM`; falta correrlo en Linux. La misma ruta por `servir` (`{"cancelar": id}`) corta la operación en el servidor en **54 ms** |

**Los tres que fallaban (2, 5 y 13) tenían una sola causa, y se cerró en M3** con los tipos
declarados. La semilla del kit usa `Date`, `DateTime` sin zona, microsegundos y `Decimal<12, 2>`; BSON
tiene `Date` (un instante, en milisegundos) y `Decimal128` (sin precisión). El conector entregaba lo que
el dato **es** y el kit marcaba la diferencia: lo que la tabla declara y el dato no lleva.

**Lo que se aprendió construyéndolo**, y que no estaba en X0:

- **`currentOp` de una vista enseña el comando de fuera** (`aggregate: "lenta"`), no el pipeline de
  dentro. La primera versión del banco buscaba el `$function` en el texto y nunca lo veía: «nada vivo
  en el origen» salía cierto por vacío. Se reconoce por el nombre de la vista.
- **Un `$function` de JS no se interrumpe por dentro**: una función que duerme 20 s no atiende
  `maxTimeMS` hasta que termina (medido: error 50 a los 20 s, no a 1 s). Solo se mira entre
  documentos. La vista lenta del banco es de 10⁶ documentos que duermen 20 ms cada uno.
- **Una vista no admite `$where`**, sí `$expr` con `$function`.
- **Cancelar es `killOp`, no cerrar el cursor**: la lectura marca su operación con un `comment`, y
  `cancelar` la busca con `currentOp` y la mata. Pendiente de medir con un usuario **de solo lectura**
  en Atlas (X0 lo dejó sin medir).
- **Una colección sin documentos no tiene tipos que inferir**: `leer` contesta el flujo de la
  proyección en texto y sin filas (el caso 6), y no guarda el esquema vacío: mañana puede haberlos.
- **Un cliente de Mongo abre varias conexiones** (monitorización y pool), así que las sesiones se
  cuentan por el `appName`, que el conector pone distinto a cada cliente.

**Todavía no hay**: `estimar`, el caso 10 en Linux y el banco con autenticación (para la segunda
credencial del caso 11 y la mala del 12).

### M3 · hecho (2026-10-19)

**Los tipos que la tabla declara viajan en la petición.** Es la pieza que faltaba y la única que toca el
protocolo. `Peticion.tipos` (`ore_driver`) es `[[columna, tipo de OOS], …]` en la clave `tipos` de primer
nivel; los orígenes con catálogo (Postgres, BigQuery) lo ignoran, y los de ficheros siguen llevando los
suyos en `fichero.tipos`. El motor lo manda en los tres sitios donde arma una petición
(`federar.rs` y dos de `materializar.rs`) para toda tabla **sin `format`**; con `format` va como
`fichero`, como siempre.

En MongoDB lo declarado manda sobre lo inferido **si el dato lo puede llevar** (`Tipo::declarado`) y, si
no, es `esquema` con la columna nombrada, nunca una conversión a escondidas:

| la tabla declara | el dato es | sale |
|---|---|---|
| `String` | lo que sea, también mezclado o compuesto | texto |
| `Date` / `DateTime` | un `Date` de BSON | el día UTC (`date32`) / la hora de pared UTC (`timestamp[us]` sin zona) |
| `DateTimeTz` | un `Date` de BSON | instante |
| `Decimal<p, s>` | `Decimal128` o entero | decimal con **esa** precisión y escala |
| `Integer`, `Float`, `Boolean` | lo mismo (`Float` también desde entero) | lo mismo |

Un `Date` es un **tramo**, no un punto: `eq` es `[inicio, inicio + 1 día)`, `<=` es `< el día
siguiente`, `<>` son los dos tramos de fuera. Hay 7 pruebas unitarias nuevas (51) y la del motor.

**Lo que el motor ya sabe de MongoDB**: `ore source add mongodb+srv://…` se normaliza a `mongodb` (el
`+srv` es cómo se encuentra el servidor, no otro origen); `ore-federation` carga `mongodb` por defecto;
`compilar-binarios.sh` y la imagen lo llevan (los guardas de huella pasan); el asistente de credenciales
de `ore-serve` enseña el usuario con el rol `read`, la IP de salida y lo que no admite
(`tlsInsecure`, una CA por fichero, un usuario que escribe, IAM de AWS).

**El kit, ahora**: 10 pasan, 0 fallan, 2 no aplican (9 y 14) contra `mongo:7`; **por la pasarela de
verdad, 8 pasan, 0 fallan, 2 no aplican**, incluida la copia completa de 10 filas. Para que el 5 y el
2 sean exigibles, el banco declara `milisegundos()`: BSON trunca los microsegundos al **escribir**, antes
de que ORE vea el dato, y el kit espera lo que el origen tiene.

**Lo que la pasarela destapó**: un `catalogo` caía entero porque **una** vista no se dejaba muestrear.
Ahora esa colección se salta con su aviso y su motivo, y solo falla si no se pudo con ninguna.

**No se hizo**: la malla (`malla/`) —no hay despliegue que probar aquí—, y el caso 10 en el CI
(`--exige` lo deja fuera hasta verlo en verde en Linux).

### M4 · hecho (2026-10-19)

**Lo que el contrato define, y se hizo:**

| pieza | qué hace |
|---|---|
| **La cara `D`, sondeada** | `catalogo` declara `{upsert, key: [_id], witness: log}` en una colección de un replica set o de un `mongos`, y `{none, none}` en una vista, una serie temporal o un servidor suelto (con su aviso). Es un hecho del servidor (`hello`), no una preferencia: lo mismo que `wal_level` en Postgres |
| **`testigo`** | el `operationTime`, con **ancho fijo** (`0000000009:0000000010`): comparar el texto es comparar el instante. Sin relleno, `9:10` es mayor que `10:2`, y un testigo ordinal que se ordena mal es peor que ninguno |
| **`cambio`** (0063 S3) | `respalda: false`: MongoDB no tiene contadores por colección que cuenten escrituras (`count` y tamaño no se mueven con una actualización en el sitio). Una colección que no está es un **error**, no «no respalda» |

**Lo que el contrato no define, y se hizo provisional.** El rango por la posición del propio origen (0016:
`start` exclusivo, `end` inclusivo, sin `cursor`) lee un change stream con `startAtOperationTime`. Las
filas son las columnas pedidas más **`_ore_op`** (`upsert` o `delete`): un alta, una actualización o un
reemplazo entregan el documento tal como está ahora (`updateLookup`), y un borrado su `_id`, que es
exactamente un tombstone por clave. Si el documento ya no existe cuando se pide, la actualización se
salta —su borrado viene después y lo dirá—. Si `end` falta, es el presente, **tomado antes de empezar**.

> ⚠️ **`_ore_op` es una invención de esta pieza.** 0016 define el rango pero no la forma de las filas de
> un changelog, y 0063 dice que ningún driver lo sabe leer todavía: el motor no consume nada de esto. Es
> la forma más corta que deja distinguir un alta de un borrado. Cuando el motor fije la suya, se cambia
> en `COLUMNA_OPERACION` y en nada más. Un changelog tampoco se filtra, ordena ni recorta en el origen (un
> borrado no trae el documento): se rechaza diciéndolo, como `start` sin posición o una posición que no es
> de este origen.

**La posición caducada, medida de verdad.** Con el oplog reducido al mínimo (990 MB) y 1,1 GB escritos
(`pruebas-de-fuego/mongodb/caduca-el-oplog.py`), una posición anterior a la primera entrada del oplog da
`ChangeStreamHistoryLost` (**286**): *«Resume of change stream was not possible, as the resume point may
no longer be in the oplog»*. El conector lo dice como `origen`, no reintentable, **«posición caducada:
hay que copiar entera»**, y falla **antes del primer byte** (se abre el change stream antes de escribir la
cabecera del flujo). Tres cosas que no estaban en ninguna documentación leída:

- **Un servidor que nunca perdió historia acepta cualquier posición anterior**: una de 1970 no falla. El
  error solo existe si el oplog se truncó. Quien pruebe la caducidad sin truncarlo prueba otra cosa.
- **No se degrada en silencio** a un recorrido completo (la regla de Debezium y Snowflake, ADR 0016):
  falla, y la copia se rehace entera.
- Una colección **borrada o renombrada** a mitad de rango también corta (`objeto`, «no hay continuidad»).

**Lo que Atlas enseñó, y un servidor local sin autenticación no** (usuario de **solo lectura**, el que usa
el conector):

- Los change streams **abren** y aceptan una posición de hace 30 min; `currentOp` funciona.
- **`killOp` sobre una operación propia funciona** (la operación se interrumpe 0,1 s después), aunque
  sobre una ajena o inexistente da `not authorized`. Un usuario no necesita el privilegio `killOp` para
  cancelar lo suyo: el conector no pide más permiso que `read`.
- ⚠️ **Mi primera `matar` no habría encontrado nada en Atlas.** Sin `$ownOps: true` un usuario sin
  `inprog` no ve ni sus operaciones, y la igualdad sobre `command.comment` en el filtro del comando **no
  casa** (`$exists` sí) —además, en la fase de `getMore` el comentario puede estar en
  `originatingCommand`—. Ahora trae las operaciones propias y compara el comentario en el cliente.

**Lo que no se hizo**: nada consume el changelog todavía (el motor), el testigo no se prueba contra un
`mongos` con shards, y la caducidad se mide en un servidor local, no en un Atlas (en un M0 el oplog no se
deja truncar a voluntad).

### M5 · hecho (2026-10-19), en `rubix-platform`

**La hoja de MongoDB** (`components/ingestion/wizard/MongoDbConnection.tsx`) sigue el patrón de la de
Postgres: la dirección se compone en un solo sitio (`direccionMongo`, la misma que se enseña y se manda) y
los modos salen de lo que `GET /fuentes/credenciales/mongodb` contesta (M3), no de una copia en el
cliente. Se abrió MongoDB en el registro (`isComingSoon` fuera) y se conectó al paso de conexión, a
`tipoDe` y a la tarjeta de protección de datos.

| lo que se elige | la dirección que sale |
|---|---|
| Atlas, o un clúster con nombre SRV | `mongodb+srv://usuario:clave@cluster0.xxxxx.mongodb.net[/base][?authSource=…]` |
| hosts directos (un replica set propio, o uno suelto) | `mongodb://usuario:clave@m1:27017,m2:27017[/base][?replicaSet=…&authSource=…]` |
| una cadena escrita a mano | manda ella, como en las demás hojas |

Lo que la hoja **no deja pasar**, con el mismo criterio que el driver (`ore-mongodb::url`), para que se
diga al escribirlo y no en el primer uso: `tlsInsecure`, `tlsAllowInvalidCertificates` y
`tlsAllowInvalidHostnames` (y sus nombres `ssl…`) con un valor que no sea `false` o `0`; un puerto o una
lista con `mongodb+srv` (el DNS dice dónde están los nodos); una cadena que no empiece por `mongodb://` o
`mongodb+srv://`. La contraseña se codifica (`@`, `:` y `/` son normales en una) y se tapa al enseñar la
dirección.

**Medido, no supuesto**: la composición de direcciones pasa 16 casos ejecutados (Atlas con una contraseña
con `@:/`, `authSource` sin base, hosts directos con replica set, la cadena a mano que manda, cuatro
formas de apagar TLS, y `tlsInsecure=false`/`=0` que sí valen); el render en servidor de la hoja, con la
respuesta de `ore-serve` simulada, pasa 11 comprobaciones (etiquetas, qué se oculta con SRV, lo no
admitido sale de la celda, los campos se declaran ignorados cuando manda la cadena); `tsc --noEmit` limpio.
**No se vio en un navegador**: el repositorio no tiene suite de interfaz y no hay un `ore-serve` detrás en
esta máquina.

**Lo que no se hizo, y por qué**:

- **«Comprobar acceso» antes del alta.** Esa comprobación se hace con una dirección **sin secreto**
  (`url_sin_secreto`, que `ore-serve` rechaza si lleva uno) y la de MongoDB lleva la contraseña dentro,
  como la de Postgres: se prueba al catalogar. Una comprobación por pasos (red, DNS/SRV, TLS, identidad,
  listar, leer) pide que el `check` del conector conteste permiso a permiso y que exista un modo de
  comprobar con la credencial en el custodio; ninguno de los dos está.
- **Lo que el esquema infiere y lo que no**, en el catálogo: la consola no pinta todavía los avisos que
  el conector escribe por `stderr` (tipos mezclados que quedan en texto, documentos más profundos que 3
  niveles como JSON) ni las columnas que son `Mixto`.
- **Una CA propia.** Va por fichero (`tlsCAFile`) y el driver corre en un Job sin tus ficheros; la hoja
  lo dice como no admitido. Se arregla cuando una CA pueda ir **dentro** de la dirección, como en
  Postgres (`sslrootcert`), y MongoDB no tiene esa opción: hay que pasarla por otro camino.

### M6 · hecho (2026-10-19)

`pruebas-de-fuego/mongodb.sh` (con `mongodb/origen.py` y `mongodb/arrow.py`): **41 comprobaciones contra
un Atlas de verdad** con el usuario de **solo lectura**, más 4 contra un replica set local que escribe.
Lo que se compara con el origen lo dice **otro cliente** (pymongo): una copia que casa porque el conector
se compara consigo mismo no demuestra nada.

| sección | qué demuestra |
|---|---|
| **1 · alta y descubrimiento** | `ore source add mongodb+srv://…` da tipo `mongodb`, el manifiesto declara de qué variable sale la credencial y no cuál es, `discover` propone 7 tablas en DRAFT con la cara de cambios **sondeada** (`upsert`/`log`; la serie temporal, ninguna), el anidado se aplana en la entidad y conserva la ruta física en la tabla, la colección **vacía** y la **vista** quedan como decisiones y no se inventan |
| **2 · la copia** | una vista SQL sobre `sample_mflix.comments` (41 079 documentos): el rango de fecha se **empuja** al clúster, llegan 6 608 y son las que cuenta pymongo, y lo calculado en DuckDB casa **año a año** con un `$group` directo |
| **3 · por la pasarela** | `ore-federation --tipos mongodb`: las mismas 6 608 filas, contadas como una copia |
| **4 · protecciones rotas** | ver abajo |
| **5 · replica set local** | el kit (12 / 0 / 2), el kit **por la pasarela** (8 / 0 / 2), un alta, una actualización y un borrado entre dos posiciones como `upsert`, `upsert`, `delete`, y con `MONGO_CADUCAR=1` una posición **caducada de verdad** |

**Las protecciones de la sección 4, cada una por el lado en que se rompe**: `tlsInsecure`,
`tlsAllowInvalidCertificates` y `tlsAllowInvalidHostnames` en la URL se rechazan, también al leer; una
contraseña que Atlas rechaza es `credencial` y el error no la repite; un servidor al que no se llega es
`conexion` y reintentable; un operador fuera del vocabulario es `operador`, no un filtro ignorado; filtrar
una columna de tipos mezclados se rechaza explicando por qué, no devuelve «0 filas»; un tipo declarado que
el dato no lleva es `esquema` con la columna; pedir 2 columnas de un documento de 5 devuelve esas 2; `timeoutMs: 1`
es `tiempo`; el rol de la fuente intenta **escribir y Atlas se lo niega**; un changelog no se filtra; y
**ninguna orden de la prueba escribió la contraseña**, ni en su salida ni en el árbol (solo `.env.local`).

**Y la prueba se probó a sí misma.** Una guarda que nunca se ha visto negarse tiene el mismo aspecto que una
que no existe, y casi me pasa: 4b pasaba **por la razón equivocada** (mi URL de «contraseña mala» llevaba un
byte `` donde iba ``, estaba rota, y una URL rota también es `credencial`). Ahora la prueba comprueba
primero que la URL está bien formada y que Atlas contesta `SCRAM failure: bad auth`. Y se sabotea el conector
a propósito, una protección cada vez, para ver que la prueba se pone roja donde debe:

| se rompió | qué se puso roja |
|---|---|
| la URL puede apagar TLS | **4a**, las cuatro comprobaciones (y `leer` con `tlsInsecure` leyó de verdad) |
| una credencial mala deja de ser `credencial` | **4b** |
| `maxTimeMS` agotado deja de ser `tiempo` | **4h** |
| una columna de tipos mezclados se empuja | **4e** |
| un tipo declarado imposible se acepta | **4f** |
| un changelog se deja filtrar | **4j** |
| el vocabulario cerrado, quitado de **una** capa | **nada**: lo sostiene también el traductor (defensa en dos capas, medida) |
| el vocabulario cerrado, quitado de **las dos** | **4d** |

**Lo que los sabotajes destaparon**: sin esas guardas el conector no devolvía un error, **entraba en
pánico** (código 101): el resto del código daba por imposible que una columna mixta o un operador
desconocido llegaran hasta ahí. Ahora el traductor lo rechaza como `operador`, con su motivo: una **tercera
capa**, y por eso quitar una sola ya no pone nada rojo. Es lo que se quería, aunque haga más difícil
sabotear la prueba.

**Lo que esta prueba no cubre**, dicho: la proyección cerrada **en el servidor** (4g mira lo que sale; que
el `find` pida solo lo proyectado lo sostienen los tests del traductor); cancelar con `killOp` (cubierto por
`se_cancela_en_atlas_con_un_usuario_de_solo_lectura`, que lanza un `$lookup` sin índice y lo mata en 0,1 s);
un `mongos` con shards; y que **no corre en el CI**: necesita la URL de un Atlas con su contraseña, como
`bigquery-real.sh`. Lo que sí corre allí es el kit y la pasarela contra un `mongo:7` (M3).

## 6. Límites conocidos

- **Un esquema inferido puede estar equivocado.** Una muestra no ve un campo que aparece tarde
  (la colección polimórfica de la prueba lo hace a propósito). El informe lo dice y el modo sin
  esquema existe; no se promete más.
- **`null`, `eq` sobre arrays y el orden no son los de SQL**, y el traductor lo paga con código y,
  quizá, con operadores que se declaren de menos.
- **Los anidados profundos van como JSON** hasta que la spec tenga structs.
- **El oplog tiene un tamaño fijo en bytes**: bajo carga, lo incremental puede degradarse a re-sync, y en un M0 ni siquiera se puede consultar cuánto aguanta.
- **El crate `mongodb` de Rust es asíncrono y pesado**: 223 crates con `mongodb+srv` y OpenSSL
  (Postgres tiene 114), con `tokio` dentro. Vive en su binario, fuera del cierre de `ore-cli` que
  vigila `dependencias.rs`; las credenciales de AWS IAM suman ~90 más y quedan para una feature aparte.

## 7. Fuera de este producto

- Escribir en Mongo: un origen se lee (0061).
- PrivateLink, PSC y túneles hasta que se pidan; Atlas Data API, Atlas SQL y el BI Connector
  (sustituibles, y fuera del contrato de driver).
- Tablas hijas por array, y `$lookup` entre colecciones (las uniones son del motor, F9).
- Structs de primera clase en OOS (spec, `C:\oos`).
- DocumentDB, Cosmos DB y FerretDB como «probados»: quedan declarados.

## Fuentes

[Fivetran · MongoDB](https://fivetran.com/docs/connectors/databases/mongodb) ·
[Airbyte · MongoDB V2](https://docs.airbyte.com/integrations/sources/mongodb-v2) ·
[MongoDB · Change streams](https://www.mongodb.com/docs/manual/changeStreams/) ·
[Databricks · MongoDB](https://docs.databricks.com/en/external-data/mongodb.html) ·
OOS v1alpha1 `02-entity` §4.4. Medidas: `ore_prueba` en el Atlas de desarrollo, 2026-10-19.
