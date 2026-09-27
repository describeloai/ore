# 0045 · El puntero es de la fuente: una database standard es sus datasets

**Estado:** aprobado (2026-09-27), B0 medido, P1 hecho; P2–P6 por hacer · **Decide:** dónde
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

## La iteración

El orden sale del punto 1 de B0: **primero el puntero deja de depender de quién lo lee**, en el
sitio de hoy; con eso los duplicados salen iguales y moverlos es mover, no decidir. Y del punto 2:
**los lectores aprenden a leer en los dos sitios antes** de que nada se mueva (binario antes que
malla).

| paso | qué | criterio de hecho | despliega |
|---|---|---|---|
| **P1 · el puntero es del objeto** ✅ | la cara `D` de la tabla la decide si el objeto **se copia en la fuente** —esta base u otra, leído de los `discover.scope.json`—, no si lo copia esta: con copia y clave, `upsert` + `key`; sin copia en ninguna, lo que sondeó el driver, tal cual (así ni `drift-detect` ni el motor ven una cara que el origen no dijo). `clave/*` y `tipo/*` se contestan **una vez por fuente** (`packages/<fuente>/discover.answers.json`); una base que los contestó distinto no re-induce: lo dice | `el_puntero_es_del_objeto.rs` (5 casos); en la copia de `victor`, tras `review --reinducir`, los 19 pares de `standard_test`/`foreign_test` **idénticos** (eran 0), `lint` y `report` iguales | sí, solo |
| **P2 · leer en los dos sitios** | ore-serve resuelve la Table por el árbol y no por la carpeta: `copias` (por `registro::clave_de`), `tablas_del_paquete` (fuente: siempre su catálogo, anotado con sus Tables; database: sus Datasets y Views unidos a la Table que nombran, esté donde esté), `objetos_fisicos`, `retirar_fuente` (cuenta las databases por `discover.scope.json`), la guarda de borrar una Table (Datasets y SQL) | tests de ore-serve en las dos disposiciones; `la-copia-se-decide.sh` verde **sin cambios** | sí, antes de P3 |
| **P3 · el inductor escribe en la fuente** | un canal aparte en `Induccion` para `packages/<fuente>/`: la Table (v1alpha13, schema **del origen**, namespace la fuente, nombre el del objeto, sin `_t`), su `schema.yaml` y su línea de `exports` en tres partes; crea o actualiza, **nunca crea el directorio** (sin fuente —CLI suelta—, el sitio de siempre); la database, solo Datasets o Views con `from: { table: <fuente>.<schema>.<obj> }`; `review --reinducir` retira las Tables viejas | tests del inductor; `standard` da N datasets y 0 tables; `la-copia-se-decide.sh` reescrita (14 aserciones, 219-226 y `_t`) | con P4 |
| **P4 · migrar los árboles** | `ore migrate fuentes`: por fuente, las Tables de todas sus databases por `(datasource, object)` → una en la fuente (tras P1 son iguales; si no, no migra y dice cuál), `exports`, reapunta `from` y SQL (`servir::renombrar`), borra las viejas. Reusa `paquete::planificar` y el `cotejo` de `migrar_v14` | `cotejo` en copias de `demo`, `prueba` y `victor`: mismos diagnósticos; `bq` 3 items, `standard_test` 19; cada árbol, **un** commit que compila solo | P3 + P4 juntos, árbol a árbol |
| **P5 · los que miran un paquete** | `drift-detect` sobre la fuente con la unión de los alcances de sus databases; la compuerta de `materialize` atribuye a cada database lo que lee; `ore pack` lo dice (la dependencia versionada, fuera de este ADR) | tests de ore-cli | sí |
| **P6 · consola** | la conexión lista sus Tables («usada por …»); «Sale de» del Dataset enlaza a la Table de la fuente; el modal de database sigue ofreciendo todo el catálogo | la `bq` de `victor` enseña tres; la ficha de la conexión, las tres Tables | sí |

⚠️ **Lo primero que se probó en P1 y no valía**: escribir la clave (y `upsert`) en la tabla siempre que se conociera. Rompió dos guardas con razón —`drift-detect` veía `upsert → none` como deriva, y «las caras sondeadas llegan al motor» dejaba de ser cierto—: la tabla afirmaba del origen algo que el origen no dijo. `upsert` es la cara de un objeto **que se copia**; por eso la decide la fuente y no cada base.

⚠️ **P4 mueve, no re-induce.** Medido al cerrar P1 en la copia de `victor`: `review --reinducir`
sobre `foreign_test` (inducida el 21-sep) no solo iguala sus punteros —19 de 19 idénticos a los de
`standard_test`, de 0—; también reescribe sus 19 vistas con el inductor de hoy (v1alpha14 SQL, en
el schema `public`: `foreign_test.x` pasa a `foreign_test.public.x`), y eso renombra lo que
entidades, funciones y SQL a mano nombran. La migración de los árboles vivos toca solo las Tables
y quien las nombra.

Fuera: `ore pack` con la fuente como dependencia versionada. Colisiones al quitar `_t` (`a-b`/`a_b`,
`Pedidos`/`pedidos` en Windows): el sufijo de siempre **solo** cuando colisionan, y se dice.

⚠️ **Coordinación**: `_t`, `migrar_v14` y `la-copia-se-decide.sh` son del trabajo de 0040 (sesión
«SQL índice y paradigma»). B1 y B3 se hablan con ella antes de escribir.

⚠️ **B3 y las propuestas por activos** (`356618c`, `POST /propuestas {activos}`): una propuesta lleva
activos por `doc_id` (`Kind:qname`), y mover una Table de paquete **cambia su id**: en una rama que
migre sale como borrada en la database y nueva en la fuente, no como movida. La propuesta de esa
migración tiene que llevar las dos mitades juntas —y los Datasets reapuntados—; `faltan` pide la
nueva por `OOS2018` pero **no** la borrada: se listan las dos a mano. Criterio añadido a B3: la migración de un árbol es **una** propuesta que
compila sola.
