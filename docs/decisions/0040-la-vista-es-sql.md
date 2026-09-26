# 0040 · La vista es SQL (OOS v1alpha14 en ORE)

**Estado:** en curso · pasos 0 a 4 hechos.
**Spec:** `C:\oos` 7d92e6e, `spec/v1alpha14/`.

## Contexto

Desde OOS v1alpha14 una `View` es sólo SQL: `spec.sql`, `spec.dialect: duckdb` y
`spec.columns`, el contrato derivado. Lo que se gobierna de ella —lo que lee, el linaje
por columna, las etiquetas, el canal lateral (`OOS4016`)— se deriva de la consulta. Lo
estructurado se queda en la semántica (`Entity`).

En ORE la forma estructurada se lee en un sitio (`ore-core/src/vistas.rs`, 162 usos en
19 módulos) y llega al motor por una costura (`ore-cli/src/vista.rs::cuerpo`, 6
llamadores). Todo supone **una raíz** por vista (`raiz` 14 usos, `raiz_de_lectura` 8,
`respaldo` 9); una vista SQL puede leer varias fuentes, y ése es el grueso del cambio.

## Decisiones (del usuario, 2026-09-25)

- **A · Una sola View dentro de ORE, la SQL.** Una View v1alpha8–13 se **traduce a SQL al
  cargarla** (la misma traducción que la migración). Las versiones anteriores siguen
  significando lo mismo, como dice la spec (§7), sin dos motores. La conformance
  v1alpha5–13 entera es la puerta.
- **B · Las Views del árbol del usuario no son datos de producción.** Se migran o se
  borran sin ceremonia; no gobiernan ninguna decisión.
- **C · Spark por `/v1` se queda sin vistas hasta la migración de dialecto.** `loadView`
  de una vista `duckdb` pedida por Spark se niega y lo dice (spec §8, PUEDE). La
  alternativa —traducir el texto DuckDB a Spark al servirlo— cambiaría la semántica en
  silencio donde los dialectos difieren; la otra —una consulta por dialecto, como las
  *representations* de Iceberg— es la migración misma, y llega con Spark.
- **D · La copia de una vista es un dataset**, y se rehace entera ejecutando la consulta.
  `ore-view` y `ore-maintain` (mantenimiento incremental) dejan de recibir vistas; su
  destino se decide aparte.
- **E · `diff`:** `OOS5028/5029` (filtro estrechado/ensanchado) se basaban en la forma.
  Pasan a comparar el contrato; cualquier cambio del texto de la consulta es cambio de
  filas (spec §9).

## Pasos

0. **Medir.** Prototipo de `vista_sql` sobre todas las Views propias y de conformance
   traducidas: el linaje derivado del SQL ¿es idéntico al del motor estructurado? Es la
   puerta para quitar la forma. Y los usuarios de `raiz` que se rompen con varias fuentes.
   **HECHO** (`medida-el-linaje-de-la-vista-sql.py`, prototipo
   `crates/ore-core/examples/vista_sql.rs`):
   - L1: de 214 Views del repositorio se traducen 213 (144 de una Table, 48 de una View,
     9 de un Dataset, **12 de la forma v1alpha7 `from: { datasource, object }`**, que no
     tiene nombre del árbol: el paso 3 le da uno sintético); la que no, es un caso
     inválido a propósito (`OOS2034`). El prototipo analiza las 213.
   - L2: **115 de 115** Views válidas dan exactamente el linaje del motor (salida, raíz,
     DIRECT/INDIRECT). Las otras 99 son casos inválidos que `ore view` no sigue; las
     cubre la conformance en el paso 3 (18 de ellas son de flujo: `OOS4001/4002/4011`).
   - Medir corrigió la spec: el motor no deja la arista del `GROUP BY` hacia las propias
     claves, sólo hacia los agregados; y el `HAVING` mira las claves. §5 de
     `01-la-vista-es-sql` lo dice ahora así (una clave no proyectada, hacia todas).
   - L3: 61 usos de la raíz única fuera de `vistas.rs`, en 13 ficheros (`vista.rs` 26,
     `materializar.rs` 7, `flow.rs` 6, `registro.rs` 5, `assets.rs` 5, …).
1. **La spec entra en ORE.** Casos de conformance v1alpha14 en `C:\oos` (válida;
   `OOS1005`; `OOS2038` por `read_parquet`, dos sentencias, `INSERT`; `OOS2039`;
   `OOS4016` con etiqueta y el mismo rango sin ella; `HAVING count(*) >= 8`; `OOS4001`
   por la arista INDIRECT de un `WHERE`). Bump de `vendor/oos`, `ApiVersion::V1Alpha14`,
   claves del spec en `document.rs`.
   **HECHO**: 24 casos en `C:\oos` 510fe10 (9 aceptan, 15 rechazan; README con la tabla),
   `vendor/oos` al día y el marcador `borrador_de_v1alpha14` en `conformance.rs`: **0 / 24**,
   todo pendiente y nada roto. `ApiVersion::V1Alpha14` y las claves de `document.rs` pasan
   al paso 3: aceptar la versión antes de que `comprobar` sepa leer una vista SQL
   convertiría los pendientes en regresiones.
2. **`vista_sql` en ore-core**, pura y sin motor: lo que lee (sin los del `WITH`;
   generadores sí, lectores por función `OOS2038`), lo que proyecta (`*` contra los
   contratos de sus fuentes), linaje por columna (directo, derivado, INDIRECT) y
   predicados clasificados para el canal lateral.
   **HECHO**: `ore_core::vista_sql::analizar(sql, columnas_de)` → `Consulta { lee,
   columnas (directas · derivadas · indirectas por columna), indirectas, predicados
   (Revela | Ordena, con su lugar: WHERE · JOIN · QUALIFY · HAVING), ambiguas,
   sin_fuente, estrellas_sin_expandir }` o `Fallo` (`NoSeAnaliza`, `NoEsUnaConsulta` y
   `LeePorFuncion`, las dos últimas `OOS2038`). `columnas_de` es el árbol: expande un `*`
   y decide una columna sin calificar entre dos fuentes. Las referencias salen a un
   nivel (a los nombres que la consulta lee); componer la cadena es del paso 3. 19 tests,
   con el corpus de la medida (todo se analiza salvo `PIVOT`). El ejemplo `vista_sql` ya
   es una envoltura del módulo, y con él la medida sigue en 115 de 115. Añadido al
   medir: un `LIMIT` con `ORDER BY` mira por lo que ordena (qué filas salen depende de
   ello); un `HAVING` sobre un agregado no es un predicado del canal lateral.
3. **Una sola View en el núcleo.** `vistas.rs` sobre `vista_sql`: `Raiz` pasa a fuentes +
   linaje; la traducción forma→SQL baja a ore-core; `comprobar` (`OOS2018/2019`, `2039`,
   `2011/2022` contra `columns`, `2020`); `flow.rs` propaga por el linaje con varias
   raíces y añade `OOS4016`; se migran los 19 módulos.
   **Tanda 1 HECHA** (medido antes de seguir, `medida-las-consumidoras-de-la-vista-sql.py` y
   `medida-todas-las-vistas-por-el-linaje.py`):
   - 3a–3c (900a8f3): v1alpha14 aceptada, `comprobar_sql`, `linaje.rs` y el flujo sobre el
     linaje con `OOS4016`.
   - **Una sola View.** Medido con todas las vistas por el linaje: `ore validate` igual en los
     410 árboles del repositorio, `ore diff` igual en los 25 casos, y 24 clasificaciones de
     assets cambiaban por un fallo de la traducción: una vista estructurada sobre la tabla del
     mismo nombre (`from: { table: ventas.clientes }`) se traducía a `FROM ventas.clientes`,
     que resolvía a la propia vista. Arreglado (la forma estructurada resuelve su fuente por
     su clave), la medida da **0 cambios**, y la puerta se quita: toda vista y todo dataset
     mantenido se gobiernan por su linaje (`linaje::por_el_linaje`).
   - **Un nombre, una cosa** (decidido por el usuario, como Unity): en v1alpha14 una tabla,
     una vista y un dataset comparten el espacio de nombres de su schema (`OOS2035`); una
     consulta que nombra una pareja de antes es `OOS2018`. Había 45 parejas en 30 árboles.
   - Una errata de la spec, medida: en la forma estructurada la ausencia es `[]`, no `null`.
   - Conformance v1alpha14 26/26.

   - **Lo que la medida no vio, y los tests sí**: tres árboles de `ore-cli/tests/vistas.rs`
     —copias v7/v8 que exponen `id` y recortan por una columna etiquetada, y la copia de
     `sum(salary)`— pasaban `ore validate` y sólo `ore view` los negaba. Con una sola View el
     linaje lleva la arista INDIRECT y `validate` da `OOS4002`. **Decidido (usuario): se acepta
     como el cierre del agujero** que la cabecera de `vista.rs` anunciaba; la spec lo dice como
     excepción de seguridad (C:\oos bcedd3c). Lección: la medida de «0 cambios» tiene que
     pasar también los tests, que escriben árboles que no están en disco.

   **Tanda 2 HECHA**, lo que la medida 1a encontró:
   - `diff` (decisión E): una vista escrita como consulta se compara por su contrato
     (`OOS5001` columna que se va, `OOS5002` tipo que cambia) y por sus filas: la consulta sin
     su proyección, reescrita (`vista_sql::filas`); si difiere, `OOS5028` y `OOS5029`, como un
     cambio incomparable de la forma. Espacios o proyección no cuentan.
   - `assets`: `define` de una vista SQL (`dialect`, `sql`, `lee`), los tipos de su contrato y
     sus `sale_de`. `aristas.rs`: una entidad sobre una vista SQL de una sola tabla entra en el
     índice de topología.
   - `sql_del_arbol` y `puestos::datos_de_vista`: una vista SQL que lee datasets se deja leer
     (`vistas::se_lee_de_datasets`); servirla es del paso 4, y `ore ask --sql` lo dice.

   Lo que era, antes de hacerlo:
   - `diff` de una vista SQL: un cambio que quita filas sale como parche (`1.0.1`) y en una
     estructurada es `OOS5028` → decisión E.
   - La faceta de `assets` (sin `define`, sin tipos del contrato, sin `sale_de`) y
     `aristas.rs` (una entidad sobre una vista SQL no entra en el índice de topología).
   - `sql_del_arbol` y `puestos::datos_de_vista` niegan leer una vista SQL que lee datasets
     (`raiz_de_lectura` no cruza una consulta).
   - `ore view` da un informe hueco de una vista SQL; `ask --sql`, `materializar`,
     `registro`, `invocar` y `funciones` son del paso 4.
4. **Servir.** `ore ask --vista --sql`, `/vistas/…/ejecutar` y `datos_del_puesto` sirven
   `spec.sql` con los nombres resueltos, como una unidad `.sql`, sin `a_sql`. La copia se
   rehace entera. `/v1` `loadView` en `duckdb`; Spark se niega (C).
   **HECHO**, en dos tandas y con dos decisiones del usuario: **servir también las
   estructuradas por su consulta** (una sola View también al servir), y **lo que se calcula
   fuera del código del usuario lo calcula un trabajo con la imagen del puesto**.
   - **4a–4b** (`abc1b5f`): `ore_core::servir` sirve la consulta de una vista con cada
     nombre del árbol resuelto: un dataset por el nombre con que lo registra quien lee
     (`"__ore_dataset"."p.n"` en un puesto; su nombre del catálogo en `/v1`), una vista como
     subconsulta con su alias, una tabla de un origen nunca. **Del todo, no dejado a
     DuckDB**: medido (`medida-servir-la-vista-como-sql.py`), un nombre de dos partes dentro
     de una vista de DuckDB se resuelve contra el schema de la vista. `ore ask --sql` sirve
     por aquí las dos formas, con los tipos del contrato o del plan; `a_sql` ya no sirve
     nada, y la vieja y la nueva dan lo mismo en las 5 vistas de la medida (S4). `/v1` da una
     sola representación, `duckdb`. `el-puesto.sh` 10b: la vista SQL da por `over()` y
     `sql()` las mismas filas que la estructurada.
   - **4c · la copia, el «Run» y `over`.** Cotejado antes con lo que hace la industria
     (Databricks/Unity, Snowflake, BigQuery, dbt, Foundry) y con el código: el refresco de
     una tabla derivada corre con una identidad de servicio y en cómputo gestionado, nunca
     en la sesión de quien lee; cada dataset mantenido tiene **un solo escritor** y lo impone
     el sistema; el refresco entero entra en **un commit atómico**; el estado es lo que queda
     en el destino; «Run» corre en la sesión de quien lee, con sus permisos. Nuestra pasada
     de la copia (0027, `malla/48`) **ya es eso** —identidad de servicio, push como *swap*
     con la forja de *compare-and-set*, el puntero como estado, reencolar idempotente por
     contenido—, así que la copia de una vista SQL **entra en ella** y no en `POST
     /trabajos` (que sólo lanza una persona, en su rama, con el estado en memoria):
     - `ore materialize --preparar DIR` vuelca la consulta servida y lo que lee
       (`ore-store volcar`, Arrow); `python -m ore.calcular DIR`, un contenedor de la imagen
       del puesto **sin el testigo montado**, la ejecuta con DuckDB cerrado al exterior;
       `ore materialize --calculado DIR` la sella (`ore-store sellar-arrow`, las columnas y
       los físicos del contrato) y la pasada empuja el puntero como siempre. Sin nada que
       calcular, los dos primeros pasos no hacen nada.
     - La cabecera: plan = digest de la consulta servida; esquema = el contrato; testigo =
       el snapshot de cada dataset que lee (`p.n@s`, ordenados). Si nada se movió, «ya
       está» sin leer una fila. Se rehace entera (D). El flujo lo comprueba el compilador
       por el linaje (`OOS4002` sobre `materialization.payload`).
     - **Un solo escritor, sin excepción nueva:** la copia la sella `ore-store` desde la
       pasada, como cualquier copia; `write()` sobre un mantenido sigue negado.
     - La copia de una vista SQL es la vista **entera** (D): un `Dataset` con `from: { view }`
       y `fields`/`where` encima no es una copia, y su puntero lo dice.
     - **Medido y arreglado de paso:** con una vista SQL con copia en el árbol, `ore view`
       salía con 65, y el Job de la copia (`ore view . || exit 1`) **se caía entero, con
       todas las copias del inquilino**. Ahora `ore view` la enseña por su consulta (qué
       lee, su contrato, de dónde se copia), su `raíz` es el lago, y sin `--calculado` su
       puntero no se toca.
     - `ore ask` de una vista SQL (y de su copia) contesta con las filas de su copia; sin
       copia, «se lee en un puesto», y `/vistas/…/ejecutar` lo da como **409**: el «Run» de
       una vista SQL es `sql()` en el puesto de quien lee (la consola, paso 7).
     - `over:` una vista SQL lee su copia: `vistas::dataset_de_lectura` la busca **encima**
       (`copia_de_la_consulta`), porque una vista SQL no tiene dataset debajo.
   - **4d:** `el-lago.sh` 14c, de punta a punta con el S3 de mentira: un `GROUP BY` y un
     `JOIN` de dos datasets —que el motor de vistas no sabe— copiados, el testigo de cada
     entrada, `ask` desde la copia, la segunda pasada «ya está», y una copia con `where`
     y una consulta que falla al ejecutarse dicen su motivo en el puntero.
   - **En `demo`, de verdad** (`la-copia-sql-en-demo.py`, 2026-09-26, imágenes de `6db8ead`):
     un `write()` desde `puesto-python` deja la entrada (6 filas) en el bucket del inquilino;
     una vista SQL (`GROUP BY`) y su copia llegan al árbol por la forja; el Job «rehacer»,
     rendido de la plantilla que trajo la convergencia, corre en `jobs-p` en **20 s**:
     `preparar` vuelca 6 filas con `ore-store-gcs`, `calcular` las ejecuta con DuckDB como
     65532 (3 filas), `copiar` sella la copia en `gs://…/catalogo/prueba_sql/…` y empuja el
     puntero como `copiador`: `copiada`, 3 filas, 6 leídas, el testigo es el snapshot de la
     entrada. Retirado después (árbol y cola; los objetos, la pasada siguiente).
     Medido de paso: un script que toma las imágenes del `HEAD` local cae en
     `ImagePullBackOff` si hay un commit sin empujar; se toman las del commit que corre.
   - Queda: `registro` (el matcher no ve una copia por consulta: una estructurada sobre
     ella no se contesta desde su copia) y el estado de `POST /trabajos` en memoria, que la
     copia ya no usa.
5. **`CREATE [OR REPLACE] VIEW` en el guion** (ADR 0039): en el puesto DuckDB describe el
   `SELECT` → `columns` → documento en la rama; los códigos OOS vuelven como error de la
   celda; resultado `object/status`. SDK `crear_vista`; `ore view add` y el inductor
   escriben SQL.
   **5a–5c HECHOS**, medidos antes (`medida-create-view.py`: `DESCRIBE` no lee una fila, <1 ms
   con 0 o con 10 M) y cotejados con Databricks/Unity, Snowflake, BigQuery, Postgres y Trino:
   - La frase: `create [or replace] view [if not exists] b.s.v [(col [comment '…'], …)]
     [comment '…'] [with schema evolution] as select …` y `drop view [if exists] b.s.v`, en el
     guion y en una celda suelta (`guion.rs`, `Sentencia::CrearVista/BorrarVista`). `or replace`
     e `if not exists` no van juntas (Databricks, Snowflake); `temp` es de DuckDB.
   - Su cotejo: la base y el schema, «ya hay», **un nombre que ya es un Dataset o una Table no se
     reemplaza nunca** (`OOS2035`), lo que lee existe (una Table también: la vista es virtual), y
     una vista del guion se lee en la sentencia siguiente.
   - `crear_vista` (SDK): la consulta **tal como se escribió** (`spec.sql`; los nombres se
     resuelven al servirla, como Snowflake y BigQuery); el contrato lo describe DuckDB sobre
     tablas vacías con los tipos del índice (sin credencial ni puntero); `PUT /documentos/View`
     en la rama del puesto, y un código OOS es el error de la celda.
   - DuckDB → OOS (0032): los enteros, `HUGEINT` incluido (`sum(bigint)`), son `Integer` —si un
     día no cabe en 64 bits la copia falla con la columna, como Databricks ANSI o BigQuery—;
     `DECIMAL` es `Decimal`, `DOUBLE` (`avg`, `/`) `Float`, `T[]` `list<T>`, `BLOB/INTERVAL/JSON`
     `Opaque`; STRUCT y MAP se niegan con el remedio (`s.campo as x`).
   - Columnas: sin alias (`id + 1`) o repetidas se niegan pidiendo alias o la lista de
     columnas (más estricto que Spark y Postgres, que inventan `(id + 1)` y `?column?`).
   - Reemplazar: añadir columnas se dice; **quitar una o cambiarle el tipo rompe a quien la lee
     y se niega salvo `with schema evolution`** (Postgres sólo deja añadir al final).
   - El dueño es el del schema o el de la base, no la persona (quién la creó lo dice el
     commit). `comment` → `description`, de la vista y de cada columna.
   - `drop view`: si otra cosa la lee el árbol empeora y no se quita.
   - **5c · el editor** (`lsp_sql.lo_que_duckdb_entiende`): el servidor de lenguaje del puesto
     diagnosticaba cada sentencia con `explain` de DuckDB, que no conoce el guion. Medido con un
     guion de `create schema/dataset/view` y `drop view`: **3 errores falsos, y el de verdad —una
     columna mal escrita dentro de la vista— escondido**. Ahora lo del guion no va a DuckDB, y de
     `create view … as` y `create dataset … as` se comprueba la consulta, en su sitio: 1 error, el
     de verdad (`el-puesto.sh` 3d).
   Queda: `ore view add` se retira y el inductor pasa al paso 6 (el nombre de la pareja
   Table/View, `OOS2035`, y las columnas sin tipo).
6. **Migrar y suprimir.** `ore migrate` reescribe v8–13 → v14 (tipos de sus fuentes) en
   `casos/`, `acme-retail` y el árbol del usuario; lo que no compila se borra. Se quita la
   forma: `cuerpo()` estructurado, emisores de `autoria` y del inductor.
   **Medido antes** (`medida-migrar-a-v14.py`, 97 árboles con vistas: los de la conformance
   v8–13 y `casos/`): 131 de 133 vistas se traducen y las 49 de los válidos tienen contrato
   entero; pero **la forma no se puede quitar del código**: v14 la mantiene en el `Dataset`
   mantenido, y la conformance v5–13 (decisión A) espera sus reglas (`OOS2020/2021/2023–2025`,
   `OOS5019/5020/5028–5033`). Lo que se suprime es lo muerto de verdad. Y dos cosas que
   romperían: una tabla y una vista con el mismo nombre (el patrón del inductor, `OOS2035` en
   v14), y la copia de una vista que lee una tabla (4c sólo lee el lago).
   - **6a · `ore migrate v1alpha14`** (`migrar_v14.rs`), en dos tiempos sobre una copia del
     árbol: **los nombres** —la vista se queda el suyo, que es el que nombran entidades,
     funciones y acciones; la tabla que se llamaba como ella pasa a `<n>_t` (su `object` no
     cambia) y el dataset a `<n>_copia` con su puntero mudado diciendo dónde siguen sus bytes;
     quien los lee por `from`, reapuntado en su sitio sin tocar el resto del fichero— y luego
     **las consultas**: cada vista es su `como_sql`, legible, con el contrato del plan sobre
     los tipos **de la fuente** (`vista::tipos_de_fuente`: el `Money` de una entidad es de la
     entidad; lo que la tabla no tipa es `String` y se avisa). La copia de una vista sobre una
     tabla pasa a leer la tabla con la forma de la vista (la siguen copiando los drivers) y la
     vista es la consulta sobre su copia; con forma encima, las dos se componen si ninguna
     agrupa. Encadena la v1alpha12 si falta. **El criterio de hecho**: el árbol migrado da
     exactamente los mismos diagnósticos, código a código —la cadena entera se ensaya en una
     copia y se coteja contra el árbol de antes—; si no, o si algo no se migra solo (una vista
     v1alpha7, una cadena de vistas hasta una tabla copiada), no se escribe nada.
     Medido: **36 de 37 árboles válidos migran** con 0 errores (el que no, `casos/con-vista`,
     es el testigo v7 de `migracion.rs` y se queda); **ningún inválido calla su código** (29 no
     migran, 31 migran y lo siguen diciendo). `acme-retail` migrado en `C:\oos` (los
     comentarios que enseñan, dichos en v14), `casos/jerarquia` también; el árbol del inquilino
     (traza local) migra con sus mismos 10 errores.
     Dos fallos que la medida sacó y ya estaban: la v1alpha12 reapuntaba el `from.table` de un
     dataset a su propia copia cuando la tabla y la vista se llamaban igual (se leía a sí
     mismo), y `OOS7014` miraba sólo `fields`, así que una función sobre una vista SQL «no
     veía» ninguna columna (ahora `vistas::expone`, que lee el contrato).
     Lo que cambia además de la forma, dicho: cada copia por consulta se rehace entera **una
     vez** (su cabecera es la consulta servida), y se pierden los comentarios de los
     documentos reemitidos, como en la v1alpha12.
   - **6b · el inductor emite v14, y `ore view add` se va.** La vista que propone por cada
     objeto es la consulta de su forma —escrita con la forma de siempre y traducida por
     `como_sql`, así que una vista inducida y una migrada son el mismo texto—, en DRAFT, con
     el contrato de los tipos que el conector tradujo (`String` donde no supo). Su tabla se
     llama **`<objeto>_t`**, siempre: la vista o el dataset que exponen el objeto se llaman
     como él, y en v14 un nombre es una sola cosa; el `object` es el del origen. El dataset
     que copia un objeto sigue llevando la forma (v14 la mantiene en él). `con_schema` ya no
     baja a v1alpha13 un documento v14. `ore view add` (`autoria.rs`), el segundo emisor, se
     retira: una vista nueva nace de un `CREATE VIEW` en un puesto; `los-documentos.sh` 12
     deja de cotejarse con él. Queda para el paso 7: el PUT estructurado de la Forge sigue
     escribiendo la forma de antes hasta que la consola escriba SQL.
     Que el inductor escriba consultas sacó a la luz lo que las herramientas que reapuntan
     nombres no sabían de una vista SQL —y que ya faltaba desde el paso 5 para cualquier
     `CREATE VIEW`—. El núcleo dice ahora **qué nombra una consulta y dónde**
     (`servir::nombrados`, por las posiciones de `sqlparser`) y lo **reescribe en su sitio**
     (`servir::renombrar`, citado como estaba). Con eso:
     - `package move`/`split`/`merge` reapuntan la consulta de quien lee lo que se mueve, y
       sus componentes cuentan lo que la consulta lee;
     - `OOS2028` mira también lo que lee la consulta: una vista SQL que cruza a un paquete
       que no lo exporta ya no pasa callada;
     - `schema rename` reconoce los nombres entre comillas;
     - `drift-detect` dice quién proyecta una columna por el linaje, no por `fields`;
     - `ore view` de una vista SQL que lee una tabla enseña su raíz y sus caras, y que se lee
       sobre un dataset que la copie.
     Y la migración no se lleva en silencio lo que el emparejador de `ore ask` hacía: una
     vista sobre una tabla que un dataset copia se contesta hoy desde esa copia, y como
     consulta no; si la hay, no se migra sola (medido: ninguna en el corpus ni en los árboles
     reales).
     Y la vista que el inductor deja sobre cada tabla es **una vista SQL v1alpha14 de pleno
     derecho**: el catálogo de assets deja de reconocerla por el `__` de su fichero y de
     esconderla como `vistaInducida` de su tabla; sale como un ítem más. El servidor de SQL del
     puesto, que la sugería al leer una tabla de otra fuente, la saca de las relaciones del
     catálogo (`produce`).
   - **6c · `OOS2021`, `OOS2023` y `OOS2029` sobre la copia de una vista SQL: no se portan,
     y es lo que dice la spec** (§10 no las cuenta entre las que siguen valiendo). `OOS2029`
     (el origen no proyecta) y `OOS2023` (copia fechada por columna sin clave) hablan de copiar
     desde una tabla: la copia de una consulta lee sólo datasets del lago y se rehace entera,
     así que no puede darse ninguna; la copia de la tabla —el dataset `from: { table }`— es
     estructurada y las sigue teniendo. `OOS2021` (entidad mutable sobre algo que sólo anexa)
     no se decide mirando una consulta: una SQL puede sacar el estado presente de un log de
     eventos (la última fila por clave), que la forma no podía; portarla daría falsos positivos.
   - **6d · lo muerto, fuera**: el crate `ore-maintain` (el mantenedor Δ de 0013), que no se
     construía en ninguna imagen ni lo llamaba nadie. Los ADR que lo describen quedan como
     historia. La forma estructurada se queda: v14 la mantiene en el dataset y la conformance
     v5–13 espera sus reglas.
7. **Consola.** (La Ontology Forge, legacy y sin entrada en la navegación, se retiró entera de
   rubix-platform el 2026-09-26: ya no hay pantalla de documentos que migrar.)
   `DocumentoView` = `sql/dialect/columns`; la Forge enseña SQL y contrato;
   «As SQL» lee `spec.sql` (fuera `como-sql.ts`); el borrador de vista es un `.sql` con
   `CREATE VIEW`; faceta del catálogo, `ordenDeCampos` y mocks.
8. **Pruebas de fuego y cierre.** Las 15 `.sh` que usan vistas; nueva
   `la-vista-es-sql.sh` de punta a punta (`CREATE VIEW` en el puesto → documento →
   linaje → `OOS4016` → `sql()` → `/v1` → `Entity` con `backedBy` → copia). Cierre.
