"""ORE 0072 F1·3 · El sustrato, medido: una tesela del lago lee sólo sus grupos de filas, venga el dataset
del SDK (`ore.write` → `ore-store`) o de Spark (`ore.spark.write`). Los mismos edificios del banco (Madrid,
EPSG:4326) por las dos vías, con el código de la imagen desplegada:

  sdk     `ore.write` al lago del inquilino (sin catálogo, como previo G: se lee por su `metadata_location`)
  spark   `ore.spark.write` a un catálogo Iceberg local (`hadoop`, en el disco del pod): el catálogo de
          ORE no deja crear un dataset a un agente (0052), y lo que se mide es cómo quedan los ficheros

y un `append` en cada una (la cobertura también en lo anexado). Por vía, tesela vacía y Madrid (z12, z14):
cuántos grupos de filas (y filas) caen dentro por las estadísticas de `geom_bbox`, y lo que tarda la
consulta del SDK (`_con_geo` + `geo.intersects_box`) con la cobertura y sin ella. Emite «F13|…». Corre en
`puesto-spark:1` en `cola-pesada` (`correr.py f1_3 --spark`); lo del lago se borra después a mano (`fin`).
"""
import json
import math
import os
import subprocess
import sys
import time
import traceback
import urllib.parse
import urllib.request

N_EDIF = int(os.environ.get("E2_N_EDIF", "5000000"))
BANCO = "https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo"
CRS = "EPSG:4326"


def di(que, **kw):
    print("F13|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)


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
from ore import geo  # noqa: E402
from ore import spark as pesado  # noqa: E402

ore.session._cabeceras = testigo()
ore.session.id = ""

# El `stage-create` de ore-serve presta la credencial del lago; `ore-store … aplicar` escribe el
# `metadata.json` sin pasar por el catálogo (previo G). La tabla de Spark no existe para ORE: 404.
_pedir = ore.session.pedir
LOCAL, STAGED, LOCS = {}, {}, []


def pedir(metodo, ruta, cuerpo=None, *a, **kw):
    if "/tables/f13_spark" in ruta:
        return 404, {"error": {"message": "local"}}
    if "/tables/f13_" in ruta:
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
            LOCAL[ruta] = {"ml": ml, "config": cfg, "loc": loc}
            return 200, {"metadata-location": ml, "metadata": {"current-snapshot-id": out.get("snapshot")}}
    if metodo == "POST" and ruta.endswith("/tables") and isinstance(cuerpo, dict) and cuerpo.get("stage-create") \
            and str(cuerpo.get("name", "")).startswith("f13_"):
        c, r = _pedir(metodo, ruta, cuerpo, *a, **kw)
        if c == 200:
            STAGED[ruta + "/" + cuerpo["name"]] = (r.get("config", {}), r["metadata"]["location"])
            LOCS.append(r["metadata"]["location"])
        return c, r
    return _pedir(metodo, ruta, cuerpo, *a, **kw)


ore.session.pedir = pedir
_, r = ore.session.pedir("GET", "/v1/namespaces")
NS = sorted(n[0] for n in r["namespaces"])[0]

con = ore._duckdb()
for e in ("spatial", "httpfs"):
    con.execute("load " + e)
con.execute("set geometry_always_xy = true")
con.execute("set memory_limit = '16GB'")
con.execute("set threads = 8")
con.execute("create or replace secret banco (type http, bearer_token '%s', scope '%s')" % (
    ore._token_de_google().replace("'", "''"), BANCO))
di("inicio", ns=NS, duckdb=duckdb.__version__, cpus=os.cpu_count(), n_edif=N_EDIF)

ML = {}
EDIF = "/tmp/edif.parquet"


@paso("datos")
def _():
    con.execute(f"""copy (select row_number() over () id, ST_AsWKB(geom::GEOMETRY) wkb
                    from read_parquet('{BANCO}/g22/a_tal_cual.parquet')
                    where ST_Intersects(geom, ST_MakeEnvelope(-4.6, 39.8, -2.8, 41.0)) limit {N_EDIF})
                    to '{EDIF}' (format parquet)""")
    return {"filas": con.execute(f"select count(*) from '{EDIF}'").fetchone()[0]}


# ── La vía del SDK: `ore.write` (ore-store pone la cobertura, el orden Hilbert y los grupos) ──────────
def mitades():
    t = con.execute(f"select id, ST_SetCRS(ST_GeomFromWKB(wkb), '{CRS}') geom from '{EDIF}' order by id").fetch_arrow_table()
    m = t.num_rows * 9 // 10
    return t.slice(0, m), t.slice(m)


@paso("sdk_escribir")
def _():
    a, b = mitades()
    t0 = time.time()
    ore.session._cabeceras = testigo()
    e = ore.write(NS + ".f13_sdk", a)
    s_over = time.time() - t0
    t0 = time.time()
    ore.session._cabeceras = testigo()
    e = ore.write(NS + ".f13_sdk", b, mode="append")
    ML["sdk"] = e["metadata_location"]
    return {"filas": e.get("rows"), "geo": e.get("geo"), "s_overwrite": round(s_over, 1),
            "s_append": round(time.time() - t0, 1)}


# ── La vía de Spark: `ore.spark.write` tal cual, con el catálogo local en lugar del de ORE ────────────
@paso("spark_escribir")
def _():
    from pyspark.sql import SparkSession

    mb = int(os.environ.get("ORE_MEMORIA_MB", "40960"))
    s = (SparkSession.builder.master("local[*]").appName("f1-3")
         .config("spark.driver.memory", "%dm" % max(1024, int((mb - 1536) * 0.6)))
         .config("spark.sql.shuffle.partitions", "64")
         .config("spark.sql.catalog.loc", "org.apache.iceberg.spark.SparkCatalog")
         .config("spark.sql.catalog.loc.type", "hadoop")
         .config("spark.sql.catalog.loc.warehouse", "file:///tmp/wh")
         .getOrCreate())
    s.sql("create namespace if not exists loc.f1")
    pesado._spark = s
    pesado._tabla = lambda nombre: "loc.f1.f13_spark"
    df = s.read.parquet(EDIF).selectExpr("id", "ST_SetSRID(ST_GeomFromWKB(wkb), 4326) geom")
    t0 = time.time()
    e1 = pesado.write(NS + ".f13_spark", df.where("id <= %d" % (N_EDIF * 9 // 10)))
    s_over = time.time() - t0
    t0 = time.time()
    pesado.write(NS + ".f13_spark", df.where("id > %d" % (N_EDIF * 9 // 10)), mode="append")
    s_app = time.time() - t0
    props = {r["key"]: r["value"] for r in s.sql("show tblproperties loc.f1.f13_spark").collect()
             if r["key"].startswith(("ore.", "write.parquet"))}
    nulas = s.sql("select count(*) n from loc.f1.f13_spark where geom_bbox is null").first()["n"]
    md = sorted(f for f in os.listdir("/tmp/wh/f1/f13_spark/metadata") if f.endswith(".metadata.json"))
    ML["spark"] = "/tmp/wh/f1/f13_spark/metadata/" + md[-1]
    return {"geo": e1["geo"], "props": props, "bbox_nulas": nulas, "s_overwrite": round(s_over, 1),
            "s_append": round(s_app, 1)}


# ── La tesela: grupos por las estadísticas de `geom_bbox`, y la consulta del SDK ─────────────────────
def scan(via):
    if via == "sdk":
        return ore._iceberg(ML["sdk"])
    return "iceberg_scan('%s')" % ML["spark"]


def relacion(via):
    """Por donde lee el SDK; si DuckDB no abre las rutas `file:` del catálogo `hadoop`, los mismos
    ficheros por `read_parquet` (sin borrados: es lo mismo), y se dice."""
    try:
        con.execute("select count(*) from %s" % scan(via)).fetchone()
        return scan(via), "iceberg_scan"
    except Exception as e:
        di("aviso", via=via, iceberg_scan=str(e)[:200])
        return "read_parquet([%s])" % ", ".join("'%s'" % f for f in ficheros(via)), "read_parquet"


def ficheros(via):
    if via == "spark":
        from pyspark.sql import SparkSession

        return [p.replace("file:", "") for (p,) in SparkSession.getActiveSession()
                .sql("select file_path from loc.f1.f13_spark.files").collect()]
    meta = scan(via).replace("iceberg_scan(", "iceberg_metadata(", 1)
    f = con.execute(f"select file_path, status from {meta}").fetchall()
    return [p.replace("gs://", "https://storage.googleapis.com/").replace("file:", "")
            for p, st in f if str(p).endswith(".parquet") and str(st) != "DELETED"]


def grupos(via):
    """(filas, xmin_min, ymin_min, xmax_max, ymax_max) de cada grupo, de los min/max de `geom_bbox`."""
    lista = ", ".join("'%s'" % f for f in ficheros(via))
    filas = con.execute(f"""
      select file_name, row_group_id, any_value(row_group_num_rows),
             max(case when path_in_schema = 'geom_bbox, xmin' then stats_min_value end),
             max(case when path_in_schema = 'geom_bbox, ymin' then stats_min_value end),
             max(case when path_in_schema = 'geom_bbox, xmax' then stats_max_value end),
             max(case when path_in_schema = 'geom_bbox, ymax' then stats_max_value end)
      from parquet_metadata([{lista}]) group by 1, 2""").fetchall()
    return [(n, float(a), float(b), float(c), float(d)) for _, _, n, a, b, c, d in filas if a is not None]


def tesela_de(lon, lat, z):
    n = 2 ** z
    return int((lon + 180) / 360 * n), int((1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n)


def caja(z, x, y):
    n = 2 ** z
    lat = lambda t: math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * t / n))))
    return x / n * 360 - 180, lat(y + 1), (x + 1) / n * 360 - 180, lat(y)


GEO = {"geom": {"crs": CRS, "covering": "geom_bbox"}}
for via in ("sdk", "spark"):
    if not ML.get(via):
        continue
    try:
        G = grupos(via)
        fs = ficheros(via)
        di("forma", via=via, ficheros=len(fs), grupos=len(G), filas=sum(g[0] for g in G),
           filas_por_grupo=sorted(g[0] for g in G)[len(G) // 2] if G else 0)
    except Exception as e:
        di("forma", via=via, error=str(e)[:400])
        G = []
    fuente, por = relacion(via)
    rel = ore._con_geo(fuente, {"geo": GEO})
    for sitio, (lon, lat) in (("vacio", (-1.0, 36.6)), ("madrid", (-3.7038, 40.4168))):
        for z in (12, 14):
            x0, y0, x1, y1 = caja(z, *tesela_de(lon, lat, z))
            toca = [g for g in G if g[1] <= x1 and g[3] >= x0 and g[2] <= y1 and g[4] >= y0]
            out = {"via": via, "por": por, "sitio": sitio, "z": z, "grupos_leidos": len(toca), "de": len(G),
                   "filas_leidas": sum(g[0] for g in toca)}
            for nombre, cob in (("cobertura", True), ("sin", False)):
                q = "select count(*) from %s where %s" % (rel, geo.intersects_box("geom", x0, y0, x1, y1, covering=cob))
                try:
                    ts = []
                    for _ in range(3):
                        t0 = time.time()
                        n = con.execute(q).fetchone()[0]
                        ts.append(time.time() - t0)
                    out[nombre] = {"frio_ms": round(ts[0] * 1000), "ms": round(min(ts[1:]) * 1000), "filas": n}
                except Exception as e:
                    out[nombre] = {"error": str(e)[:300]}
            di("tesela", **out)
di("fin", ml=ML, locs=LOCS)
