"""ORE 0068 C2·3 pieza 6 (6c) · El join espacial de C0 en el puesto pesado DE VERDAD: un Job de medida en
`cola-pesada` de t-victor, con `puesto-spark:1` y los recursos de un Build pesado (7 CPU / 48 GiB, el heap
que daría el arnés). Sin lago y sin red: 20 M puntos y 20 k polígonos sintéticos en EPSG:25830, generados en
Spark, y `ST_Contains` agrupado por polígono. Una línea `6C|clave|valor` por medida.
"""
import os
import resource
import time

T0 = time.time()


def di(k, v):
    print("6C|%s|%s" % (k, v), flush=True)


def rss_mb():
    """El pico de memoria del contenedor entero (la JVM de Spark es hija viva: `getrusage` no la cuenta),
    del cgroup v2; si no está, el de este proceso."""
    for f in ("/sys/fs/cgroup/memory.peak", "/sys/fs/cgroup/memory/memory.max_usage_in_bytes"):
        try:
            return round(int(open(f).read().strip()) / 2**20)
        except (OSError, ValueError):
            pass
    return round(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024)


di("arranque_del_proceso_s", round(T0 - float(os.environ.get("T_POD", T0)), 1))
mb = int(os.environ.get("ORE_MEMORIA_MB", "49152"))
heap = "%dm" % max(1024, int((mb - 1536) * 0.8))   # la regla de `ore/spark.py` (_memoria_del_driver)
from pyspark.sql import SparkSession  # noqa: E402

s = (SparkSession.builder.master("local[*]").appName("6c").config("spark.driver.memory", heap)
     .config("spark.sql.shuffle.partitions", "64").getOrCreate())
di("spark_sesion_s", round(time.time() - T0, 1))
di("heap", heap)
di("nucleos", s.sparkContext.defaultParallelism)

N, Z = int(os.environ.get("PUNTOS", "20000000")), int(os.environ.get("ZONAS", "20000"))
# Una caja de 100 km × 100 km en EPSG:25830 (Madrid): los puntos al azar, los polígonos buffers de 300 m
# en una rejilla (unos 0,28 km² cada uno: cubren ~28 % del área, como C0).
t = time.time()
s.sql("""select id, ST_SetSRID(ST_Point(400000 + rand(1) * 100000, 4430000 + rand(2) * 100000), 25830) geom
         from range(%d)""" % N).createOrReplaceTempView("puntos")
lado = int(Z ** 0.5)
s.sql("""select id, ST_SetSRID(ST_Buffer(ST_Point(400000 + (id %% %d) * %f, 4430000 + floor(id / %d) * %f), 300), 25830) zona
         from range(%d)""" % (lado, 100000 / lado, lado, 100000 / lado, Z)).createOrReplaceTempView("zonas")
r = s.sql("""select z.id, count(*) n from zonas z join puntos p on ST_Contains(z.zona, p.geom) group by z.id""")
from pyspark.sql import functions as F  # noqa: E402

fila = r.agg(F.sum("n").alias("pares"), F.count("id").alias("zonas")).first()
di("join_s", round(time.time() - t, 1))
di("pares", fila["pares"])
di("zonas_con_puntos", fila["zonas"])
plan = r._jdf.queryExecution().executedPlan().toString()
di("join_del_plan", "RangeJoin" if "RangeJoin" in plan else ("BroadcastIndexJoin" if "BroadcastIndexJoin" in plan else "otro"))

# Y otra vez, ya caliente (la JVM, el JIT): lo que tarda un segundo build en el mismo nodo.
t = time.time()
r2 = s.sql("""select z.id, count(*) n from zonas z join puntos p on ST_Contains(z.zona, p.geom) group by z.id""")
igual = r2.agg(F.sum("n").alias("pares")).first()["pares"] == fila["pares"]
di("join_otra_vez_s", round(time.time() - t, 1) if igual else "DISTINTO")
di("pico_rss_mb", rss_mb())
di("total_s", round(time.time() - T0, 1))
s.stop()
