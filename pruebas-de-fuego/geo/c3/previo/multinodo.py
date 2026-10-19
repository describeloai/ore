"""ORE 0068 C3 · previo · El driver de la prueba multinodo (`multinodo.sh`): Spark + Sedona contra el master
*standalone* del pod 0, con un ejecutor por nodo. El mismo join con 1 y con N nodos: 6c a más escala
(puntos al azar × 20 k buffers) y «municipios» (8 000 polígonos detallados) difundidos a mano.
Una línea `C3|caso|clave=valor …` por caso.
"""
import os
import time

from pyspark.sql import SparkSession
from pyspark.sql import functions as F

N = int(os.environ.get("PUNTOS", "200000000"))
NODOS = int(os.environ["NODOS"])
t0 = time.time()
s = (SparkSession.builder.master(os.environ["MASTER"]).appName("c3-multinodo")
     .config("spark.driver.host", os.environ["YO"])
     .config("spark.driver.port", "7078").config("spark.blockManager.port", "7079")
     .config("spark.driver.memory", "4g")
     .config("spark.executor.memory", os.environ.get("MEM_EJECUTOR", "30g"))
     .config("spark.executor.cores", "7").config("spark.cores.max", str(7 * NODOS))
     .config("spark.sql.shuffle.partitions", str(64 * NODOS))
     .config("spark.local.dir", "/tmp/spark").getOrCreate())
# Que estén todos los ejecutores antes de medir (un nodo que llega tarde no es escalar).
while s.sparkContext._jsc.sc().getExecutorMemoryStatus().size() - 1 < NODOS and time.time() - t0 < 300:
    time.sleep(1)
ejecutores = s.sparkContext._jsc.sc().getExecutorMemoryStatus().size() - 1
print("C3|sesion|nodos=%d ejecutores=%d s=%.1f" % (NODOS, ejecutores, time.time() - t0), flush=True)

partes = 64 * NODOS
s.sql("select id, ST_SetSRID(ST_Point(400000 + rand(1) * 100000, 4430000 + rand(2) * 100000), 25830) geom "
      "from range(0, %d, 1, %d)" % (N, partes)).createOrReplaceTempView("puntos")


def zonas(z, seg):
    lado = int(z ** 0.5)
    return s.sql("select id, ST_SetSRID(ST_Buffer(ST_Point(400000 + (id %% %d) * %f, 4430000 + floor(id / %d) * %f), "
                 "300, false, 'quad_segs=%d'), 25830) zona from range(%d)" % (
                     lado, 100000 / lado, lado, 100000 / lado, seg, z))


for nombre, z, seg in (("6c", 20000, 8), ("municipios", 8000, 128)):
    F.broadcast(zonas(z, seg)).createOrReplaceTempView("zonas")
    t = time.time()
    r = s.sql("select z.id, count(*) n from zonas z join puntos p on ST_Contains(z.zona, p.geom) group by z.id")
    plan = r._jdf.queryExecution().executedPlan().toString()
    join = next((j for j in ("BroadcastIndexJoin", "RangeJoin", "BroadcastNestedLoopJoin") if j in plan), "otro")
    f = r.agg(F.sum("n").alias("pares")).first()
    print("C3|%s|nodos=%d puntos=%d join=%s s=%.1f pares=%s" % (
        nombre, NODOS, N, join, time.time() - t, f["pares"]), flush=True)
s.stop()
