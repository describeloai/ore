"""ORE 0072 I2·4 · los 600 M de viajes de SpatialBench sf100 (`trip.parquet` del banco, 50 GB, sin orden ni marcas
geo) como **tabla Iceberg**, sin copiar un byte: PyIceberg `add_files` escribe sólo los metadatos (en el banco,
junto al fichero) y el Job del índice la lee como cualquier tabla del lago. De paso, la geometría llega a DuckDB
como `BLOB` con WKB: la otra rama de la lectura.

  docker run --rm -e TOKEN=… -v <i2>:/i2 python:3.12-slim sh -c "pip -q install 'pyiceberg[gcsfs,sql-sqlite]' && python -I /i2/tabla_600m.py"

Imprime `TABLA|<metadata_location>`. El token va por el entorno (no se imprime).
"""
import os
import time

from pyiceberg.catalog.sql import SqlCatalog
import pyarrow.parquet as pq
import gcsfs

BUCKET = "project-8853a180-450d-47be-b83-banco-geo"
FICHERO = f"gs://{BUCKET}/spatialbench/sf100/trip.parquet"
ALMACEN = f"gs://{BUCKET}/i2/tablas"
props = {"gcs.oauth2.token": os.environ["TOKEN"], "gcs.project-id": "project-8853a180-450d-47be-b83",
         "gcs.oauth2.token-expires-at": str(int((time.time() + 3000) * 1000))}

fs = gcsfs.GCSFileSystem(token=os.environ["TOKEN"])
esquema = pq.read_schema(FICHERO.removeprefix("gs://"), filesystem=fs)
cat = SqlCatalog("banco", uri="sqlite:////tmp/cat.db", warehouse=ALMACEN, **props)
cat.create_namespace_if_not_exists("i2")
t = cat.create_table_if_not_exists("i2.trip_600m", schema=esquema, location=f"{ALMACEN}/trip_600m")
t.add_files([FICHERO], check_duplicate_files=True)
t = cat.load_table("i2.trip_600m")
print("FILAS|%s" % t.current_snapshot().summary.get("total-records"), flush=True)
print("TABLA|%s" % t.metadata_location, flush=True)
