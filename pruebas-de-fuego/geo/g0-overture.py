"""G0·1 (ADR 0067): los recortes de Overture para el banco geo (península y Baleares).

Lee el release público de Overture en S3 (anónimo), recorta por la columna `bbox` (que poda
grupos de filas) y escribe un GeoParquet por tema en /salida. Uso: python g0_overture.py [tema...]
"""
import sys
import time

import duckdb

R = "s3://overturemaps-us-west-2/release/2026-09-23.1"
CAJA = "bbox.xmin > -9.6 and bbox.xmax < 4.6 and bbox.ymin > 35.8 and bbox.ymax < 44.0"
TEMAS = {
    "divisiones_es": f"select * from read_parquet('{R}/theme=divisions/type=division_area/*', hive_partitioning=0) where country = 'ES'",
    "lugares_es": f"select * from read_parquet('{R}/theme=places/type=place/*', hive_partitioning=0) where {CAJA}",
    "carreteras_es": f"select * from read_parquet('{R}/theme=transportation/type=segment/*', hive_partitioning=0) where subtype = 'road' and {CAJA}",
    "edificios_es": f"select * from read_parquet('{R}/theme=buildings/type=building/*', hive_partitioning=0) where {CAJA}",
}
c = duckdb.connect()
c.execute("install httpfs; load httpfs; install spatial; load spatial")
c.execute("set s3_region='us-west-2'; set threads=16; set preserve_insertion_order=false")
for nombre in sys.argv[1:] or TEMAS:
    t0 = time.time()
    c.execute(f"copy ({TEMAS[nombre]}) to '/salida/{nombre}.parquet' (format parquet, compression zstd, row_group_size 100000)")
    n = c.execute(f"select count(*) from '/salida/{nombre}.parquet'").fetchone()[0]
    print(f"{nombre}: {n} filas en {time.time() - t0:.0f} s", flush=True)
