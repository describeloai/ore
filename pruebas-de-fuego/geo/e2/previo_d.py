"""ORE 0072 E2·0 · Previo D (T1 y T2, DuckDB con memoria): ¿por qué DuckDB lee la capa entera en cada
tesela? y DuckDB en su mejor caso frente al motor de Rust (previo B). Sobre las capas de `previo_a.py`
(`E2_ML_H`, `E2_ML_M`), con 30 GB de memoria y la caché de pies encendida. Emite «E2E|…».

Por variante del filtro, en una tesela vacía y en Madrid (z10, z12, z14): tiempo, y qué filtro llega al
scan (el `extra_info` del perfil: si `&&` se empuja a `READ_PARQUET`, poda por el bbox de cada grupo).
"""
import concurrent.futures as cf
import json
import math
import os
import random
import statistics
import sys
import time

sys.path.insert(0, "/opt/ore")
os.environ.setdefault("ORE_SERVE", "http://ore-serve.%s.svc.cluster.local:8080" % os.environ.get("NS", "t-victor"))
import ore  # noqa: E402

SEG = int(os.environ.get("E2_SEGUNDOS", "40"))


def di(que, **kw):
    print("E2E|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)


con = ore._duckdb()
for e in ("spatial", "httpfs"):
    con.execute("load " + e)
con.execute("set geometry_always_xy = true")
con.execute("set memory_limit = '30GB'")
con.execute("set threads = 8")
con.execute("set parquet_metadata_cache = true")
con.execute("set enable_http_metadata_cache = true")


def ficheros(ml):
    meta = ore._iceberg(ml).replace("iceberg_scan(", "iceberg_metadata(", 1)
    f = con.execute(f"select * from {meta}").fetchdf()
    return [p.replace("gs://", "https://storage.googleapis.com/") for p, st in zip(f["file_path"], f["status"])
            if str(p).endswith(".parquet") and str(st) != "DELETED"]


H = ficheros(os.environ["E2_ML_H"])
SCAN_H = ore._iceberg(os.environ["E2_ML_H"])
di("tipos", columnas=[list(map(str, r)) for r in con.execute(f"describe select * from read_parquet('{H[0]}')").fetchall()])
di("geo_meta", r=[list(map(str, r)) for r in con.execute(
    f"select row_group_id, path_in_schema, stats_min, stats_max, geo_bbox from parquet_metadata('{H[0]}') where path_in_schema = 'geom' limit 3").fetchall()])


def tesela_de(lon, lat, z):
    n = 2 ** z
    return int((lon + 180) / 360 * n), int((1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n)


def caja(z, x, y):
    return f"ST_Transform(ST_TileEnvelope({z}, {x}, {y}), 'EPSG:3857', 'EPSG:25830', true)"


lista = ", ".join("'%s'" % f for f in H)
VARIANTES = {
    # la columna tal como la lee read_parquet (GEOMETRY del fichero), sin envolver
    "parquet_crudo": lambda c: f"from read_parquet([{lista}]) where geom && {c}",
    # la caja como BOX_2D constante (lo que la poda por estadísticas compara)
    "parquet_extent": lambda c: f"from read_parquet([{lista}]) where ST_Intersects_Extent(geom, (select {c}))",
    # envuelta en el cast con CRS, como en previo A
    "parquet_cast": lambda c: f"from (select * replace (geom::GEOMETRY('EPSG:25830') as geom) from read_parquet([{lista}])) where geom && {c}",
    # el camino del SDK: iceberg_scan + ST_GeomFromWKB
    "sdk": lambda c: f"from (select * replace (ST_GeomFromWKB(geom)::GEOMETRY('EPSG:25830') as geom) from {SCAN_H}) where geom && {c}",
    # las columnas de cobertura (min/max de x, y por grupo)
    "cobertura": lambda c: f"from read_parquet([{lista}]) where x between ST_XMin({c}) and ST_XMax({c}) and y between ST_YMin({c}) and ST_YMax({c})",
}


def consulta(var, z, x, y):
    env = f"ST_TileEnvelope({z}, {x}, {y})"
    return (f"select ST_AsMVT({{'geom': ST_AsMVTGeom(ST_Transform(geom, 'EPSG:25830', 'EPSG:3857', true), ST_Extent({env}), 4096, 64, true), 'id': id}}, 'capa', 4096, 'geom') mvt, count(*) n "
            + VARIANTES[var](caja(z, x, y)))


def filtro_empujado(q):
    try:
        p = json.loads(con.execute("explain (analyze, format json) " + q).fetchall()[0][1])
        p = p[0] if isinstance(p, list) else p
        out = []

        def r(n):
            if n.get("operator_type") == "TABLE_SCAN":
                out.append({k: v for k, v in (n.get("extra_info") or {}).items() if k in ("Filters", "Function", "Total Files Read", "Scanning Files")})
            for h in n.get("children", []):
                r(h)
        r(p)
        return out
    except Exception as e:
        return str(e)[:200]


MADRID = (-3.7038, 40.4168)
VACIO = (-1.0, 36.6)
for var in VARIANTES:
    for sitio, (lon, lat) in (("vacio", VACIO), ("madrid", MADRID)):
        for z in (10, 12, 14):
            x, y = tesela_de(lon, lat, z)
            q = consulta(var, z, x, y)
            try:
                ts = []
                for _ in range(3):
                    t0 = time.time()
                    mvt, n = con.execute(q).fetchone()
                    ts.append(time.time() - t0)
                di("tesela", var=var, sitio=sitio, z=z, frio_ms=round(ts[0] * 1000), ms=round(min(ts[1:]) * 1000), filas=n,
                   kb=round(len(mvt or b"") / 1024, 1), **({"scan": filtro_empujado(q)} if z == 14 else {}))
            except Exception as e:
                di("tesela", var=var, sitio=sitio, z=z, error=str(e)[:300])

# La concurrencia con la mejor variante de las medidas de arriba (la que dicen los tiempos de la tesela vacía)
for var in ("parquet_crudo", "cobertura"):
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
                c.execute(consulta(var, z, x, y)).fetchone()
                ts.append(time.time() - t0)
            return ts
        try:
            with cf.ThreadPoolExecutor(lectores) as ex:
                todos = sorted(t for ts in ex.map(uno, range(lectores)) for t in ts)
            di("concurrencia", var=var, lectores=lectores, teselas=len(todos), por_s=round(len(todos) / SEG, 1),
               p50_ms=round(statistics.median(todos) * 1000), p95_ms=round(todos[max(int(len(todos) * 0.95) - 1, 0)] * 1000))
        except Exception as e:
            di("concurrencia", var=var, lectores=lectores, error=str(e)[:300])
di("fin")
