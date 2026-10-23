# Query Q0 · las medidas del motor por consulta (2026-10-23)

Lo que dicen y lo que se decide con ellas: [`docs/query.md`](../../docs/query.md) §2 (Q0b en §2.7).

| fichero | qué |
|---|---|
| `consultas.py` | la escalera: nueve consultas sobre trip (6 M, 60 M, 600 M) con un DuckDB configurado como el puesto; calcula los GB que un enrutador sabría antes de correr |
| `connect.py` | Spark Connect dentro del driver de un Build elástico (configuración de `ore.spark._elastico()`); `SOLO=aislamiento` para la prueba corta |
| `dialecto.py` + `idiomas.sql` | el SQL de DuckDB traducido con sqlglot y comparado en Spark (TPC-H, TPC-DS e idiomas) |
| `q0b.py` | Q0b: un trabajador DuckDB compartido (instancia por consulta, concurrencia, topes, aislamiento, suelo de latencia) |
| `q0b_spark.py` | Q0b: Spark Connect con un catálogo REST de mentira: ¿lleva cada sesión su identidad, o la pone el cliente? |
| `medir.sh` | un Job de Kueue en t-victor por escalón: `sesion`, `grande`, `dialecto`, `connect`, `q0b`, `arranque` |
| `sembrar.py` | la escalera de Q0 como datasets del lago de victor (`query.trip_6m/60m/600m`, `query.customer`, lugares tipados EPSG:4326; valen también para Maps). Son transforms: van en `transforms/` de un repositorio Heavyweight y se lanzan con un Build por salida |
| `resultados/` | las líneas JSON y los registros de cada Job |

⛔ **`sesion` derrama en el nodo del sistema** (el puesto no tiene tope de disco hasta Q1): a 600 M llenó
10 GiB y puso el nodo en `DiskPressure` (§2.4). No lanzarla con `ESCALAS=600` hasta Q1. Y no editar
`medir.sh` mientras corre: bash lo lee según avanza.

Los datos de `dialecto` y las ruedas del cliente de Spark Connect viven en
`gs://…-banco-geo/query/`.

