# 0045 · El puntero es de la fuente: una database standard es sus datasets

**Estado:** propuesto (2026-09-27), pendiente de la medida B0 y del visto bueno · **Decide:** dónde
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

## La iteración

| paso | qué | criterio de hecho |
|---|---|---|
| **B0** | **medir** lo que A no midió: que el compilador resuelve `from.table` a otro paquete del mismo árbol con `exports` (`OOS2028`) y sin `dependencies` versionadas; qué rutas leen la carpeta `tables/` **hermana** (`copia.rs:440`, `tablas_del_paquete`, `objetos_fisicos`, `retirar_fuente`, `deriva.rs`, `registro::restricciones`); qué pasa con la clave cuando dos databases la contestan distinto | un informe con cada sitio y su arreglo; si algo obliga a tocar la spec, vuelve aquí |
| **B1** | el inductor escribe las Tables en el paquete de la fuente (crea las que falten, no reescribe las que estén) y las exporta; la database solo sus Datasets o Views | tests del inductor; `standard` da N datasets y 0 tables en la database |
| **B2** | ore-serve lee las Tables de la fuente: esquema, copias (clave), objetos físicos, retirar fuente | `la-copia-se-decide.sh` reescrita (14 aserciones) en verde |
| **B3** | `ore migrate`: mueve las Tables de cada database a su fuente, fusiona duplicados idénticos, reapunta los `from` y el SQL; si dos databases discrepan en un objeto, **no migra** y lo dice | `cotejo`: mismos diagnósticos antes y después, en los árboles de `demo`, `prueba` y `victor` |
| **B4** | consola: la database lista sus datasets; la conexión, sus tablas; «Sale de» del dataset enlaza a la tabla de la fuente | la `bq` de `victor` enseña tres |

Orden: B0 primero y sola; B1–B2 juntas (una sin la otra rompe la copia); B3 antes de desplegar
(los árboles vivos tienen el par); B4 al final.

⚠️ **Coordinación**: `_t`, `migrar_v14` y `la-copia-se-decide.sh` son del trabajo de 0040 (sesión
«SQL índice y paradigma»). B1 y B3 se hablan con ella antes de escribir.

⚠️ **B3 y las propuestas por activos** (`356618c`, `POST /propuestas {activos}`): una propuesta lleva
activos por `doc_id` (`Kind:qname`), y mover una Table de paquete **cambia su id**: en una rama que
migre sale como borrada en la database y nueva en la fuente, no como movida. La propuesta de esa
migración tiene que llevar las dos mitades juntas —y los Datasets reapuntados—; `faltan` las pide por
`OOS2018` si falta una. Criterio añadido a B3: la migración de un árbol es **una** propuesta que
compila sola.
