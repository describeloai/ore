# ADR 0067 · El cierre de G2, en una celda de Python de una sesión de victor (abierta por una persona:
# lo que se escribe tiene dueño, ADR 0052). Pon en PAQUETE uno donde puedas escribir.
#   G2·3·5  ore.write de un GeoDataFrame, ore.over de vuelta, filtro por zona y H3.
#   V5      los fijos (gs://…-banco-geo/g2/) escritos por este ORE (B, su fichero A), promovidos (C) y
#           leídos con los mismos lectores y el mismo juicio que en local.
#   V6      la tabla v3 ajena del banco (Iceberg 1.12): leída, copiada con ore.write y releída.
# Deja PAQUETE.medida_g2_*. Copia la salida entera de vuelta.
PAQUETE = "geo"

import glob, hashlib, json, os, subprocess, sys, time, traceback
import duckdb, geopandas, shapely, numpy as np
import ore
from google.cloud import storage

BANCO = "project-8853a180-450d-47be-b83-banco-geo"
os.makedirs("/tmp/g2/m", exist_ok=True)
for b in storage.Client().list_blobs(BANCO, prefix="g2/"):
    b.download_to_filename("/tmp/g2/m/" + b.name.split("/")[-1])

def di(que, **kw): print("G2|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)
def paso(nombre):
    def deco(f):
        t0 = time.time()
        try:
            di(nombre, ok=True, s=round(time.time() - t0, 2), **(f() or {}))
        except Exception as e:
            di(nombre, ok=False, s=round(time.time() - t0, 2), error=(type(e).__name__ + ": " + str(e))[:500])
    return deco
def bajar(vista, destino):
    """La tabla de `vista` entera a `destino`, con la credencial que `datos` presta para ella."""
    _, r = ore._fuente_de(vista)
    raiz = r["metadata_location"].rsplit("/metadata/", 1)[0]
    n = 0
    for ruta, contenido in ore._duckdb().execute("select filename, content from read_blob('%s/**')" % raiz).fetchall():
        p = destino + "/" + ruta[len(raiz) + 1:]
        os.makedirs(os.path.dirname(p), exist_ok=True); open(p, "wb").write(contenido); n += 1
    return r["metadata_location"], n, r.get("geo")
def correr(*a, entrada=None):
    p = subprocess.run([sys.executable, *a], input=entrada, capture_output=True, text=True)
    return p.stdout + (("\n" + p.stderr[-300:]) if p.stderr.strip() else "")
di("imagen", geopandas=geopandas.__version__, shapely=shapely.__version__, duckdb=duckdb.__version__)

# ── G2·3·5 ────────────────────────────────────────────────────────────────────
N = 100_000
rng = np.random.default_rng(7)
x = rng.uniform(430000, 450000, N); y = rng.uniform(4465000, 4485000, N)
g = geopandas.GeoDataFrame({"id": np.arange(N, dtype="int64"), "valor": rng.normal(size=N)},
                           geometry=geopandas.points_from_xy(x, y), crs="EPSG:25830").rename_geometry("geom")
V = PAQUETE + ".medida_g2_puntos"
@paso("g235_write")
def _():
    e = ore.write(V, g); return {"filas": e.get("rows")}
@paso("g235_over")
def _():
    l = ore.over(V)
    iguales = isinstance(l, geopandas.GeoDataFrame) and bool(l.sort_values("id").geometry.reset_index(drop=True).geom_equals(g.geometry).all())
    return {"tipo": type(l).__name__, "filas": len(l), "crs": str(getattr(l, "crs", None)), "iguales": iguales}
caja = (435000, 4470000, 437000, 4472000)
esperado = int(((x >= caja[0]) & (x <= caja[2]) & (y >= caja[1]) & (y <= caja[3])).sum())
@paso("g235_zona")
def _():
    z = ore.sql("select id, geom from %s where %s" % (V, ore.geo.intersects_box("geom", *caja)))
    return {"tipo": type(z).__name__, "filas": len(z), "esperado": esperado, "crs": str(getattr(z, "crs", None))}
@paso("g235_h3")
def _():
    h = ore.sql("select h3_latlng_to_cell(ST_Y(p), ST_X(p), 8) celda, count(*) n from "
                "(select ST_Transform(geom, 'EPSG:25830', 'EPSG:4326', true) p from %s) group by 1" % V)
    return {"celdas": len(h), "filas": int(h["n"].sum())}

# ── V5 ────────────────────────────────────────────────────────────────────────
fijos = json.load(open("/tmp/g2/m/fijos.json"))
for crs, f in fijos.items():
    d = crs.replace(":", "-")
    VF = PAQUETE + ".medida_g2_v5_" + d.lower().replace("-", "_")
    gf = geopandas.GeoDataFrame(
        {"id": [r["id"] for r in f["filas"]], "subtipo": [r["subtipo"] for r in f["filas"]]},
        geometry=[None if r["wkb"] is None else shapely.from_wkb(bytes.fromhex(r["wkb"])) for r in f["filas"]],
        crs=crs).rename_geometry("geom")
    raiz = "/tmp/g2/v5/" + d
    @paso("v5_write_" + d)
    def _():
        e = ore.write(VF, gf); return {"filas": e.get("rows")}
    @paso("v5_bajar_" + d)
    def _():
        ml, n, geo = bajar(VF, raiz); return {"ficheros": n, "ml": ml, "geo_del_puntero": geo}
    metas = sorted(glob.glob(raiz + "/metadata/*.metadata.json"))
    if not metas:
        continue
    v0 = os.path.basename(metas[-1])[: -len(".metadata.json")]
    print(correr("/tmp/g2/m/promover.py", raiz)[-200:])
    v1 = os.path.basename(sorted(glob.glob(raiz + "/metadata/*.metadata.json"))[-1])[: -len(".metadata.json")]
    a = (glob.glob(raiz + "/data/*.parquet") or [""])[0]
    args = ["A=" + a, "B=%s@%s" % (raiz, v0), "C=%s@%s" % (raiz, v1)]
    lineas = "".join(correr("/tmp/g2/m/lectores.py", l, "/tmp/g2/m/fijos.json", *args)
                     for l in ("duckdb", "duckdb-spatial", "pyarrow", "geopandas"))
    print(correr("/tmp/g2/m/juzgar.py", "/tmp/g2/m/fijos.json", crs, entrada=lineas))

# ── V6 ────────────────────────────────────────────────────────────────────────
ajena = {}
@paso("v6_leer")
def _():
    raiz = "gs://%s/lakehouse/ajena/lugares_madrid" % BANCO
    ml = sorted("gs://%s/%s" % (BANCO, b.name) for b in storage.Client().list_blobs(BANCO, prefix="lakehouse/ajena/lugares_madrid/metadata/")
                if b.name.endswith(".metadata.json"))[-1]
    ore._iceberg(ml)
    q = "select id, nombre, categoria, geom from iceberg_scan('%s')" % ml
    con = ore._duckdb()
    tipo = con.execute("select typeof(geom) from (%s) limit 1" % q).fetchone()[0]
    t = con.execute("select id, nombre, categoria, ST_AsWKB(geom) w from (%s) order by id" % q).fetch_arrow_table()
    ajena["t"] = t
    ajena["md5"] = hashlib.md5(b"".join(w.as_py() for w in t.column("w") if w.is_valid)).hexdigest()
    return {"tipo": tipo, "filas": t.num_rows, "md5": ajena["md5"]}
VA = PAQUETE + ".medida_g2_ajena"
@paso("v6_copiar")
def _():
    t = ajena["t"]
    ga = geopandas.GeoDataFrame({"id": t.column("id").to_numpy(), "nombre": t.column("nombre").to_pylist(),
                                 "categoria": t.column("categoria").to_pylist()},
                                geometry=shapely.from_wkb(t.column("w").to_numpy(zero_copy_only=False)),
                                crs="OGC:CRS84").rename_geometry("geom")
    e = ore.write(VA, ga); return {"filas": e.get("rows")}
@paso("v6_releer")
def _():
    l = ore.over(VA)
    l = l.sort_values("id")
    md5 = hashlib.md5(b"".join(shapely.to_wkb(v) for v in l.geometry if v is not None)).hexdigest()
    return {"tipo": type(l).__name__, "filas": len(l), "crs": str(getattr(l, "crs", None)), "igual": md5 == ajena["md5"]}
di("fin")
