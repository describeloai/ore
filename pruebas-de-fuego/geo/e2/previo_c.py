"""ORE 0072 E2·0 · Previo C (T3): dónde deja de llegar lo dinámico, sobre los 600 M de viajes de
SpatialBench sf100 (el banco, `trip.parquet`, sin orden espacial). Emite «E2C|…».

  1. Lo dinámico sobre 600 M sin orden: una tesela z12 donde más viajes hay, leyendo el fichero entero.
  2. Precalcular la pirámide H3 (res 8 de una pasada; 7…3 subiendo por `h3_cell_to_parent`): tiempo y filas.
  3. Servir desde la pirámide: una tesela agregada z4–z10 desde la tabla precalculada.
"""
import json
import math
import os
import sys
import time

sys.path.insert(0, "/opt/ore")
os.environ.setdefault("ORE_SERVE", "http://ore-serve.%s.svc.cluster.local:8080" % os.environ.get("NS", "t-victor"))
import ore  # noqa: E402

BANCO = "https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo"
TRIP = BANCO + "/spatialbench/sf100/trip.parquet"


def di(que, **kw):
    print("E2C|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)


con = ore._duckdb()
for e in ("spatial", "h3", "httpfs"):
    con.execute("load " + e)
con.execute("set geometry_always_xy = true")
con.execute("set memory_limit = '40GB'")
con.execute("create or replace secret banco (type http, bearer_token '%s', scope '%s')" % (ore._token_de_google(), BANCO))


def t(nombre, q, **extra):
    t0 = time.time()
    try:
        r = con.execute(q).fetchall()
        di(nombre, ok=True, s=round(time.time() - t0, 1), r=[list(map(str, x)) for x in r[:5]], **extra)
        return r
    except Exception as e:
        di(nombre, ok=False, s=round(time.time() - t0, 1), error=str(e)[:400], **extra)
        return None


t("metadatos", f"select count(*), sum(num_rows) from parquet_metadata('{TRIP}') group by all limit 1")
t("filas_y_grupos", f"select count(distinct row_group_id), max(row_group_num_rows) from parquet_metadata('{TRIP}')")

# 2 · La pirámide (antes que 1: dice dónde está lo denso)
r = t("piramide_res8", f"""
  create or replace table h8 as
  select h3_latlng_to_cell(ST_Y(p), ST_X(p), 8) c, count(*)::bigint n
  from (select ST_GeomFromWKB(t_pickuploc) p from read_parquet('{TRIP}')) group by 1""")
t("piramide_res8_filas", "select count(*), sum(n) from h8")
con.execute("create or replace table piramide as select 8 res, c, n from h8")
t0 = time.time()
for res in (7, 6, 5, 4, 3):
    con.execute(f"insert into piramide select {res}, h3_cell_to_parent(c, {res}), sum(n) from piramide where res = {res + 1} group by 2")
di("piramide_subir", s=round(time.time() - t0, 1),
   filas={str(k): v for k, v in con.execute("select res, count(*) from piramide group by 1 order by 1").fetchall()})

denso = con.execute("select h3_cell_to_lng(c), h3_cell_to_lat(c), n from h8 order by n desc limit 1").fetchone()
di("lo_mas_denso", lon=denso[0], lat=denso[1], viajes_res8=denso[2])


def tesela_de(lon, lat, z):
    n = 2 ** z
    return int((lon + 180) / 360 * n), int((1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n)


# 1 · Lo dinámico sobre 600 M sin orden (z12 y z8 donde más hay)
for z in (12, 8):
    x, y = tesela_de(denso[0], denso[1], z)
    env = f"ST_TileEnvelope({z}, {x}, {y})"
    t("dinamico_sin_orden", f"""
      select count(*), octet_length(ST_AsMVT({{'g': ST_AsMVTGeom(ST_Transform(p, 'EPSG:4326', 'EPSG:3857', true), ST_Extent({env}), 4096, 64, true)}}, 'capa', 4096, 'g'))
      from (select ST_GeomFromWKB(t_pickuploc) p from read_parquet('{TRIP}'))
      where p && ST_Transform({env}, 'EPSG:3857', 'EPSG:4326', true)""", z=z)

# 3 · Servir desde la pirámide: la resolución H3 que toca a cada zoom
for z, res in ((4, 3), (6, 4), (8, 5), (10, 7)):
    x, y = tesela_de(denso[0], denso[1], z)
    env = f"ST_TileEnvelope({z}, {x}, {y})"
    q = f"""
      select count(*), octet_length(ST_AsMVT({{'g': ST_AsMVTGeom(ST_Transform(ST_Point(h3_cell_to_lng(c), h3_cell_to_lat(c)), 'EPSG:4326', 'EPSG:3857', true),
                                                                ST_Extent({env}), 4096, 64, true), 'n': n}}, 'capa', 4096, 'g'))
      from piramide where res = {res}
        and ST_Intersects(ST_Point(h3_cell_to_lng(c), h3_cell_to_lat(c)), ST_Transform({env}, 'EPSG:3857', 'EPSG:4326', true))"""
    t("piramide_tesela", q, z=z, res=res)
    t("piramide_tesela_2", q, z=z, res=res)
di("fin")
