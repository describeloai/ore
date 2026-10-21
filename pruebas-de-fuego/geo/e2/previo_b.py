"""ORE 0072 E2·0 · Previo B (T1): el motor de teselas en Rust (`teselas/`, el binario en el bucket del
banco) frente a DuckDB en su mejor caso (los mismos ficheros por `read_parquet`, con la caché de pies y de
metadatos HTTP encendida), sobre las capas que dejó `previo_a.py` (`E2_ML_H`: puntos en 25830; `E2_ML_M`:
en 3857). Mismo nodo, mismas teselas, misma concurrencia. Emite «E2R|…» (Rust) y «E2D|…» (DuckDB).
"""
import concurrent.futures as cf
import json
import math
import os
import random
import statistics
import subprocess
import sys
import time
import urllib.request

sys.path.insert(0, "/opt/ore")
os.environ.setdefault("ORE_SERVE", "http://ore-serve.%s.svc.cluster.local:8080" % os.environ.get("NS", "t-victor"))
import ore  # noqa: E402

BANCO = "https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo"
SEG = int(os.environ.get("E2_SEGUNDOS", "40"))


def di(que, **kw):
    print("E2D|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)


tok = ore._token_de_google()
req = urllib.request.Request(BANCO + "/computo/e2/e2-teselas", headers={"authorization": "Bearer " + tok})
open("/tmp/e2-teselas", "wb").write(urllib.request.urlopen(req, timeout=120).read())
os.chmod("/tmp/e2-teselas", 0o755)

con = ore._duckdb()
for e in ("spatial", "httpfs"):
    con.execute("load " + e)
con.execute("set geometry_always_xy = true")


def ficheros(ml):
    scan = ore._iceberg(ml)  # el secreto de su raíz, con el token del pod
    meta = scan.replace("iceberg_scan(", "iceberg_metadata(", 1)
    f = con.execute(f"select * from {meta}").fetchdf()
    return [p for p, st in zip(f["file_path"], f["status"]) if str(p).endswith(".parquet") and str(st) != "DELETED"]


CAPAS = {"25830": ficheros(os.environ["E2_ML_H"]), "3857": ficheros(os.environ["E2_ML_M"])}
di("ficheros", **{k: len(v) for k, v in CAPAS.items()})

# ── Rust ──────────────────────────────────────────────────────────────────────
for crs, fs in CAPAS.items():
    env = dict(os.environ, E2R_FICHEROS=",".join(fs), E2R_CRS=crs, E2R_SEGUNDOS=str(SEG))
    p = subprocess.run(["/tmp/e2-teselas"], env=env, capture_output=True, text=True, timeout=1800)
    for l in p.stdout.splitlines():
        if l.startswith("E2R|"):
            k, v = l.split("|", 2)[1:]
            d = json.loads(v)
            d["crs"] = crs
            print("E2R|%s|%s" % (k, json.dumps(d)), flush=True)
    if p.returncode != 0:
        print("E2R|error|" + json.dumps({"crs": crs, "rc": p.returncode, "stderr": p.stderr[-800:]}), flush=True)

# ── DuckDB en su mejor caso ───────────────────────────────────────────────────
con.execute("set parquet_metadata_cache = true")
con.execute("set enable_http_metadata_cache = true")


def tesela_de(lon, lat, z):
    n = 2 ** z
    return int((lon + 180) / 360 * n), int((1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n)


def sql(fs, crs, z, x, y):
    env = f"ST_TileEnvelope({z}, {x}, {y})"
    lista = ", ".join("'%s'" % f.replace("gs://", "https://storage.googleapis.com/") for f in fs)
    caja = env if crs == "3857" else f"ST_Transform({env}, 'EPSG:3857', 'EPSG:{crs}', true)"
    g = "geom" if crs == "3857" else f"ST_Transform(geom, 'EPSG:{crs}', 'EPSG:3857', true)"
    return (f"select ST_AsMVT({{'geom': ST_AsMVTGeom({g}, ST_Extent({env}), 4096, 64, true), 'id': id}}, 'capa', 4096, 'geom') mvt, count(*) "
            f"from (select * replace (geom::GEOMETRY('EPSG:{crs}') as geom) from read_parquet([{lista}])) where geom && {caja}")


MADRID = (-3.7038, 40.4168)
for crs, fs in CAPAS.items():
    for z in (4, 6, 8, 10, 12, 14):
        x, y = tesela_de(*MADRID, z)
        t = []
        for _ in range(2):
            t0 = time.time()
            mvt, n = con.execute(sql(fs, crs, z, x, y)).fetchone()
            t.append(time.time() - t0)
        di("tesela", crs=crs, z=z, frio_ms=round(t[0] * 1000, 1), ms=round(t[1] * 1000, 1), filas=n, kb=round(len(mvt or b"") / 1024, 1))

    for lectores in (1, 8, 32, 64):
        def uno(k):
            c = con.cursor()
            rnd = random.Random(k)
            ts = []
            fin = time.time() + SEG
            while time.time() < fin:
                z = rnd.choice([10, 11, 12, 13, 14])
                x, y = tesela_de(MADRID[0] + rnd.uniform(-0.6, 0.6), MADRID[1] + rnd.uniform(-0.4, 0.4), z)
                t0 = time.time()
                c.execute(sql(fs, crs, z, x, y)).fetchone()
                ts.append(time.time() - t0)
            return ts

        with cf.ThreadPoolExecutor(lectores) as ex:
            todos = sorted(t for ts in ex.map(uno, range(lectores)) for t in ts)
        di("concurrencia", crs=crs, lectores=lectores, teselas=len(todos), por_s=round(len(todos) / SEG, 1),
           p50_ms=round(statistics.median(todos) * 1000), p95_ms=round(todos[max(int(len(todos) * 0.95) - 1, 0)] * 1000))
di("fin")
