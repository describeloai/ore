"""ORE 0068 C2·2 pieza 2 · El orden de los ejes en EPSG:4326: el mismo punto (EPSG:25830, la Puerta del
Sol) llevado a 4326 por DuckDB (el motor Lightweight) y por Sedona (Heavyweight), en la MISMA imagen.
Un Transform no puede dar puntos distintos según el motor del repositorio.
    docker run --rm --network none --hostname localhost ore-puesto-spark:local python3 /ejes.py
"""
import duckdb
from pyspark.sql import SparkSession

P = "POINT (440000 4474000)"
d = duckdb.connect()
d.execute("set extension_directory = '/opt/ore/duckdb'")
d.execute("load spatial")
print("duckdb", duckdb.__version__)
for nombre, q in [
    ("ST_Transform(g, 'EPSG:25830', 'EPSG:4326')",
     f"select ST_AsText(ST_Transform(ST_GeomFromText('{P}'), 'EPSG:25830', 'EPSG:4326'))"),
    ("… always_xy := true",
     f"select ST_AsText(ST_Transform(ST_GeomFromText('{P}'), 'EPSG:25830', 'EPSG:4326', always_xy := true))"),
    ("… a 'OGC:CRS84'",
     f"select ST_AsText(ST_Transform(ST_GeomFromText('{P}'), 'EPSG:25830', 'OGC:CRS84'))"),
]:
    try:
        print(f"  duckdb · {nombre:45} → {d.execute(q).fetchone()[0]}")
    except Exception as e:
        print(f"  duckdb · {nombre:45} → FALLA {str(e).splitlines()[0][:160]}")
# ⭐ El ajuste global (DuckDB 1.5): con él, el orden de Sedona y el de GeoParquet (x = lon) sin decirlo en cada llamada.
d.execute("set geometry_always_xy = true")
q = f"select ST_AsText(ST_Transform(ST_GeomFromText('{P}'), 'EPSG:25830', 'EPSG:4326'))"
print(f"  duckdb · {'set geometry_always_xy = true; ST_Transform':45} → {d.execute(q).fetchone()[0]}")

s = SparkSession.builder.master("local[1]").getOrCreate()
for nombre, q in [
    ("ST_Transform(g, 'EPSG:25830', 'EPSG:4326')",
     f"select ST_AsText(ST_Transform(ST_GeomFromText('{P}'), 'EPSG:25830', 'EPSG:4326')) t"),
    ("… a 'OGC:CRS84'",
     f"select ST_AsText(ST_Transform(ST_GeomFromText('{P}'), 'EPSG:25830', 'OGC:CRS84')) t"),
]:
    try:
        print(f"  sedona · {nombre:45} → {s.sql(q).first().t}")
    except Exception as e:
        print(f"  sedona · {nombre:45} → FALLA {str(e).splitlines()[0][:160]}")
s.stop()
