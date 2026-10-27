# 0062 · Estadísticas del catálogo — las filas de lo que nadie copió

**Estado:** aceptado (2026-10-12). **E0–E3 hechos** (2026-10-12). E4 (las fuentes de base de datos
que hoy no dicen nada) y E5 (la cuenta exacta de un CSV, en segundo plano o a demanda), por construir.

## Contexto

La ficha de un activo —el hover del explorador, la columna About, la pestaña Details— dice
`Rows`. En un dataset del lago sale de su puntero (`datasets/<p>_<n>.json`, la última transacción:
se cuenta al escribir). En **un puntero a un origen** —la `Table` o el `ObjectTable` del paquete de
una fuente, lo que una foreign database lee en vivo— no hay transacciones, y decía **«—» siempre**:
un contador que nunca cuenta (visto con `azure_blob` y `mnist`, 0061 AZ5).

Y el dato **se calculaba y se tiraba**:

- `ore-read-objetos` leía el pie de cada Parquet, que dice sus filas (`num_rows`), y BigQuery las da
  en `__TABLES__.row_count`. Las dos van a `rows` en el catálogo de la fuente.
- Ese catálogo **ya está en el árbol**: `packages/<fuente>/discover.catalog.json`, en el mismo
  commit que los punteros que el inductor escribe de él. Lo vimos en la forja: `"rows": 630889` en
  `azure_blob`.
- Pero el inductor sólo lo usaba para preguntar si una tabla de cero filas entra, y el índice de
  activos no lo miraba.
- Un CSV o un JSONL no decía nada: el catálogo lee 64 KB (0046 F1, «nunca un fichero entero») y no
  contaba ni estimaba.

## Lo que hacen los demás (investigado el 2026-10-12)

| Quién | Cómo | Precio |
|---|---|---|
| Iceberg, Delta | `record_count` por fichero en el manifiesto y `total-records` en el snapshot: **al escribir**, exacto | 0 |
| AWS Glue (crawler) | `recordCount` **estimado** de los tamaños y la cabecera, con `averageRecordSize` y `sizeKey`; AWS no lo garantiza | 0 |
| Databricks / Unity | `ANALYZE TABLE`; la optimización predictiva lo lanza sola sólo en tablas *managed* | explícito |
| Postgres, BigQuery, Snowflake | la estadística del motor (`reltuples`, `__TABLES__`, `ROW_COUNT`) | 0, sin escanear |

Tres reglas comunes: **la estadística no es la definición**; **de la fuente más barata que sea
verdad**; **cada número dice si es exacto, estimado o desconocido, y de cuándo**.

## Decisión

1. **Las estadísticas son del catálogo de la fuente**, no de la `Table`. No van al documento —cambian
   con el origen y harían de cada recatalogación un cambio de spec—, sino al
   `discover.catalog.json` que el Job de catálogo ya escribe y ya se hace commit. No se añade ningún
   almacén ni ningún commit.
2. **Contadas y estimadas, nunca mezcladas.** `rows` es lo contado; `rowsEstimate` (clave nueva del
   catálogo, `ore-driver::catalogo::FORMA`) lo estimado. Quien pregunta «¿cero filas?» (el inductor)
   sólo lee `rows`.
3. **De lo más barato que sea verdad**, sin leer un byte más que antes:

   | Qué | Cómo | |
   |---|---|---|
   | Parquet | el pie (`num_rows`) | contadas |
   | CSV / JSONL que cabe en la muestra (64 KB) | sus filas | contadas |
   | CSV / JSONL mayor | `(tamaño − cabecera) ÷ bytes por fila de la muestra`, como Glue | estimadas |
   | Varios ficheros (Hive, o una carpeta que comparte esquema) | la suma de los leídos (hasta 20); si no se leyeron todos, extrapolada por bytes | contadas si todos, si no estimadas |
   | `ObjectTable` | cuántos objetos y cuánto pesan, del listado (`count`, `bytes`, que ya estaban) | — |
   | BigQuery | `__TABLES__.row_count` (ya estaba) | contadas |

4. **El índice de activos las sirve** (`ore-serve`, `GET /assets`): cada `Table` y `ObjectTable` de un
   paquete de fuente lleva `estadisticas` — `{filas, filas_son: contadas|estimadas, catalogado}` o
   `{objetos, bytes, catalogado}`—, del catálogo de su paquete por `<schema>.<nombre>`. `catalogado`
   es el último commit del catálogo. Un catálogo se lee una vez por índice, que se cachea por cabeza.
5. **La consola dice cuál es**: `630,889` (*Counted by the catalog on …*), `≈ 59,800 (estimated)`
   (*Estimated … from a sample*), `Not counted`. Nunca un «—» en una tabla: parece un contador roto.
   Un object table: `4 · 11.1 MB` objetos, y el patrón aparte. La ficha del origen, su árbol, el
   modal de crear base y el hover dicen lo mismo, porque todos lo toman de la ficha del puntero.

### Y de paso: una carpeta no es un objeto

Al medir `mnist` salió `default.raiz_binary`, un object table de **un objeto de 0 bytes**:
`processedManifest`, el marcador que algunas herramientas dejan al crear una carpeta, junto a
`processedManifest/manifest.json`. Ahora se salta un objeto vacío que se llama como la carpeta de
otros, como ya se saltaba el que acaba en `/`.

## Lo que no se hace (todavía)

- **E4 · Fuentes de base de datos que no dicen nada.** `ore-read-postgres` deja `rows` vacío;
  `pg_class.reltuples` es gratis y es una estimación (sería `rowsEstimate`).
- **E5 · La cuenta exacta de un CSV**: leerlo entero contando saltos de línea, en segundo plano y con
  presupuesto, o con un botón «Count rows». Cuesta leerlo entero; el catálogo no lo hace por regla.
- **Incremental por ETag**: hoy cada catalogación relee los pies (dos rangos por Parquet, hasta 20
  por tabla). Guardar la cuenta por `(clave, ETag)` y releer sólo lo que cambió, como un manifiesto
  de Iceberg, cuando un origen con miles de ficheros lo pida.
- **Una foreign database** no tiene `Table` propias: lee las de su fuente, y su ficha enseña las del
  puntero.

## Verificación

- `ore-read-objetos`: `las_filas_de_un_csv_se_cuentan_o_se_estiman` (100 000 filas en 1,1 MB →
  `rowsEstimate: 100000`; un CSV pequeño, contado) y
  `las_filas_se_cuentan_o_se_estiman_y_una_carpeta_no_es_un_objeto` (`mnist` en pequeño: CSV grande
  estimado, pequeño contado, Hive mixto estimado, Parquet contado, el marcador fuera, ida y vuelta).
- `ore-driver`: el catálogo máximo lleva `rowsEstimate` y su ida y vuelta es estable.
- `ore-serve`: `un_puntero_lleva_las_estadisticas_del_catalogo_de_su_fuente`.
- En producción, tras recatalogar: `azure_blob.part_00066` → `630,889`; `mnist` `train.csv` → unas
  60 000 estimadas; `t10k.csv` → unas 10 000.
