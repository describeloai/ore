"""ORE 0068 C4·2–C4·3 · El cotejo: el SQL que genera ORE, corrido sobre los datos de PostGIS (`/x/*.csv`), contra
la respuesta de PostGIS (`/x/postgis.csv`), fila a fila. `MOTOR=duckdb` (por defecto: `/x/enrich.sql` en el
DuckDB de la imagen del puesto) o `MOTOR=spark` (`/x/enrich-spark.sql` en Spark + Sedona de la imagen pesada,
con su `spark-defaults.conf` y el objetivo en 8 particiones: el número de fila de `monotonically_increasing_id`
tiene que ser el mismo en cada CTE que lo lee).
Los números, con tolerancia relativa (`TOL`, 1e-6): el buffer lo calcula cada motor a su manera.
"""
import csv
import math
import os
import re
import sys
import time

TOL = float(os.environ.get("TOL", "1e-6"))
MOTOR = os.environ.get("MOTOR", "duckdb")
TABLAS = (("municipios", "geom"), ("secciones", "geom"), ("competidores", "geom"), ("clientes", "ubicacion"))


def consulta(fichero):
    # La sentencia del guion → su consulta, sobre las tablas de aquí.
    sql = open(fichero).read()
    sql = re.sub(r"^create or replace dataset \S+ as\s*", "", sql)
    return re.sub(r"\bventas\.(\w+)", r"\1", sql)


def con_duckdb():
    import duckdb
    c = duckdb.connect()
    c.execute("set extension_directory = '/opt/ore/duckdb'")
    c.execute("load spatial")
    for t, geo in TABLAS:
        c.execute(f"create table {t} as select * replace (ST_GeomFromHEXEWKB({geo}) as {geo}) "
                  f"from read_csv('/x/{t}.csv', header = true)")
    cur = c.execute(consulta("/x/enrich.sql"))
    cols = [d[0] for d in cur.description]
    return cols, cur.fetchall()


def con_spark():
    from pyspark.sql import SparkSession
    s = SparkSession.builder.master("local[4]").appName("cotejo").getOrCreate()
    s.sparkContext.setLogLevel("ERROR")
    for t, geo in TABLAS:
        df = s.read.option("header", True).option("inferSchema", True).csv(f"/x/{t}.csv")
        df = df.selectExpr("* except (%s)" % geo, "ST_SetSRID(ST_GeomFromWKB(unhex(%s)), 25830) %s" % (geo, geo))
        if t == "clientes":
            df = df.repartition(8)
        df.createOrReplaceTempView(t)
    df = s.sql(consulta("/x/enrich-spark.sql"))
    plan = df._jdf.queryExecution().executedPlan().toString()
    joins = sorted({j for j in ("BroadcastIndexJoin", "RangeJoin", "DistanceJoin", "BroadcastNestedLoopJoin",
                                "SortMergeJoin", "BroadcastHashJoin", "CartesianProduct") if j in plan})
    print("COT|spark|joins=%s" % ",".join(joins))
    filas = [tuple(r) for r in df.collect()]
    return df.columns, filas


t0 = time.time()
cols, filas = con_spark() if MOTOR == "spark" else con_duckdb()
ore = {r[0]: dict(zip(cols, r)) for r in filas}
print("COT|%s|%.1f s|%d filas (%d distintas)" % (MOTOR, time.time() - t0, len(filas), len(ore)))

pg = {r["cliente_id"]: r for r in csv.DictReader(open("/x/postgis.csv"))}
if set(pg) != set(ore) or len(filas) != len(ore):
    print("COT|⛔ filas distintas: postgis %d, ore %d (%d distintas)" % (len(pg), len(filas), len(ore)))
    sys.exit(1)


def igual(a, b):
    if b in ("", None):
        return a is None
    if a is None:
        return False
    try:
        x, y = float(a), float(b)
    except (TypeError, ValueError):
        return str(a) == b
    return math.isclose(x, y, rel_tol=TOL, abs_tol=TOL)


malas = {}
fila = next(iter(pg.values()))
for k, f in pg.items():
    for col, v in f.items():
        if not igual(ore[k][col], v):
            malas.setdefault(col, []).append((k, ore[k][col], v))
for col in fila:
    n = len(malas.get(col, []))
    no_nulas = sum(1 for f in pg.values() if f[col] != "")
    print("COT|%s|%s|%s|%d distintas de %d (%d con valor)" % (MOTOR, "bien" if n == 0 else "MAL", col, n, len(pg),
                                                           no_nulas))
    for k, a, b in malas.get(col, [])[:3]:
        print("COT|  %s: ore=%r postgis=%r" % (k, a, b))
sys.exit(1 if malas else 0)
