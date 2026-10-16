"""vara-v4.py (ADR 0067 G0·3, caso V4): leer por zona, en un puesto de victor.

Hoy no hay geometría en el lago de ORE, así que la línea base es **el techo de fuera**: DuckDB del puesto
sobre los 34 M de edificios de Overture en el bucket (GeoParquet tal como lo publica Overture, recortado),
filtrando por la columna `bbox`. Mide, por zona (ocho ciudades), cuántos grupos de filas se descartan
por las estadísticas del `bbox` y cuánto tarda, con 1, 8 y 32 lectores concurrentes (p50 y p95).
Lee por HTTPS con el token de la cuenta del puesto (servidor de metadatos). Emite líneas «V4|…».
"""
import concurrent.futures as cf
import json
import statistics
import time
import urllib.request

import duckdb

URL = ("https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo/"
       "vectorial/overture/2026-09-23.1/edificios_es.parquet")
ZONAS = {  # xmin, ymin, xmax, ymax: el centro de cada ciudad, unos 8 × 8 km
    "madrid": (-3.75, 40.38, -3.65, 40.46), "barcelona": (2.10, 41.35, 2.22, 41.45),
    "valencia": (-0.42, 39.43, -0.33, 39.51), "sevilla": (-6.03, 37.35, -5.94, 37.42),
    "bilbao": (-2.97, 43.23, -2.90, 43.29), "zaragoza": (-0.93, 41.62, -0.84, 41.69),
    "malaga": (-4.47, 36.69, -4.38, 36.75), "coruna": (-8.44, 43.34, -8.37, 43.39),
}
r = urllib.request.Request("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token",
                           headers={"Metadata-Flavor": "Google"})
TOKEN = json.load(urllib.request.urlopen(r))["access_token"]


def conexion():
    c = duckdb.connect()
    c.execute("set extension_directory = '/opt/ore/duckdb'")  # donde la imagen del puesto las preinstala
    c.execute("load httpfs")
    c.execute(f"create secret g (type http, bearer_token '{TOKEN}')")
    return c


def consulta(z):
    x0, y0, x1, y1 = ZONAS[z]
    return (f"select count(*) from read_parquet('{URL}') "
            f"where bbox.xmax >= {x0} and bbox.xmin <= {x1} and bbox.ymax >= {y0} and bbox.ymin <= {y1}")


c = conexion()
t0 = time.time()
md = c.execute(f"""select row_group_id, path_in_schema, stats_min, stats_max
                   from parquet_metadata('{URL}') where path_in_schema in ('bbox, xmin', 'bbox, xmax', 'bbox, ymin', 'bbox, ymax')""").fetchall()
grupos = {}
for g, col, lo, hi in md:
    grupos.setdefault(g, {})[col.split(", ")[1]] = (float(lo), float(hi))
print(f"V4|metadatos|-|-|{len(grupos)} grupos de filas, leídos en {time.time() - t0:.1f} s")
for z, (x0, y0, x1, y1) in ZONAS.items():
    tocan = sum(1 for s in grupos.values()
                if s["xmax"][1] >= x0 and s["xmin"][0] <= x1 and s["ymax"][1] >= y0 and s["ymin"][0] <= y1)
    t0 = time.time()
    n = c.execute(consulta(z)).fetchone()[0]
    print(f"V4|{z}|1 lector|-|{n} edificios · grupos que tocan la zona {tocan}/{len(grupos)} "
          f"(descarte {100 * (1 - tocan / len(grupos)):.1f} %) · {time.time() - t0:.2f} s")


def una(z):
    cc = conexion()
    t0 = time.time()
    cc.execute(consulta(z)).fetchone()
    return time.time() - t0


for lectores in (1, 8, 32):
    zs = [list(ZONAS)[i % len(ZONAS)] for i in range(max(16, lectores * 2))]
    t0 = time.time()
    with cf.ThreadPoolExecutor(lectores) as ex:
        ts = sorted(ex.map(una, zs))
    p95 = ts[min(len(ts) - 1, int(0.95 * len(ts)))]
    print(f"V4|concurrencia|{lectores} lectores|-|{len(ts)} consultas en {time.time() - t0:.1f} s · "
          f"p50 {statistics.median(ts):.2f} s · p95 {p95:.2f} s")
print("## ok")
