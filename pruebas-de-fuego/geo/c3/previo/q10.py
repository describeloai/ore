"""ORE 0068 C3 · previo · Q10, el `LEFT JOIN` espacial, que en C0 falló en 8 s en Spark + Sedona («o61.
collectToPython», sin la causa). SpatialBench Q10: cada zona con sus viajes, también las que no tienen. Aquí,
el mismo hecho con datos generados (los de 6c): la causa entera, el plan de cada forma y lo que tarda.

    bash pruebas-de-fuego/geo/c3/previo/en-victor.sh q10.py PUNTOS=20000000
"""
import os
import threading
import time
import traceback

from pyspark.sql import SparkSession

N = int(os.environ.get("PUNTOS", "2000000"))
s = (SparkSession.builder.master("local[*]").appName("c3-q10")
     .config("spark.driver.memory", os.environ.get("HEAP", "5g"))
     .config("spark.sql.shuffle.partitions", "64").getOrCreate())
s.sql("select id, ST_SetSRID(ST_Point(400000 + rand(1) * 100000, 4430000 + rand(2) * 100000), 25830) geom "
      "from range(%d)" % N).createOrReplaceTempView("puntos")
# 20 k zonas en rejilla, y 2 000 más FUERA de la caja: ésas no tienen puntos (lo que Q10 tiene que contar).
s.sql("select id, ST_SetSRID(ST_Buffer(ST_Point(400000 + (id % 141) * 709.2, 4430000 + floor(id / 141) * 709.2"
      " + if(id >= 20000, 200000, 0)), 300), 25830) zona from range(22000)").createOrReplaceTempView("zonas")

FORMAS = {
    # La de SpatialBench: se conservan las zonas (el lado pequeño).
    "zonas LEFT JOIN puntos": "select z.id, count(p.id) n from zonas z left join puntos p "
                              "on ST_Contains(z.zona, p.geom) group by z.id",
    # Al revés: se conservan los puntos (el lado grande); la zona, nula si no cae en ninguna.
    "puntos LEFT JOIN zonas": "select z.id, count(*) n from puntos p left join zonas z "
                              "on ST_Contains(z.zona, p.geom) group by z.id",
    # Lo que podría hacer ORE: el join interno (indexado) y, aparte, las zonas sin pareja con cero.
    "reescrita: interno + las que faltan": """
        with dentro as (select z.id, count(*) n from zonas z join puntos p on ST_Contains(z.zona, p.geom)
                        group by z.id)
        select id, n from dentro
        union all
        select z.id, 0 n from zonas z left anti join dentro d on z.id = d.id""",
}

LIMITE = int(os.environ.get("LIMITE_S", "300"))
# Una forma cancelada (un producto cartesiano) sigue comiendo CPU: para medir otra, sola (`SOLO=reescrita`).
SOLO = os.environ.get("SOLO", "")
for nombre, q in FORMAS.items():
    if SOLO and SOLO not in nombre:
        continue
    t = time.time()
    try:
        r = s.sql(q)
        plan = r._jdf.queryExecution().executedPlan().toString()
        join = ",".join(j for j in ("BroadcastIndexJoin", "RangeJoin", "BroadcastNestedLoopJoin", "CartesianProduct",
                                     "BroadcastHashJoin", "SortMergeJoin") if j in plan) or "otro"
        print("C3|%s|plan join=%s" % (nombre, join), flush=True)
        hecho = {}

        def correr():
            # El grupo es del hilo que lanza el trabajo (propiedad local de Spark): aquí, no fuera.
            s.sparkContext.setJobGroup(nombre, nombre, True)
            try:
                hecho["f"] = r.selectExpr("count(*) zonas", "sum(n) pares", "sum(if(n = 0, 1, 0)) vacias").first()
            except Exception as e:  # noqa: BLE001
                hecho["e"] = e

        h = threading.Thread(target=correr, daemon=True)
        h.start()
        h.join(LIMITE)
        if h.is_alive():
            s.sparkContext.cancelJobGroup(nombre)
            h.join(60)
            print("C3|%s|LIMITE >%d s" % (nombre, LIMITE), flush=True)
            continue
        if "e" in hecho:
            raise hecho["e"]
        f = hecho["f"]
        print("C3|%s|join=%s s=%.1f zonas=%s pares=%s vacias=%s" % (
            nombre, join, time.time() - t, f.zonas, f.pares, f.vacias), flush=True)
    except Exception as e:  # noqa: BLE001
        texto = "".join(traceback.format_exception_only(type(e), e))
        causa = [l.strip() for l in texto.splitlines() if "Exception" in l or "Error" in l or "Caused" in l]
        print("C3|%s|ERROR s=%.1f %s" % (nombre, time.time() - t, " ‖ ".join(causa[:4])[:600]), flush=True)
