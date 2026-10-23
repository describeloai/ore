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
| `q4/sembrar.py` | Q4·4: la escalera como datasets del lago de victor (`query.trip_6m/60m/600m`, `query.customer`, lugares tipados EPSG:4326). Son transforms: van en `transforms/` de un repositorio Heavyweight y se lanzan con un Build por salida (la sesión de ese repositorio es ligera) |
| `q4/encender.sh` | `estado`, `encender [TECHO=2]` y `apagar` (en seco salvo `APLICAR=1`): suspende `inquilino-victor`, `ore-serve --consultas` y el suelo a 1; apagar reanuda Flux y retira de la cola los de quita y pon |
| `q4/medir.py` | Q4·4 y Q5·0 por `POST /consultas` con `ORE_TOKEN` (de una persona; ni se imprime ni se guarda): `escalera`, `aislamiento`, `concurrencia`, `arranque` |
| `resultados/` | las líneas JSON y los registros de cada Job |

⛔ **`sesion` derrama en el nodo del sistema** (el puesto no tiene tope de disco hasta Q1): a 600 M llenó
10 GiB y puso el nodo en `DiskPressure` (§2.4). No lanzarla con `ESCALAS=600` hasta Q1. Y no editar
`medir.sh` mientras corre: bash lo lee según avanza.

Los datos de `dialecto` y las ruedas del cliente de Spark Connect viven en
`gs://…-banco-geo/query/`.

**Q4·4 / Q5·0, en orden** (con lo desplegado trayendo Q4, y Q5·3 para el `arranque`):
1. `sembrar.py` como `transforms/escalera.py` de un repositorio Heavyweight de victor, y un Build por salida (la persona), una vez.
2. `bash q4/encender.sh estado`, y `APLICAR=1 bash q4/encender.sh encender TECHO=2`.
3. `ORE_TOKEN=… python q4/medir.py` (o una fase: `escalera`, `aislamiento`, `concurrencia`, `arranque`).
4. `APLICAR=1 bash q4/encender.sh apagar`, y `estado` para ver que no queda nada.
