"""ORE 0068 C2·2 pieza 0 · ¿Quién sale a la red en ST_Transform? La traza de la JVM, filtrada a lo que
no es Spark: Sedona, GeoTools, proj, http.
    docker run --rm --network none --hostname localhost ore-puesto-spark:previo python /sonda_crs.py
"""
from pyspark.sql import SparkSession

s = (SparkSession.builder.master("local[1]").config("spark.ui.enabled", "false")
     .config("spark.sql.extensions", "org.apache.sedona.sql.SedonaSqlExtensions")
     .config("spark.sedona.crs.geotools", __import__("os").environ.get("GEOTOOLS", "false")).getOrCreate())
for q in ("select ST_AsText(ST_Transform(ST_Point(440000, 4474000), 'EPSG:25830', 'EPSG:4326')) t",
          "select ST_AsText(ST_Transform(ST_SetSRID(ST_Point(440000, 4474000), 25830), 'EPSG:4326')) t"):
    try:
        print("OK   ", q, "→", s.sql(q).first().t, flush=True)
    except Exception as e:
        ls = [l.strip() for l in str(e).splitlines()]
        print("FALLA", q, flush=True)
        for l in ls:
            if any(k in l.lower() for k in ("sedona", "geotools", "proj", "http", "url", "caused", "crs", "epsg")):
                print("   ", l[:220], flush=True)
s.stop()
