# 0053 · ORE Federation Engine

**Estado:** **aceptado** (2026-10-04) · F0 medido · **F1 cerrado** (contratos en [`docs/federation.md`](../federation.md); spec
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
| **F2** · los conectores v2 | Postgres (Arrow, operadores, `limit`, `statement_timeout`, sólo lectura) y BigQuery (operadores, `limit`, Storage Read con `row_restriction`); el **kit de conformidad** | dos conectores que lo pasan |
| **F3** · la pasarela | `ore-federation` por celda: rol `driver`, NetworkPolicy, conectores calientes, flujo Arrow, salud; cotas por origen | servicio en la malla (binario antes que malla) |
| **F4** · el coordinador | en `ore-serve`: la lectura federada —conducto, acceso (A8), credencial, anotación, tope—, en la rama | ruta + prueba de fuego |
| **F5** · el reparto | en `ore-core`: columnas, predicados y `limit` por relación; `explain` | biblioteca + tests |
| **F6** · consultar | en el puesto: las tablas del origen y las vistas foráneas en el `FROM`, juntas con el lago | consulta en vivo |
| **F7** · crear | `create view` sobre el origen, `create dataset … as select`, `insert into … select`, `create materialized view`; los trabajos de copia leen por la pasarela | crear desde el origen |
| **F8** · una vía | catálogo, comprobación y explorar por la pasarela; ningún camino lanza un conector por su cuenta | una sola vía |
| **F9** · más allá | `ObjectTable` en el `FROM`; agregados y juntas empujados donde se declaren; Flight SQL para motores de fuera (Spark, BI) | |

## Lo que este ADR no decide

- **Una caché de resultados**: no por defecto (Lakehouse Federation tampoco); se decidirá con
  medidas de F0.
- **Los orígenes después de Postgres y BigQuery**: S3 con tablas de ficheros, y los demás, entran
  por el kit de conformidad.
- **Federar fuera de la celda** (un motor de otra organización): fuera de alcance.

## Deuda y pistas

- El nombre del conducto, los códigos `OOS` nuevos y las cotas por defecto salen de F0 y F1.
- `maxRowsPerRequest` y `joinPushdown` están en el vocabulario de `reads` y nada los lee
  (`ore-view/src/capabilities.rs`): F2 y F9 los leen.
- El servicio por celda suma un deployment y una cuenta con permisos al origen: la malla y el IAM
  necesitan el visto bueno de quien los gobierna antes de F3.
