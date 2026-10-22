"""ORE 0072 F1·0 · Previo G: el sustrato. (1) Cómo leer el lago por el SDK sin que el envoltorio de la geometría
mate la poda: cinco variantes sobre 20 M de puntos (EPSG:25830) y 5 M de edificios (EPSG:4326) escritos por `ore.write`.
(2) De dónde sale el `Binary` que rompe el `append` de geometría. Escribe al lago sin catálogo (como previo A) y
lo borra al acabar. Emite «E2|…». Lo lanza `correr.py previo_g`.
"""
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

N = int(os.environ.get("E2_N", "20000000"))
N_EDIF = int(os.environ.get("E2_N_EDIF", "5000000"))
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
    if "/tables/medida_f1_" in ruta:
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
            and str(cuerpo.get("name", "")).startswith("medida_f1_"):
        c, r = _pedir(metodo, ruta, cuerpo, *a, **kw)
        if c == 200:
            STAGED[ruta + "/" + cuerpo["name"]] = (r.get("config", {}), r["metadata"]["location"])
            LOCS.append(r["metadata"]["location"])
        return c, r
    return _pedir(metodo, ruta, cuerpo, *a, **kw)


ore.session.pedir = pedir
_, r = ore.session.pedir("GET", "/v1/namespaces")
NS = sorted(n[0] for n in r["namespaces"])[0]
di("inicio", ns=NS, duckdb=duckdb.__version__, cpus=os.cpu_count(), n=N, n_edif=N_EDIF)

con = ore._duckdb()
for e in ("spatial", "h3", "httpfs"):
    con.execute("load " + e)
con.execute("set geometry_always_xy = true")
con.execute("set arrow_large_buffer_size = true")  # LargeBinary, como guarda el escritor: los `append` casan
con.execute("create or replace secret banco (type http, bearer_token '%s', scope '%s')" % (
    ore._token_de_google().replace("'", "''"), BANCO))

ML = {}  # capa -> metadata_location

# ── ORE 0072 F1·0 · Previo G: el SDK sin envoltorio que mate la poda, y el `append` de geometría ──────────
CRS = {"pts": "EPSG:25830", "edif": "EPSG:4326"}
NUCLEOS = [(440000, 4474000, 30, 12000), (430000, 4581000, 18, 9000), (726000, 4373000, 9, 7000),
           (235000, 4142000, 8, 7000), (505000, 4790000, 6, 5000), (675000, 4615000, 6, 6000),
           (373000, 4065000, 5, 6000), (546000, 4801000, 4, 5000), (240000, 4815000, 3, 6000),
           (300000, 4400000, 11, 150000)]
di("versiones", duckdb=duckdb.__version__,
   ext=[list(map(str, r)) for r in con.execute(
       "select extension_name, extension_version from duckdb_extensions() where loaded").fetchall()])


@paso("datos")
def _():
    pesos = [n[2] for n in NUCLEOS]
    con.execute("create or replace table nucleos (i int, x double, y double, s double, w double)")
    con.executemany("insert into nucleos values (?, ?, ?, ?, ?)",
                    [(i, n[0], n[1], n[3], n[2] / sum(pesos)) for i, n in enumerate(NUCLEOS)])
    con.execute(f"""
      create or replace table puntos as
      with acum as (select i, x, y, s, sum(w) over (order by i) hasta from nucleos),
      r as (select range id, random() u, random() a, random() b, (random() * 8)::int cat, random() * 100 val
            from range({N}))
      select r.id, cat, val,
             n.x + n.s * sqrt(-2 * ln(greatest(a, 1e-12))) * cos(2 * pi() * b) x,
             n.y + n.s * sqrt(-2 * ln(greatest(a, 1e-12))) * sin(2 * pi() * b) y
      from r asof join acum n on r.u <= n.hasta
    """)
    return {"filas": con.execute("select count(*) from puntos").fetchone()[0]}


def a_large(t):
    import pyarrow as pa
    i = t.schema.get_field_index("geom")
    f = t.schema.field(i)
    col = t.column(i)
    meta = dict(f.metadata or {})
    if isinstance(f.type, pa.ExtensionType):
        meta[b"ARROW:extension:name"] = f.type.extension_name.encode()
        meta[b"ARROW:extension:metadata"] = f.type.__arrow_ext_serialize__()
        col = pa.chunked_array([c.storage for c in col.chunks])
    col = col.cast(pa.large_binary())
    return t.set_column(i, pa.field("geom", pa.large_binary(), metadata=meta), col)


def escribir(nombre, rel_sql, modo="overwrite", large=True):
    t = con.execute(rel_sql).fetch_arrow_table()
    if large:
        t = a_large(t)
    ore.session._cabeceras = testigo()
    e = ore.write(NS + ".medida_f1_" + nombre, t, mode=modo)
    ML[nombre] = e["metadata_location"]
    return e


@paso("escribir_pts")
def _():
    # x, y: la cobertura de un punto es él mismo (x0 = x1 = x)
    e = escribir("pts", "select id, cat, val, x, y, ST_SetCRS(ST_Point(x, y), 'EPSG:25830') geom from puntos")
    return {"filas": e.get("rows")}


@paso("escribir_edif")
def _():
    e = escribir("edif", f"select id, ST_SetCRS(geom::GEOMETRY, 'EPSG:4326') geom, "
                         f"ST_XMin(geom) x0, ST_YMin(geom) y0, ST_XMax(geom) x1, ST_YMax(geom) y1 "
                         f"from read_parquet('{BANCO}/g22/a_tal_cual.parquet') "
                         f"where ST_Intersects(geom, ST_MakeEnvelope(-4.6, 39.8, -2.8, 41.0)) limit {N_EDIF}")
    return {"filas": e.get("rows")}


# ── Incógnita 2: el `append` de geometría y los tipos que llegan ─────────────────────────────────────────
@paso("append_tipos")
def _():
    q = ("select id, cat, 'n' || id::varchar nombre, ST_SetCRS(ST_Point(x, y), 'EPSG:25830') geom "
         "from puntos where id < 1000")
    con.execute("set arrow_large_buffer_size = false")  # lo de serie del SDK
    t = con.execute(q).fetch_arrow_table()
    llega = {f.name: str(f.type) for f in t.schema}
    con.execute("set arrow_large_buffer_size = true")
    ore.session._cabeceras = testigo()
    ore.write(NS + ".medida_f1_append", t)
    ML["append"] = None
    try:
        ore.session._cabeceras = testigo()
        ore.write(NS + ".medida_f1_append", t, mode="append")
        anexa, error = True, None
    except Exception as e:
        anexa, error = False, str(e)[:400]
    return {"llega": llega, "anexa": anexa, "error": error}


def ficheros(ml):
    meta = ore._iceberg(ml).replace("iceberg_scan(", "iceberg_metadata(", 1)
    f = con.execute(f"select * from {meta}").fetchdf()
    return [p.replace("gs://", "https://storage.googleapis.com/") for p, st in zip(f["file_path"], f["status"])
            if str(p).endswith(".parquet") and str(st) != "DELETED"]


@paso("tabla_tipos")
def _():
    out = {}
    for capa in ("pts", "edif"):
        scan = ore._iceberg(ML[capa])
        out[capa] = {"iceberg_scan": [list(map(str, r[:2])) for r in con.execute(f"describe select * from {scan}").fetchall()]}
        fs = ficheros(ML[capa])
        out[capa]["read_parquet"] = [list(map(str, r[:2])) for r in con.execute(f"describe select * from read_parquet('{fs[0]}')").fetchall()]
        meta = scan.replace("iceberg_scan(", "iceberg_metadata(", 1)
        out[capa]["contenido"] = [list(map(str, r)) for r in con.execute(
            f"select content, count(*) from {meta} group by 1").fetchall()]
        out[capa]["ficheros"] = len(fs)
    return out


# ── Incógnita 1: cinco formas de leer, sobre la tesela vacía y Madrid ────────────────────────────────────
con.execute("set memory_limit = '30GB'")
con.execute("set threads = 8")
con.execute("set parquet_metadata_cache = true")
con.execute("set enable_http_metadata_cache = true")


def tesela_de(lon, lat, z):
    n = 2 ** z
    return int((lon + 180) / 360 * n), int((1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n)


def variantes(capa):
    crs = CRS[capa]
    scan = ore._iceberg(ML[capa])
    lista = ", ".join("'%s'" % f for f in ficheros(ML[capa]))
    envuelta = f"ST_GeomFromWKB(geom)::GEOMETRY('{crs}')"
    if capa == "pts":
        cob = lambda c: f"x between ST_XMin({c}) and ST_XMax({c}) and y between ST_YMin({c}) and ST_YMax({c})"
    else:
        cob = lambda c: f"x1 >= ST_XMin({c}) and x0 <= ST_XMax({c}) and y1 >= ST_YMin({c}) and y0 <= ST_YMax({c})"
    return {
        # hoy: `_con_geo` sobre iceberg_scan, el filtro sobre la expresión
        "sdk_hoy": lambda c: f"from (select * replace ({envuelta} as geom) from {scan}) where geom && {c}",
        # sin el CRS: sólo ST_GeomFromWKB (¿es el cast lo que mata, o la función?)
        "sdk_sin_crs": lambda c: f"from (select * replace (ST_GeomFromWKB(geom) as geom) from {scan}) where geom && {c}",
        # (c) la cobertura primero, dentro del iceberg_scan, y el envoltorio después
        "sdk_cobertura": lambda c: f"from (select * replace ({envuelta} as geom) from {scan} where {cob(c)}) where geom && {c}",
        # (a) los ficheros de la instantánea por read_parquet: la GEOMETRY del fichero
        "parquet_nativo": lambda c: f"from read_parquet([{lista}]) where geom && {c}",
        # (b) empujar el filtro antes del envoltorio, con la caja como BOX_2D sobre el WKB crudo
        "sdk_extent": lambda c: f"from (select * replace ({envuelta} as geom) from {scan} "
                                f"where ST_Intersects_Extent(ST_GeomFromWKB(geom), (select {c}))) where geom && {c}",
    }


def caja(capa, z, x, y):
    env = f"ST_TileEnvelope({z}, {x}, {y})"
    return env if CRS[capa] == "EPSG:3857" else f"ST_Transform({env}, 'EPSG:3857', '{CRS[capa]}', true)"


def explicar(q):
    try:
        p = json.loads(con.execute("explain (analyze, format json) " + q).fetchall()[0][1])
        p = p[0] if isinstance(p, list) else p
        out = []

        def r(n):
            if n.get("operator_type") in ("TABLE_SCAN", "ICEBERG_SCAN") or "SCAN" in str(n.get("operator_name", "")):
                info = n.get("extra_info") or {}
                out.append({"op": n.get("operator_name"), "filas": n.get("operator_cardinality"),
                            **{k: v for k, v in info.items() if k in ("Filters", "Function", "Total Files Read", "Scanning Files")}})
            for h in n.get("children", []):
                r(h)
        r(p)
        return out
    except Exception as e:
        return str(e)[:200]


MADRID = (-3.7038, 40.4168)
VACIO = (-1.0, 36.6)
for capa in ("pts", "edif"):
    if not ML.get(capa):
        continue
    V = variantes(capa)
    for var, f in V.items():
        for sitio, (lon, lat) in (("vacio", VACIO), ("madrid", MADRID)):
            for z in ((10, 12, 14) if capa == "pts" else (12, 14)):
                x, y = tesela_de(lon, lat, z)
                q = "select count(*) n " + f(caja(capa, z, x, y))
                try:
                    ts = []
                    for _ in range(3):
                        t0 = time.time()
                        n = con.execute(q).fetchone()[0]
                        ts.append(time.time() - t0)
                    di("tesela", capa=capa, var=var, sitio=sitio, z=z, frio_ms=round(ts[0] * 1000),
                       ms=round(min(ts[1:]) * 1000), filas=n, **({"scan": explicar(q)} if z == 14 else {}))
                except Exception as e:
                    di("tesela", capa=capa, var=var, sitio=sitio, z=z, error=str(e)[:300])
di("fin", ml=ML, locs=LOCS)
