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
di("inicio", ns=NS, duckdb=duckdb.__version__, cpus=os.cpu_count(), n=N, n_edif=N_EDIF)

con = ore._duckdb()
for e in ("spatial", "h3", "httpfs"):
    con.execute("load " + e)
con.execute("set geometry_always_xy = true")
con.execute("set arrow_large_buffer_size = true")  # LargeBinary, como guarda el escritor: los `append` casan
con.execute("create or replace secret banco (type http, bearer_token '%s', scope '%s')" % (
    ore._token_de_google().replace("'", "''"), BANCO))

ML = {}  # capa -> metadata_location
CRS = {"pts_h": "EPSG:25830", "pts_m": "EPSG:3857", "edif": "EPSG:4326", **{f"pts_a{k}": "EPSG:25830" for k in range(10)}}

# ── Los datos ─────────────────────────────────────────────────────────────────
# Núcleos urbanos (x, y en 25830, peso, dispersión en m): la densidad de un mapa real, no uniforme.
NUCLEOS = [(440000, 4474000, 30, 12000), (430000, 4581000, 18, 9000), (726000, 4373000, 9, 7000),
           (235000, 4142000, 8, 7000), (505000, 4790000, 6, 5000), (675000, 4615000, 6, 6000),
           (373000, 4065000, 5, 6000), (546000, 4801000, 4, 5000), (240000, 4815000, 3, 6000),
           (300000, 4400000, 11, 150000)]


@paso("datos")
def _():
    pesos = [n[2] for n in NUCLEOS]
    con.execute("create or replace table nucleos (i int, x double, y double, s double, w double)")
    con.executemany("insert into nucleos values (?, ?, ?, ?, ?)",
                    [(i, n[0], n[1], n[3], n[2] / sum(pesos)) for i, n in enumerate(NUCLEOS)])
    # Un punto por fila: su núcleo por peso acumulado, y gaussiana alrededor (Box-Muller).
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
    """`geom` como LargeBinary con su marca geoarrow: así la guarda el escritor, y un `append` casa."""
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


def escribir(nombre, rel_sql, modo="overwrite"):
    t = a_large(con.execute(rel_sql).fetch_arrow_table())
    e = ore.write(NS + ".medida_e2_" + nombre, t, mode=modo)
    ML[nombre] = e["metadata_location"]
    return e


@paso("escribir_pts_h")
def _():
    e = escribir("pts_h", "select id, cat, val, x, y, ST_SetCRS(ST_Point(x, y), 'EPSG:25830') geom from puntos")
    return {"filas": e.get("rows"), "ml": e["metadata_location"]}


@paso("escribir_pts_m")
def _():
    e = escribir("pts_m", "select id, cat, val, x, y, ST_SetCRS(ST_Transform(ST_Point(x, y), 'EPSG:25830', 'EPSG:3857', true), 'EPSG:3857') geom from puntos")
    return {"filas": e.get("rows")}


@paso("append_geo")
def _():
    # ¿Anexa el SDK geometría? (lo que un usuario haría: la misma forma dos veces)
    q = "select id, cat, val, x, y, ST_SetCRS(ST_Point(x, y), 'EPSG:25830') geom from puntos where id < 1000"
    t = con.execute(q).fetch_arrow_table()
    ore.write(NS + ".medida_e2_append", t)
    ore.write(NS + ".medida_e2_append", t, mode="append")
    return {"anexa": True}


@paso("escribir_pts_a")
def _():
    # 10 `append` dejan 10 ficheros, cada uno ordenado por sí mismo: los mismos ficheros que el escritor
    # deja si cada parte se escribe aparte (el `append` de geometría falla hoy: `append_geo`).
    partes = 10
    for k in range(partes):
        escribir(f"pts_a{k}", f"select id, cat, val, x, y, ST_SetCRS(ST_Point(x, y), 'EPSG:25830') geom from puntos "
                              f"where hash(id) % {partes} = {k}")
    return {"partes": partes}


@paso("escribir_edif")
def _():
    e = escribir("edif", f"select id, ST_SetCRS(geom::GEOMETRY, 'EPSG:4326') geom, "
                         f"ST_XMin(geom) x0, ST_YMin(geom) y0, ST_XMax(geom) x1, ST_YMax(geom) y1 "
                         f"from read_parquet('{BANCO}/g22/a_tal_cual.parquet') "
                         f"where ST_Intersects(geom, ST_MakeEnvelope(-4.6, 39.8, -2.8, 41.0)) limit {N_EDIF}")
    return {"filas": e.get("rows")}


@paso("escribir_pts_s")
def _():
    os.makedirs("/tmp/s", exist_ok=True)
    con.execute("copy (select id, cat, val, x, y, ST_AsWKB(ST_Point(x, y)) geom from puntos order by random()) "
                "to '/tmp/s/pts_s.parquet' (format parquet, row_group_size 100000)")
    return {"bytes": os.path.getsize("/tmp/s/pts_s.parquet")}


# ── Las fuentes ───────────────────────────────────────────────────────────────
def prestada(capa):
    for l in LOCAL.values():
        if l["ml"] == ML[capa]:
            return l["config"].get("gcs.oauth2.token")


def scan_sdk(capa):
    """El camino del SDK (`_con_geo`): iceberg_scan con la geometría envuelta."""
    return "(select * replace (ST_GeomFromWKB(geom)::GEOMETRY('%s') as geom) from %s)" % (CRS[capa], ore._iceberg(ML[capa], prestada(capa)))


FICHEROS = {}


@paso("ficheros")
def _():
    for capa, ml in ML.items():
        scan = ore._iceberg(ml, prestada(capa))  # pone el secreto de su raíz
        meta = scan.replace("iceberg_scan(", "iceberg_metadata(", 1)
        filas = con.execute(f"select * from {meta}").fetchdf()
        if not FICHEROS:
            di("iceberg_metadata", columnas=list(filas.columns), muestra=filas.head(2).astype(str).to_dict("records"))
        FICHEROS[capa] = [f for f, st in zip(filas["file_path"], filas["status"]) if str(f).endswith(".parquet") and str(st) != "DELETED"]
    return {c: len(f) for c, f in FICHEROS.items()}


def scan_parquet(capa):
    fs = [f.replace("gs://", "https://storage.googleapis.com/") for c, l in FICHEROS.items()
          if c == capa or (capa == "pts_a" and c.startswith("pts_a")) for f in l]
    return "read_parquet([%s])" % ", ".join("'%s'" % f for f in fs)


# ── La tesela ─────────────────────────────────────────────────────────────────
def tesela_de(lon, lat, z):
    n = 2 ** z
    x = int((lon + 180) / 360 * n)
    y = int((1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n)
    return x, y


def sql_tesela(fuente, crs, z, x, y, filtro="caja", h3=None, poligonos=False):
    env = f"ST_TileEnvelope({z}, {x}, {y})"
    caja = env if crs == "EPSG:3857" else f"ST_Transform({env}, 'EPSG:3857', '{crs}', true)"
    if filtro == "caja":
        donde = f"geom && {caja}"
    elif filtro == "cobertura":  # puntos: x/y; polígonos: x0..y1
        donde = (f"x between ST_XMin({caja}) and ST_XMax({caja}) and y between ST_YMin({caja}) and ST_YMax({caja})"
                 if not poligonos else
                 f"x1 >= ST_XMin({caja}) and x0 <= ST_XMax({caja}) and y1 >= ST_YMin({caja}) and y0 <= ST_YMax({caja})")
    else:
        donde = "true"
    g = "geom" if crs == "EPSG:3857" else f"ST_Transform(geom, '{crs}', 'EPSG:3857', true)"
    if h3 is not None:
        # El agregado: una celda H3 por fila, su centro como punto, y el conteo.
        g4 = "geom" if crs == "EPSG:4326" else f"ST_Transform(geom, '{crs}', 'EPSG:4326', true)"
        return f"""
          with p as (select h3_latlng_to_cell(ST_Y(c), ST_X(c), {h3}) celda from (select {g4} c from {fuente} where {donde})),
          a as (select celda, count(*) n from p group by 1)
          select ST_AsMVT({{'geom': ST_AsMVTGeom(ST_Transform(ST_Point(h3_cell_to_lng(celda), h3_cell_to_lat(celda)), 'EPSG:4326', 'EPSG:3857', true),
                                                   ST_Extent({env}), 4096, 64, true), 'n': n}}, 'capa', 4096, 'geom') mvt, count(*) filas
          from a"""
    return f"""
      select ST_AsMVT({{'geom': ST_AsMVTGeom({g}, ST_Extent({env}), 4096, 64, true), 'id': id}}, 'capa', 4096, 'geom') mvt, count(*) filas
      from {fuente} where {donde}"""


def perfil(sql_, analizar=True):
    """Tiempo, y con `analizar` los bytes leídos y las filas que el scan recorre
    (`explain (analyze, format json)`, en otra pasada: no cuenta en el tiempo)."""
    t0 = time.time()
    mvt, filas = con.execute(sql_).fetchone()
    s = time.time() - t0
    bytes_, recorridas = None, 0
    if analizar:
        try:
            p = json.loads(con.execute("explain (analyze, format json) " + sql_).fetchall()[0][1])
            p = p[0] if isinstance(p, list) else p
            bytes_ = p.get("total_bytes_read")

            def recorre(n):
                nonlocal recorridas
                if n.get("operator_type") == "TABLE_SCAN":
                    recorridas += int(n.get("operator_rows_scanned") or 0)
                for h in n.get("children", []):
                    recorre(h)
            recorre(p)
        except Exception as e:
            di("perfil_error", error=str(e)[:300])
    return {"ms": round(s * 1000, 1), "filas": filas, "kb": round(len(mvt or b"") / 1024, 1),
            "leidas": recorridas, "mb_leidos": round((bytes_ or 0) / 2**20, 2)}


MADRID = (-3.7038, 40.4168)
VACIO = (-1.0, 36.6)  # el mar, al sur de Almería
ZOOMS = [4, 6, 8, 10, 12, 14]


@paso("teselas")
def _():
    res = 0
    casos = [("pts_h", "sdk"), ("pts_h", "parquet"), ("pts_h", "cobertura"), ("pts_a", "parquet"),
             ("pts_m", "sdk"), ("edif", "sdk"), ("edif", "parquet"), ("edif", "cobertura"), ("pts_s", "parquet"),
             ("pts_s", "cobertura")]
    for capa, via in casos:
        if capa not in ML and capa not in ("pts_s", "pts_a"):
            continue
        crs = "EPSG:25830" if capa in ("pts_s", "pts_a") else CRS[capa]
        if capa == "pts_s":
            fuente = "(select * replace (ST_GeomFromWKB(geom)::GEOMETRY('EPSG:25830') as geom) from read_parquet('/tmp/s/pts_s.parquet'))"
        elif via == "parquet":
            fuente = f"(select * replace (geom::GEOMETRY('{crs}') as geom) from {scan_parquet(capa)})"
        else:
            fuente = scan_sdk(capa)
        filtro = "cobertura" if via == "cobertura" else "caja"
        for sitio, (lon, lat) in (("madrid", MADRID), ("vacio", VACIO)):
            for z in ZOOMS:
                if capa == "edif" and z < 8:
                    continue
                x, y = tesela_de(lon, lat, z)
                q = sql_tesela(fuente, crs, z, x, y, filtro, poligonos=capa == "edif")
                try:
                    frio = perfil(q, analizar=False)
                    caliente = perfil(q)
                    di("tesela", capa=capa, via=via, sitio=sitio, z=z, frio_ms=frio["ms"], ms=caliente["ms"],
                       filas=caliente["filas"], kb=caliente["kb"], leidas=caliente["leidas"], mb=caliente["mb_leidos"], mb_frio=frio["mb_leidos"])
                    res += 1
                except Exception as e:
                    di("tesela", capa=capa, via=via, sitio=sitio, z=z, error=str(e)[:300])
    return {"medidas": res}


@paso("agregado_h3")
def _():
    for z, h3 in ((4, 3), (6, 4), (8, 6), (10, 7)):
        x, y = tesela_de(*MADRID, z)
        for via in ("sdk", "cobertura"):
            q = sql_tesela(scan_sdk("pts_h"), "EPSG:25830", z, x, y, "cobertura" if via == "cobertura" else "caja", h3=h3)
            try:
                m = perfil(q)
                perfil(q)
                m2 = perfil(q)
                di("h3", z=z, res=h3, via=via, frio_ms=m["ms"], ms=m2["ms"], celdas=m2["filas"], kb=m2["kb"], leidas=m2["leidas"], mb=m2["mb_leidos"])
            except Exception as e:
                di("h3", z=z, res=h3, via=via, error=str(e)[:300])


@paso("validez_poligonos")
def _():
    # T4: la geometría recortada a la tesela sigue siendo válida (z12 sobre Madrid).
    x, y = tesela_de(*MADRID, 12)
    env = f"ST_TileEnvelope(12, {x}, {y})"
    q = (f"select count(*), count(*) filter (where not ST_IsValid(g)) from (select ST_AsMVTGeom(ST_Transform(geom, 'EPSG:4326', 'EPSG:3857', true), "
         f"ST_Extent({env}), 4096, 64, true) g from {scan_sdk('edif')} where geom && ST_Transform({env}, 'EPSG:3857', 'EPSG:4326', true)) where g is not null")
    n, malos = con.execute(q).fetchone()
    return {"poligonos": n, "invalidos": malos}


def concurrencia(lectores, segundos=40):
    """Lectores con su propia conexión, teselas z10–z14 al azar alrededor de Madrid."""
    fuente = scan_sdk("pts_h")

    def uno(k):
        c = con.cursor()
        rnd = random.Random(k)
        tiempos = []
        fin = time.time() + segundos
        while time.time() < fin:
            z = rnd.choice([10, 11, 12, 13, 14])
            lon = MADRID[0] + rnd.uniform(-0.6, 0.6)
            lat = MADRID[1] + rnd.uniform(-0.4, 0.4)
            x, y = tesela_de(lon, lat, z)
            t0 = time.time()
            c.execute(sql_tesela(fuente, "EPSG:25830", z, x, y)).fetchone()
            tiempos.append(time.time() - t0)
        return tiempos

    with cf.ThreadPoolExecutor(lectores) as ex:
        todos = [t for ts in ex.map(uno, range(lectores)) for t in ts]
    todos.sort()
    return {"lectores": lectores, "teselas": len(todos), "por_s": round(len(todos) / segundos, 1),
            "p50_ms": round(statistics.median(todos) * 1000), "p95_ms": round(todos[int(len(todos) * 0.95) - 1] * 1000)}


@paso("concurrencia")
def _():
    for n in (1, 8, 32):
        try:
            di("concurrencia", **concurrencia(n))
        except Exception as e:
            di("concurrencia", lectores=n, error=str(e)[:300])


di("fin", ml=ML, locs=LOCS)
