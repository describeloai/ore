# 0039 · SQL paradigms in code repositories

**Estado:** **aceptado · en vivo** (2026-09-27) · **Decide:** qué es SQL en un code repository de
ORE: **un `.sql` es código del árbol**, que se lee sin ejecutarlo —qué lee, qué escribe y cómo— y
corre por los mismos caminos que el código de Python. Una sentencia que lee es un análisis; una
que escribe, un transform; varias, un guion. Se apoya en [`0036`](0036-code-repositories.md)
(las clases de repositorio), [`0038`](0038-assets-catalog-namespaces.md) (`base.schema.nombre`),
[`0033`](0033-el-dataset.md) (lo que se escribe es un dataset) y [`0031`](0031-el-puesto.md) (el
puesto). La vista SQL es su propio producto: [`0040`](0040-sql-views.md) (SQL Views).

## Qué es

Un repositorio de la clase **`transforms-sql`** —familia *transforms*, junto a Python y Java— es una
carpeta de ficheros `.sql`. Nace con su semilla, `transforms/ejemplo.sql`:

```sql
CREATE OR REPLACE DATASET mi_base.mi_schema.mi_resumen AS
SELECT pais, count(*) AS n
FROM mi_base.mi_schema.mi_dataset
GROUP BY pais
```

El dialecto es el de DuckDB, el motor que corre la frase, y los nombres son los del catálogo, en
tres partes (`base.schema.nombre`; dos partes es `base.default.nombre`, y se avisa:
`ORE-SQL-2P`). No hay una imagen de SQL aparte: un `.sql` corre en el entorno de Python del puesto.

## El vocabulario

En el nuestro, no en el de Databricks:

| la frase | es |
|---|---|
| `select …` | **un análisis**: lee y no escribe |
| `create or replace dataset b.s.d as select …` | **un transform** que sobrescribe |
| `insert into b.s.d [(cols)] select … \| values (…)` | un transform que anexa |
| `insert or replace into b.s.d select …` | un transform que hace upsert, por la clave del dataset |
| `create [standard \| foreign] database b [from origin o include (s.t, s.*)]` | una base, vacía o sobre un origen |
| `create schema b.s` | un schema declarado |
| `create dataset b.s.d (col tipo, …) [primary key (…)]` | un dataset vacío, con su esquema y su clave |
| `create [or replace \| materialized] view …` · `drop view …` | una vista (0040) |
| `create media collection …` | una colección de medios ([`0049`](0049-media-paradigms-in-code-repositories.md)) |

- **Lo que se escribe es un Dataset.** Una `Table` es un puntero a un objeto de un origen: nace del
  descubrimiento y no guarda bytes. `create table` se niega diciéndolo.
- **Lo que se lee** es un `Dataset` o una `View` que se pueda leer; una `Table` de un origen, no
  (se lee por la vista o el dataset que la expone).
- **Lo que no se hace en SQL** se niega con su motivo: leer por función (`read_parquet`), escribir
  donde se lee, `returning`, `on conflict`, y escribir en una copia mantenida.

## Se lee sin ejecutarlo

`ore_core::sql_del_arbol` analiza un `.sql` con `sqlparser` y lo **coteja con el árbol**: qué lee,
qué escribe, en qué modo y en qué línea. Lo que lee tiene que existir, lo que escribe tiene que ser
un dataset que admite escritura, y la base y el schema tienen que estar. Con varias sentencias,
**en orden**: lo que crea la primera existe para la segunda.

`ore sql <fichero> [--json]` lo enseña: la consulta, lo que lee, lo que escribe y su modo, los
fallos y los avisos con su posición, y cada sentencia de un guion.

## Cómo corre

- **Como trabajo** (`POST /trabajos` con el fichero): `ore-serve` lo coteja en el commit que se
  pide —un fallo es 422, con su línea—, y un `.sql` que escribe corre como **el mismo `@transform`
  que se escribiría en Python**: `write(salida, sql(consulta), modo=…)`, con sus `inputs` y su
  `output` declarados. La procedencia lleva el fichero y su commit.
- **En la sesión de un puesto**: una celda SQL que lee contesta con su tabla; una que escribe en un
  dataset del árbol **escribe de verdad**, por el mismo camino que el trabajo. Lo que no es del árbol
  —un `create schema tmp` de la sesión— se queda en DuckDB.
- **La escritura** llega al lago como la de `write()`: Arrow, `ore-store`, el commit por el catálogo.
  Anexar o hacer upsert lleva cada valor al tipo de su columna, y un `insert` sin alias casa las
  columnas por posición, como en SQL.

## El guion: varias sentencias en una ejecución

Un `.sql` con varias sentencias es **una lista de unidades**, no un fichero con otra semántica:
cada una es la frase de arriba, y cada una que crea algo es el verbo que ya existe (el alta de una
base, `createNamespace` y `createTable` de `/v1`, la vista de 0040).

- **Coteja entero antes de correr.** Si no coteja y escribe en el árbol, **no corre ninguna**: una
  celda de error con los diagnósticos de todas, cada uno en su línea.
- **Una celda por sentencia**, en orden y en la misma sesión: lo que crea o escribe una lo ve la
  siguiente.
- **Se para en el primer error.** Las de detrás salen como *saltadas*, diciendo por cuál. Lo que ya
  corrió, corrió: no hay transacción entre sentencias.
- **Todo resultado es una tabla**, como en Databricks:

  | la sentencia | su resultado |
  |---|---|
  | `select` | su tabla |
  | `create or replace dataset … as`, `insert` | `num_affected_rows`, `num_inserted_rows` |
  | `insert or replace` | `num_affected_rows`, `num_updated_rows`, `num_inserted_rows` |
  | lo que crea o borra | `object`, `status` (`created`, `already exists`, `replaced`, `dropped`, `not found`) |

## `sql()` y los nombres del árbol

Desde el código de un puesto —Python, Node o Java—, `sql(consulta)` lee el árbol con DuckDB: el
servidor dice qué nombres de la consulta son del árbol (`nombres_a_resolver`, por el tokenizador) y
resuelve cada uno como `over()` lo haría; el SDK lo registra en un catálogo de DuckDB por base, con
su schema. Un nombre que no está es `LookupError`; lo que no es del árbol se queda en DuckDB.

**El gobierno es el de cualquier lectura:**
- **El conducto**: lo que la superficie del puesto no admite es 403 (`OOS4002`), en el servidor.
- **Lo declarado**: mientras corre un transform, sólo se leen sus `inputs` y sólo se escribe su
  `output`, también en el servidor.
- **La procedencia** —el puesto, lo que leyó, el transform y el código— va con lo escrito.
- **El dueño** de lo que crea es quien lo crea ([`0052`](0052-ownership.md)).

## El editor

El servidor de lenguaje de SQL corre **dentro del agente del puesto** (`lsp_sql.py`), y
`ore-serve` sólo reparte sus mensajes ([`0037`](0037-language-servers-in-code-repositories.md)). Ve los nombres del árbol:

- **Completa** `base.schema.nombre` después de `FROM`, los schemas después de `base.`, y las
  columnas de un alias.
- **Diagnostica** cada sentencia con DuckDB sobre tablas vacías, sin leer una fila: una columna mal
  escrita sale en su sitio. Lo que es del guion y DuckDB no entiende no va a DuckDB; de
  `create view … as` y `create dataset … as` se comprueba la consulta. Avisa de una `Table` que
  `sql()` no puede leer y de `ORE-SQL-2P`.
- **Hover**: el tipo de una columna y lo que es un dataset.

En la consola, el editor ejecuta el fichero, y el panel de resultados enseña **«Result i of N»**:
cada sentencia con lo que dejó, *failed*, *skipped* o *running*, y «Go to line» a la suya; la tabla,
con filtro y CSV. Desde el catálogo, **Create › View, Materialized o Dataset** abre un
`untitled.sql` con su `CREATE …`.

## Aceptación

`el-puesto.sh`:
- **3d**: el editor (diagnósticos en su sitio, completar, hover, el guion sin errores falsos).
- **7**: `sql()` sobre el lago, nombres que no están, la sesión.
- **10c**: un `.sql` que escribe en la sesión, con su procedencia y por posición.
- **10d**: el guion, lote, saltadas, upsert con clave y un guion que no coteja.
- **10e**: las vistas desde SQL.
- **11b**: un `.sql` como trabajo.
- **13** y **16**: el conducto y lo declarado, en el servidor.

Y el cliente de la consola de verdad contra el agente, en `el-editor-sql.py` y, a escala y con
fallos, en `el-editor-sql-a-fondo.py`.
