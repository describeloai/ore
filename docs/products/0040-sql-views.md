# 0040 · SQL Views

**Estado:** **aceptado · en vivo** (2026-09-27) · **Decide:** que en ORE una vista **es su
consulta**: una `View` es SQL, y todo lo que se gobierna de ella —lo que lee, el linaje de cada
columna, sus etiquetas, lo que revela por sus predicados— se deriva de esa consulta. Gramática:
[OOS v1alpha14 `01-la-vista-es-sql`](../../vendor/oos/spec/v1alpha14/01-la-vista-es-sql.md). Se
apoya en [`0038`](0038-assets-catalog-namespaces.md) (`base.schema.nombre`), [`0039`](0039-sql-paradigms-in-code-repositories.md)
(SQL en los code repositories) y [`0033`](0033-el-dataset.md) (lo que tiene bytes es un dataset).

## Qué es

Una `View` declara tres cosas:

```yaml
apiVersion: oos.dev/v1alpha14
kind: View
metadata: { name: por_pais, namespace: ventas }
spec:
  owner: user:ana
  dialect: duckdb
  sql: |
    SELECT pais, count(*) AS pedidos, sum(total) AS importe
    FROM ventas.pedidos
    GROUP BY pais
  columns:
    pais: { type: String }
    pedidos: { type: Integer }
    importe: { type: Decimal }
```

- **`sql`**: un `SELECT` que lee por nombre —una `Table`, una `View`, un `Dataset` o un
  `ObjectTable` del árbol—, con `WITH`, `UNION`, joins, expresiones, agregados o ventanas. Nada
  que escriba, ni lectores por función (`read_parquet`): eso es `OOS2038`.
- **`dialect`**: `duckdb`. El dialecto se guarda, porque es parte de lo que la consulta significa.
- **`columns`**: **el contrato**, lo que la consulta proyecta con sus tipos. Si no coincide, es
  `OOS2039`.

Lo estructurado —propiedades, claves, relaciones— vive en la semántica (`Entity`, que respalda una
vista con `backedBy`) y en el `Dataset` mantenido. **Dentro de ORE hay una sola View, la SQL**: una
vista de las versiones v1alpha8–13 se traduce a su consulta al cargarla, y significa lo mismo que
significaba.

## Cómo se gobierna

`vista_sql` analiza la consulta sin ejecutarla y sin motor, y de ahí sale todo lo demás:

- **Lo que lee**: los nombres del árbol, sin los del `WITH`. Cada uno tiene que existir
  (`OOS2018`), y **un nombre es una sola cosa**: una tabla, una vista y un dataset comparten el
  espacio de nombres de su schema (`OOS2035`), como en Unity Catalog.
- **El linaje de cada columna**: directo, derivado o indirecto. Un `*` se expande contra los
  contratos de sus fuentes; una columna sin calificar se decide entre ellas.
- **El flujo, por el linaje**: las etiquetas viajan por las aristas, y una vista con varias
  fuentes tiene varias raíces. Lo que una copia o un conducto no admite es `OOS4002`, también por
  la arista indirecta de un `WHERE`.
- **El canal lateral** (`OOS4016`): un predicado que ordena o filtra por una columna etiquetada
  revela lo que la proyección esconde, aunque no la proyecte. Se mira en el `WHERE`, el `JOIN`, el
  `QUALIFY`, el `HAVING` y el `ORDER BY` de un `LIMIT`.
- **Lo que nunca es nulo**, derivado de la consulta ([`0051`](0051-ore-null-contract.md)).
- **Los cambios** (`ore diff`): por el contrato —una columna que se va es `OOS5001`, un tipo que
  cambia, `OOS5002`— y por las filas. Cualquier cambio de la consulta que no sea de forma (espacios,
  proyección) es un cambio de filas: `OOS5028`/`OOS5029`.

## Cómo se sirve

**Servir una vista es servir su consulta**, con cada nombre del árbol resuelto por completo
(`ore_core::servir`): un dataset por el nombre con que lo registra quien lee, otra vista como
subconsulta con su alias, una tabla de un origen nunca.

- **En el puesto**: `over()` y `sql()` la leen con DuckDB sobre sus datasets, con los nombres
  resueltos por el servidor.
- **Por el catálogo REST** (`/v1`, `loadView`): una sola representación, `duckdb`. Un motor de otro
  dialecto (Spark) no encuentra la suya y lo dice: traducir el texto cambiaría la semántica en
  silencio donde los dialectos difieren.
- **`ore ask --sql`** la sirve igual; sin copia hecha, una vista SQL se lee en un puesto.

## Cómo se copia

**La copia de una vista es un dataset, y se rehace entera** ejecutando la consulta. Va en la
pasada de copia de siempre ([`0027`](0027-model-serving.md) para la figura, `malla/48`), en tres
contenedores sobre el mismo clon:

1. **preparar** (`ore materialize --preparar`): la consulta servida y lo que lee, en Arrow.
2. **calcular** (`python -m ore.calcular`, imagen del puesto): DuckDB la ejecuta **sin credencial
   y cerrado al exterior**; lo que lee ya está en el disco.
3. **copiar** (`ore materialize --calculado`): `ore-store` la sella con las columnas y los tipos del
   contrato, y la pasada empuja el puntero.

La cabecera de la copia es la consulta servida, el contrato y **el snapshot de cada dataset que
lee**: si nada se movió, «ya está» sin leer una fila. Un solo escritor, sin excepciones: `write()`
sobre una copia sigue negado. La copia es la vista **entera**: un `Dataset` con `from: { view }` y
un `where` encima no es una copia, y su puntero lo dice.

## Cómo se crea

**Desde SQL, en el puesto** (el guion de 0039, o una celda suelta):

```sql
CREATE [OR REPLACE] VIEW [IF NOT EXISTS] base.schema.vista
  [(columna [COMMENT '…'], …)] [COMMENT '…'] [WITH SCHEMA EVOLUTION]
  AS SELECT …;
DROP VIEW [IF EXISTS] base.schema.vista;
CREATE MATERIALIZED VIEW base.schema.vista AS SELECT …;
```

- **La consulta se guarda tal como se escribió**, y los nombres se resuelven al servirla. El
  contrato lo describe DuckDB sobre tablas vacías con los tipos del índice, sin leer una fila.
  Los tipos van a los de OOS (0032): enteros a `Integer`, `DECIMAL` a `Decimal`, `DOUBLE` a
  `Float`, `T[]` a `list<T>`. Una columna sin alias o repetida se niega pidiendo uno.
- **Reemplazar** añade columnas sin más; quitar una o cambiarle el tipo rompe a quien la lee, y se
  niega salvo `WITH SCHEMA EVOLUTION`. Un nombre que ya es un `Dataset` o una `Table` no se
  reemplaza nunca.
- **`DROP VIEW`** se niega si otra cosa del árbol la lee, y dice quién.
- **`CREATE MATERIALIZED VIEW`** escribe la vista y su copia, el dataset `vista_copia`, que es lo
  que se lee. Como la copia se calcula sobre el lago, una vista materializada no lee una tabla de
  un origen.
- **El dueño es quien la crea** ([`0052`](0052-ownership.md)), y reemplazarla le conserva el que
  tenía. `COMMENT` es `description`.
- El SDK lo hace con `crear_vista`, y el editor del puesto comprueba la consulta de un
  `CREATE VIEW` en su sitio, sin pasar el guion a DuckDB.

**Desde la consola**: Create › View, Materialized o Dataset abre un `untitled.sql` con su
`CREATE …` en una instancia de SQL transforms de la base. Nada se escribe hasta el commit, y la
vista vive en su schema.

**Lo que trae una fuente**: el inductor deja, sobre cada tabla `<objeto>_t`, una vista SQL de pleno
derecho que se llama como el objeto.

## Las versiones de antes

- **v1alpha8–13** se traducen al cargarlas (`linaje::como_sql`), y la conformance de esas versiones
  sigue siendo la puerta.
- **`ore migrate v1alpha14`** reescribe un árbol entero, sobre una copia. Las vistas se quedan su
  nombre; la tabla que se llamaba como una vista pasa a `<n>_t`, y el dataset a `<n>_copia`, con
  su puntero mudado; cada vista se escribe como su consulta. **El criterio**: el árbol migrado da
  exactamente los mismos diagnósticos, código a código. Si no, no se escribe nada.
- La forma estructurada se queda en el `Dataset` mantenido, que v1alpha14 conserva.

## Aceptación

- **La medida que abrió la puerta**: el linaje derivado del SQL es el del motor estructurado en
  **115 de 115** vistas válidas (`medida-el-linaje-de-la-vista-sql.py`), y `ore validate` da lo
  mismo en los 410 árboles del repositorio.
- **Conformance** v1alpha14: 26/26.
- **De punta a punta**: `el-lago.sh` 14c (la copia de un `GROUP BY` y un `JOIN` de dos datasets,
  el testigo de cada entrada, «ya está», y los motivos en el puntero); `el-puesto.sh` 10b (la
  vista SQL y la estructurada dan las mismas filas por `over()` y `sql()`), 10e (quitar una vista
  que otra lee) y 3d (el editor).
- **En `demo`, de verdad** (`la-copia-sql-en-demo.py`): un `write()` desde el puesto, una vista con
  `GROUP BY` y su copia calculada por el Job de tres contenedores en 20 s.
