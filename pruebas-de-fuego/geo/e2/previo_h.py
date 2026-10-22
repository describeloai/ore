"""ORE 0072 F1·0 · Previo H (incógnita 3): cómo escribe Spark para que el lago se pode. Spark guarda la
geometría como `binary`, sin la caja por grupo que deja `ore-store`, así que la poda va por las columnas de
cobertura (previo G). Aquí: con qué orden y qué tamaño de grupo. 20 M de puntos (EPSG:25830, los núcleos de
previo A/G) en un catálogo Iceberg local (`hadoop`, en el disco del pod), de cuatro maneras:

  sin_orden      como escribe hoy `ore/spark.py` (`writeTo(...).append()`)
  hilbert        ordenados por la clave Hilbert de (x, y) y grupos de ~100 000 filas (como `ore-store`)
  hilbert_serie  el mismo orden con el tamaño de grupo de serie (128 MB)
  zorder         sin orden y luego `rewrite_data_files` con `zorder(x, y)` de Iceberg

Por variante: lo que tarda en escribirse y, para la tesela vacía y Madrid (z10, z12, z14), cuántos grupos de
filas (y filas) tendría que leer quien pode por las estadísticas min/max de `x` e `y` de cada grupo. Emite
«E2H|…». Corre en `puesto-spark:1` en `cola-pesada` (`correr.py previo_h`).
"""
import glob
import json
import os
import time

import numpy as np
import pyarrow.parquet as pq


def di(que, **kw):
    print("E2H|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)


N = int(os.environ.get("E2_N", "20000000"))
mb = int(os.environ.get("ORE_MEMORIA_MB", "40960"))
from pyspark.sql import SparkSession  # noqa: E402
from pyspark.sql import functions as F  # noqa: E402
from pyspark.sql.functions import pandas_udf  # noqa: E402

s = (SparkSession.builder.master("local[*]").appName("previo-h")
     .config("spark.driver.memory", "%dm" % max(1024, int((mb - 1536) * 0.8)))
     .config("spark.sql.shuffle.partitions", "64")
     .config("spark.sql.catalog.loc", "org.apache.iceberg.spark.SparkCatalog")
     .config("spark.sql.catalog.loc.type", "hadoop")
     .config("spark.sql.catalog.loc.warehouse", "file:///tmp/wh")
     .getOrCreate())
di("inicio", n=N, nucleos=s.sparkContext.defaultParallelism, spark=s.version)
s.sql("create namespace if not exists loc.f1")

NUCLEOS = [(440000, 4474000, 30, 12000), (430000, 4581000, 18, 9000), (726000, 4373000, 9, 7000),
           (235000, 4142000, 8, 7000), (505000, 4790000, 6, 5000), (675000, 4615000, 6, 6000),
           (373000, 4065000, 5, 6000), (546000, 4801000, 4, 5000), (240000, 4815000, 3, 6000),
           (300000, 4400000, 11, 150000)]
tot = sum(n[2] for n in NUCLEOS)
acum, filas = 0.0, []
for x, y, w, sd in NUCLEOS:
    filas.append((acum, acum + w / tot, float(x), float(y), float(sd)))
    acum += w / tot
s.createDataFrame(filas, "lo double, hi double, cx double, cy double, sd double").createOrReplaceTempView("nucleos")
t0 = time.time()
s.sql(f"""
  select r.id, cast(rand(4) * 8 as int) cat, rand(5) * 100 val,
         n.cx + n.sd * sqrt(-2 * ln(greatest(r.a, 1e-12))) * cos(2 * pi() * r.b) x,
         n.cy + n.sd * sqrt(-2 * ln(greatest(r.a, 1e-12))) * sin(2 * pi() * r.b) y
  from (select id, rand(1) u, rand(2) a, rand(3) b from range({N})) r
  join nucleos n on r.u >= n.lo and r.u < n.hi
""").selectExpr("id", "cat", "val", "x", "y", "ST_AsBinary(ST_SetSRID(ST_Point(x, y), 25830)) geom") \
    .cache().createOrReplaceTempView("puntos")
di("datos", filas=s.table("puntos").count(), s=round(time.time() - t0, 1))

ext = s.sql("select min(x) x0, min(y) y0, max(x) x1, max(y) y1 from puntos").first()
X0, Y0, X1, Y1 = ext["x0"], ext["y0"], ext["x1"], ext["y1"]


@pandas_udf("long")
def hilbert(x, y):
    """La clave Hilbert de (x, y) en una rejilla de 2^16 × 2^16 sobre la extensión (la de `ore-store`)."""
    n = 1 << 16
    xi = np.clip(((x.to_numpy() - X0) / max(X1 - X0, 1e-9) * (n - 1)).astype(np.int64), 0, n - 1)
    yi = np.clip(((y.to_numpy() - Y0) / max(Y1 - Y0, 1e-9) * (n - 1)).astype(np.int64), 0, n - 1)
    d = np.zeros(len(xi), dtype=np.int64)
    k = n // 2
    while k > 0:
        rx = (xi & k) > 0
        ry = (yi & k) > 0
        d += k * k * ((3 * rx.astype(np.int64)) ^ ry.astype(np.int64))
        gira = ~ry
        voltea = gira & rx
        xi = np.where(voltea, n - 1 - xi, xi)
        yi = np.where(voltea, n - 1 - yi, yi)
        xi, yi = np.where(gira, yi, xi), np.where(gira, xi, yi)
        k //= 2
    import pandas as pd
    return pd.Series(d)


GRUPO_100K = str(4 * 1024 * 1024)  # ~100 000 filas de estos puntos por grupo (se comprueba abajo)


def escribir(nombre, df, props):
    t = time.time()
    s.sql(f"drop table if exists loc.f1.{nombre}")
    w = df.writeTo(f"loc.f1.{nombre}").using("iceberg")
    for k, v in props.items():
        w = w.tableProperty(k, v)
    w.create()
    return round(time.time() - t, 1)


base = s.table("puntos")
T = {}
T["sin_orden"] = escribir("sin_orden", base, {})
T["hilbert"] = escribir("hilbert", base.withColumn("_h", hilbert("x", "y")).orderBy("_h").drop("_h"),
                        {"write.parquet.row-group-size-bytes": GRUPO_100K})
T["hilbert_serie"] = escribir("hilbert_serie", base.withColumn("_h", hilbert("x", "y")).orderBy("_h").drop("_h"), {})
t = escribir("zorder", base, {"write.parquet.row-group-size-bytes": GRUPO_100K})
t2 = time.time()
r = s.sql("call loc.system.rewrite_data_files(table => 'f1.zorder', strategy => 'sort', "
          "sort_order => 'zorder(x, y)', options => map('rewrite-all', 'true'))").first()
T["zorder"] = round(t + time.time() - t2, 1)
di("escribir_s", **T, zorder_reescritos=list(r.asDict().values()) if r else None)

# Las cajas de las teselas, en 25830 (las de previo G: vacía y Madrid)
import math  # noqa: E402


def tesela_de(lon, lat, z):
    n = 2 ** z
    return int((lon + 180) / 360 * n), int((1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n)


def caja(z, x, y):
    n = 2 ** z
    lon0, lon1 = x / n * 360 - 180, (x + 1) / n * 360 - 180
    lat1 = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * y / n))))
    lat0 = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * (y + 1) / n))))
    q = s.sql(f"""select ST_Transform(ST_SetSRID(ST_PolygonFromEnvelope({lon0}, {lat0}, {lon1}, {lat1}), 4326),
                  'EPSG:4326', 'EPSG:25830') g""").selectExpr("ST_XMin(g) a", "ST_YMin(g) b", "ST_XMax(g) c", "ST_YMax(g) d").first()
    return q["a"], q["b"], q["c"], q["d"]


CAJAS = {}
for sitio, (lon, lat) in (("vacio", (-1.0, 36.6)), ("madrid", (-3.7038, 40.4168))):
    for z in (10, 12, 14):
        CAJAS[(sitio, z)] = caja(z, *tesela_de(lon, lat, z))

for nombre in T:
    fs = [p.replace("file:", "") for (p,) in s.sql(f"select file_path from loc.f1.{nombre}.files").collect()]
    grupos = []  # (filas, xmin, xmax, ymin, ymax) por grupo
    for f in fs:
        md = pq.ParquetFile(f).metadata
        cols = {md.schema.column(i).name: i for i in range(md.num_columns)}
        for g in range(md.num_row_groups):
            rg = md.row_group(g)
            sx, sy = rg.column(cols["x"]).statistics, rg.column(cols["y"]).statistics
            grupos.append((rg.num_rows, sx.min, sx.max, sy.min, sy.max))
    filas_g = [g[0] for g in grupos]
    di("forma", var=nombre, ficheros=len(fs), grupos=len(grupos), filas_por_grupo=int(np.median(filas_g)),
       mb=round(sum(os.path.getsize(f) for f in fs) / 2**20))
    for (sitio, z), (a, b, c, d) in CAJAS.items():
        toca = [g for g in grupos if g[2] >= a and g[1] <= c and g[4] >= b and g[3] <= d]
        di("poda", var=nombre, sitio=sitio, z=z, grupos_leidos=len(toca), de=len(grupos),
           filas_leidas=sum(g[0] for g in toca))
di("fin")
