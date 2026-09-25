# 0038 · Los tres niveles: `base.schema.nombre`, como en Unity Catalog

**Estado:** decidido (P0, P1 y P2 hechos; P3–P7 pendientes) · **Fecha:** 2026-09-24 ·
**Decide:** cómo se nombra lo que un inquilino tiene en el catálogo —en los documentos, en SQL,
por `/v1` y en la consola—, ahora que el **schema** es parte del nombre. Sigue a
[`0033`](0033-el-dataset.md) (el dataset), [`0034`](0034-el-catalogo-de-assets.md) ④ (el
schema como carpeta, que esto corrige) y a la gramática,
[OOS v1alpha13](../../vendor/oos/spec/v1alpha13/00-scope.md).

## El problema

Los datos del cliente viven en el **catálogo de assets**, que se ordena como cualquier catálogo:
**base de datos → schema → tabla / dataset / vista**. La consola ya lo pinta así (0034 ④). El
lenguaje no: un nombre era `paquete.nombre`, y el schema, la carpeta entre el paquete y el
fichero, **ordenaba sin nombrar**. Medido (`pruebas-de-fuego/medida-los-tres-niveles.py`,
2e0a384 · 7e6401d · ce736d7):

- Dos `pedidos` en `ventas/espana/` y `ventas/francia/` son **`OOS2035`**: el nombre es único por
  paquete, no por schema.
- **Los árboles no tienen schemas**: el 100% de los assets (76 en el de victor, 5000 en el
  sintético) cae en `""`. Y no por falta de uso: **«Create schema» de la consola no llama al
  servidor** (sólo estado de React; se pierde al recargar) y **una carpeta vacía no existe en
  git**.
- **El origen sí los tiene, y se aplanan**: `discover` escribe `public.ai_insights` como la
  tabla `public_ai_insights` (38 de 38 en `foreign_test`; `identificador()`, inductor.rs).
- **Los punteros ya chocan**: `a_b.c` y `a.b_c` son el mismo `datasets/a_b_c.json`; borrar el
  paquete `ventas` se lleva los punteros de `ventas_eu` (copia.rs).
- El nombre de dos partes está en **~90 sitios de 11 piezas** (analizador, `sql()`, `/v1`,
  `write()` en los tres SDK, LSP, índice, ore-view, consola).

## Lo decidido (2026-09-24)

1. **Tres niveles, como Unity Catalog.** `<base>.<schema>.<nombre>`. La base es **el paquete**
   (su `namespace`: letras, dígitos y `_`); el schema, un segundo nivel del nombre.
2. **Único por schema**, porque el schema es parte del nombre. Comprobado en Unity: la API
   identifica una tabla por `catalog_name.schema_name.table_name`.
3. **`default`** es el schema de lo que no dice otro (Unity crea `default` en cada catálogo).
4. **Dos partes se admiten por ahora** y se resuelven a `base.default.nombre`, pero **se tratan**:
   no pasan en silencio (§ «Las dos partes»).
5. **`discover` lleva el schema del origen al del catálogo**: `foreign_test.public.ai_insights`.
6. **Repositorios y schemas no se mezclan**: un repositorio (README con `plantilla`) vive en el
   paquete de su proyecto; lo que escribe va a un schema de una base del catálogo. Un paquete de
   proyecto (`test-project`) no es una base: su nombre ni siquiera es un `namespace`.

## La gramática (P0, hecho): OOS v1alpha13

La especificación decide lo que no podía decidir ORE: **la identidad nunca es la ruta**
(90-canonical-form §5.2). Así que el schema **se declara** y la carpeta se ata a él, como
`OOS2030` ata el `namespace` al paquete:

- `kind: Schema` (`packages/<base>/<schema>/schema.yaml`): hace existir el schema —una carpeta
  vacía no existe— y lleva su dueño. Ni `default` ni `information_schema`.
- `metadata.schema` en el contenido gobernado (Entity, View, Table, Dataset, Function, Action,
  TrainedModel); sin ella, `default`. El vocabulario compartido no la lleva.
- `OOS2036` (la carpeta no es la del schema) y `OOS2037` (el schema no está declarado).
- Referencias: **una parte** = mismo paquete y schema; **dos** = `base.nombre` → `base.default`;
  **tres** = completo. A una propiedad: la propiedad es el último segmento, siempre.
- Un documento anterior no cambia de forma canónica ni de digest: está en `default`.

## Lo que ORE hace con ello

**Los motores, como Unity.**
- **`/v1` (Iceberg REST): la base es el `prefix`** (el `warehouse` del cliente) **y el schema, el
  namespace de un nivel**. Es lo que hace el Iceberg REST de Unity, y Spark nombra entonces
  `ventas.espana.pedidos`, el mismo nombre que el SQL. Los namespaces de dos niveles (`0x1F`) no
  hacen falta —y hoy ni llegarían: ore-entrada tira la query y no decodifica `%1F`—.
- **DuckDB: un catálogo por base** (`ATTACH ':memory:' AS <base>`, schemas dentro): 0.33 ms por
  vista con 5000. `default` se escribe sin comillas. Pero **dos partes van a `main`**, no a
  `default`: mientras se admitan, cada vista de `default` lleva su alias en `main`.

**Los punteros**: `datasets/<base>/<schema>/<nombre>.json`, un separador que ningún nombre lleva
—arregla el choque de hoy—. El lago no se mueve: el puntero guarda `metadata_location`.

**Las dos partes, tratadas.** Donde aparezcan —un `.sql`, una celda, un YAML— resuelven a
`base.default.nombre` **y lo dicen**: un aviso con código propio en `ore validate`, en el editor
(el LSP ofrece el nombre de tres partes) y en la salida de la celda. Más adelante, error.

## El plan

| # | Paso | Tamaño |
|---|---|---|
| **P0** | La gramática: OOS v1alpha13 (spec, esquemas, errores) y esta ADR | hecho |
| **P1** | La identidad en el núcleo: `qname()`, `qualify()`, `metadata_keys()`, `pertenencia` (`OOS2036`/`2037`), `sin_propiedad()`, el índice; la conformidad de v1alpha13, medida. Sin schema = `default`: los árboles de hoy compilan sin tocarlos | hecho |
| **P2** | Los punteros `datasets/<base>/<schema>/<nombre>.json` y su migración | hecho |
| **P3** | SQL de tres partes (medido y partido, § P3): P3a el analizador (hecho), P3b ore-serve (hecho), P3c los tres SDK (ATTACH + alias), P3d el LSP, P3e `ore datasets` y `ore ask`; las dos partes con aviso `ORE-SQL-2P` | grande |
| **P4** | `/v1` como Unity (base = prefix, schema = namespace), crear y listar schemas | medio |
| **P5** | `discover` con el schema del origen | pequeño |
| **P6** | La consola: schemas de verdad (crear/renombrar al servidor), refs de tres partes, borradores en la carpeta del schema | medio |
| **P7** | Las semillas (SQL, Python, Java) escribiendo en la base y el schema que el asistente pregunte | pequeño |

Cada paso termina en verde de punta a punta antes del siguiente.

## P1, hecho: la identidad en el núcleo

**La clave del motor es la forma CORTA** (`normalize::corto`): `p.n` en `default`, `p.s.n` en
otro schema. Es biyectiva con la completa —lo de `default` tiene dos partes y lo demás tres—, así
que dos documentos nunca comparten clave, y es la que todo el que ya habla con el motor escribe:
`qname()`, las búsquedas, `OOS2035`, el índice y los consumidores de fuera (ore-serve, los SDK, la
consola) ven lo mismo que antes para todo lo de hoy. La **completa** (`completo`, tres partes con
`default`) es la de la forma canónica de un documento de v1alpha13 y su `docId`.

- `Loaded::schema()` (lo declarado o `default`, en los siete kinds del catálogo) y `qname()` corto;
  las búsquedas (`entity/view/table/dataset`) aceptan las dos formas (`a_corto`).
- `normalize::qualify_catalogo` / `link::cualificar`: una parte = su paquete **y su schema**; dos =
  `default`; tres = completa. `qualify` sigue siendo la del vocabulario compartido. Cambiadas donde
  lo nombrado es del catálogo: enlazado, vistas, `fuente`, `actuar` (`call`), `cedar_schema`,
  `governance` (la función de un deber), `exporta` (según el kind de destino) y el índice
  (`ref_doc`, `ref_qn` con el schema de quien enlaza).
- `sin_propiedad`: la propiedad es **siempre** el último segmento (con tres niveles, adivinar por
  el número de puntos cortaba `hr.rrhh.Employee` a `hr.rrhh`; y `Employee.salary` no se cortaba).
- `ApiVersion::V1Alpha13`, `Kind::Schema` (en `DEL_PAQUETE`: su namespace es el paquete),
  `metadata.schema` desde v1alpha13 (antes, o en el vocabulario, `OOS1005` con su porqué) y
  `schema.rs` (`OOS2036`, `OOS2037`, `default`/`information_schema`, el `owner`), tras la
  pertenencia y antes del enlazado.
- Forma canónica de v1alpha13: N1 a tres partes (y `from.dataset`, que v1alpha12 no expandía),
  N2 escribe `schema: default`, `docId` de tres partes. Lo anterior, igual.
- El índice lleva `schema` en cada ítem (`null` en el vocabulario).

**Medido**: conformidad v1alpha13 **15/15** (OOS `d0f2628`, los `expects` medidos antes de
escribirse) y las demás versiones enteras; sobre el árbol de victor el binario de antes y el de
después dan el mismo `validate`, los mismos 58 ítems con las mismas refs y **el mismo digest de
bundle**; el índice sólo gana el campo `schema`.

## P2, hecho: los punteros

Medido antes (`pruebas-de-fuego/medida-los-punteros.sh`), y con tres hallazgos que no eran de
nombres sino de **borrar lo que no se debía**:

- **P2a** · El Job de la copia (`ore materialize --recoger`) le pasaba a `recoger-huerfanas`
  sólo los datasets de las vistas mantenidas: **un dataset escrito se borraba entero del
  bucket** (M1: 4 → 0 objetos) y su puntero quedaba apuntando a nada; `resultados/`, igual,
  también en la pasada diaria. Ahora se reclama **todo puntero del árbol**
  (`datasets::reclamados`), y el de una vista retirada se quita antes. el-lago 10b.
- **P2b** · `recoger-huerfanas` cortaba el nombre a dos segmentos (un dataset anidado era
  huérfano aunque se reclamara, M3) y el mantenimiento de una tabla borraba lo que hubiera bajo
  su ubicación: `datasets/ventas_x` es prefijo de `datasets/ventas_x/default/n`. Ahora se
  reclama por prefijo hasta una `/`, y una tabla sólo toca sus `metadata/` y `data/`.
- **P2c** · Los punteros, en su sitio. `ore_core::punteros` decide dónde vive el de cada
  nombre (`datasets/<base>/<schema>/<n>.json`), lee el de antes (`<p>_<n>.json`) mientras
  quede, dice qué nombre es cada fichero y los lista a cualquier profundidad; todo el que leía
  o escribía uno a mano pasa por ahí (`ore datasets`, `ore materialize`, el índice, ore-serve:
  copias, informes, retirar un Dataset o un paquete, preguntar, funciones, puestos).
  **Quien escribe un puntero lo deja en su sitio y retira el de antes**, y `ore migrate` los
  mueve todos (y los de `copias/`) escribiendo el `dataset` que el fichero ya no dice. Borrar
  un paquete retira los punteros **de su base** —el nombre que dice cada uno, no un prefijo del
  fichero (M4)—.

**El nombre en el lago** de un dataset que nace es `catalogo/<base>/<schema>/<n>`, no
`datasets/<base>/…`: `datasets/ventas_x` (de antes) sería prefijo de `datasets/ventas_x/default/n`,
y la credencial prestada al primero —acotada a su prefijo— alcanzaría el segundo. Lo que ya
existe no se mueve. Y **el `dataset` de un puntero sale de su `metadata_location`** siempre que
se sabe: es donde están los bytes, lo que tiene que reclamar, aunque lo haya elegido otro
escritor (el swap de `confirmar`).

Lo que queda de nombres de dos partes (`/v1`, los SDK, `ore datasets --tabla p.t`) es de P3/P4.

## P3: medido, y el analizador hecho

**Medido** (`pruebas-de-fuego/medida-el-sql-de-tres-partes.sh`, un puesto de Python de verdad
sobre un árbol con `ventas.espana` declarado): con dos partes, todo; con tres, **nada llega**, y
cada pieza falla por su cuenta y con su frase —`write()` y `over()` las rechazan en el SDK,
`sql()` y la celda las dejan pasar crudas a DuckDB (`Catalog "ventas" does not exist`: el
tokenizador descartaba `a.b.c` en silencio), la celda que escribe no se reconoce como tal, el LSP
ofrece `ventas.clientes` para lo que está en `espana` (un nombre que no existe) y
`/v1/namespaces/espana/tables` da 200 vacío—. El núcleo, en cambio, ya estaba: `validate`, el
índice (`schema` de cada ítem) y los punteros.

La partición, por dependencias: **P3a** el analizador → **P3b** ore-serve (la celda y el
aviso en su salida, `datos_del_puesto`, `declarar_transform`, `datos_de`, el `Dataset` que
nace en un schema en v1alpha13 y en su carpeta) → **P4** `/v1` (base = prefix, schema =
namespace; `write()` escribe por ahí, así que va antes que los SDK) → **P3c** los tres SDK →
**P3d** el LSP → **P3e** `ore datasets --tabla` (y el `identificador()` que se queda con el
último nivel del namespace) y `ore ask --sql`.

**P3a, hecho.** `sql_del_arbol::Nombre` lleva `schema` y `referencia()` es la forma corta (lo
de `default` sigue siendo `p.n` para todos los que ya lo consumen); `completo()`, las tres.
`analizar` acepta `base.schema.nombre`; `base.nombre` es `default` **con un aviso**
`ORE-SQL-2P` por nombre (`Unidad::avisos`: no para la frase; `Fallo::codigo`); una parte o
cuatro, fallo. `cotejar` dice el schema que no está declarado. `nombres_a_resolver` y
`escribe_en_el_arbol` reconocen `a.b.c` (en su forma corta) en vez de descartarlo. `ore sql`
enseña los avisos (`aviso[ORE-SQL-2P]`, y `avisos` en `--json`).

**P3b, hecho.** ore-serve, el lado del puesto: `datos_del_puesto` acepta dos o tres partes (a
su forma corta: la clave del árbol, la de los punteros y la del transform); `declarar_transform`
también; `datos_de` busca en el árbol compilado por la forma corta —antes, por carpetas
(`packages/<p>/datasets|views|tables`), que no ven el schema—, y `datos_de_vista` parte los
datasets de la pregunta con su schema. **El aviso llega a la celda**: `avisos_de_celda` (el
tokenizador: lo que se lee y el destino, en su sitio, una vez) da los `ORE-SQL-2P` de una celda
`sql`, lea o escriba, y los de un `.sql` como trabajo salen de `Unidad::avisos`; van en la ficha
de la celda (`avisos`, diagnósticos con `severidad: aviso`), junto a la salida. Medido después:
el servidor ya resuelve `ventas.espana.clientes` (el 409 de «nadie lo escribió todavía»), y lo
que falla con tres partes es ya sólo del SDK (`write()`, `over()`, `transform()`, el DuckDB de
`sql()`) y de `/v1`. **`ore datasets` con tres partes pasa a P4**: trabaja con pares
`(paquete, tabla)` y carpetas, y es lo que `/v1` llama —con base = prefix y schema =
namespace—; el `Dataset` que nace en un schema (v1alpha13, en su carpeta) se escribe ahí.

## Lo que no cambia

El paquete sigue siendo la base (nada que migrar), el lago físico igual, y el proyecto sigue
siendo una lente sobre el catálogo (0035).
