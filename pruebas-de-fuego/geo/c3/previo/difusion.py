"""ORE 0068 C3 · previo · ¿Cuándo difunde Spark + Sedona el lado pequeño de un join espacial por sí solo?

En 6c (victor) el join de 20 M puntos × 20 k polígonos salió `BroadcastIndexJoin` sin pedirlo; en C0, B2
(2,3 M lugares × 7 992 municipios) no, y tardó más de 600 s. Aquí, en `ore-puesto-spark:local`, el mismo
join con el lado de polígonos de dos tamaños (6c: buffers simples; «municipios»: polígonos detallados) y de
dos orígenes (generado en memoria, como `frio.sh`; leído de Parquet en WKB, como lee un Build del lago).
Por caso: el tamaño que estima Spark del lado pequeño, el umbral, el join que elige y lo que tarda.
Una línea `C3|caso|clave=valor …` por caso.

    bash pruebas-de-fuego/geo/c3/previo/difusion.sh
"""
import os
import time

from pyspark.sql import SparkSession
from pyspark.sql import functions as F

N = int(os.environ.get("PUNTOS", "2000000"))
LIMITE = int(os.environ.get("LIMITE_S", "180"))
s = (SparkSession.builder.master("local[*]").appName("c3-previo")
     .config("spark.driver.memory", os.environ.get("HEAP", "5g"))
     .config("spark.sql.shuffle.partitions", "16").getOrCreate())
conf = s.sparkContext.getConf()
umbral_spark = s.conf.get("spark.sql.autoBroadcastJoinThreshold")
umbral_sedona = s.conf.get("sedona.join.autoBroadcastJoinThreshold", "(sin poner)")
print("C3|conf|spark.sql.autoBroadcastJoinThreshold=%s sedona.join.autoBroadcastJoinThreshold=%s" % (
    umbral_spark, umbral_sedona), flush=True)

PUNTOS = ("select id, ST_SetSRID(ST_Point(400000 + rand(1) * 100000, 4430000 + rand(2) * 100000), 25830) geom "
          "from range(%d)" % N)


def poligonos(z, segmentos):
    """`z` buffers de 300 m en rejilla; `segmentos` por cuarto de círculo (ST_Buffer: 8 da 33 vértices)."""
    lado = int(z ** 0.5)
    return ("select id, ST_SetSRID(ST_Buffer(ST_Point(400000 + (id %% %d) * %f, 4430000 + floor(id / %d) * %f), "
            "300, false, 'quad_segs=%d'), 25830) zona from range(%d)" % (
                lado, 100000 / lado, lado, 100000 / lado, segmentos, z))


def a_parquet(sql, nombre, col):
    ruta = "/tmp/c3/%s" % nombre
    s.sql(sql).select("id", F.expr("ST_AsBinary(%s)" % col).alias(col)).write.mode("overwrite").parquet(ruta)
    return (s.read.parquet(ruta).select("id", F.expr("ST_SetSRID(ST_GeomFromWKB(%s), 25830)" % col).alias(col)))


def caso(nombre, zonas, puntos):
    zonas.createOrReplaceTempView("zonas")
    puntos.createOrReplaceTempView("puntos")
    r = s.sql("select z.id, count(*) n from zonas z join puntos p on ST_Contains(z.zona, p.geom) group by z.id")
    est = zonas._jdf.queryExecution().optimizedPlan().stats().sizeInBytes()
    plan = r._jdf.queryExecution().executedPlan().toString()
    join = next((j for j in ("BroadcastIndexJoin", "RangeJoin", "DistanceJoin", "BroadcastNestedLoopJoin",
                             "CartesianProduct") if j in plan), "otro")
    # Qué lado difunde Sedona: la línea del join dice `LeftSide` o `RightSide` (zonas es la izquierda).
    linea = next((l for l in plan.splitlines() if "BroadcastIndexJoin" in l), "")
    if linea:
        join += "(difunde %s)" % ("zonas" if "LeftSide" in linea else "puntos" if "RightSide" in linea else "?")
    t = time.time()
    import threading
    hecho = {}

    def correr():
        # El grupo es del hilo que lanza el trabajo (propiedad local de Spark): aquí, no fuera.
        s.sparkContext.setJobGroup(nombre, nombre, True)
        try:
            hecho["fila"] = r.agg(F.sum("n").alias("pares")).first()
        except Exception as e:  # noqa: BLE001
            hecho["error"] = str(e).splitlines()[0][:160]

    h = threading.Thread(target=correr, daemon=True)
    h.start()
    h.join(LIMITE)
    if h.is_alive():
        s.sparkContext.cancelJobGroup(nombre)
        h.join(30)
        res = "limite>%ds" % LIMITE
    else:
        res = hecho.get("error") or "pares=%s" % hecho["fila"]["pares"]
    print("C3|%s|tam_estimado_MB=%.1f join=%s s=%.1f %s" % (
        nombre, int(str(est)) / 2**20, join, time.time() - t, res), flush=True)


puntos_mem = s.sql(PUNTOS)
puntos_pq = a_parquet(PUNTOS, "puntos", "geom")
CASOS = os.environ.get("CASOS", "6c,municipios").split(",")
for nombre, z, seg in (("6c", 20000, 8), ("municipios", 8000, 128)):
    if nombre not in CASOS:
        continue
    caso("%s·memoria" % nombre, s.sql(poligonos(z, seg)), puntos_mem)
    caso("%s·parquet" % nombre, a_parquet(poligonos(z, seg), "zonas-" + nombre, "zona"), puntos_pq)

# Y lo que haría ORE: difundir el lado pequeño a mano (la pista), en el caso que no se difunde solo.
zonas = a_parquet(poligonos(8000, 128), "zonas-municipios", "zona")
caso("municipios·parquet·difundido", F.broadcast(zonas), puntos_pq)
