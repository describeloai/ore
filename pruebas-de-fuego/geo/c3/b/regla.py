"""ORE 0068 C3·b · La regla `ore.spark.ReglasDeOre` (el LEFT JOIN espacial → la forma de Sedona), en local con
pocos datos: el plan, las cuentas contra la forma de referencia y lo que no debe tocar. Una línea por caso
y al final `n/m casos`.

    bash pruebas-de-fuego/geo/c3/b/regla.sh
"""
import os

from pyspark.sql import SparkSession
from pyspark.sql import functions as F

N = int(os.environ.get("PUNTOS", "200000"))
s = (SparkSession.builder.master("local[*]").appName("c3b-regla")
     .config("spark.driver.memory", "2g").config("spark.sql.shuffle.partitions", "8")
     .config("spark.driver.extraClassPath", "/l/ore-spark-reglas.jar")
     .config("spark.sql.extensions", "org.apache.iceberg.spark.extensions.IcebergSparkSessionExtensions,"
             "org.apache.sedona.sql.SedonaSqlExtensions,ore.spark.ReglasDeOre")
     .config("spark.sql.autoBroadcastJoinThreshold", "-1")   # que nada se difunda sin marca
     .getOrCreate())
s.sql("select id, ST_SetSRID(ST_Point(400000 + rand(1) * 10000, 4430000 + rand(2) * 10000), 25830) geom "
      "from range(%d)" % N).write.mode("overwrite").parquet("/tmp/b/puntos")
s.sql("select id, ST_SetSRID(ST_Buffer(ST_Point(400000 + (id % 44) * 227.0, 4430000 + floor(id / 44) * 227.0"
      " + if(id >= 2000, 50000, 0)), 100), 25830) zona from range(2200)").write.mode("overwrite").parquet("/tmp/b/zonas")
# Sedona guarda en Parquet su tipo de geometría: se lee ya como geometría.
puntos = s.read.parquet("/tmp/b/puntos")
zonas = s.read.parquet("/tmp/b/zonas")
puntos.createOrReplaceTempView("puntos")
zonas.createOrReplaceTempView("zonas")
F.broadcast(zonas).createOrReplaceTempView("zonas_dif")

casos = []


def caso(nombre, ok, detalle):
    casos.append(ok)
    print("%s %s · %s" % ("OK   " if ok else "FALLA", nombre, detalle), flush=True)


def plan(df):
    p = df._jdf.queryExecution().executedPlan().toString()
    return [j for j in ("BroadcastIndexJoin", "RangeJoin", "BroadcastNestedLoopJoin") if j in p]


def reescrita(df):
    return "__ore_k" in df._jdf.queryExecution().analyzed().toString()


Q10 = "select z.id, count(p.id) n from zonas z left join puntos p on ST_Contains(z.zona, p.geom) group by z.id"
REF = """with d as (select z.id zid, p.id pid from zonas z join puntos p on ST_Contains(z.zona, p.geom))
         select z.id, count(d.pid) n from zonas z left join d on z.id = d.zid group by z.id"""


def cuentas(df):
    f = df.selectExpr("count(*) zonas", "sum(n) pares", "sum(if(n = 0, 1, 0)) vacias").first()
    return (f.zonas, f.pares, f.vacias)


q10, ref = s.sql(Q10), s.sql(REF)
caso("1 · Q10 se reescribe y usa el índice", reescrita(q10) and "BroadcastNestedLoopJoin" not in plan(q10),
     "%s %s" % (reescrita(q10), plan(q10)))
c, r = cuentas(q10), cuentas(ref)
# Vacías: las 200 fuera de la caja y alguna de la rejilla sin ningún punto al azar.
caso("2 · y da lo mismo que la forma de Sedona a mano", c == r and c[0] == 2200 and c[2] >= 200, "%s · %s" % (c, r))

todo = s.sql("select * from zonas z left join puntos p on ST_Contains(z.zona, p.geom)")
filas = todo.count()
caso("3 · select * conserva sus columnas y sus filas", todo.columns == ["id", "zona", "id", "geom"]
     and filas == r[1] + r[2], "%s · %d filas (pares + vacías = %d)" % (todo.columns, filas, r[1] + r[2]))

der = s.sql("select z.id, count(p.id) n from puntos p right join zonas z on ST_Contains(z.zona, p.geom) group by z.id")
caso("4 · RIGHT JOIN, lo mismo", reescrita(der) and cuentas(der) == r, "%s %s" % (plan(der), cuentas(der)))

dif = s.sql("select z.id, count(*) n from puntos p left join zonas_dif z on ST_Contains(z.zona, p.geom) group by z.id")
caso("5 · con el otro lado difundido no se toca (Sedona lo indexa)", not reescrita(dif) and "BroadcastIndexJoin" in plan(dif),
     "%s %s" % (reescrita(dif), plan(dif)))

normal = s.sql("select z.id, p.id from zonas z left join puntos p on z.id = p.id")
caso("6 · un LEFT JOIN sin predicado espacial no se toca", not reescrita(normal) and normal.count() == 2200,
     "%s" % reescrita(normal))

# ── C3·c · LadoGrandeALaDerecha: el lado con más filas, a la derecha del predicado ──────────────────
def optimizado(df):
    return df._jdf.queryExecution().optimizedPlan().toString()


dentro = "select count(*) n from puntos p join zonas z on %s"
pc = s.sql(dentro % "ST_Contains(z.zona, p.geom)")
caso("7 · ST_Contains(zona, punto) ya tiene el grande a la derecha: no se toca",
     "st_contains" in optimizado(pc).lower() and "st_within" not in optimizado(pc).lower(), optimizado(pc)[:160])
pw = s.sql(dentro % "ST_Within(p.geom, z.zona)")
caso("8 · ST_Within(punto, zona) se gira a ST_Contains(zona, punto), con los mismos pares",
     "st_contains" in optimizado(pw).lower() and pw.first().n == pc.first().n,
     "%s · %d/%d" % (optimizado(pw)[:120], pw.first().n, pc.first().n))
pi = s.sql(dentro % "ST_Intersects(p.geom, z.zona)")
caso("9 · ST_Intersects(punto, zona) se gira y da lo mismo",
     pi.first().n == pc.first().n, "%d · %s" % (pi.first().n, optimizado(pi)[:120]))
pd = s.sql("select count(*) n from puntos p join zonas_dif z on ST_Within(p.geom, z.zona)")
caso("10 · con un lado difundido no se gira", "st_within" in optimizado(pd).lower(), optimizado(pd)[:120])

# ── C3·e · DifundirLaSubconsulta: la geometría de una subconsulta escalar, como lado difundido ──────
SUB = "(select z.zona from zonas z where z.id = %d limit 1)"
REF2 = "select /*+ BROADCAST(z) */ count(*) n from puntos p join zonas z on ST_Intersects(p.geom, z.zona) where z.id = %d"
q2 = s.sql("select count(*) n from puntos p where ST_Intersects(p.geom, %s)" % (SUB % 7))
r2 = s.sql(REF2 % 7).first().n
caso("11 · Q2 (subconsulta con LIMIT 1) usa el índice y da lo mismo que q2d",
     "BroadcastIndexJoin" in plan(q2) and q2.first().n == r2 and r2 > 0, "%s · %d/%d" % (plan(q2), q2.first().n, r2))
q0 = s.sql("select count(*) n from puntos p where ST_Intersects(p.geom, %s)" % (SUB % -1))
caso("12 · una subconsulta sin filas da 0", q0.first().n == 0, "%s · %d" % (plan(q0), q0.first().n))
qo = s.sql("select count(*) n from puntos p where p.id < 10 or ST_Intersects(p.geom, %s)" % (SUB % 7))
caso("13 · dentro de un OR no se toca", "BroadcastIndexJoin" not in plan(qo), "%s" % plan(qo))
ql = s.sql("select count(*) n from puntos p where ST_Intersects(p.geom, (select z.zona from zonas z where z.id = 7))")
caso("14 · sin cota de filas (sin LIMIT) no se toca, y da lo mismo",
     "BroadcastIndexJoin" not in plan(ql) and ql.first().n == r2, "%s · %d" % (plan(ql), ql.first().n))
qa = s.sql("select count(*) n from puntos p where p.id %% 2 = 0 and ST_Intersects(p.geom, %s)" % (SUB % 7))
ra = s.sql((REF2 % 7) + " and p.id % 2 = 0").first().n
caso("15 · con otra condición al lado se reescribe igual",
     "BroadcastIndexJoin" in plan(qa) and qa.first().n == ra, "%s · %d/%d" % (plan(qa), qa.first().n, ra))

print("\n%d/%d casos" % (sum(casos), len(casos)))
