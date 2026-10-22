"""ORE 0072 E2·0 · Previo A (T2, T4 y T1 con DuckDB): qué lee y cuánto tarda una tesela MVT sobre el lago
real de victor. Corre en un Job de t-victor (imagen del puesto, credencial del agente) en el pool pesado;
lo lanza `correr.py`. Emite «E2|…».

Capas, escritas con `ore.write` (el escritor de Rust: Hilbert, grupos de 100 000 filas, bbox por grupo):
  pts_h     N puntos en EPSG:25830, con forma de España (núcleos urbanos), de una vez
  pts_a     los mismos, en 10 `append` barajados (el orden se degrada)
  pts_m     los mismos en EPSG:3857 (T4: sin reproyectar al servir)
  edif      hasta 5 M de edificios (polígonos, EPSG:4326) de los 34 M del banco (g22)
Y, fuera del lago, `pts_s`: los puntos en un Parquet sin ordenar (lo que escribe Spark hoy).

Por capa y zoom (z4…z14, sobre Madrid y sobre un sitio vacío):
  - la tesela por el camino del SDK (`iceberg_scan` + `ST_GeomFromWKB`, como `_con_geo`) con `&&`;
  - la misma por `read_parquet` de sus ficheros (¿poda el camino del SDK?);
  - y por las columnas de cobertura x/y (estadísticas min/max de Parquet);
  - filas en la tesela, bytes del MVT, filas leídas por el scan, tiempo frío y caliente;
  - a zoom bajo, el agregado H3 en lugar de los puntos.
Concurrencia: 1, 8 y 32 lectores pidiendo teselas z10–z14 al azar.
"""
import concurrent.futures as cf
import json
import math
import os
import random
import statistics
import sys
import time
import traceback
import urllib.parse
import urllib.request

BANCO = "https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo"


def di(que, **kw):
    print("E2|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)


def paso(nombre):
    def deco(f):
        t0 = time.time()
        try:
            r = f()
            di(nombre, ok=True, s=round(time.time() - t0, 2), **(r or {}))
        except Exception as e:
            di(nombre, ok=False, s=round(time.time() - t0, 2), error=(type(e).__name__ + ": " + str(e))[:600],
               donde=traceback.format_exc().strip().splitlines()[-3][:200])
        return f
    return deco


def testigo():
    d = os.environ["DIRECCION"].rstrip("/")
    realm = os.environ.get("REALM", "rubix")
    cli = open("/puesto/agente-cliente").read().strip()
    sec = open("/puesto/agente-secreto").read().strip()
    datos = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": cli, "client_secret": sec}).encode()
    with urllib.request.urlopen(d + "/realms/%s/protocol/openid-connect/token" % realm, data=datos, timeout=20) as r:
        return {"authorization": "Bearer " + json.load(r)["access_token"]}


sys.path.insert(0, "/opt/ore")
os.environ.setdefault("ORE_SERVE", "http://ore-serve.%s.svc.cluster.local:8080" % os.environ["NS"])
import duckdb  # noqa: E402
import ore  # noqa: E402

ore.session._cabeceras = testigo()
ore.session.id = ""

# Sin crear nada en el catálogo (0052: sólo una persona crea un dataset): el escritor de Rust de
# siempre (`ore-store-gcs escribir`) deja los ficheros en el lago del inquilino, con la credencial que
# presta el `stage-create`, y el mismo binario (`aplicar`) escribe el `metadata.json`. Ningún puntero del
# árbol los nombra: se leen por su `metadata_location` y se borran al acabar (LOCS).
import subprocess  # noqa: E402

_pedir = ore.session.pedir
LOCAL, STAGED, LOCS = {}, {}, []


def pedir(metodo, ruta, cuerpo=None, *a, **kw):
    if "/tables/medida_e2_" in ruta:
        if metodo == "GET":
            if ruta in LOCAL:
                l = LOCAL[ruta]
                return 200, {"metadata-location": l["ml"], "config": l["config"], "metadata": {"location": l["loc"]}}
            return 404, {"error": {"message": "sin crear"}}
        if metodo == "POST":
            cfg, loc = (LOCAL[ruta]["config"], LOCAL[ruta]["loc"]) if ruta in LOCAL else STAGED[ruta]
            binario, env = ore._ore_store(cfg, loc)
            pet = {"dataset": "medida/" + ruta.rsplit("/", 1)[1], "requirements": cuerpo["requirements"], "updates": cuerpo["updates"]}
            if ruta in LOCAL:
                pet["metadata_location"] = LOCAL[ruta]["ml"]
            p = subprocess.run([binario, "aplicar"], input=json.dumps(pet).encode() + b"\n", capture_output=True, env=env)
            if p.returncode != 0:
                return 400, {"error": {"message": "aplicar: " + p.stderr.decode("utf-8", "replace")[-400:]}}
            out = json.loads(p.stdout.decode("utf-8"))
            ml = out.get("metadata_location") or out.get("metadata-location")
            if not ml:
                return 400, {"error": {"message": "aplicar sin metadata_location: " + str(list(out))}}
            LOCAL[ruta] = {"ml": ml, "config": cfg, "loc": loc}
            return 200, {"metadata-location": ml, "metadata": {"current-snapshot-id": out.get("snapshot")}}
    if metodo == "POST" and ruta.endswith("/tables") and isinstance(cuerpo, dict) and cuerpo.get("stage-create") \
            and str(cuerpo.get("name", "")).startswith("medida_e2_"):
        c, r = _pedir(metodo, ruta, cuerpo, *a, **kw)
        if c == 200:
            STAGED[ruta + "/" + cuerpo["name"]] = (r.get("config", {}), r["metadata"]["location"])
            LOCS.append(r["metadata"]["location"])
        return c, r
    return _pedir(metodo, ruta, cuerpo, *a, **kw)


ore.session.pedir = pedir
_, r = ore.session.pedir("GET", "/v1/namespaces")
NS = sorted(n[0] for n in r["namespaces"])[0]
di("inicio", ns=NS, cpus=os.cpu_count())

con = ore._duckdb()
for e in ("spatial", "h3", "httpfs"):
    con.execute("load " + e)
con.execute("set geometry_always_xy = true")
con.execute("set arrow_large_buffer_size = true")  # LargeBinary, como guarda el escritor: los `append` casan
con.execute("create or replace secret banco (type http, bearer_token '%s', scope '%s')" % (
    ore._token_de_google().replace("'", "''"), BANCO))

ML = {}  # capa -> metadata_location
CRS = {"pts_h": "EPSG:25830", "pts_m": "EPSG:3857", "edif": "EPSG:4326", **{f"pts_a{k}": "EPSG:25830" for k in range(10)}}


# ── ORE 0072 E2·0 · Previo F (T11b): los 600 M de viajes de SpatialBench, agregados por celda H3 (res 2–5 en todo el
# planeta; res 6, res 7 y los puntos en tres zonas densas), escritos al lago sin catálogo (como previo A) para
# bajarlos y teselarlos fuera. Emite «E2|…».
TRIP = BANCO + "/spatialbench/sf100/trip.parquet"
con.execute("set memory_limit = '40GB'"); con.execute("set threads = 8")
ZONAS = {"africa": (7.0, 19.0, 10.0, 22.0), "china": (44.0, 116.0, 46.0, 118.0), "sinai": (28.0, 33.0, 30.0, 36.0)}
zona_sql = " or ".join(f"(y between {a} and {c} and x between {b} and {d})" for a, b, c, d in ZONAS.values())

@paso("leer")
def _():
    con.execute(f"create table p as select ST_X(g)::double x, ST_Y(g)::double y, t_fare::double tarifa from (select ST_GeomFromWKB(t_pickuploc) g, t_fare from read_parquet('{TRIP}'))")
    return {"filas": con.execute("select count(*) from p").fetchone()[0]}

def tabla(nombre, sql):
    t = con.execute(sql).fetch_arrow_table()
    ore.session._cabeceras = testigo()  # el token del agente dura minutos: uno nuevo para cada escritura
    e = ore.write(NS + ".medida_e2_" + nombre, t)
    ML[nombre] = e["metadata_location"]
    return {"filas": t.num_rows}

SOLO = os.environ.get("E2_SOLO", "")
for res in (() if SOLO == "puntos" else (2, 3, 4, 5)):
    paso(f"agg_r{res}")(lambda res=res: tabla(f"sb_r{res}", f"select h3_latlng_to_cell(y, x, {res})::bigint celda, count(*)::bigint n, avg(tarifa) tarifa from p group by 1"))
for res in (() if SOLO == "puntos" else (6, 7)):
    paso(f"agg_r{res}")(lambda res=res: tabla(f"sb_r{res}", f"select h3_latlng_to_cell(y, x, {res})::bigint celda, count(*)::bigint n, avg(tarifa) tarifa from p where {zona_sql} group by 1"))
paso("puntos")(lambda: tabla("sb_puntos", f"select x, y, tarifa from p where {zona_sql}"))
di("fin", ml=ML, locs=LOCS)
