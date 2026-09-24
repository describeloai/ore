# 0038 · Los tres niveles: `base.schema.nombre`, como en Unity Catalog

**Estado:** decidido (P0 hecho: la gramática, OOS v1alpha13; P1–P7 pendientes) · **Fecha:** 2026-09-24 ·
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
| **P1** | La identidad en el núcleo: `qname()`, `qualify()`, `metadata_keys()`, `pertenencia` (`OOS2036`/`2037`), `sin_propiedad()`, el índice; la conformidad de v1alpha13, medida. Sin schema = `default`: los árboles de hoy compilan sin tocarlos | medio |
| **P2** | Los punteros `datasets/<base>/<schema>/<nombre>.json` y su migración | medio |
| **P3** | SQL de tres partes: analizador, `sql()`, `write()`, los tres SDK (ATTACH + alias), el LSP, `ore datasets`; las dos partes con aviso | grande |
| **P4** | `/v1` como Unity (base = prefix, schema = namespace), crear y listar schemas | medio |
| **P5** | `discover` con el schema del origen | pequeño |
| **P6** | La consola: schemas de verdad (crear/renombrar al servidor), refs de tres partes, borradores en la carpeta del schema | medio |
| **P7** | Las semillas (SQL, Python, Java) escribiendo en la base y el schema que el asistente pregunte | pequeño |

Cada paso termina en verde de punta a punta antes del siguiente.

## Lo que no cambia

El paquete sigue siendo la base (nada que migrar), el lago físico igual, y el proyecto sigue
siendo una lente sobre el catálogo (0035).
