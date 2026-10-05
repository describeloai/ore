# 0053 · ORE Federation Engine

**Estado:** **aceptado** (2026-10-04) · F0 medido · **F1 cerrado** · **F2 cerrado** (2026-10-05: conectores v2 de Postgres, BigQuery y S3, en vivo) · **F3 cerrado** (2026-10-05: la pasarela `ore-federation` en cada celda, medida en vivo) (contratos en [`docs/federation.md`](../federation.md); spec
[v1alpha24 `01-leer-el-origen`](../../vendor/oos/spec/v1alpha24/01-leer-el-origen.md)) · **Decide:** **leer el origen es un producto**, y
uno solo: el ORE Federation Engine es la única vía por la que una celda lee un origen —consultarlo
en vivo desde SQL, describirlo, catalogarlo, comprobarlo y copiarlo—, con sus conectores, sus
conexiones, su contrato de petición, su reparto del plan entre el origen y el motor, su protección
del origen y su gobierno. Corre como un servicio por celda (`ore-federation`); el motor SQL sigue
siendo DuckDB, en el puesto y en los trabajos. Corrige [`0008`](0008-el-protocolo-del-driver.md),
[`0031`](0031-el-puesto.md) y [`0040`](0040-sql-views.md) donde dicen que un origen no se lee más
que copiándolo.

## Qué es

Una base foránea de ORE es el *foreign catalog* de Lakehouse Federation: sus `Table` nombran tablas
de un origen y sus vistas las dejan ver. Hasta hoy **nadie podía leerlas**: `sql()` contesta 409 a
una `Table` y a una vista virtual, `/v1` también, y `ore ask` lee sólo copias. Un origen se podía
copiar entero y luego mirar la copia; no se podía preguntar.

El Federation Engine es lo que contesta. Una consulta que nombra una tabla del origen se reparte:
lo que el origen sabe hacer —columnas, filtros, `limit`, y más adelante agregados y juntas— **se
hace en el origen**; lo demás —juntar con el lago, agregar, ordenar— lo hace DuckDB con lo que
llega. Las filas viajan en Arrow.

> **«Nunca del origen» queda como lo que era: una regla de escritura.** Leer el origen es la
> condición para consultarlo, describirlo y sacar de él datasets y vistas. Escribir sigue siendo
> siempre en el lago ([`0018`](0018-la-ontologia-es-el-sistema-de-registro.md) no cambia).

## Por qué un producto, y centralizado

Una celda tendrá decenas de orígenes y la plataforma cientos. Si cada camino lee a su manera —el
catálogo con su trabajo, la comprobación con el suyo, la copia con el suyo, una consulta en vivo
con otro—, cada origen nuevo multiplica las credenciales repartidas, las rutas de red, las formas
de tratar mal la base de un cliente y los sitios donde no queda constancia de quién leyó qué. La
industria converge en lo mismo: **la federación funciona cuando es un sitio** (el catálogo de
Unity, el coordinador de Trino, el SDK de conectores de Athena).

### Lo que hace la industria

| | dónde corre el conector | lo que hace el origen | credencial y red |
|---|---|---|---|
| AWS Athena Federated Query | **fuera del motor**: una Lambda por fuente (Query Federation SDK) | filtros, columnas, `limit`; bloques Arrow | rol IAM y VPC del conector |
| BigQuery `EXTERNAL_QUERY` | en el origen, por un recurso *connection* | el SQL del origen | la *connection* |
| Databricks Lakehouse Federation | dentro del cómputo (JDBC) | filtros, columnas, `limit`, agregados y juntas según la fuente | *connections* de Unity; red por NCC |
| Trino / Starburst | dentro de los workers | según el conector (`applyFilter/Projection/Limit/Aggregation/Join`) | el catálogo del clúster |
| Arrow Flight SQL / ADBC | un servidor delante del origen | el protocolo estándar fragmento → Arrow | el servidor |
| DuckDB `postgres_scanner` | dentro del motor | filtros y columnas | la credencial en el motor |

Dos escuelas: **el conector dentro del motor** (sirve si el motor es de confianza) y **el conector
fuera del motor**, aislado, con su identidad y su red. ORE es de la segunda por construcción: su
motor corre en el puesto, al lado del código del usuario, y ahí no deben estar ni la credencial del
origen ni la salida a internet. Es la razón de Athena para poner cada conector en una Lambda.

### Lo medido (F0, 2026-10-04, `t-victor`)

| desde | BigQuery (443) | Neon / Postgres (5432) | identidad |
|---|---|---|---|
| puesto (`jobs-p`) | ✓ acceso privado de Google, ~47 ms | ✗ sin salida a internet (`salida-del-puesto`) | la del puesto |
| ore-serve (`sistema-spot`) | ✓ (`salida-del-control`: 443) | ✗ (sin 5432; imagen sin conectores) | — |
| rol `driver` | ✓ | ✓ 443 y 5432 por el NAT `salida-a-origenes` | cuenta del driver + custodio |

Y **un conector sólo corre hoy como un trabajo por petición** (catálogo, comprobación, copia): la
última copia, 46 s de punta a punta. Una consulta interactiva no puede pagar eso.

**M2 · los conectores desde donde corren** (`pruebas-de-fuego/medida-f0-los-conectores-en-la-celda.yaml`,
un trabajo con la cuenta `driver` en `t-victor`, sólo lectura):

| | cifra |
|---|---|
| del trabajo creado a la primera medida | **29 s** (16 s hasta arrancar el pod, 12 s el agente y sus secretos) |
| Neon · una petición (proceso + TLS + consulta), por la clave | **~890 ms**, estable; la primera 2,1 s (Neon despierta) |
| Neon · `olist.customers` entera (99 441 filas, 19 MB) / filtrada `= 'SP'` (41 746) | 2,6 s / 2,0 s |
| BigQuery · filtrada (0 filas) | **0,6–0,8 s** |
| BigQuery · `ventas.ore_e2e_sintetica` entera (**2 000 000 de filas**, declarada `fullScan: expensive`) | **~117 s**, tres veces: nada lo impidió |

**M3 · el conector de Postgres, en local** (`pruebas-de-fuego/medida-f0-el-conector-de-postgres.sh`,
un Postgres de pruebas con 10⁶ filas; la consulta en el propio origen: `limit 10` en 2,7 ms):

| | cifra |
|---|---|
| coste fijo de una petición (proceso + conexión + consulta) | **~270 ms** (el origen tarda ~3) |
| la tabla entera, 5 columnas | **41,5 s · 128 MB de texto · el conector llega a 551 MB de memoria**: lo carga todo antes de contestar |
| un `eq` empujado (5 % de las filas) | 2,1 s |
| diez filas sin `limit` | **la tabla entera: 41,5 s** (con `limit`, una petición: ~0,27 s) |
| una columna frente a cinco | 9,4 s / 16 MB frente a 41,5 s / 128 MB |
| texto frente a Arrow | 128 → 93 MB; **9 s sólo en parsear el texto**, y todo llega como cadena |
| 50 / 120 peticiones a la vez (`max_connections` 40) | **7 / 68 fallan** con «too many clients»: ni cola ni conexiones compartidas |

**Lo que las cifras deciden:**

1. **El camino en vivo no puede ser un trabajo** (29 s antes de empezar): hace falta la pasarela.
2. **Conexiones calientes y reutilizadas**: casi todo el coste de una petición pequeña es abrir
   proceso y conexión (270 ms en local, ~890 ms contra Neon desde la celda), no la consulta.
3. **`limit` es lo primero** del contrato v2: es la mayor diferencia medida (41,5 s → 0,27 s).
4. **Flujo Arrow y no un texto entero en memoria**: 551 MB de memoria por 10⁶ filas, 9 s de
   parseo y los tipos perdidos.
5. **Proteger el origen no es opcional**: sin cola por origen, 50 peticiones a la vez tumban
   conexiones; y una tabla que se declara `fullScan: expensive` se leyó entera tres veces.
6. **Empujar columnas también cuenta** (×4,4 en tiempo, ×8 en bytes).

## La decisión

### 1 · Un producto, una vía

**Toda lectura de un origen pasa por el Federation Engine**: la consulta en vivo, `describe`, el
catálogo, la comprobación de una fuente, explorar y la copia. Los trabajos siguen existiendo para
lo que dura (copiar, catalogar), pero **leen por el motor**, no lanzando un conector por su cuenta.

### 2 · Las piezas

1. **Conectores.** Uno por tipo de origen (los `ore-read-<tipo>` de hoy), versionados, y cada uno
   **declara** lo que sabe hacer: operadores, `limit`, orden, agregados, juntas, Arrow, paginación.
   Con cientos de orígenes lo que se empuja no se adivina: se declara y se coteja. Un **kit de
   conformidad** que todo conector pasa antes de entrar.
2. **Conexiones.** Una fuente del árbol (`datasources`) + su credencial en el custodio, como hoy;
   en un solo sitio. La identidad con la que el conector llega al origen es la del driver de la
   celda (BigQuery por Workload Identity, [`0042`](0042-origin-rest-bigquery.md)).
3. **El contrato de petición v2** (amplía [`0008`](0008-el-protocolo-del-driver.md)): proyección, filtros con
   `eq, ne, lt, le, gt, ge, in, is null, like`, `limit`, orden; la respuesta, **siempre Arrow**
   (Postgres incluido, hoy texto). Un operador que el conector no declara no viaja: lo hace el motor.
4. **El reparto del plan** (en `ore-core`): de una sentencia, por cada relación del origen, qué
   columnas, qué predicados conjuntivos y qué `limit` se empujan, y qué queda para DuckDB. Lo
   mismo se enseña con **`explain`**.
5. **La pasarela `ore-federation`**: un servicio **por celda** (como `ore-serve` y `ore-medios`),
   sin estado, con el rol `driver` —su red y su cuenta—, que ejecuta conectores ya calientes y
   devuelve un flujo Arrow. Por celda y no central: un origen de un cliente nunca comparte proceso
   ni identidad con el de otro.
6. **El coordinador** es `ore-serve`, como para la media: decide (conducto, acceso, cotas), trae la
   credencial del custodio, anota la lectura y pasa el flujo al puesto o al trabajo.
7. **Proteger el origen.** Es la base de un cliente, a menudo en producción: concurrencia y ritmo
   por origen, tiempo máximo por petición, tope de filas por defecto, `requiredFilters` y
   `fullScan: expensive` que se exigen (hoy declarados y no leídos), sesiones de sólo lectura y
   conexiones reutilizadas.
8. **Gobierno.** Un conducto propio para leer el origen en vivo (nombre en F1; sin él, `OOS4011`),
   el acceso al dato que hoy no pregunta ([`0047`](0047-ore-access-control.md) A8), las etiquetas
   que se propagan (`OOS4002`), y **cada lectura anotada**: quién, qué tabla, qué predicados,
   cuántas filas.
9. **Observabilidad por origen**: salud, latencia, errores, lo empujado y lo no empujado (el
   estado de la fuente de hoy, `/fuentes/{n}/estado`, es su embrión).

### 2b · Lo decidido (2026-10-04)

| | decisión | por qué |
|---|---|---|
| el conducto | **`federation.read`**, una clase propia (OOS v1alpha24 §2) | copiar y leer en vivo dejan cosas distintas: una copia queda, una lectura se usa y no queda; son dos autorizaciones |
| la credencial | **la trae `ore-serve`** del custodio y se la pasa a la pasarela en cada petición | la pasarela no guarda estado ni habla con el custodio; la autorización se decide en un sitio |
| tope por lectura en vivo | **100 000 filas o 64 MB**, lo primero que llegue | más es una copia; M3: 10⁶ filas en texto = 128 MB y 551 MB de memoria |
| tiempo máximo por petición | **30 s** | M2: una tabla de 2·10⁶ filas tardó ~117 s y nada la paró |
| concurrencia por origen | **4, con cola**, configurable por fuente | M3: 50 a la vez y 7 fallan con «too many clients»; es la base de un cliente |
| las vistas de una base foránea | **legibles en vivo por defecto**, con `federation.read` autorizado | son la cara de una base foránea; hoy nadie puede leerlas |
| el coste declarado | `forbidden` sin filtro empujado → `OOS2044`, falta un `requiredFilter` empujado → `OOS2045`, al planificar; **`expensive` se lee con un presupuesto** (filas o bytes y tiempo) que la corta si lo supera, estimado antes cuando el origen lo permite | v1alpha24 §4. Lo que el dueño exige es error (el `require_partition_filter` de BigQuery, el `always_filter` de Looker); lo caro es presupuesto (`maximum_bytes_billed`, el *workgroup* de Athena, `max-scan-physical-bytes` de Trino). Un `LIMIT` no protege al origen: en BigQuery ni reduce lo cobrado |

**No es del Federation Engine:** escribir el lago (la copia y `ore-store`), servir la ontología
(se sirve de copias), ni ser el motor SQL (DuckDB).

### 3 · Lo que se puede hacer (la superficie, en SQL)

| sentencia | qué hace | corre en |
|---|---|---|
| `select … from <base foránea>.s.t where … limit n` | consulta en vivo, sólo lectura | puesto |
| juntar una tabla del origen con el lago | cada lado con lo suyo empujado; la junta en DuckDB | puesto |
| las vistas de una base foránea | se leen (hoy 409) | puesto |
| `describe table` / `describe object table` | ya hecho ([`0049`](0049-media-paradigms-in-code-repositories.md) B8·3), del árbol | — |
| `create view b.s.v as select … from <origen> …` | vista virtual sobre el origen, en vivo | puesto |
| `create dataset … as select … from <origen> where …`, `insert into … select …` | copia de una selección, con el filtro en el origen | trabajo |
| `create materialized view` sobre el origen | vista + su copia mantenida | trabajo |
| `select … from <object table> where …` | el listado de un bucket en vivo | puesto |
| `explain …` | qué va al origen y qué queda en el motor | árbol |

## Lo que corrige de decisiones anteriores

- **[`0008`](0008-el-protocolo-del-driver.md)** · el protocolo del driver: la petición crece (operadores,
  `limit`, orden, capacidades declaradas, Arrow siempre) y el driver deja de ser sólo un proceso
  de un trabajo: es el conector de un servicio.
- **[`0031`](0031-el-puesto.md)** · «el puesto lee copias»: lee copias **y** el origen por el
  Federation Engine, con su conducto.
- **[`0040`](0040-sql-views.md)** · «una tabla de un origen nunca»: una vista puede leer el
  origen; virtual, en vivo; materializada, su copia.
- **`ore-core` `sql_del_arbol::cotejar`** · el rechazo de una `Table` y de un `ObjectTable` en el
  `FROM` («nunca del origen») pasa a: se lee por el Federation Engine si su conducto lo deja.
- **[`0018`](0018-la-ontologia-es-el-sistema-de-registro.md)** y
  **[`0029`](0029-donde-corre-una-funcion.md)** · no cambian: se escribe en la copia, nunca en el origen.

## El plan de ataque

Por el fundamento y no por la superficie: primero el contrato y la pasarela, que todo lo demás usa.

| fase | qué | sale |
|---|---|---|
| **F0** · medir | M1 red (✓ hecho, arriba) · M2 en el clúster: un trabajo de medida con rol `driver` —arranque en frío y en caliente, primera fila, filtrado frente a entero, contra Neon y BigQuery— · M3 en local: el conector de Postgres contra el Postgres de pruebas —lo que cuesta cada petición, lo que ahorra empujar filtros y `limit`, texto frente a Arrow, concurrencia | las cifras que fijan cotas y diseño |
| **F1** · el contrato | este ADR aceptado; en OOS: el conducto de lectura en vivo, lo que un conector declara (capacidades), los códigos nuevos; el contrato de petición v2 escrito | spec + ADR |
| **F2** · los conectores v2 | Postgres (Arrow, operadores, `limit`, `statement_timeout`, sólo lectura), BigQuery (operadores, `limit`, Storage Read con `row_restriction`) y **S3 con tablas de ficheros** (descarte por partición y por estadísticas, operadores sobre las filas, `limit`); el **kit de conformidad** | tres conectores que lo pasan |
| **F3** · la pasarela | `ore-federation` por celda: rol `driver`, NetworkPolicy, conectores calientes, flujo Arrow, salud; cotas por origen | servicio en la malla (binario antes que malla) |
| **F4** · el coordinador | en `ore-serve`: la lectura federada —conducto, acceso (A8), credencial, anotación, tope—, en la rama | ruta + prueba de fuego |
| **F5** · el reparto | en `ore-core`: columnas, predicados y `limit` por relación; `explain` | biblioteca + tests |
| **F6** · consultar | en el puesto: las tablas del origen y las vistas foráneas en el `FROM`, juntas con el lago | consulta en vivo |
| **F7** · crear | `create view` sobre el origen, `create dataset … as select`, `insert into … select`, `create materialized view`; los trabajos de copia leen por la pasarela | crear desde el origen |
| **F8** · una vía | catálogo, comprobación y explorar por la pasarela; ningún camino lanza un conector por su cuenta | una sola vía |
| **F9** · más allá | `ObjectTable` en el `FROM`; agregados y juntas empujados donde se declaren; Flight SQL para motores de fuera (Spark, BI) | |

### F2 en hitos

| hito | qué | sale |
|---|---|---|
| **F2·0** · la base común | la petición v2 en `ore-driver` (filtros con valor, lista o ninguno; `limit`, `orderBy`, `timeoutMs`, `id`); los diez operadores, `ORDER BY … NULLS LAST` y `LIMIT` en `ore-sql`; los errores tipados y `tapar`; `capacidades` y su comprobación; el bucle de `servir` con su marco en trozos y las conexiones por `url` | ✓ sin cambiar lo que sirve ningún conector: una petición v1 se lee igual |
| **F2·1** · el kit | `ore-conector-kit`: los 14 casos contra `kit.tipos` y `kit.grande` (10⁶ filas); Postgres como servicio del CI y el S3 de mentira del repositorio (no MinIO: no hace falta una imagen de fuera), BigQuery con respuestas grabadas en F2·3 | ✓ la línea de base, abajo |
| **F2·2** · Postgres v2 | Arrow en flujo, operadores, `limit`, `orderBy`, `statement_timeout`, cancelar, `servir`, `estimar` (`EXPLAIN`) | ✓ **14/14**; 10⁶ filas en 15,3 s con **pico de 14 MiB y primer byte a los 323 ms** (v1: 33 s, 388 MiB, 32,9 s); `SIGTERM` deja el origen en 2 ms; 100 peticiones en `servir` por una sesión en 742 ms |
| **F2·3** · BigQuery v2 | operadores a `row_restriction`, `limit` en la Storage Read, `orderBy` por consulta, REST también en Arrow, `jobs.cancel`, `estimar` (*dry run*), `maximumBytesBilled` | ✓ **en vivo contra `ore_kit` (EU): 12 pasan, 2 no aplican**; 10⁶ filas por la Storage Read en 5,2 s con pico de 21 MiB (M2: ~117 s); grabada la cinta (41 intercambios, 177 KB, por consulta, sin el caso 7) y reproducida sin red: 11 pasan, 2 no aplican; el CI la reproduce con `--exige todos` |
| **F2·4** · S3 v2 | descarte por partición y por estadísticas de Parquet, operadores sobre filas, `limit`, `estimar` por el listado; `orderBy: false` | ✓ **12 pasan, 2 no aplican** (sólo lectura y cancelar: un bucket no tiene SQL ni consultas vivas); 10⁶ filas con pico de 27 MiB; un fichero de una partición que no cumple y un grupo de filas cuyo mínimo y máximo no pueden cumplir no se bajan |
| **F2·5** · cierre | el kit en el CI; las copias en vivo de test6 dan las mismas filas y huella con los v2 | ✓ **F2 cerrado** (2026-10-05): las copias de `bq`, `s3_pedidos` y `standard_test` rehechas desde un puesto de test6 con los v2 (commits `446345d`, `9748d94`, `2f9dcd4` del copiador) dan **las mismas filas, la misma cabecera y el mismo contenido** (sha256 de las filas ordenadas) que antes en los 10 datasets medidos —de 4 a 1500 filas—; `ore_e2e_sintetica` (2·10⁶ filas) copiada; la copia pide Arrow y los errores llegan tipados |

**Lo que F2·5 dejó dicho.** (1) Para rehacer la copia desde un puesto hizo falta abrir ese verbo en la puerta del agente (`POST /paquetes/{n}/copia/rehacer`, en la rama del puesto y en nombre de quien lo abrió; nunca en `main`). (2) `standard_test.public.brain_embeddings` sigue en `error`, igual que antes de F2: su columna es `vector` (pgvector), un tipo de extensión que el conector de Postgres no sabe leer por texto ni tiene en Arrow. Es un hueco del conector, no una regresión; queda para cuando haga falta (lista de tipos de extensión, o `vector` → lista de `float4`). (3) Entre F2·4 y F2·5 el incidente de los custodios ([`0054`](0054-converger-sin-romper.md)) dejó t-demo y t-victor sin base: el cierre esperó a recuperarlos.

### F3 en hitos

| hito | qué | sale |
|---|---|---|
| **F3·1** · el binario | `ore-federation`: conectores `servir` calientes por familia y credencial, cola por origen (4, 16, 10 s), presupuesto (filas, bytes, tiempo) cortado en la pasarela y cancelado en el origen, `DELETE`/desconexión, *trailers* (nuevos en `ore-entrada`) y `GET /v1/read/{id}`, `/v1/connectors`, `/v1/origins`, `/v1/health`; en la imagen `ore-drivers` | ✓ |
| **F3·2** · el kit de la pasarela | `ore-kit --pasarela`: 8 casos con la pasarela de verdad delante del conector; el CI los exige | ✓ **Postgres 8/8, S3 7 + 1 no aplica** (su `url` no lleva clave). Postgres: el mismo flujo lote a lote; 6 lecturas, ningún proceso nuevo (7–8 ms en caliente); 1000 filas justas con el origen limpio en ~5 ms; tiempo a los 1510 ms con tope 1500; **40 a la vez y nunca más de 4 sesiones en el origen**, el resto 503 con `Retry-After`, la que no cabe ni en la cola en 1–3 ms; `DELETE` y desconexión (~150–210 ms) dejan el origen limpio; la clave ni en el registro, ni en las respuestas, ni en `argv`/entorno de la pasarela y sus conectores; el ocioso se cierra con su sesión |
| **F3·3** · la malla | `malla/58-la-pasarela.yaml`, plantilla del inquilino: Deployment (sin root, raíz de sólo lectura, `RollingUpdate` con `maxUnavailable: 0`), Service :8099, entrada sólo desde `ore-serve` y la salida de `ore-serve` hacia ella; la KSA y el rol del driver (sin IAM nuevo) | ✓ (ba8ba0a) el aprovisionador la pintó **solo** en t-demo y t-victor en su pasada siguiente —ninguna migración a mano; una celda nueva la recibe en su alta—; las dos `1/1` con sus tres conectores |
| **F3·4** · en vivo | `ore-serve federar-probar` dentro de `ore-serve` de t-victor, con la credencial del custodio como el agente (el camino de F4) | ✓ (5e20358) **Neon**: fría 2045 ms, caliente p50 669 ms; 50 a la vez → 20 completas y 30 `503` (cola llena), 0 errores. **BigQuery**: fría 1397 ms, caliente p50 626 ms; 20 a la vez, 20 completas; **2·10⁶ filas cortadas en 100 000 justas en 531 ms** (F0: ~117 s sin freno). **S3**: fría 415 ms, caliente p50 222 ms; corte a 100 en 107 ms; 20 a la vez, 20 completas. Ni la pasarela ni `ore-serve` dejan una credencial en su registro |

### F4 en hitos

**Decidido** (2026-10-05, con el usuario): leer en vivo es **una propiedad de la fuente**, no de una
rama —la credencial ya es de la celda, y las cotas que protegen el origen también—. Un interruptor
`federation` en la fuente (alta, `PUT /fuentes/{n}`, consola), leído siempre de `main`; la política
de conductos que manda, la de `main`; encender autoriza `federation.read` (`DRAFT`) si no estaba.
Detalle en `docs/federation.md` §4.

| hito | qué | sale |
|---|---|---|
| **F4·1** · el plan | `ore federate` (el plan desde el árbol, hermético), `ore source federation <n> on|off`, `ore source add --federation`, `flow::lectura_del_origen` (`federation.read` sin autorización es ⊥ aunque no haya etiquetas; la carga de la fuente y de cada columna; desde un puesto, también su conducto) | ✓ `el-plan-federado.sh` 14/14: apagada → 403; encendida **sólo en la rama** → sigue 403 (manda main); en main sin conducto → `OOS4011`; filtro no empujable → `422 empuje`; `forbidden` → `OOS2044`; `requiredFilters` → `OOS2045`; apagar en main la apaga en la rama |
| **F4·2** · la ruta | `POST /federation/read` (paso 0 y 1–8), `PUT /fuentes/{n}`, `federation` en `GET /fuentes` y en el alta, la puerta del puesto, el evento `federation:read` sin valores; la consola con su interruptor | ✓ `el-coordinador.sh` 14/14 con `ore-serve`, la pasarela y Postgres de verdad: 3 filas, filtro empujado, `limit`, `OOS2044`, `empuje`, 404, apagar → 403, 8 lecturas anotadas sin valores de filtro, y la clave del origen en ningún registro ni respuesta |
| **F4·3** · en vivo | encender las fuentes de victor en `main` (consola: «Enable federation») y leer desde un puesto contra Neon, BigQuery y S3; la huella desde el puesto (`iam 050`, `puesto:abrir`) | ✓ (6a143dc, 44ac508) desde test6, `pruebas-de-fuego/celda-federada-test6.py`: **Neon** 4 filas (la tabla entera) en 2,1 s; **BigQuery** 5 filas en 1,5 s; **S3** 5 filas en 0,3 s; **BigQuery `expensive`** cortada en 10⁶ filas justas (`cortado filas`, 64 MB) en 2,7 s. Las cuatro en `iam.huella` a nombre de quien abrió el puesto, con su decisión |

**F4 CERRADO** (2026-10-05).

**Lo que F4·3 destapó.** **Lo que pasaba en un puesto no llegaba a la huella.** La celda habla con el
token de su pod (`x-ore-pod`), no con el de la persona, y `ore-iam` sólo anota con `Ore-Sujeto` o con
una decisión viva —una celda no puede atribuirle a nadie lo que `ore-iam` no autorizó—: las cuatro
primeras lecturas dieron `400` y fueron a `muertos/`. No era de la federación: le pasaba a todo evento
que saliera de un puesto (la actividad, `coleccion:servir`). Arreglo, sin tocar esa regla: al abrir el
puesto, con el token de la persona delante, `ore-serve` pregunta `puesto:abrir` (por defecto a quien
pertenece, `iam 050`) y guarda la decisión con el puesto; cada evento que sale de él la nombra, y quien
lo abre otra vez la renueva (vive 24 h). Si `ore-iam` niega o no contesta, el puesto se abre igual y
se dice en el registro.

**Siguiente: F5** (el reparto), y después F6–F9. **Y al terminar el producto, decidido con el
usuario:** cerrada la federación entera —no F4—, el paso inmediato es
rehacer las **bases foráneas** como la cara SQL de un origen federado, con su propia spec: hoy son
vistas sobre un origen anteriores a este ADR, que no se consultan bien en SQL; con la federación, una
foránea son las tablas de un origen federado leídas en vivo por la pasarela, y no existe sin el
interruptor encendido.

### F5 en hitos

**Decidido** (2026-10-05, con el usuario): el reparto vive en `ore-core` (`reparto.rs`) y decide,
por cada `Table` que una sentencia lee, columnas, filtros conjuntivos y `limit` — y aplica ahí el
coste y el gobierno, antes de conectar. Tres reglas de fondo: **(A)** una lectura por tabla y
sentencia (el puesto registra cada nombre como una vista y no reescribe la sentencia): con dos
apariciones, la unión de columnas y sólo los filtros comunes; **(B)** lo que sqlparser no analiza
se lee sin empujar, con las mismas reglas de coste y el tope de siempre; **(C)** lo empujado se
vuelve a evaluar siempre en el motor — por eso un filtro del `WHERE` que rechaza el nulo baja
también al lado de un `LEFT JOIN` que puede quedar a nulo.

| hito | qué | sale |
|---|---|---|
| **F5·1** · el reparto | `ore_core::reparto::repartir`: columnas usadas (`*` todas), conjunciones `col op literal` (`=`, `<>`, rangos, `IN`, `IS [NOT] NULL`, `LIKE`, `BETWEEN`) que admiten la tabla **y** el conector, los `ON` (el lado de dentro de un `LEFT JOIN` sí; el conservado y un `FULL JOIN` no), `limit` + `OFFSET` + `ORDER BY` sólo sin nada en el motor, vistas/`WITH`/subconsultas que son proyección limpia, subconsultas correlacionadas, A, B y C; coste (`OOS2045` antes que `OOS2044`, como fija la spec) y gobierno por tabla | ✓ 26 tests, entre ellos **los 9 casos `plan/` de v1alpha24, corridos por primera vez** |
| **F5·2** · `explain` | `ore explain "<sentencia>" [--file] [--policy] [--json]`: por tabla, lo que va al origen (columnas, filtros, `limit` y orden), lo que queda en DuckDB, el coste y los avisos; un no, `error[CÓDIGO]`. `ore federate` escribe su petición como la sentencia que es y decide con el reparto (lo que quedaría en el motor sigue siendo `422 empuje`: esa ruta no tiene motor). `explain` en la lista de verbos de `ore-serve` | ✓ `el-plan-federado.sh` 13 + 5 de `explain`; **la suite de conformidad corre `plan/` con `ore explain`: v1alpha24 13/13** |
| **F5·3** · medido | `pruebas-de-fuego/medida-el-reparto.py`: las 52 frases de la sintaxis de DuckDB (`medida-el-terreno-de-la-regex.py`) y 15 consultas de analista sobre el árbol de `main` de victor | ✓ (abajo) |

**F5 CERRADO** (2026-10-05).

**Lo que F5·3 midió.** (1) **Las 52 frases**: 32 se analizan, 11 se leen sin empujar (B: `PIVOT`,
`UNPIVOT`, `SUMMARIZE`, `DESCRIBE`, `ASOF`, `USING SAMPLE`, `TABLESAMPLE`, `LATERAL`, `CREATE … AS`,
`INSERT … BY NAME` y dos sentencias en una), 9 no leen ningún origen, **0 se niegan**. (2) **Tres
errores, arreglados con su test**: `FROM t` a secas (DuckDB) y `COLUMNS(…)` pedían una columna en vez
de todas, y `TABLESAMPLE` se analizaba (un `LIMIT` empujado antes de la muestra cambia las filas: ahora
B). (3) **Las 15 consultas de victor**: 14 piden menos columnas que la tabla; 5 empujan filtro; 3
empujan `limit` (con su `ORDER BY` en Neon). **Pero 4 dejan en el motor un filtro que el conector sabe
hacer** —un rango de fechas, un `IN`, un `total > 500`, el `=` de S3— porque **la inducción declara
`predicatePushdown: [eq]` en Postgres y BigQuery, y nada en S3**, cuando sus conectores ponen los diez
operadores (F2). Esas cuatro leen la tabla entera (cortada en el tope o el presupuesto). Es la política
del dueño de la tabla y no del reparto. **Decidido** (2026-10-05, con el usuario): la inducción declara lo
que el conector v2 pone **menos `like`** —`[eq, neq, in, range, isNull]`, `ore_driver::EMPUJE_INDUCIDO`—
en Postgres, BigQuery y S3; `like` lo añade el dueño (un `%x%` sin índice es caro en el origen). Vale
para lo que se catalogue desde ahora: una tabla ya declarada no se toca sola (la línea puede ser de
alguien); se recataloga o se edita.

### F6 en hitos

**Decidido** (2026-10-05, con el usuario): F6 es el **mecanismo**, sin decidir nombres —la `Table`
por su nombre (`<origen>.<schema>.<tabla>`) y cualquier vista sin copia cuya raíz es una `Table`—, así
que las bases foráneas de hoy se leen en vivo sin cambiar su modelo; qué *es* una base foránea lo
decide su spec, después de F9. El listado de una `ObjectTable` en el `FROM` sigue en F9. Y tres
reglas: **(1)** el SDK pide cada lectura a `POST /federation/read` (la ruta de F4, con su gobierno, su
tope y su huella); **(2)** un resultado cortado se devuelve con un aviso, y con `strict=True` es un
error —el final lo da `GET /federation/read/{id}`, porque ningún SDK lee *trailers*—; **(3)** Python
primero, Node y JVM en un hito aparte.

| hito | qué | sale |
|---|---|---|
| **F6·1** · servidor | `sql_del_puesto`: si un nombre llega a un origen, el reparto de la sentencia entera (`ore explain --json --from-workspace`, en la rama y con la política de main) y en `fuentes` cada lectura ya decidida (`federada`) y el SQL de cada vista viva (`vistaFederada`); un no del reparto es la respuesta (`422 OOS2044`…). El reparto lee también las vistas de antes (`from`/`fields`, como `foreign_test`). `POST /puestos/{id}/explain` (plan y texto). `GET /federation/read/{id}` (el final, sólo a quien leyó, 15 min) | ✓ `el-coordinador.sh` +6 con Postgres de verdad |
| **F6·2** · SDK Python | `ore.sql()` pide cada lectura repartida a `/federation/read` (una por tabla), la registra en DuckDB bajo su nombre y el de quien la nombra, pone encima las vistas vivas y ejecuta la sentencia tal cual; `TruncatedReadWarning` si se corta (`strict=True`: error); `OriginReadError` con su código para un no del reparto; `ore.explain()` | ✓ `la-lectura-en-vivo-en-python.py` 6/6 (en el CI, con `el-puesto.sh`) |
| **F6·3** · en vivo | test6: Neon, BigQuery y S3 por su nombre, `bq_foreign` y `foreign_test`, juntas entre orígenes y con una copia, un corte, `explain`, la huella | pendiente |
| **F6·4** · Node y JVM | el mismo contrato | pendiente |

### F7 en hitos

**Decidido** (2026-10-05, con el usuario): `create view` sobre el origen es una **vista**: se guarda la
consulta y se lee en vivo (F6), con `federation.read`. Lo que **guarda** datos del origen —`create or
replace dataset … as select`, `insert into … select`, `create materialized view`— es una **copia**: la
hace un **Job** (sin el tope de una lectura en vivo, y el coste en vivo no le aplica: spec §4), con
`materialization.payload`. Dos formas: **copiar una tabla** (lo que baja entero: un dataset mantenido
con su `where` ampliado, spec v1alpha25) y **copiar un cálculo** (juntas, agregados, varios orígenes: el
Job lee lo repartido de cada tabla, calcula en DuckDB y publica). «Los trabajos de copia leen por la
pasarela» pasa a **F8** (toca la malla; F7 no lo necesita: el conector v2 ya pone los diez operadores).

| hito | qué | sale |
|---|---|---|
| **F7·1** · la vista y el agujero | `create view` sobre una `Table` ya compila con `federation.read` (si no, `OOS4011`: v1alpha24 `invalid/a-view-over-a-table-without-federation-read`) y se lee en vivo (F6). **`write()` de lo leído en vivo se niega** (`sql()` marca su resultado; en un transform, sus `inputs` leídos en vivo): guardar eso es una copia | ✓ `la-lectura-en-vivo-en-python.py` 8/8 |
| **F7·2** · copiar una tabla | spec v1alpha25 (`where` con los operadores del reparto), `create or replace dataset … as select` de una tabla que baja entera → dataset mantenido; el Job empuja los filtros | pendiente |
| **F7·3** · copiar un cálculo | `create materialized view`, `… as select` con juntas o agregados, `insert into … select` sobre el origen | pendiente |
| **F7·4** · en vivo | test6 | pendiente |

**Lo que F3·4 dejó dicho.** (1) **Neon en caliente sigue en ~670 ms** aunque ni proceso ni conexión se
abren: son las idas y vueltas por lectura del conector de Postgres (el catálogo de la tabla, `BEGIN READ
ONLY`, `SET LOCAL`, el portal, el `FETCH`, el `COMMIT`) contra un origen lejano. Guardar el plan por
`(url, objeto)` y juntar las sentencias lo bajaría a una o dos idas: es del conector, para cuando F6
consulte en vivo. (2) **Con 50 a la vez contra Neon, 30 se quedan fuera** (`503`, cola de 16): es la
protección que se decidió —ni una pasó de 4 sesiones—, y la cola es configurable por celda
(`ORE_FED_COLA`). (3) **Ninguna celda hubo que migrarla**: una plantilla del inquilino llega a todas
por la convergencia (0054 la hace segura).

**Lo que F3·2 destapó.** (1) El conector de Postgres no paraba al cancelar a mitad de un flujo: cancelaba
la consulta en curso, y entre dos `FETCH` del portal no hay ninguna; el bucle seguía hasta el final
(6,1 s para soltar el origen tras una desconexión). Mira una marca antes de cada `FETCH` (~150 ms);
el kit del conector sigue 14/14. (2) Un conector caliente con la conexión muerta fallaba la lectura
siguiente (`57P01`): se relanza una vez. (3) Dos relojes —el `timeoutMs` empujado y el de la
pasarela— son el mismo corte por tiempo.

**La línea de base de F2·1** (2026-10-04, en local; el CI la repite en cada empuje):

| caso | Postgres v1 | S3 v1 |
|---|---|---|
| 1 proyección | pasa (texto) | pasa |
| 2 operadores | 30/30 filas bien, sin declararlos | 7/30: sólo `eq`, rechaza los demás |
| 3 no declarado | falla: sin `capacidades` | falla: sin `capacidades` |
| 4 `limit`, `orderBy` | los sirve bien (por `ore-sql`), sin declararlos | los rechaza |
| 5 tipos | falla: texto, no Arrow | pasa: los 9 tipos y sus valores exactos |
| 6 vacía | falla: en texto no hay esquema | pasa |
| 7 flujo, 10⁶ filas | falla: 33 s, **pico 388 MiB, primer byte a los 32,9 s** —lo junta todo— | pasa: 130 lotes, pico 26 MiB, primer byte a los 7,2 s |
| 8 `timeoutMs` | falla: lo ignora | falla: lo ignora |
| 9 sólo lectura | pasa: la sesión de sólo lectura para la escritura | no aplica |
| 10 cancelar | falla: la consulta sigue viva en el origen | no aplica |
| 11 `servir` | falla | falla |
| 12 secretos | pasa (la credencial mala sale sin tipar) | pasa |
| 13 `capacidades` | falla | falla |
| 14 `estimar` | no aplica | no aplica |
| | **3 pasan, 10 fallan** | **4 pasan, 7 fallan, 3 no aplican** |

El kit midió dos cosas que no eran del conector: el tipo de un instante (el lago lo llama `+00:00`,
BigQuery `UTC`; el kit acepta los dos) y la memoria de S3 contra el S3 de mentira (en Python: sus
80 s de `grande` son del servidor, no del conector).

**Lo que F2·2 decidió.** Leer por un portal de 8192 filas dentro de una transacción de sólo
lectura (la memoria es la de un lote); tipar cada columna por el catálogo con la traducción de
`catalogo` —la que escribió la `Table`—, y sólo lo exacto: un `numeric` sin precisión, lo opaco o
una lista salen como texto y el almacén los estrecha como antes; cada valor, del cable a su texto
canónico (`texto.rs`) y de ahí a su tipo con `ore_core::tipos`, el analizador del almacén. Una
copia de humo —Postgres v2 → `ore-store-r2 sellar-flujo` → el S3 de mentira, y de vuelta— da las
10 filas de `kit.tipos` exactas, sin nada sin estrechar: las copias en vivo pasan a Arrow por el
mismo camino que BigQuery. `leer` sin `formato: arrow` sigue en texto.

**Lo que F2·3 decidió.** El tipo de cada columna de `tables.get`, con la precisión de sus
`NUMERIC(p, s)` (la Storage Read los da `decimal128(38, 9)` y se llevan al suyo, sin `safe`: un
valor que no cupiera sería un error); la Storage Read cuando se puede, con `limit` cortando los
streams, y si no la consulta por REST, ahora también en Arrow; `jobTimeoutMs` en el job y el reloj
de quien pide, agotado `jobs.cancel`; `estimar` es un *dry run*. Para el kit sin red ni coste, una
**cinta** en el conector (`ORE_BQ_CINTA`, grabar o reproducir, por consulta: la Storage Read es
gRPC y se prueba en la pasada de verdad) y un banco de BigQuery que carga la semilla por DDL
(`GENERATE_ARRAY`: cargar `grande` no procesa bytes).

**La pasada de verdad de F2·3** (2026-10-04) destapó un defecto del kit, no del conector: con
`ORDER BY importe NULLS LAST` las dos filas de `importe` nulo empatan y su orden no está fijado
—BigQuery dio 9, 7 y Postgres, por azar, 7, 9—. El caso 4 compara ahora las claves de orden, no los
`id`.

**Lo que F2·0 destapó.** `ore-sql` y el lector de JSONL traducían todo operador que no fuera `gt`
como una igualdad: con dos operadores no se notaba, con diez un `in` habría contestado otra cosa
sin fallar. Ahora cada operador tiene su forma y ninguno va por defecto, y un conector rechaza con
`operador` lo que no declaró (`Capacidades::admite`). Y la `row_restriction` de BigQuery salía de
recortar el `SELECT` del texto entero: un `ORDER BY` o un `LIMIT` detrás se habrían colado en ella.

## Lo que este ADR no decide

- **Una caché de resultados**: no por defecto (Lakehouse Federation tampoco); se decidirá con
  medidas de F0.
- **Los orígenes después de Postgres, BigQuery y S3**: entran por el kit de conformidad. S3 entró
  en F2 (y no después) por lo que prueba: es el único de los tres sin motor —empujar un filtro es
  pedir menos bytes y el conector hace de motor—, y un contrato comprobado sólo contra dos bases
  SQL se habría quedado con supuestos de SQL. Sus `ObjectTable` siguen en F9.
- **Federar fuera de la celda** (un motor de otra organización): fuera de alcance.

## Deuda y pistas

- El nombre del conducto, los códigos `OOS` nuevos y las cotas por defecto salen de F0 y F1.
- `maxRowsPerRequest` y `joinPushdown` están en el vocabulario de `reads` y nada los lee
  (`ore-view/src/capabilities.rs`): F2 y F9 los leen.
- El servicio por celda suma un deployment y una cuenta con permisos al origen: la malla y el IAM
  necesitan el visto bueno de quien los gobierna antes de F3.
