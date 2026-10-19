"""ORE 0068 C3·a · ¿Hasta cuánto se puede difundir? El lado pequeño de un join espacial, difundido a mano
(`F.broadcast`, lo que hará `over()`), con 100 k, 1 M y 5 M polígonos contra 20 M puntos, en el pod de un
Build pesado (heap 38 g). Por caso: las filas y los bytes del lado difundido en Parquet (lo que dirá el
catálogo), el join, el tiempo, si termina, y el pico de memoria del contenedor. Una línea `C3A|…` por caso.

    bash pruebas-de-fuego/geo/c3/previo/en-victor.sh ../a/memoria.py
"""
import os
import threading
import time

from pyspark.sql import SparkSession
from pyspark.sql import functions as F

N = int(os.environ.get("PUNTOS", "20000000"))
LIMITE = int(os.environ.get("LIMITE_S", "600"))
s = (SparkSession.builder.master("local[*]").appName("c3a-memoria")
     .config("spark.driver.memory", os.environ.get("HEAP", "5g"))
     .config("spark.driver.maxResultSize", "0")
     .config("spark.sql.shuffle.partitions", "64").getOrCreate())


def pico_mb():
    for f in ("/sys/fs/cgroup/memory.peak", "/sys/fs/cgroup/memory/memory.max_usage_in_bytes"):
        try:
            return round(int(open(f).read().strip()) / 2**20)
        except (OSError, ValueError):
            pass
    return -1


def ahora_mb():
    try:
        return round(int(open("/sys/fs/cgroup/memory.current").read().strip()) / 2**20)
    except (OSError, ValueError):
        return -1


def a_parquet(df, ruta):
    df.write.mode("overwrite").parquet(ruta)
    tam = sum(os.path.getsize(os.path.join(d, f)) for d, _, fs in os.walk(ruta) for f in fs if f.endswith(".parquet"))
    return tam


ruta_p = "/tmp/c3a/puntos"
a_parquet(s.sql("select id, ST_AsBinary(ST_Point(400000 + rand(1) * 100000, 4430000 + rand(2) * 100000)) geom "
                "from range(%d)" % N), ruta_p)
puntos = s.read.parquet(ruta_p).select("id", F.expr("ST_SetSRID(ST_GeomFromWKB(geom), 25830)").alias("geom"))
puntos.createOrReplaceTempView("puntos")
print("C3A|base|puntos=%d pico_MB=%d" % (N, pico_mb()), flush=True)

for z in [int(x) for x in os.environ.get("ZONAS", "100000,1000000,5000000").split(",")]:
    lado = int(z ** 0.5)
    paso = 100000 / lado
    radio = max(5.0, paso * 0.4)   # que no se solapen: más zonas, más pequeñas
    ruta = "/tmp/c3a/zonas-%d" % z
    tam = a_parquet(s.sql(
        "select id, ST_AsBinary(ST_Buffer(ST_Point(400000 + (id %% %d) * %f, 4430000 + floor(id / %d) * %f), %f)) zona "
        "from range(%d)" % (lado, paso, lado, paso, radio, z)), ruta)
    zonas = s.read.parquet(ruta).select("id", F.expr("ST_SetSRID(ST_GeomFromWKB(zona), 25830)").alias("zona"))
    # DIFUNDIR=0: lo que hace Spark solo (sin la pista; el umbral de bytes de Spark, apagado), para comparar.
    if os.environ.get("DIFUNDIR", "1") == "0":
        s.conf.set("spark.sql.autoBroadcastJoinThreshold", "-1")
        zonas.createOrReplaceTempView("zonas")
    else:
        F.broadcast(zonas).createOrReplaceTempView("zonas")
    r = s.sql("select z.id, count(*) n from zonas z join puntos p on ST_Contains(z.zona, p.geom) group by z.id")
    plan = r._jdf.queryExecution().executedPlan().toString()
    join = next((j for j in ("BroadcastIndexJoin", "RangeJoin", "BroadcastNestedLoopJoin") if j in plan), "otro")
    antes = ahora_mb()
    t = time.time()
    hecho = {}

    def correr():
        s.sparkContext.setJobGroup("z%d" % z, "z%d" % z, True)
        try:
            hecho["f"] = r.agg(F.sum("n").alias("pares"), F.count("id").alias("con")).first()
        except Exception as e:  # noqa: BLE001
            hecho["e"] = str(e).splitlines()[0][:300]

    h = threading.Thread(target=correr, daemon=True)
    h.start()
    h.join(LIMITE)
    if h.is_alive():
        s.sparkContext.cancelJobGroup("z%d" % z)
        res = "LIMITE>%ds" % LIMITE
    elif "e" in hecho:
        res = "ERROR %s" % hecho["e"]
    else:
        res = "pares=%s zonas_con_puntos=%s" % (hecho["f"]["pares"], hecho["f"]["con"])
    print("C3A|zonas=%d|parquet_MB=%.1f join=%s s=%.1f antes_MB=%d pico_MB=%d %s" % (
        z, tam / 2**20, join, time.time() - t, antes, pico_mb(), res), flush=True)
    if h.is_alive():
        break   # lo cancelado sigue comiendo CPU: lo que viniera detrás saldría contaminado
