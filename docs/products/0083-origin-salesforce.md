# 0083 · Origen Salesforce

**Leer Salesforce** —los objetos estándar y personalizados de una org— como origen **tabular** de ORE: un driver
`ore-read-salesforce` que implementa `ore_driver::origen::Origen` (`capacidades`, `leer`, `estimar`, `cancelar`,
`servir`, más `catalogo`, `check`, `explorar`, `testigo` y `cambio`), con **SOQL** como lenguaje de consulta, un
traductor propio de la petición a SOQL, y lectura por **REST** y por **Bulk API 2.0** hacia Arrow.

**Estado (2026-10-31): X0 medido y pasos 1 a 4 hechos, sin commit.** Rama `origen-salesforce`, worktree
`C:\ORE-salesforce` (sale de `origin/main` `9cd59268`). Todo está en el árbol de trabajo, **sin commitear ni
empujar**. §1 es lo construido y decidido; §2 lo publicado por otros; §3 lo medido en una Developer Edition; §5 la
hoja de ruta con su estado; **§6 lo que falta y lo que no se pudo verificar**; **§7 cómo retomar**.

Sigue el método de [`0070`](0070-origin-cassandra.md), [`0077`](0077-origin-clickhouse.md),
[`0080`](0080-origin-cockroachdb.md) y [`0082`](0082-origin-google-drive.md): medir primero, construir después. El
modelo más cercano es **MongoDB** (traductor puro, lo que no se sabe escribir se rechaza y lo demás se reevalúa en el
conector) con la identidad de **Drive** (sin sesión de usuario).

## 1. Lo que se decidió y se construyó

Cada punto cita la medición de §3 que lo sostiene (A1…H2).

1. **Un origen tabular nativo, no un dialecto Airbyte.** Implementa `Origen` de `ore-driver` (el de filas, no el de
   `ore-objetos`, que es de bytes). La razón es medida: **SOQL empuja la proyección** (B1), así que la garantía de
   `ore-driver` —«una propiedad `redact` no está en el plan, luego no está en la petición»— se cumple. Con Airbyte
   debajo, la columna sale del origen y se tira después. La fila «API SaaS: a medias» de
   `pruebas-de-fuego/medida-el-protocolo-de-fuentes.py` **no vale para Salesforce** (y `reads.projectionPushdown:
   false`, OOS2029, ya existe para las APIs que sí leen el stream entero: esas pruebas están desfasadas en ese punto).
2. **Cliente propio sobre `ureq 2` + `native-tls`**, como `ore-gcp` y `ore-gdrive`; ningún crate de la comunidad
   (`salesforce_core`, `force`, `cirrus`: de menos de 8 meses y de uno o dos autores, con `reqwest` y `tokio`).
   **`ureq 2` con `native-tls` no trae el conector TLS: hay que dárselo** (`native_tls::TlsConnector`), como hace
   `ore-gcp`. Lo cazó el laboratorio, no una prueba pura. `tonic` solo hará falta para el CDC (paso 6).
3. **Identidad: JWT bearer sobre una External Client App** (F1, F2), firmado en Rust con `rsa` 0.9 + `sha2` 0.10
   (el mismo par que `ore-entrada`; sin `ring` ni OpenSSL). Ámbitos **`api` y `refresh_token`**: el segundo es
   obligatorio aunque no se emita ningún refresh token (F1). Política «administradores preautorizados» sobre un perfil.
   - **La URL:** `salesforce://<dominio>/?cliente=<Consumer Key>&usuario=<usuario>[&clave=<PEM, con %XX>][&version=68.0][&objetos=A,B]`.
     La Consumer Key y el usuario **no son secretos**.
   - **La clave privada** va en `clave=` dentro de la URL, que es como la guarda el custodio (la `conexion` de un Job,
     como la contraseña de Postgres o Mongo). Si no está, se lee de `ORE_SALESFORCE_CLAVE` (el PEM) o
     `ORE_SALESFORCE_CLAVE_FICHERO` (su ruta): es lo que usan el laboratorio y los tests.
   - `ore_driver::fallo::es_secreto` **tapa `clave`** (no contenía «key»: era un hueco). `Fuente` no imprime la clave
     en `{:?}`. La URL **no viaja en claro por la cola** (`url_sin_secreto` la rechaza, como a Postgres/Mongo).
   - Un JWT firmado es una credencial hacia quien lo reciba: **solo se manda a `*.salesforce.com`, `*.force.com` y
     `*.salesforce-setup.com`**. Sandbox y producción son una variable de la fuente (el host), no dos conectores.
   - **No se admite OAuth de usuario** ni el refresh token rotatorio: el cofre no sabe reescribir un secreto y la
     ingesta no debe depender de una persona (como Drive, 0082 D-O2). Anotado como un posible segundo modo.
4. **El catálogo sale de `describe`** (A1–A5): solo los objetos `queryable`; `replicateable` se informa, no filtra
   (591 son consultables y no replicables). Columnas planas: un **compuesto** (`address`, `location`) no es columna y
   sus subcampos sí (B6); ⚠ **`Account.Name` declara `compoundFieldName: "Name"` apuntando a sí mismo** (A3): «tiene
   `compoundFieldName`» NO significa «es un subcampo»; los `base64` se excluyen (A5, C4); las **fórmulas** se marcan en
   la descripción como no incrementales. `catalogo` sin `objetos=` recorre un **núcleo de 13 objetos de CRM** más los
   propios (describir los ~1.700 gastaría ~11 % de la cuota diaria): **decisión mía, a revisar**.
5. **`reads` y el traductor** (`traductor.rs`, puro). `projectionPushdown: true`; `predicatePushdown: eq, neq, in,
   range, like, isNull`; `fullScan: expensive`; `maxRowsPerRequest: 2000`. El traductor:
   - emite **siempre `NULLS LAST`** (B9: el orden por defecto pone los nulos primero **en ASC y en DESC**; y el
     contrato de `Orden` dice que van al final en los dos sentidos);
   - hace que **`neq` numérico excluya los nulos** (`(c != v AND c != NULL)`): SOQL los incluye (B3) y SQL no;
   - trata el texto como **superconjunto**: SOQL no distingue mayúsculas (B2). `eq`, `like` e `in` se empujan **y
     quedan como residual**; `neq`, `<`, `<=`, `>`, `>=` de texto **no se empujan** (un `!= 'a'` de SOQL quitaría
     `'A'`, que SQL devuelve) y quedan solo como residual. El residual se reevalúa exacto, con mayúsculas y sin nulos
     (`residual.rs`);
   - **trae la columna del residual aunque no se proyecte** (`Traduccion::columnas`) y la tira al armar el lote;
   - **no empuja el `LIMIT` al servidor si hay residuales**: cortaría a *n* y el residual quitaría algunas, dando de
     menos. Corta quien lee, ya reevaluado. (Un error de diseño del paso 1, corregido en el 2.)
   - paréntesis en todo `NOT` (B8); rechaza texto largo (B7), compuestos en `WHERE` (B6), comparar dos campos (B12),
     un valor que no es del tipo del campo (no hay inyección) y un `LIKE` con `\`;
   - sin `OFFSET` (tope 2000, B10); parte las listas `IN` largas en consultas de ≤ 8.000 caracteres (B14).
6. **Dos vías de lectura: REST y Bulk 2.0** (`lectura.rs`, `bulk.rs`). Siempre se empieza por REST: su primera página
   trae **`totalSize`**, el total de filas, sin un `COUNT()` aparte. Si pasa de **50.000** y no hay un `limit` corto,
   se descarta esa página y se hace por Bulk. `ORE_SALESFORCE_API=rest|bulk` y `ORE_SALESFORCE_UMBRAL_BULK` lo fuerzan.
   Bulk no admite compuestos, `GROUP BY`, `COUNT()`, `OFFSET`, subconsultas padre→hijo ni blobs (C2–C4): el traductor
   nunca los genera. **Cada sondeo de un job cuenta como petición de API** (C7): espera 1, 2, 3, 4, 5, 5… s.
   - **Un solo convertidor** (`valores.rs`): las dos vías entregan **texto** y un único módulo lo convierte (C10: REST
     dice `…000+0000` y Bulk `…000Z`; `20.0` frente a `10`). Un valor que no cabe es un error con columna y fila,
     **nunca un nulo ni un redondeo** (`1.005` no cabe en `Decimal<18,2>`).
   - Tipos Arrow: `int/long → Int64`; `double/currency/percent → Decimal128(p,s)` desde `describe`; `boolean`;
     `date → Date32`; `datetime → Timestamp(µs, +00:00)`; el resto, `Utf8`.
   - ⚠ **Bulk escribe el nulo como `""` (entrecomillado)**, no como celda vacía. Mi lector de CSV lo tomaba por un texto
     vacío y fallaba con ``"" no es un entero``. Cazado solo por la prueba real (G1); en Bulk, nulo y vacío son lo mismo.
7. **La cuota es un presupuesto, no un error.** Se lee `Sforce-Limit-Info` de cada respuesta (F4) y hay un **tope del
   80 %** (`ORE_SALESFORCE_TOPE_CUOTA`): al llegar, **no se empieza otra lectura** y se devuelve un fallo
   **reintentable** que dice cuánto va gastado. **No hay «degradar de Bulk a REST»** como en Fivetran (la corrección
   respecto al borrador): Bulk gasta **la misma** cuota (C7), así que no ahorraría nada. `REQUEST_LIMIT_EXCEEDED` no se
   reintenta (esperar segundos no devuelve una cuota de 24 h).
8. **Los cambios** (paso 4). La especificación manda y es más estricta que el borrador:
   - **`append` + `field` no compila** y `append` no mantiene una entidad (OOS2021, OOS2023: *«una marca de agua no ve
     borrados»*). Por tanto la cara `D` es **`{mode: upsert, key: [Id], witness: field, field: SystemModstamp}`**, y
     con clave **re-entregar el borde es idempotente** (spec v1alpha8 `02-view` §5.4).
   - **Solo donde hay borrados que leer:** el objeto tiene `SystemModstamp`/`LastModifiedDate` e `IsDeleted`, es
     replicable y no está en `SIN_CAMBIOS` (`OpportunityLineItem`, `OpportunitySplit`, `OpportunityLineItemSchedule`,
     `QuoteLineItem`, `AccountTeamMember`, `OpportunityTeamMember`: borran sin pasar por la Papelera o dependen de otro
     objeto; **sacada de la documentación de Fivetran, no medida**). Lo demás declara `none`: no se inventa.
   - **Los borrados son `queryAll … IsDeleted = true`** con el mismo cursor (D6): al segundo y sin el tope de 600.000 ids
     de `getDeleted`, que además trunca los segundos (D7) y cuyo `latestDateCovered` va unas horas por detrás (D8).
   - **`_ore_op`**: columna virtual **provisional** (la misma de `ore-mongodb`, que dice que el motor aún no consume un
     changelog). **Solo sale si la petición la pide** en su proyección; quien hoy lee por cursor sin pedirla recibe lo de
     siempre. Con ella: las filas son `upsert` y los borrados `delete` con solo `Id` (el resto, nulo). Exige `Id` en la
     proyección y rechaza filtros, orden y límite (*un changelog no se recorta: un borrado no trae el documento*).
   - **Relectura solapada**: con `_ore_op`, `start` se retrasa **600 s** (`ORE_SALESFORCE_RELECTURA`; valor tomado de
     Airbyte `PT10M`, **sin medir**). Cubre las filas que comparten segundo (D1: 150 filas con el mismo sello) y las
     transacciones largas que confirman tarde; la clave `Id` absorbe lo repetido. **No hace falta el *keyset*
     `(SystemModstamp, Id)`** que proponía el borrador: se pagina por `nextRecordsUrl`/`locator`.
   - **`end`, si falta, es la hora de la org** (cabecera `Date`, no el reloj local), tomada antes de leer: el testigo y
     las filas son el mismo instante. `testigo` devuelve `{"modo":"field","valor":"2026-10-31T21:54:45Z"}`, o `none` si
     el objeto no dice sus cambios.
   - **La Papelera:** un `start` anterior a `earliestDateAvailable` de `getDeleted` (o a 15 días) **se niega** con
     «hace falta una copia completa»: servir el resto sería una copia que no se entera de lo retirado y no avisa
     (un administrador puede vaciar la Papelera).
   - `cambio` contesta **`respalda: false`** (sin contadores por objeto; las fórmulas no mueven el sello).
   - **CDC por Pub/Sub** (gRPC, `witness: log`): **descartado como origen de cambios** (paso 6, 2026-11-01; §6.4). Medido
     con un cliente Python de laboratorio (E1–E12); no hay cliente Rust.
9. **Cableado (paso 3).** `salesforce://` ya daba el tipo y el lector en `ore-cli/fuente.rs` (con test).
   `ore-serve`: asistente de credenciales `GET /fuentes/credenciales/salesforce` (JWT, los dos ámbitos, la
   preautorización, la clave en la URL, y qué no se admite con su porqué); `canjeador_de` no canjea nada;
   `url_sin_secreto` la rechaza. `Dockerfile` y `ci/compilar-binarios.sh` incluyen `ore-read-salesforce`. La malla ya
   deja salir por el 443 a `0.0.0.0/0` (`20-driver.yaml`): no hay que tocarla. El CI (`cargo clippy --workspace` y
   `cargo test --workspace --all-targets`) cubre los crates nuevos solos; las pruebas de laboratorio son `#[ignore]`.
10. **Fuera de alcance de 0083:** escritura; los ficheros (`ContentVersion`: sería un origen de objetos aparte);
    Data Cloud y Zero Copy (federación hacia otros almacenes, no una lectura directa); OAuth de usuario; el CDC.

## 2. Lo que hacen los demás (publicado, sin medir)

| | Airbyte | Fivetran | Estuary | dlt | Databricks |
|---|---|---|---|---|---|
| Autenticación | OAuth (id/secret + refresh token); RTR desde 2.8.0; PKCE desde 2.9.1 | app conectada propia o *external client app*; ≤ 4 conexiones por app | OAuth o usuario + token | usuario + contraseña + token; *client credentials* | OAuth (sin M2M ni contraseña) |
| API | Bulk por defecto; REST si hay `base64`, `complexvalue` o el objeto no admite Bulk | Bulk en histórico y volumen; REST en el resto y de respaldo | Bulk 2.0 + REST | Bulk 2.0 con PK chunking; REST si no | no documentado |
| Cambios | cursor + *lookback* `PT10M` | `SystemModstamp`, `LastModifiedDate`, `CreatedDate`, `LoginTime`; sin ellos, recarga | marca de tiempo en ventanas; CDC en el nativo | `SystemModstamp`, `merge` por `Id` | no documentado |
| Borrados | `isDeleted=true` | `getDeleted()` (Papelera, 15 días) y lista de objetos que reimporta | no documentado | `include_deleted`; los purgados no se ven | no documentado |
| Fórmulas | reset y *backfill* si cambian | **excluidas**; metadatos en `FIVETRAN_FORMULA*` | refresco por cron (diario) | no mueven el sello | *snapshot* completo |
| Compuestos | REST con `nextRecordsUrl` | **subcampos como columnas** (`billing_*`) | no documentado | campos sueltos | no documentado |
| Cuota | termina bien y sigue en la siguiente | **90 %: de Bulk a REST; de REST, pospone 1 h**; tope configurable | cuota propia de Bulk | un job de Bulk > 24 h falla | — |

Lo que ninguno resuelve y este origen sí: **proyección y filtros empujados** (todos leen el stream y filtran después), y
un **catálogo con lo que se puede filtrar campo a campo**. Se diverge de ellos en dos cosas, ambas medidas: no se
«degrada» de Bulk a REST (misma cuota), y los borrados salen de `queryAll` y no de `getDeleted`.

Fuentes leídas: [Airbyte](https://docs.airbyte.com/integrations/sources/salesforce) y
[Fivetran](https://fivetran.com/docs/connectors/applications/salesforce) (la página principal; sus subpáginas solo por
resumen de búsqueda), [Estuary](https://docs.estuary.dev/reference/Connectors/capture-connectors/Salesforce/salesforce-native),
[dlt](https://dlthub.com/docs/dlt-ecosystem/verified-sources/salesforce) y
[Databricks](https://docs.databricks.com/aws/en/ingestion/lakeflow-connect/salesforce).

## 3. Lo que se midió (X0)

Org **Developer Edition** (`orgfarm-4328800370-dev-ed`), API **v68.0**, sembrada con 154 cuentas, 400 contactos y 100
oportunidades `X0-` (casos adversos: `NA`, `None`, `null`, `N/A`, comas, comillas, saltos de línea, Unicode) y dos
campos propios (`X0_Formula__c`, `X0_Texto__c`). Resultados crudos: `x0-resultados.log` y los scripts `x0.py`, `cdc_lab.py`
(en el *scratchpad* de la sesión, **no versionados**: ver §7).

### 3.1 Resultados

**A · descubrimiento**

| id | medido |
|---|---|
| A1 | `describeGlobal`: **1744** objetos, 1462 consultables, 872 replicables, 591 consultables y no replicables |
| A2 | `Account/describe`: 70 campos; cada uno trae ~55 propiedades (`filterable`, `sortable`, `groupable`, `calculated`, `compoundFieldName`, `nillable`, `picklistValues`…) |
| A3 | compuestos: `BillingAddress`/`ShippingAddress` (tipo `address`) y sus subcampos con `compoundFieldName`; **`Account.Name` apunta a sí mismo** |
| A4 | `Account` no trae ningún campo `calculated`: hubo que crear uno |
| A5 | `base64`: `ContentVersion.VersionData`, `Attachment.Body` (`ContentVersion` no es replicable) |

**B · consulta (REST)**

| id | medido |
|---|---|
| B1 | **proyección empujada**: `SELECT Name` devuelve `Name` y `attributes` |
| B2 | `eq` y `like` **no distinguen mayúsculas** |
| B3 | `!=` **incluye los nulos**: 112 filas; con `AND Industry != NULL`, 75 |
| B4 | `isNull`, `in`, rango, `OR`, fechas y `LAST_N_DAYS` funcionan |
| B5 | `like` con `_` escapado se acepta |
| B6 | `SELECT BillingAddress` devuelve un objeto; `WHERE BillingAddress = …` da `INVALID_FIELD`; el subcampo sí filtra |
| B7 | texto largo (`Description`): ni se filtra ni se ordena; `describe` lo marca `filterable=false` |
| B8 | `AND NOT x` y `NOT x AND y` dan `MALFORMED_QUERY`; `AND (NOT x)` funciona |
| B9 | `ORDER BY` por defecto: **nulos primero en ASC y en DESC**; `NULLS LAST` lo cambia |
| B10 | `OFFSET 2001` → `NUMBER_OUTSIDE_VALID_RANGE` (tope 2000) |
| B11 | agregados, `GROUP BY`, hijo→padre, padre→hijo, semi-join y `TYPEOF` funcionan |
| B12 | `A < B` entre campos: `MALFORMED_QUERY` |
| B13 | `batchSize` mínimo 200 (100 se sube a 200); sin cabecera hasta 2000 por página; `nextRecordsUrl` |
| B14 | por GET: 400 ids (8,4 KB) bien; 800 (16,8 KB) → `431`; 1200 (25 KB) → `414` |
| B15 | texto vacío guardado vuelve `null`; fórmula `BlankAsZero` devuelve `0.0`; fechas con `+0000` |

**C · Bulk API 2.0**

| id | medido |
|---|---|
| C1 | crear job, sondear hasta `JobComplete`, leer resultados; CSV con `locator` y `Sforce-NumberOfRecords`; **cada página trae su cabecera** |
| C2 | no soporta compuestos, `GROUP BY`, `COUNT()` ni `OFFSET` |
| C3 | no soporta subconsultas padre→hijo con CSV |
| C4 | no soporta blobs con CSV (`VersionData`) |
| C5 | sí: `LIMIT`, `ORDER BY`, hijo→padre, semi-join, `TYPEOF`; `Attachment` también va (Airbyte decía REST) |
| C6 | `maxRecords=100` → 4 páginas con `locator`; el `locator` es opaco |
| C7 | **el sondeo cuenta como petición de API**: 73 peticiones para ~18 jobs |
| C8 | cada job tarda de **3 a 23 s**, aunque devuelva pocas filas |
| C9 | consulta de 25 KB por el cuerpo: sin límite de URL (23 s) |
| C10 | formatos distintos a REST: `…000Z`, `20.0` en una fórmula, `10` en un entero |
| C11 | nulo y vacío son iguales en el CSV; `NA`, `None`, `null`, comas, comillas, saltos y Unicode llegan intactos |

**D · cambios**

| id | medido |
|---|---|
| D1 | `SystemModstamp` y `LastModifiedDate` con **precisión de segundos**; las 150 cuentas sembradas comparten sello |
| D2 | tocar un contacto **no** mueve el sello de su cuenta |
| D3 | *keyset* `SystemModstamp = t AND Id > último ORDER BY SystemModstamp, Id`: 145 filas en 3 páginas, sin pérdida (**no se usa**: se pagina por locator) |
| D4 | `SystemModstamp` no es agrupable |
| D5 | `getUpdated` devuelve las altas y los cambios (152 ids en 15 min) |
| D6 | `queryAll … IsDeleted = true`: las cuentas borradas con el `SystemModstamp` del borrado; la consulta normal no las ve |
| D7 | `getDeleted` **trunca los segundos**: con `end` = ahora devolvió 0; con `end` en el futuro, 2. Trae `deletedDate` y `earliestDateAvailable` |
| D8 | `latestDateCovered` de `getUpdated`: unas 17 h por detrás de la hora real, y estable |
| D9 | `start` > 30 días → `INVALID_REPLICATION_DATE`; ventana < 1 min → error (mensaje invertido); `start > end` y `start == end` → **200 con 0 filas** |
| D10 | cascada: los contactos hijos de una cuenta borrada salen en `getDeleted` y `queryAll`; **las oportunidades no se borraron** |

**E · CDC (Pub/Sub API, gRPC)** — cliente Python de laboratorio (`cdc_lab.py`), **sin cliente Rust todavía**

| id | medido |
|---|---|
| E1 | sin activar, **ningún objeto tiene CDC**; se activa con un `PlatformEventChannelMember` en `ChangeEvents` |
| E2 | `api.pubsub.salesforce.com:7443`: TLS 1.3, h2; `GetTopic` → `can_subscribe: true`; `GetSchema` → Avro de 45 campos |
| E3 | latencia commit → recepción **~1,4–2,2 s** |
| E4 | `replay_id` **opaco de 29 bytes**; `CUSTOM` desde un id devuelve los **siguientes** (exclusivo); `EARLIEST` devuelve lo retenido |
| E5 | control de flujo: `num_requested=1` entrega 1 y se detiene (`pending_num_requested=0`) hasta pedir más |
| E6 | `CREATE` trae todos los campos no nulos; **`UPDATE` solo los cambiados**; `DELETE` solo la cabecera |
| E7 | `changedFields` y `nulledFields` son **bitmaps** (`0x402800`); un evento puede traer **varios `recordIds`** |
| E8 | las fechas llegan como **epoch en ms**, no como ISO |
| E9 | **`latest_replay_id` no es la cola del stream**: junto a un evento retenido repite el `replay_id` de ese evento. Con `LATEST` no llegó **nada en 20 s**; el mensaje vacío (keepalive, que sí lo lleva) llega «dentro de 270 s» (documentado, no esperado hasta el final) |
| E10 | un `replay_id` **por delante de la cola se rechaza** (`INVALID_ARGUMENT: The Replay ID validation failed`), con el sufijo real o con ceros, y uno de 1 byte también: no hay manera de preguntar «¿dónde está la cola?» probando ids |
| E11 | un `replay_id` son **8 bytes de contador creciente** (`0x30cc5b`, `0x30cc5c`…) **+ 21 de sufijo fijo** (`00134e2d636f7265312e736664632d387467747435`, un nombre de pod); como texto hexadecimal **se ordena** |
| E12 | `EventBusSubscriber` (con su campo `Tip`) es consultable por SOQL, pero **vacío incluso con una suscripción de Pub/Sub abierta**: lista suscriptores de Apex y CometD, no clientes gRPC. `PlatformEventChannelMember` **no es consultable por SOQL** (solo Tooling API) |

**F · identidad y límites**

| id | medido |
|---|---|
| F1 | JWT bearer **sin** el ámbito `refresh_token`: `invalid_request: refresh_token scope is required…`; con él, `200` |
| F2 | con `AdminApprovedPreAuthorized` y un perfil, el JWT canjea sin pasos de usuario |
| F3 | límites de la org: **15.000** peticiones/día, **10.000** jobs de Bulk, **5 MB** de datos, **10.000** eventos entregados/día |
| F4 | `Sforce-Limit-Info: api-usage=n/15000` en cada respuesta |

**G · lectura real desde Rust** (`tests/laboratorio.rs`, `tests/lectura_real.rs`; paso 2)

| id | medido |
|---|---|
| G1 | **Bulk escribe el nulo como `""` entrecomillado**: un lector que lo toma por texto vacío falla en una columna numérica |
| G2 | REST y Bulk dan **las mismas 148 filas**, con los mismos tipos, fila a fila |
| G3 | `neq` de texto da **75 filas, las mismas que SQL**; `eq` con otras mayúsculas da 0 (SOQL daba 1) |
| G4 | una lista `IN` de 1.200 ids se parte en 4 consultas y se lee entera: 395 de 395 filas reales |
| G5 | `limit` con residual da exactamente *n* filas; `orderBy`+`limit` por Bulk deja los nulos al final |
| G6 | con un umbral bajo, `Auto` pasa a Bulk y da lo mismo (≥ 4 peticiones en lugar de 1–2) |
| G7 | el JWT firmado en Rust con `rsa` lo acepta la org |

**H · cambios reales desde Rust** (`tests/cambios_real.rs`; paso 4)

| id | medido |
|---|---|
| H1 | un alta, una actualización y un borrado reales salen **`upsert`, `upsert`, `delete`**; el borrado trae el nombre nulo; **REST y Bulk (`queryAll`) dan el mismo changelog** |
| H2 | sin relectura, un alta anterior al `start` no sale (0 filas); **con ella, se re-entrega** (1) |
| H3 | un `start` anterior a la Papelera (`2026-01-01`; la org guarda desde `2026-10-26T19:44:00Z`) **se niega** con «hace falta una copia completa» |
| H4 | sin pedir `_ore_op`, el rango es el de siempre: sin columna `op` y sin borrados mezclados |
| H5 | `testigo` = hora de la org (`2026-10-31T21:54:45Z`, a ±2 min de la local); `OpportunityLineItem` → `none` |
| H6 | el catálogo declara `upsert` en `Account` y `Contact` y `none` en `OpportunityLineItem` |
| H7 | `cambio` contesta `respalda: false` |

**I · el tiempo de un `datetime`, medido en la org real el 2026-11-01** (a raíz de la org de mentira del paso 5: su
primera pasada del kit falló en `instante = …12:30:00.5Z` y `instante >= …23:59:59.999999Z`; para escribir la mentira
fiel hubo que preguntárselo a la org). Con la CLI, sobre `Account.CreatedDate` y un campo `DateTime` propio recién creado:

| id | medido |
|---|---|
| I1 | un `datetime` **guarda segundos**: al escribir `2026-10-31T12:30:00.500Z` en un campo propio, la org devuelve `…12:30:00.000+0000` |
| I2 | **un literal de SOQL ignora la fracción de segundo**: `CreatedDate = …17.999Z` casa con un registro de `…17.000` (152 filas); `>= …17.999Z` también; `> …17.500Z` y `< …17.999Z`, 0. Acepta cualquier número de decimales (`.0004Z`, `.999999Z`, `.9999999999Z`) sin error |
| I3 | consecuencia: empujar un literal con fracción **trae filas de más** (`= 12:30:00.5` casa con `12:30:00`) **o de menos** (`< 12:30:00.5` pierde `12:30:00`, porque el literal vale `12:30:00`) |

**Arreglo** (`traductor::instante_con_fraccion`, con pruebas): como el valor guardado nunca lleva fracción, la pregunta
exacta se escribe con el segundo entero: `= t.f` → ninguna fila (`vacia`); `!= t.f` → `!= NULL`; `< t.f` y `<= t.f` →
`<= t`; `> t.f` y `>= t.f` → `> t`; un `IN` pierde los miembros con fracción. Es exacto **mientras todo `datetime` de la
org guarde segundos** (visto en `CreatedDate` y en un campo propio; **no** en todos los objetos: un objeto de eventos o un
big object podría guardar milisegundos, sin medir).

### 3.2 Lo que NO se midió
- **Volumen.** La org tiene ~650 filas: no hay filtros no selectivos ni sus errores (hacen falta > 200.000 filas), ni se
  ve dónde cruzan REST y Bulk. **El umbral de 50.000 y el tope del 80 % son estimaciones mías.**
- `REQUEST_LIMIT_EXCEEDED` y `429`, la **caducidad del access token** (se asume 30 min, como Airbyte) y su renovación.
  *(Desde el paso 5 se ejercitan contra la org de mentira —`mentira.rs`—, que reproduce lo que **dice** la documentación y
  el código del conector, no lo que **hace** la org: no cuentan como medidas.)*
- **Si algún `datetime` guarda milisegundos** (§3.1 I): solo se vio en `CreatedDate` y en un campo propio.
- La **relectura de 600 s** (viene de Airbyte) y la **lista `SIN_CAMBIOS`** (viene de Fivetran): no medidas aquí.
- Fórmulas que dependen de **otro objeto** y no mueven el sello.
- CDC: el **latido** con la conexión ociosa, la retención de 72 h, la caducidad de un `replay_id`, los eventos de hueco,
  la renovación del token a mitad de suscripción, la reconexión y el `undelete`.
- **Sandbox**, My Domain de producción, `hardDelete`, visibilidad por compartición de registros.
- **Vaciado de la Papelera por un administrador** (se confía en `earliestDateAvailable`; no se provocó).
- La documentación **oficial** de Salesforce: `developer.salesforce.com` dio `403` a la lectura directa. Lo «oficial» de
  §3 es lo medido; lo publicado, resúmenes de búsqueda.

## 4. El laboratorio

- **Org:** Developer Edition gratuita (`victor@paladio.io`), My Domain
  `orgfarm-4328800370-dev-ed.develop.my.salesforce.com`, usuario `victor.b681f1c134a0@agentforce.com`.
- **App:** External Client App `SALESFORCE`, local, JWT bearer, ámbitos `api` y `refresh_token`, política
  `AdminApprovedPreAuthorized` con el perfil «Administrador del sistema». La Consumer Key no es secreta (está en la
  URL de §7); **la clave privada y el certificado** están en `C:\Users\PC\.ore-labs\salesforce\` (`ore-sf-jwt.key`,
  `ore-sf-jwt.crt`), **fuera de cualquier repo**.
- **Herramientas:** Salesforce CLI en `C:\Users\PC\.ore-labs\sf-cli` (alias `ore-dev`; `undici` fijado a 7.x porque la 8
  pide Zstandard y Node 22.14 no lo trae); metadatos en `sf-cli\proyecto`; cliente de CDC en `sf-cli\cdc` (venv con
  `grpcio`, `pubsub_api.proto` bajado de `forcedotcom/pub-sub-api`).
- **Datos que hay en la org:** filas `X0-` (cuentas, contactos, oportunidades), `X0-P4-…` (las de las pruebas del paso 4,
  que se borran solas), los campos `Account.X0_Formula__c`, `X0_Texto__c` y `X0_Instante__c` (un `DateTime` creado el
  2026-11-01 para medir §3.1 I; dos cuentas `X0-` lo tienen puesto), el conjunto de permisos `X0_Campos` y el
  canal de CDC de `Account`.
- **A borrar al cerrar el laboratorio:** la org entera (es de pruebas), la clave y el certificado, la sesión de la CLI.

## 5. La hoja de ruta

| paso | contenido | estado |
|---|---|---|
| **X0** | descubrimiento, SOQL, Bulk 2.0, cambios, CDC (cliente de laboratorio) | **hecho** |
| **1 · esqueleto** | crates `ore-salesforce` y `ore-read-salesforce`, URL, identidad JWT, `catalogo`/`check`/`explorar`, traductor puro | **hecho** |
| **2 · lectura** | REST y Bulk, residuales, umbral con `totalSize`, normalización de formatos, presupuesto de cuota, Arrow | **hecho** |
| **3 · cableado** | `ore-driver` (`clave` secreta), `ore-serve` (credenciales, cola, canje), `ore-cli`, `Dockerfile`, `compilar-binarios.sh`; la malla no cambia | **hecho**, salvo el banco del kit (paso 5) |
| **4 · cambios** | cara `D` `upsert`+`field`, `_ore_op`, borrados por `queryAll`, relectura, hora de la org, guarda de la Papelera, `testigo`, `cambio` | **hecho** |
| **5 · mentira** | **hecho 2026-11-01.** Elegida la mentira y no la cinta: el conector tiene lógica propia (residuales, salto a Bulk, cuota, keyset) que varía con los datos. `pruebas-de-fuego/salesforce-de-mentira.py` (JWT verificado con `openssl`, REST, Bulk 2.0, SOQL con las rarezas de §3, cuota, `503`, token que caduca, escrituras REST), su prueba `prueba-salesforce-de-mentira.py`, banco `bancos/salesforce.rs` (`ore-kit --banco salesforce`) y `o7-salesforce.sh`, que corre en el CI: kit 1-13, pasarela, `cambios_real.rs` (7) y `mentira.rs` (8). Nacieron de ella **cuatro arreglos del conector** (§3.1 I, `timeoutMs` por petición, aborto del job, resumen del mensaje de error) | hecho |
| **6 · CDC** | **decidido no hacerlo (2026-11-01)**: el testigo `log` de 0016 pide «hasta dónde está el origen ahora» antes de leer, y Pub/Sub no puede decirlo (E9-E12). Los cambios siguen por `field` (`SystemModstamp` + `queryAll`, paso 4). Detalle y condiciones para retomarlo en §6.4 | descartado |
| **7 · consola** | hoja de `rubix-platform` para dar de alta la fuente | pendiente |

## 6. Lo que falta, y lo que no se pudo verificar

### 6.1 Cerrado el 2026-11-01
Clippy `-D warnings` limpio sobre `ore-salesforce` y `ore-read-salesforce` tras el paso 4; los tres ficheros de laboratorio
pasan contra la org real (7 + 7 + 10). `el_presupuesto_pospone…` dependía del uso de la org (con tope 1 % pasaba solo si ya
se habían gastado 150 peticiones): ahora usa tope 0. Los 3 errores de clippy de `ore-serve/lugar.rs` salen **igual sin
mis cambios** (verificado apartándolos): son del base, código muerto en Windows. Queda: rebasar sobre `origin/main`
(22 commits) y el punto 5 (suite de `ore-serve`).

### 6.1 (original) Sin verificar al cerrar la sesión (2026-10-31)
1. **Clippy de `ore-serve` falla en `lugar.rs`** (`puerta_heredada`, `heredada`: *nunca usadas*, dead code en Windows). **No
   toqué `lugar.rs`** (`git status` solo da `cola.rs`, `credenciales.rs`, `datasets.rs` en `ore-serve`), así que parece un
   fallo del `main` base y en Windows; **no lo comprobé contra `origin/main`**. Pendiente de confirmar. Aparte,
   `origin/main` ha avanzado respecto a `9cd59268` (otra sesión hizo `git fetch`): conviene rebasar.
2. **El resultado de clippy sobre `ore-salesforce` y `ore-read-salesforce` tras el paso 4 no se vio**: la orden combinada
   se cortó en `ore-serve`. Tras el paso 2 estaba limpio con `-D warnings`.
3. **Las pruebas de laboratorio `laboratorio.rs` (7) y `lectura_real.rs` (10) no se han vuelto a ejecutar tras el paso 4**
   (cambió `lectura.rs`, `Politica` y `traductor.rs`; se editó `lectura_real.rs` para los literales de `Politica`).
   **Solo `cambios_real.rs` se compiló y ejecutó** (7/7 verdes): ni siquiera se sabe si los otros dos compilan con los
   cambios. Hay que correr los tres juntos.
4. **No se ha commiteado nada.** Si se pierde el worktree se pierde el trabajo (`git status` en `C:\ORE-salesforce`).
5. Los tests de `ore-cli` `la_coleccion_es_un_activo` fallan **por el entorno**: `vendor/oos` (submódulo) está vacío en
   este worktree y su fixture no se encuentra. Y `ore-serve` tiene, según 0082, 11 fallos propios de Windows (git y
   rutas) que ya tenía `origin/main`; no se ejecutó la suite entera de `ore-serve`, solo los tests que toqué.

### 6.2 Decisiones abiertas
- **`_ore_op` es provisional.** El contrato (0016) no define la forma de las filas de un changelog y el motor no consume
  uno todavía. La cara `D` declara `upsert`, que **promete borrados**: hoy se entregan solo si la petición pide `_ore_op`.
  Quien lea por cursor sin pedirla (un motor que aún no sabe) **no se entera de los borrados**. Es una brecha de la
  plataforma, la misma que tiene Mongo; hay que decidirlo con quien consuma los cambios.
- **`neq` excluye nulos** y **la relectura de 600 s**, **el tope del 80 %**, **el umbral de 50.000**, **el núcleo de 13
  objetos del catálogo**: decididos por mí con lo medido o lo publicado; a revisar con volumen.
- **`rsa` 0.9 tiene un aviso conocido de canal lateral de tiempo (RUSTSEC-2023-0071, el ataque Marvin)** que, hasta donde
  sé, no tiene versión corregida. Hoy solo `ore-entrada` usa `rsa`, para *verificar*; aquí se usa para **firmar** con la
  clave privada. El riesgo es bajo para un conector que firma localmente y manda el JWT a Salesforce, pero está sin
  resolver. La alternativa es `openssl`, que ya está en el árbol.
- **La clave privada en la URL del custodio** es lo coherente con Postgres/Mongo, pero un PEM es largo y multilínea
  (~1,7 KB con `%XX`). Si el custodio tiene un tope de tamaño, no se comprobó.
- **Cómo se obtiene la clave en una celda real** (paso 3 completo): hoy `ore-serve` solo guarda la URL tal cual en el
  custodio. No se probó un alta de punta a punta (consola → custodio → Job → lector).

### 6.2 bis · Lo que la mentira encontró (2026-11-01)
1. **Literales `datetime` con fracción**: traían filas de más o de menos (§3.1 I). Arreglado.
2. **`timeoutMs` no cortaba una primera página REST bloqueada** (el kit, caso 8: esperaba 9 s con `timeoutMs 1000`): el plazo
   solo se miraba entre peticiones. Ahora cada petición espera, como mucho, lo que queda (`Cliente::fijar_plazo`).
3. **El aborto de un job de Bulk lo impedía ese mismo plazo vencido** (job vivo gastando cuota): `abortar` tiene el suyo.
4. **El motivo de un error de consulta salía vacío** (el mensaje real empieza por un salto de línea): se cita lo que viene
   tras `ERROR at Row`.

**Límite de la mentira:** reproduce lo que **dicen** el código y lo medido en X0, no lo que hace la org. Sus tipos del
banco son sintéticos (`id` `int`, `entero` `long`, `real` `double` sin precisión: un campo propio real da `Decimal`), y
`local` no se compara (Salesforce no tiene hora local). Lo no cubierto sigue en los 24 tests contra la org de verdad.

### 6.4 El CDC y por qué no (paso 6, 2026-11-01)
El contrato (0016) fija el rango de un `log` con **dos posiciones del propio origen**: el `testigo` se toma **antes** de leer
y es el `end` de la lectura. Pub/Sub API es un flujo de empuje: se suscribe uno **desde** una posición y recibe lo que va
llegando, y **no ofrece la cola** (E9, E10, E12). Las salidas posibles, medidas o razonadas:

| camino | por qué no |
|---|---|
| `testigo` espera el keepalive de `LATEST` | hasta **270 s** por llamada |
| `testigo` drena `EARLIEST` (72 h) y toma el último id | cada evento cuenta como entregado (**10.000/día** en esta org) y crece con el tráfico |
| `leer` lee hasta el silencio y el testigo es lo que dejó el último `leer` | **requiere cambiar 0016**: que el driver devuelva la posición final en el trailer, y que el testigo deje de ser «antes de leer» |
| el UPDATE trae solo lo cambiado (E6) | un conector sin fila previa no puede fusionar; habría que **releer cada Id por SOQL**, con lo que el CDC solo sirve de *aviso* |

**Decisión (con el usuario):** los cambios de Salesforce siguen por el cursor `SystemModstamp` + `queryAll` (paso 4): latencia de
sondeo, con borrados reales y la guarda de la Papelera. No se escribe el cliente gRPC (ni `tonic` en `ore-salesforce`).

**Para retomarlo hace falta una decisión de plataforma, no de este conector:** o un testigo `log` que el driver pueda
devolver *después* de leer, o un contrato de empuje (`servir` con eventos). Entonces el camino es: `tonic` sin features de TLS sobre
`hyper-tls`/`native-tls` con ALPN `h2` (como `ore-read-bigquery`, sin `protoc`: mensajes `prost` a mano), cabeceras
`accesstoken`/`instanceurl`/`tenantid`, y decodificar solo la cabecera Avro (`ChangeEventHeader`, el primer campo), releyendo
los Ids por SOQL. Los ids se comparan como hexadecimal (E11). Lo medido (E1–E12) está en `sf-cli\cdc`.

### 6.3 Código que no existe
- `estimar` (`capacidades` dice `estimar: false`).
- La hoja de la consola (paso 7). (El CDC en Rust queda descartado: §6.4.)
- Un `huella_fiable("salesforce")`, si se quiere que `cambio` respalde algo.

## 7. Para retomar (siguiente sesión)

**Dónde está todo:** worktree `C:\ORE-salesforce`, rama `origen-salesforce`, sin commitear. Crates nuevos:
`crates/ore-salesforce/` (`url`, `identidad`, `esquema`, `traductor`, `valores`, `residual`, `csv`, `bulk`, `salida`,
`lectura`, `transporte`, `verbos`; pruebas en `tests/laboratorio.rs`, `tests/lectura_real.rs`, `tests/cambios_real.rs`)
y `crates/ore-read-salesforce/` (un `main` de una línea). Tocados fuera: `ore-driver/src/fallo.rs`,
`ore-serve/src/{credenciales,cola,datasets}.rs`, `ore-cli/src/fuente.rs` (un test), `Dockerfile`,
`ci/compilar-binarios.sh`, `Cargo.lock`, y este ADR.

**Compilar y probar (Windows):** `scripts\dev-windows.ps1` **no existe en este worktree**, está en `C:\ORE\scripts\`:
`. C:\ORE\scripts\dev-windows.ps1; Set-Location C:\ORE-salesforce; cargo test -p ore-salesforce --lib` (93 pruebas
puras). El destino de compilación es `C:\ore-target`.

**Laboratorio (piden la org; `#[ignore]`):**
```
ORE_SALESFORCE_LAB_URL="salesforce://orgfarm-4328800370-dev-ed.develop.my.salesforce.com/?cliente=3MVG9HtWXcDGV.nFr1jQIe28Ij_llSHXMddEYf.fTnGSZloCf_ifsomKKEuEUk9V8iTSlqLQVSyu9ojYAQIZT&usuario=victor.b681f1c134a0%40agentforce.com"
ORE_SALESFORCE_CLAVE_FICHERO="C:\Users\PC\.ore-labs\salesforce\ore-sf-jwt.key"
cargo test -p ore-salesforce --tests -- --ignored --nocapture --test-threads=1
```
Gastan ~150 peticiones de las 15.000 diarias y tardan ~3 minutos (Bulk).

**Variables de entorno del lector:** `ORE_SALESFORCE_CLAVE`, `ORE_SALESFORCE_CLAVE_FICHERO`, `ORE_SALESFORCE_ENDPOINT`
(sustituye el sitio y salta la lista de dominios: para el simulador), `ORE_SALESFORCE_API=rest|bulk`,
`ORE_SALESFORCE_UMBRAL_BULK`, `ORE_SALESFORCE_TOPE_CUOTA`, `ORE_SALESFORCE_RELECTURA`.

**Trampas de la sesión (para no repetirlas):**
- Las órdenes de `bash` con bloques largos o con comillas se cortan: **escribir los ficheros con la herramienta de
  escritura**, y los parches de Python como fichero.
- `ore-read-salesforce check`/`explorar`/`testigo` esperan `{"url": "…"}` por stdin; `catalogo`, la URL a secas; `leer`,
  la petición completa. **No usar la tubería de PowerShell 5.1** con comillas: pasar los bytes desde `bash`.
- `openssl` en Git Bash convierte `-subj /CN=…` en una ruta: `MSYS_NO_PATHCONV=1`. Con esa variable activa, `openssl`
  quiere rutas de Windows (`C:/…`), no `/c/…`.
- Los tokens de acceso **no se pegan en el chat**; el script `sf-jwt-sonda.sh` los oculta.

**Paso 5 hecho (2026-11-01).** Probar: `python3 pruebas-de-fuego/prueba-salesforce-de-mentira.py` y
`bash pruebas-de-fuego/o7-salesforce.sh --dentro` (≈6 min; en Windows desde PowerShell con `dev-windows.ps1`). **Siguiente:**
decisión sobre `_ore_op`, paso 7 (consola) (el 6, CDC, se descartó: §6.4); limpiar la org al cerrar (incluido `Account.X0_Instante__c`).

*(Lo que sigue es el orden de antes del paso 5.)* **Siguiente, en orden:** (1) cerrar §6.1 (ejecutar los tres ficheros de laboratorio y clippy sobre los dos crates,
rebasar sobre `origin/main`, y commitear); (2) paso 5, que es lo que deja el CI ejerciendo el conector; (3) la
decisión sobre `_ore_op` (§6.2); (4) paso 6 (CDC en Rust); (5) paso 7 y limpiar la org al cerrar.

## 8. Fuentes
- Airbyte, Fivetran, Estuary, dlt y Databricks: enlaces en §2.
- Salesforce, [Pub/Sub API](https://github.com/forcedotcom/pub-sub-api) (`pubsub_api.proto`).
- Crates evaluados y descartados: [`salesforce_core`](https://docs.rs/salesforce_core),
  [`force`](https://docs.rs/crate/force/latest), [`cirrus`](https://docs.rs/crate/cirrus/),
  [`force-pubsub`](https://docs.rs/force-pubsub).
- Especificación OOS v1alpha8: `01-table.md` §4 (modos de cambio y testigo), `02-view.md` §5.2 y §5.4 (OOS2021, OOS2023).
- ADR 0016 (el testigo y el rango), ADR 0053 (el conector v2 y el protocolo 2),
  [`0061`](0061-origenes-de-objetos.md), [`0066`](0066-geo-paradigm.md) y [`0082`](0082-origin-google-drive.md).
