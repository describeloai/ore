# 0045 · El puntero es de la fuente: una database standard es sus datasets

**Estado:** aprobado (2026-09-27); B0 medido; el puntero medido como producto y como implementación; P1, P1′, P1.5, P2 y P3′ hechos; P4 escrito y medido (la migración de los árboles, tras desplegar); P5–P6 por hacer · **Decide:** dónde
vive la `Table` que apunta al origen, y por tanto qué hay dentro de una database. Revisa la
ubicación que fijaron P1 I4b (`f7580aa`) y [`0033`](0033-el-dataset.md); **no** revisa lo que
0033 decidió sobre el `Dataset` (§ «Lo que no se hace»). Toca el `_t` de
[`0040`](0040-la-vista-es-sql.md) paso 6b.

## Lo visto

La primera database standard de BigQuery (`victor`, `bq`, 2026-09-27) enseña **seis** objetos
en el catálogo para tres tablas del origen: `pedidos` (Dataset, la copia) y `pedidos_t` (Table, el
puntero), y así con cada una. El cliente pulleó tres tablas y espera tres datasets, como en
Foundry, donde un *sync* produce **un** dataset y lo que apunta al origen es del sync, no un
segundo objeto del proyecto.

No es nuevo, y no es de BigQuery. Contado en los árboles del cluster:

| database | clase | Dataset | Table | View |
|---|---|---|---|---|
| victor `standard_test` (Postgres, 21-sep) | standard | 19 | 19 | 0 |
| victor `foreign_test` (Postgres, **la misma fuente**) | foreign | 0 | 19 | 19 |
| victor `bq` (BigQuery, hoy) | standard | 3 | 3 | 0 |
| demo `olist_copia` | standard | 3 | 3 | 1 |

Una standard ha escrito siempre el par `Table` + copia. Hasta el 26-sep los dos se llamaban igual
y el catálogo pintaba un nodo; `664b8c1` (v1alpha14: Table, View y Dataset comparten nombre,
`OOS2035`) renombró la tabla a `<n>_t` y desde entonces se ven los dos. Y la tabla de abajo enseña
el segundo síntoma: **la misma fuente tiene 38 punteros para 19 objetos**, porque cada database
escribe los suyos.

## Lo medido: que el Dataset lleve su origen (opción A)

La forma obvia —`Dataset` con `from: { datasource, object }`, tipos físicos, `reads` y `changes`—
se midió entera (spec, compilador, copia, consola) y **no se hace**:

- **La spec la rechaza por escrito**, con esta misma forma: *«un `Dataset` que llevara
  `datasource` + `object` + `columns` físicas sería `Binding` otra vez»*
  (`oos/spec/v1alpha12/00-scope.md:59-62`; y 0033 § «La `Table` no se absorbe»). `OOS1005` prohíbe
  las claves, el `oneOf` de `from` tiene tres formas, y mantenido y escrito se excluyen.
- **`reads` y `changes` tendrían dos significados en un documento**: en la Table son las caras del
  origen (qué empuja, qué testigo); en el Dataset son las del lago (fijas) y lo que acepta quien
  escribe.
- **La raíz cambia de naturaleza**: todo lo que baja por `from` hasta una Table —herencia de
  etiquetas del datasource y de sus **columnas**, `OOS2020/21/23/24/29`, `OOS7008`, el camino de
  efectos— se apoya en `vistas::raiz`/`suelo`. Medido: ~35 funciones, 24 brazos de
  `Fuente::Tabla`, ~45 lecturas de `datasource|object|reads|changes`. Con la clave nueva esas
  comprobaciones **dejan de correr sin error**, y un Dataset raíz y mantenido a la vez da una
  recursión en `carga_de → etiquetas_de_raices`.
- **Una standard no sería solo datasets de todos modos**: una tabla modelada sin clave es una
  `View` que lee la Table (`inductor.rs:790-819`), y ascender una foránea a standard borraría las
  Tables que leen sus vistas y el SQL escrito a mano.
- Pide `v1alpha16`, esquemas y conformance nuevos, y deja `retirar_fuente`, `ore drift`,
  `registro::restricciones` y el esquema de `ore-serve` ciegos a una fuente.

## Lo propuesto: el puntero vive en la fuente (opción B)

Foundry no mete el origen dentro del dataset: lo pone **en el sync, que es de la fuente**, y en
el proyecto queda el dataset. La traducción aquí no pide tocar la spec, pide mover un fichero:

1. **Las `Table` se escriben en el paquete de la fuente**, que ya existe
   (`packages/<fuente>/`, con su `discover.catalog.json`, lo crea el Job de catálogo). Una por
   objeto del origen que alguna database use, **una vez**, con el nombre del objeto (sin `_t`:
   otro paquete, otro espacio de nombres). El paquete las **exporta** (`exports`, v1alpha8).
2. **Una database standard es sus Datasets**: `from: { table: <fuente>.<schema>.<objeto> }`.
3. **Una database foreign es sus Views** sobre las mismas Tables.
4. **Lo que es del objeto se decide una vez**: la clave (`changes.key`), las etiquetas de columna
   y el testigo son del objeto del origen, no de quien lo lee. Hoy dos databases de la misma
   fuente pueden contestar dos claves distintas para la misma tabla; con esto no.

Lo que resuelve de lo medido en A, sin tocarlo: la raíz sigue siendo una Table (no cambia ni una
regla ni una comprobación); la standard sin clave lee la Table de la fuente; ascender cambia Views
por Datasets y las Tables no se mueven; `retirar_fuente` las encuentra en el paquete de la fuente.

En el catálogo: la database enseña **tres** datasets; los punteros se ven bajo la **conexión**,
que es donde están (la ficha de la fuente ya lista su esquema).

## Lo que no se hace

- **No se oculta** la `_t` en la consola. El problema está en qué escribe el pull, no en qué pinta
  el catálogo, y ocultar deja 38 punteros donde hay 19 objetos.
- **No se revisa el `Dataset` de 0033** ni la regla de v1alpha12: el Dataset sigue sin llevar el
  origen. Lo que cambia es de quién es la Table.

## B0 · lo medido (2026-09-27)

**Empírico**, sobre una copia del árbol vivo de `victor` con el `ore` de `main`, moviendo a mano:

| prueba | resultado |
|---|---|
| `bq`: sus 3 Tables a `bigquery_20260927_1428`, sin `exports` | `OOS2028` ×3: **resuelve** (no es `OOS2018`), falta hacerlo público |
| `exports` en dos partes (`ventas.pedidos`) | `OOS2027`: se cualifica sin namespace; **va en tres partes** |
| `exports` en tres partes | `ok · sin errores`; `ore view`, `report`, `lint` y `datasets` **idénticos byte a byte**; índice de assets: `bq` 6 → 3 items, «sale de» apunta a `table:bigquery_…ventas.pedidos` |
| `standard_test` + `foreign_test` (misma fuente) comparten 19 Tables | `ok · sin errores`; `lint`, `report` y `datasets` idénticos; en `view` cambia **solo** la línea `caras` de las 19 vistas foráneas: ahora ven la clave del objeto. `standard_test` 38 → 19 items, `foreign_test` 39 → 20 |
| vistas SQL v1alpha14 que leen `fuente.schema.objeto`, con join a la propia database | resuelven: raíz, caras y esquema correctos |

**Leído** (compilador, inductor, ore-serve, consola). El compilador carga el árbol entero y resuelve
en plano: la referencia entre paquetes no necesita `dependencies`; raíz, linaje, etiquetas,
`OOS2035`, `servir` y la copia (`48-la-copia.yaml` corre desde la raíz) funcionan; `exports` vale en
un `package.yaml` v1alpha1. **Ningún bloqueo en la spec ni en ore-core.** Lo que sí hay:

1. **El puntero depende de quién lo lee.** `tabla_yaml` escribe `mode: upsert, key` si la tabla se
   copia y `mode: none` si no: el mismo `products` es `upsert · key: [id]` en `standard_test` y
   `none` en `foreign_test`. Y `clave/<obj>` y `tipo/<obj>.<col>` se contestan por database. Dos
   databases pueden escribir dos punteros distintos del mismo objeto, y «fusionar si son iguales»
   no fusionaría casi nada.
2. **ore-serve lee la carpeta `tables/` hermana**: `copias` (la clave, `copia.rs:440`),
   `tablas_del_paquete` (con ella el esquema, `GET /paquetes` y la ficha de la conexión: si la
   fuente gana `tables/` deja de leer su catálogo; si la database la pierde, lee el suyo entero),
   `objetos_fisicos`, y la guarda de `retirar_fuente` (dejaría borrar la fuente con sus Tables).
3. **El inductor no escribe fuera de su paquete**: `escribir_paquete` sobrescribe todo, la ruta de
   schema se rompe con `../`, y nadie escribe `exports`. **Y no debe crear `packages/<fuente>/`**:
   que exista es «catalogada» para ore-serve, y el Job de catálogo se la salta.
4. **Tres comandos miran un paquete solo**: `ore drift-detect --path <db>` (0 Tables: todo sería
   deriva), la compuerta de `materialize` (atribuye los diagnósticos de la Table a la fuente, que no
   tiene copias) y `ore pack` (el `.oob` de una database deja colgando su `from.table`).
5. **Las propuestas**: una Table movida es `borrado` + `nuevo`; `faltan` encuentra la mitad nueva
   (`OOS2018`) pero **no** la borrada.
6. **La consola no confunde la fuente con una database** (filtra por `scoped`, que es tener
   `discover.scope.json`); lo que cambia es el esquema de la conexión (punto 2).

## La medida del puntero (2026-09-27, tras P1)

Se midió después de empujar P1, y no antes, que es el orden equivocado: todo lo de abajo —los
Datasets de una standard y las Views de una foreign— cuelga del puntero, y la idea y su
implementación tenían que medirse antes de construir encima.

**Como producto, la idea es el estado del arte.** Que el puntero sea del origen, se registre una
vez y lo nombren sus consumidores es lo que hacen dbt `sources` (el análogo más cercano:
declarado una vez, `source()` desde cualquier modelo, nodo aparte en el DAG), el auto-registro de
Foundry (un proyecto gestionado, espejo de la fuente, que nadie edita), los *foreign catalogs* de
Unity Catalog y las *catalog-linked databases* de Snowflake. Foundry y Databricks enseñan además
el síntoma de no hacerlo: dos syncs de la misma tabla no comparten nada. **Lo que el mercado
divide** es dónde van la clave y la semántica de cambios: los ingestores (syncs de Foundry,
Lakeflow Connect) los ponen en cada copia, porque dos copias pueden querer mantenerse distinto
(SCD1 frente a SCD2). La lectura buena: **el puntero lleva la verdad del objeto** —su esquema, su
clave, lo que emite el origen— y **la copia decide cómo se mantiene**.

**Como implementación, P1 no lo era.** Su regla —`upsert` + `key` si alguna base de la fuente
copia el objeto— escribía en el objeto lo que la copia necesitaba, y era una herencia: el inductor
ya reescribía a `upsert` cualquier tabla copiada con clave. Medido en el código:

- **La copia no necesita `upsert`**: `materializar` funde por `changes.key` con cualquier modo
  (`registro::clave_de`), que es lo que dice la spec —la clave «es legal siempre», y la leen el
  mantenedor, la copia y la escritura (v1alpha8 `01-table` §6)—.
- **Escondía una pérdida**: el driver de Postgres dice `{ append, log }` de una tabla sin clave
  primaria con WAL lógico **a propósito**, para que salte `OOS2021` (una entidad copiada de un
  origen que solo anexa se queda con lo borrado dentro). Contestar una clave la reescribía a
  `upsert`, la regla callaba y la copia mentía.
- **Informaba mal al motor**: un origen mudo (`none`) pasaba a «emite upserts».
- **El escaneo de alcances era el mismo defecto un nivel más arriba**: el puntero seguía
  dependiendo de quién lo lee, y quedaba viejo hasta que las demás bases se re-indujeran.
- **Un bug**: con respuestas chocadas, `review` abortaba antes de leer las nuevas, y el mensaje
  decía «contéstalo otra vez», que era imposible.

## La iteración (revisada)

| paso | qué | criterio de hecho | despliega |
|---|---|---|---|
| **P1 · el puntero es del objeto** ✅ `af5a474` | `clave/*` y `tipo/*` una vez por fuente (`packages/<fuente>/discover.answers.json`; el paquete de la fuente nunca se crea desde una base). Su regla de la cara `D` la sustituye P1′ | — | sí |
| **P1′ · la verdad del origen** ✅ | la cara `D` es **la del driver, tal cual, más `key` si se conoce**: sin `upsert` inventado y sin escaneo de alcances (fuera `Regla::copiadas_en_la_fuente`); `registro::restricciones` toma la clave como identidad con cualquier modo; lo que el origen no deja mantener **no se copia y se dice** (`Induccion::sin_copia`: `OOS2021` para una entidad sobre `append`, `OOS2023` para `append` + `field`), en el informe de `discover` y en el de `review`; en un choque de respuestas **manda la fuente**, con aviso | `el_puntero_es_del_objeto.rs` (4 casos; el de `OOS2021`: la tabla gana la clave, sigue diciendo `append`, la entidad no se copia y el árbol no da `OOS2021`); `la-copia-se-decide.sh` 0–10 en local | sí |
| **P1.5 · la fuente se llama como la conexión** ✅ `6a9223c` · consola `1171684` | el nombre del paquete de la fuente —y de su datasource— sale del nombre que el usuario da en el **paso 2 del wizard**, no de `<tipo>_<fecha>`: `bigquery_20260927_1428.ventas.pedidos` quedaría escrito en cada `from` y cada SQL, y renombrar después es reescribirlos todos | una conexión nueva nace con su nombre; las guardas de 422/409 probadas en ore-serve; las de hoy se renombran en P4 | sí, antes de P3′ |
| **P2 · leer en los dos sitios** ✅ | ore-serve resuelve la Table por el árbol, no por la carpeta hermana (`punteros.rs`): `copias` (la clave, de la Table que nombra `from.table`), `tablas_del_paquete` —**fuente**: siempre su catálogo entero, y en cada objeto su puntero (`table`) y quién lo usa (`usedBy`), las dos detrás de `object` y solo si dicen algo; **database**: sus Tables y las que leen sus Datasets y Views, estén donde estén—, `GET /paquetes` y el esquema con un solo `cargar_paquete`, `objetos_fisicos` (también los Datasets), `retirar_fuente` (`bases_que_salen_de`: por alcance, por `Table` con `datasource` y por lo que lee una Table de la fuente) y la guarda de borrar una Table (Datasets, y el nombre cualificado). Una fuente es el paquete sin alcance cuyo catálogo dice `source` = su nombre | 5 tests nuevos sobre dos árboles —uno por disposición— que compilan sin un diagnóstico; en la copia de `victor`, el árbol de hoy sale **idéntico** a antes (salvo `table`/`usedBy` en la fuente) y el movido **igual** al de hoy (esquema, 19 copias con clave, recuentos); `la-copia-se-decide.sh`, `servidor.sh`, `servidor-forja.sh` y `los-documentos.sh` verdes **sin cambios** | sí, antes de P3′ |
| **P3′ · un solo escritor de la fuente** ✅ | `ore source induce <fuente>` (`fuente_inducida.rs`): escribe en `packages/<fuente>/` la Table de cada objeto que **alguna** base usa —la unión de los alcances—, con el catálogo y las respuestas **de la fuente**, con el nombre del objeto (sin `_t`; `_2` solo si dos objetos dan el mismo identificador en un schema, calculado sobre el catálogo entero para que no dependa de quién lo lea), su `schema.yaml`, sus `exports` en la forma corta, y retira lo marcado que ya no usa nadie. Lo corren `discover`, `review`, `model` y `copy` al terminar una base con alcance (en proceso: es una función del árbol, dé igual el orden), y ore-serve al retirar una base (`mando`: `source induce`). La base, si su fuente tiene paquete, no escribe ninguna Table (`Regla::fuente_aparte`) y nombra `<fuente>.<schema>.<objeto>`; sin paquete de la fuente —el CLI suelto, una prueba— todo sigue en la base. **No** lo lanza el Job de catálogo: cataloga una vez y en ese momento ninguna base usa nada | `el_puntero_es_del_objeto.rs` (5: una vez y en la fuente, retirar lo que nadie usa, y los tres de P1′); una standard de `bq` sobre la copia de `victor`: **3 datasets y 0 tables**, compila; `la-copia-se-decide.sh` 0–10 reescrita (el puntero en `pg/olist/tables/`, `pg.olist.*` exportados, retirar una base deja lo que otra lee) | con P4 |
| **P4 · migrar los árboles** ✅ (código) | `ore migrate punteros` (`migrar_punteros.rs`): por fuente con paquete, `clave/*` y `tipo/*` de cada base a la fuente (manda la fuente, y se dice); fuera las Tables de las bases con esa `datasource`; quien las leía —`from` y la consulta de una View SQL (`servir::nombrados`)— **reapuntado**, sin re-inducir; y la fuente las escribe. Se ensaya en una copia y **no escribe nada** si aparece un diagnóstico, si una Table llevaba una etiqueta que la de la fuente no lleva, o si un objeto no está en el catálogo de la fuente. **No se renombran las fuentes**: las de hoy se llaman como su conexión —el `<tipo>_<fecha>` que el paso 2 rellenaba y nadie cambió—, no hay otro nombre que darles, y renombrar es mover el secreto del cofre | `el_puntero_pasa_a_la_fuente.rs` (2); sobre los árboles vivos (§ «P4 · lo medido»): mismos diagnósticos, `lint`, `report` y `datasets` **idénticos**; `bq` 3 datasets, `standard_test` 19, ninguna base con Tables | P3′ + P4 juntos, árbol a árbol, **tras** desplegar el binario |
| **P5 · los que miran un paquete** | `drift-detect` sobre el paquete de la fuente contra su propio catálogo (ya no hace falta la unión de alcances); la compuerta de `materialize` atribuye a cada database lo que lee; `ore pack` lo dice (la dependencia versionada, fuera de este ADR) | tests de ore-cli | sí |
| **P6 · consola** | la conexión lista sus Tables («usada por …»); «Sale de» del Dataset enlaza a la Table de la fuente; el modal de database sigue ofreciendo todo el catálogo | la `bq` de `victor` enseña tres; la ficha de la conexión, las tres Tables | sí |

### P4 · lo medido (2026-09-27, árboles vivos)

`ore migrate punteros` sobre copias de los tres árboles del cluster, bajados de la forja:

| árbol | fuentes con bases | punteros fuera de las bases | en la fuente | reapuntados | diagnósticos | `lint` · `report` · `datasets` |
|---|---|---|---|---|---|---|
| `victor` (`43dd261`) | 2 | 41 (19 + 19 + 3) | 22 | 41 | 0 → 0 | idénticos |
| `demo` (`48fe3b2`) | 2 | 11 | 11 | 11 | 0 → 0 | idénticos |
| `prueba` (`7cabca6`) | 0 | — | — | — | — | sin paquetes: nada que migrar |

Queda: `victor` · `bq` 3 datasets, `standard_test` 19 datasets, `foreign_test` 19 vistas, las Tables en
`bigquery_20260927_1428` (3) y `postgresql_20260921_2055` (19); `demo` · `olist_copia` 3 datasets y 1 vista, `olist`
8 vistas, las Tables en sus dos fuentes. Lo que cambia sin romper, y lo dice la migración:

- **`victor`, la cara `D` (P1′)**: las 19 foráneas pasan a ver la clave del objeto, y las 19 copias de
  `standard_test` dejan el `upsert` que el origen no emitía; su plan pasa de `REFRESH_MODE = INCREMENTAL` a
  `FULL` («declara no emitir cambios»). Es la verdad del origen, la que P1′ fijó.
- **`demo`, los tipos**: sus Tables son de antes de 0032 §3 y no llevaban el tipo del conector; re-inducidas lo
  llevan. **32 columnas de 9 punteros** dejan de servirse como `String` (`customer_zip_code_prefix: Integer`,
  `freight_value: Float`, `shipping_limit_date: DateTime`…): cambia el esquema de sus vistas y el digest del plan de
  las 3 copias de `olist_copia`, que se rehacen enteras una vez.

### P2 · lo medido

Levantando ore-serve sobre la copia de `victor` con los punteros movidos a mano (el árbol de B0):
el esquema de una database caía al catálogo entero (todo `copied: false`, sin `dataset` ni `view`),
el de la fuente pasaba a leer sus Tables —y dejaba de ofrecer los objetos que nadie usa—, y la clave
de las copias de `standard_test` desaparecía (19 → 0). Y uno grave, leído: `retirar_fuente` no veía
ninguna database, retiraba la fuente con sus Tables, y el filtro de diagnósticos —que ignora los
que mencionan la fuente— dejaba subir el árbol roto.

### P1.5 · lo medido

- **El nombre ya lo pide el paso 2** (`NameStep.tsx`, «Nombre»), pero llega **relleno** con
  `<tipo>_<aaaammdd>_<hhmm>` (`SourceSetupWizard.tsx:59-64`, `nombrePorDefecto`): casi nadie lo
  cambia. No hay nombre humano aparte; lo único humano es `description` del manifiesto («BigQuery ·
  dado de alta desde la consola»), que la consola pide y **no enseña**: en todas partes se ve el id.
- **El nombre es muchas cosas a la vez**: el datasource del manifiesto, `packages/<n>/`, el secreto
  `fuente-<n>` del cofre, la variable `<ORG>_<N>_URL`, el fichero de la cola
  `44-el-catalogo-<obj>.yaml`, `GET /fuentes/{n}/…` y —tras P3′— cada `from` y cada SQL.
- **La regla de hoy no es la intersección de las suyas.** La consola y `ore source add` admiten
  `^[A-Za-z][A-Za-z0-9_]{0,127}$`; el cofre solo admite `fuente-<n>` en minúsculas y hasta 63
  (`n ≤ 56`), y la cola corta a 30 y pasa a minúsculas (`cola::nombre_de_objeto`). Así que hoy
  **ya se rompen en silencio**: `Ventas` falla al guardar la credencial —502 con el árbol ya
  escrito—, y `Ventas`/`ventas`, `a_b`/`a__b` o dos nombres largos con el mismo prefijo comparten
  fichero de cola: el segundo catálogo pisa al primero.
- **Colisiones que nadie mira**: un `packages/<n>/` que ya existe (de una database con ese nombre:
  la fuente nace «catalogada» y el Job se la salta para siempre) y un secreto `fuente-<n>` que
  sobrevivió a una baja.
- Y un error mal dicho: cualquier fallo de `ore source add`, un nombre inválido incluido, sale
  **409** (`rutas.rs:1283`); es un 422.

### P1.5 · lo que se hace

| pieza | qué |
|---|---|
| **la regla** | `^[a-z][a-z0-9]*(_[a-z0-9]+)*$`, **≤ 30**: la intersección de cofre, cola y namespace (`OOS2030`). Minúsculas y sin `__` ni `_` en los bordes hacen `nombre_de_objeto` biyectivo; 30 es su corte |
| **ore-serve** (la guarda de verdad) | `nombre_de_fuente` en `alta_de_fuente`, antes del CLI: **422** con la regla y el identificador sugerido si no cumple; **409** si ya es un datasource, si `packages/<n>/` existe, si su fichero de cola es de otra fuente o si `fuente-<n>` ya está en el cofre, sugiriendo `<n>_2`. Y el fallo de `ore source add` por nombre, 422 |
| **consola** | el paso 2 pide **«Nombre de la conexión»**, texto libre y vacío (fuera `nombrePorDefecto`), y enseña debajo, en vivo, el identificador que sale de él (sin acentos, minúsculas, lo demás `_`, ≤ 30; si empieza por dígito, con el tipo delante: `pg_2024_ventas`), editable; avisa de colisión antes de enviar con los nombres que ya hay. Manda `description` = el nombre humano, y las listas lo enseñan con el identificador debajo |
| **orden** | ore-serve primero (una consola vieja manda nombres que la guarda nueva acepta o rechaza con 422); la consola después. Las fuentes de hoy se renombran en P4 |

⚠️ **`la-copia-se-decide.sh` cambió de origen, no de expectativa.** Su `customers` era
`{ append, log }` y el guion construía diez pasos sobre su copia de entidad: con P1′ esa copia no
se mantiene (`OOS2021`), que es justo lo que P1′ corrige. Pasa a `{ none, none }` —el otro
Postgres sin clave real, sin WAL lógico, que se recomputa entero— y el caso `append` lo fija
`el_puntero_es_del_objeto.rs`.

> **Inciso · la entidad sobre un origen que solo anexa.** Desde P1′ su copia no se emite: lo
> borrado en el origen no llegaría (`OOS2021`), y el informe lo dice. Si se quiere mantenerla
> igual —recomputándola entera en cada refresco—, eso es de la copia, no del puntero: una perilla
> del Dataset (`refresh: recompute | merge`), con `OOS2021`/`OOS2023` solo para `merge`. Es spec
> (`C:\oos`), fuera de este ADR.

Fuera: `ore pack` con la fuente como dependencia versionada. Colisiones al quitar `_t` (`a-b`/`a_b`,
`Pedidos`/`pedidos` en Windows): el sufijo de siempre **solo** cuando colisionan, y se dice.

⚠️ **Coordinación**: `_t`, `migrar_v14` y `la-copia-se-decide.sh` son del trabajo de 0040 (sesión
«SQL índice y paradigma»). P3′ y P4 se hablan con ella antes de escribir.

⚠️ **P4 y las propuestas por activos** (`356618c`, `POST /propuestas {activos}`): una propuesta lleva
activos por `doc_id` (`Kind:qname`), y mover una Table de paquete **cambia su id**: en una rama que
migre sale como borrada en la database y nueva en la fuente, no como movida. La propuesta de esa
migración tiene que llevar las dos mitades juntas —y los Datasets reapuntados—; `faltan` pide la
nueva por `OOS2018` pero **no** la borrada: se listan las dos a mano. Criterio añadido a P4: la migración de un árbol es **una** propuesta que compila sola.
