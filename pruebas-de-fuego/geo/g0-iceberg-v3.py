"""G0·2 (ADR 0067): una tabla Iceberg v3 AJENA con columna `geometry`, en el bucket del banco.

La escribe pyiceberg (no ORE), como la tendría quien viene de Snowflake o S3 Tables: format-version 3,
`geometry` con CRS, datos en gs://…/lakehouse/. El catálogo es SQL (sqlite) y se sube junto a ella.
Uso: GCS_TOKEN=$(gcloud auth print-access-token) python g0_iceberg_v3.py /salida/lugares_es.parquet
"""
import os
import sys
import time

import duckdb
import pyarrow as pa
from pyiceberg.catalog.sql import SqlCatalog
from pyiceberg.schema import Schema
from pyiceberg.types import GeometryType, LongType, NestedField, StringType

B = "gs://project-8853a180-450d-47be-b83-banco-geo/lakehouse"
c = duckdb.connect()
c.execute("install spatial; load spatial")
t = c.execute(f"""select row_number() over () :: bigint id, names.primary nombre, basic_category categoria,
                        ST_AsWKB(geometry) geom
                 from '{sys.argv[1]}'
                 where bbox.xmin between -4.6 and -3.0 and bbox.ymin between 39.8 and 41.2""").fetch_arrow_table()
cat = SqlCatalog("banco", uri="sqlite:////salida/lakehouse-catalogo.db", warehouse=B,
                 **{"gcs.oauth2.token": os.environ["GCS_TOKEN"], "gcs.oauth2.token-expires-at": str(int((time.time() + 3000) * 1000)), "gcs.project-id": "project-8853a180-450d-47be-b83"})
cat.create_namespace_if_not_exists("ajena")
esquema = Schema(
    NestedField(1, "id", LongType(), required=False),
    NestedField(2, "nombre", StringType(), required=False),
    NestedField(3, "categoria", StringType(), required=False),
    NestedField(4, "geom", GeometryType("OGC:CRS84"), required=False),
)
if cat.table_exists("ajena.lugares_madrid"):
    cat.drop_table("ajena.lugares_madrid")
tabla = cat.create_table("ajena.lugares_madrid", esquema, properties={"format-version": "3"})
tabla.append(pa.table({
    "id": t["id"], "nombre": t["nombre"], "categoria": t["categoria"],
    "geom": t["geom"].cast(pa.binary()),
}).cast(tabla.schema().as_arrow()))
tabla = cat.load_table("ajena.lugares_madrid")
print("filas:", t.num_rows, "| versión:", tabla.format_version, "| esquema:", tabla.schema())
print("metadatos:", tabla.metadata_location)
