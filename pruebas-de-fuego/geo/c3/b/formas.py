"""ORE 0068 C3·b · El `LEFT JOIN` espacial, a mano, antes de la regla de Catalyst. La documentación de Sedona
(Optimizer): un LEFT JOIN espacial usa el índice si el lado DERECHO lleva la marca de difundir; si no, cae en
`BroadcastNestedLoopJoin` (el previo: > 300 s). Y para el caso en que la derecha es la grande (Q10: zonas
LEFT JOIN viajes), su reescritura: un join interno espacial que da el id de la izquierda, y un LEFT JOIN
por ese id. Aquí, en el pod de un Build pesado, las formas con los datos del previo (20 M puntos, 22 000
zonas de las que 2 000 no tienen puntos). Todas tienen que dar 22 000 zonas, 2 000 vacías y los mismos
pares. Una línea `C3B|forma|…` por forma.

    bash pruebas-de-fuego/geo/c3/previo/en-victor.sh ../b/formas.py
"""
import os
import threading
import time

from pyspark.sql import SparkSession
from pyspark.sql import functions as F

N = int(os.environ.get("PUNTOS", "20000000"))
LIMITE = int(os.environ.get("LIMITE_S", "300"))
s = (SparkSession.builder.master("local[*]").appName("c3b-formas")
     .config("spark.driver.memory", os.environ.get("HEAP", "5g"))
     .config("spark.sql.shuffle.partitions", "64").getOrCreate())
# Del lago, como un Build: Parquet en WKB (las ids se generan una vez y quedan escritas).
s.sql("select id, ST_AsBinary(ST_Point(400000 + rand(1) * 100000, 4430000 + rand(2) * 100000)) geom "
      "from range(%d)" % N).write.mode("overwrite").parquet("/tmp/c3b/puntos")
s.sql("select id, ST_AsBinary(ST_Buffer(ST_Point(400000 + (id % 141) * 709.2, 4430000 + floor(id / 141) * 709.2"
      " + if(id >= 20000, 200000, 0)), 300)) zona from range(22000)").write.mode("overwrite").parquet("/tmp/c3b/zonas")
puntos = s.read.parquet("/tmp/c3b/puntos").select("id", F.expr("ST_SetSRID(ST_GeomFromWKB(geom), 25830)").alias("geom"))
zonas = s.read.parquet("/tmp/c3b/zonas").select("id", F.expr("ST_SetSRID(ST_GeomFromWKB(zona), 25830)").alias("zona"))
puntos.createOrReplaceTempView("puntos")
zonas.createOrReplaceTempView("zonas")
F.broadcast(zonas).createOrReplaceTempView("zonas_dif")
# Las zonas sin columna id: lo que la regla tendría que numerar ella.
zonas.select("zona").createOrReplaceTempView("zonas_sin_id")

FORMAS = {
    # 1 · grande LEFT JOIN pequeño, con el pequeño marcado (lo que hará C3·a): Sedona lo indexa.
    "1 puntos LEFT JOIN zonas difundidas": (
        "select z.id, count(p.id) n from puntos p left join zonas_dif z on ST_Contains(z.zona, p.geom) "
        "where z.id is not null group by z.id "
        "union all select null, count(*) from puntos p left join zonas_dif z on ST_Contains(z.zona, p.geom) "
        "where z.id is null", "puntos sin zona en la fila de id nulo"),
    # 2 · Q10: pequeño LEFT JOIN grande, reescrito como dice Sedona, con el id que ya tiene la izquierda.
    "2 Q10 a la Sedona, con id": (
        """with dentro as (select z.id zid, p.id pid from zonas z join puntos p on ST_Contains(z.zona, p.geom))
           select z.id, count(d.pid) n from zonas z left join dentro d on z.id = d.zid group by z.id""", ""),
    # 3 · lo mismo sin id: la regla lo numera (monotonically_increasing_id). Spark calcula `zonas_k` dos
    #   veces; si el número no sale igual las dos, las cuentas salen mal.
    "3 Q10 a la Sedona, id generado": (
        """with zonas_k as (select monotonically_increasing_id() k, zona from zonas_sin_id),
                dentro as (select z.k, p.id pid from zonas_k z join puntos p on ST_Contains(z.zona, p.geom))
           select z.k id, count(d.pid) n from zonas_k z left join dentro d on z.k = d.k group by z.k""", ""),
}

for nombre, (q, nota) in FORMAS.items():
    t = time.time()
    r = s.sql(q)
    plan = r._jdf.queryExecution().executedPlan().toString()
    join = ",".join(j for j in ("BroadcastIndexJoin", "RangeJoin", "BroadcastNestedLoopJoin", "BroadcastHashJoin",
                                 "SortMergeJoin") if j in plan) or "otro"
    hecho = {}

    def correr():
        s.sparkContext.setJobGroup(nombre, nombre, True)
        try:
            hecho["f"] = r.selectExpr("count(id) zonas", "sum(if(id is not null, n, 0)) pares",
                                      "sum(if(id is not null and n = 0, 1, 0)) vacias",
                                      "max(if(id is null, n, null)) sin_zona").first()
        except Exception as e:  # noqa: BLE001
            hecho["e"] = str(e).splitlines()[0][:300]

    h = threading.Thread(target=correr, daemon=True)
    h.start()
    h.join(LIMITE)
    if h.is_alive():
        s.sparkContext.cancelJobGroup(nombre)
        print("C3B|%s|join=%s LIMITE>%ds" % (nombre, join, LIMITE), flush=True)
        break   # lo cancelado sigue comiendo CPU
    if "e" in hecho:
        print("C3B|%s|join=%s ERROR %s" % (nombre, join, hecho["e"]), flush=True)
        continue
    f = hecho["f"]
    print("C3B|%s|join=%s s=%.1f zonas=%s pares=%s vacias=%s sin_zona=%s %s" % (
        nombre, join, time.time() - t, f.zonas, f.pares, f.vacias, f.sin_zona, nota), flush=True)
