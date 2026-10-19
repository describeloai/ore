"""ORE 0068 C3·b · Q10 (el LEFT JOIN espacial) con la regla `ReglasDeOre`, en el pod de un Build pesado, con los
datos del previo (20 M puntos, 22 000 zonas, 2 000 sin puntos): el previo dio > 300 s sin ella. La regla
llega en el jar de /m (CON=../b/ore-spark-reglas.jar). Líneas `C3B|…`.

    CON=../b/ore-spark-reglas.jar bash pruebas-de-fuego/geo/c3/previo/en-victor.sh ../b/q10-con-regla.py
"""
import os
import time

from pyspark.sql import SparkSession
from pyspark.sql import functions as F

N = int(os.environ.get("PUNTOS", "20000000"))
s = (SparkSession.builder.master("local[*]").appName("c3b-q10")
     .config("spark.driver.memory", os.environ.get("HEAP", "5g"))
     .config("spark.driver.extraClassPath", "/m/ore-spark-reglas.jar")
     .config("spark.sql.extensions", "org.apache.iceberg.spark.extensions.IcebergSparkSessionExtensions,"
             "org.apache.sedona.sql.SedonaSqlExtensions,ore.spark.ReglasDeOre")
     .config("spark.sql.shuffle.partitions", "64").getOrCreate())
s.sql("select id, ST_AsBinary(ST_Point(400000 + rand(1) * 100000, 4430000 + rand(2) * 100000)) geom "
      "from range(%d)" % N).write.mode("overwrite").parquet("/tmp/q/puntos")
s.sql("select id, ST_AsBinary(ST_Buffer(ST_Point(400000 + (id % 141) * 709.2, 4430000 + floor(id / 141) * 709.2"
      " + if(id >= 20000, 200000, 0)), 300)) zona from range(22000)").write.mode("overwrite").parquet("/tmp/q/zonas")
s.read.parquet("/tmp/q/puntos").select("id", F.expr("ST_SetSRID(ST_GeomFromWKB(geom), 25830)").alias("geom")) \
    .createOrReplaceTempView("puntos")
s.read.parquet("/tmp/q/zonas").select("id", F.expr("ST_SetSRID(ST_GeomFromWKB(zona), 25830)").alias("zona")) \
    .createOrReplaceTempView("zonas")
for nombre, q in (("zonas LEFT JOIN puntos", "select z.id, count(p.id) n from zonas z left join puntos p "
                                              "on ST_Contains(z.zona, p.geom) group by z.id"),
                  ("puntos RIGHT JOIN zonas", "select z.id, count(p.id) n from puntos p right join zonas z "
                                               "on ST_Contains(z.zona, p.geom) group by z.id")):
    t = time.time()
    r = s.sql(q)
    reescrita = "__ore_k" in r._jdf.queryExecution().analyzed().toString()
    p = r._jdf.queryExecution().executedPlan().toString()
    join = ",".join(j for j in ("BroadcastIndexJoin", "RangeJoin", "BroadcastNestedLoopJoin") if j in p)
    f = r.selectExpr("count(*) zonas", "sum(n) pares", "sum(if(n = 0, 1, 0)) vacias").first()
    print("C3B|%s|reescrita=%s join=%s s=%.1f zonas=%s pares=%s vacias=%s" % (
        nombre, reescrita, join, time.time() - t, f.zonas, f.pares, f.vacias), flush=True)
