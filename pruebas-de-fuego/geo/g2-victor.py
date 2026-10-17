"""ADR 0067 · El cierre de G2 en victor: un Job en un puesto, con la imagen desplegada.

  G2·3·5  la imagen trae `spatial`, `h3` y geopandas; `ore.write` de un GeoDataFrame, `ore.over` de vuelta
          como GeoDataFrame con su CRS, el filtro por zona y H3;
  V5      los fijos (`v5/fijos.json`) escritos por el ORE desplegado con `ore.write` (B; su fichero, A),
          promovidos (C), bajados y leídos con los mismos `v5/lectores.py` y `v5/juzgar.py` que en local
          (DuckDB, pyarrow, GeoPandas: los de la imagen del puesto);
  V6      la tabla v3 ajena del banco (Iceberg 1.12, 214 328 filas): leída, copiada con `ore.write` y
          vuelta a leer igual.

Cada paso falla por separado. Deja `<paquete>.medida_g2_*` en el primer paquete del árbol.

Uso:  python pruebas-de-fuego/geo/g2-victor.py [--ensayo]      deja g2-victor-<fecha>.log
"""
import base64
import os
import sys
import time
import zlib

S = os.path.dirname(os.path.abspath(__file__)) + os.sep
sys.path.insert(0, S)
import vara  # noqa: E402

ENSAYO = "--ensayo" in sys.argv
BANCO = "gs://project-8853a180-450d-47be-b83-banco-geo/lakehouse/ajena/lugares_madrid"


def empaquetar(ruta):
    return base64.b64encode(zlib.compress(open(ruta, "rb").read(), 9)).decode()


GUION = r'''
import base64, glob, hashlib, json, os, subprocess, sys, time, traceback, urllib.parse, urllib.request, zlib
ENSAYO = __ENSAYO__
BANCO = "__BANCO__"
os.makedirs("/tmp/m", exist_ok=True)
for nombre, b in __FICHEROS__.items():
    open("/tmp/m/" + nombre, "wb").write(zlib.decompress(base64.b64decode(b)))
def di(que, **kw): print("G2|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)
def paso(nombre):
    def deco(f):
        t0 = time.time()
        try:
            r = f(); di(nombre, ok=True, s=round(time.time() - t0, 2), **(r or {}))
        except Exception as e:
            di(nombre, ok=False, s=round(time.time() - t0, 2), error=(type(e).__name__ + ": " + str(e))[:500],
               donde=traceback.format_exc().strip().splitlines()[-3][:200])
    return deco
def testigo():
    d = os.environ["DIRECCION"].rstrip("/"); realm = os.environ.get("REALM", "rubix")
    cli = open("/puesto/agente-cliente").read().strip(); sec = open("/puesto/agente-secreto").read().strip()
    datos = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": cli, "client_secret": sec}).encode()
    with urllib.request.urlopen(d + "/realms/%s/protocol/openid-connect/token" % realm, data=datos, timeout=20) as r:
        return {"authorization": "Bearer " + json.load(r)["access_token"]}

sys.path.insert(0, "/opt/ore")
# Fuera de una sesión, el SDK no tiene su ore-serve en 127.0.0.1: el del inquilino.
os.environ.setdefault("ORE_SERVE", "http://ore-serve.%s.svc.cluster.local:8080" % os.environ["NS"])
import duckdb, geopandas, shapely, pyproj, numpy as np
import ore
di("imagen", geopandas=geopandas.__version__, shapely=shapely.__version__, pyproj=pyproj.__version__, duckdb=duckdb.__version__)
c0 = ore._duckdb()
for e in ("spatial", "h3", "iceberg", "httpfs"):
    try:
        c0.execute("load " + e); di("extension", nombre=e, ok=True)
    except Exception as ex:
        di("extension", nombre=e, ok=False, error=str(ex)[:200])

ore.session._cabeceras = testigo(); ore.session.id = ""
_pedir = ore.session.pedir
GEO = {}
def pedir(metodo, ruta, *a, **kw):
    if ruta.startswith("/puestos//datos/"):
        v = ruta.rsplit("/", 1)[1]
        c, r = _pedir("GET", "/datasets/" + v.replace(".", "/"))
        if isinstance(r, dict) and v in GEO:
            r["geo"] = GEO[v]
        return c, r
    return _pedir(metodo, ruta, *a, **kw)
ore.session.pedir = pedir
c_, r = ore.session.pedir("GET", "/v1/namespaces")
ns = sorted(n[0] for n in r["namespaces"])[0]
di("paquete", ns=ns)

def bajar(ml, destino):
    """La tabla entera (metadatos y datos) a `destino`, con DuckDB y la credencial del pod."""
    con = ore._duckdb(); ore._iceberg(ml)
    raiz = ml.rsplit("/metadata/", 1)[0]
    n = 0
    for ruta, contenido in con.execute("select filename, content from read_blob('%s/**')" % raiz).fetchall():
        p = destino + "/" + ruta[len(raiz) + 1:]
        os.makedirs(os.path.dirname(p), exist_ok=True); open(p, "wb").write(contenido); n += 1
    return n

def juzgar(crs, lineas):
    p = subprocess.run([sys.executable, "/tmp/m/juzgar.py", "/tmp/m/fijos.json", crs], input=lineas,
                       capture_output=True, text=True)
    for l in p.stdout.splitlines():
        print(l, flush=True)

# ── G2·3·5 ────────────────────────────────────────────────────────────────────
N = 1000 if ENSAYO else 100_000
rng = np.random.default_rng(7)
x = rng.uniform(430000, 450000, N); y = rng.uniform(4465000, 4485000, N)
g = geopandas.GeoDataFrame({"id": np.arange(N, dtype="int64"), "valor": rng.normal(size=N)},
                           geometry=geopandas.points_from_xy(x, y), crs="EPSG:25830").rename_geometry("geom")
V = ns + ".medida_g2_puntos"
GEO[V] = {"geom": {"type": "Geometry<Point, EPSG:25830>", "crs": "EPSG:25830"}}
@paso("g235_write")
def _():
    e = ore.write(V, g); return {"filas": e.get("rows"), "ml": e.get("metadata_location")}
@paso("g235_over")
def _():
    l = ore.over(V)
    return {"tipo": type(l).__name__, "filas": len(l), "crs": str(getattr(l, "crs", None)),
            "iguales": bool(l.sort_values("id").geometry.reset_index(drop=True).geom_equals(g.geometry).all())}
caja = (435000, 4470000, 437000, 4472000)
esperado = int(((x >= caja[0]) & (x <= caja[2]) & (y >= caja[1]) & (y <= caja[3])).sum())
@paso("g235_zona")
def _():
    z = ore.sql("select id, geom from %s where %s" % (V, ore.geo.intersects_box("geom", *caja)))
    return {"tipo": type(z).__name__, "filas": len(z), "esperado": esperado, "crs": str(getattr(z, "crs", None))}
@paso("g235_h3")
def _():
    h = ore.sql("select h3_latlng_to_cell(ST_Y(p), ST_X(p), 8) celda, count(*) n from (select ST_Transform(geom, 'EPSG:25830', 'EPSG:4326', true) p from %s) group by 1" % V)
    return {"celdas": len(h), "filas": int(h["n"].sum())}

# ── V5 · los fijos, escritos por el ORE desplegado ────────────────────────────
fijos = json.load(open("/tmp/m/fijos.json"))
for crs, f in fijos.items():
    d = crs.replace(":", "-")
    VF = ns + ".medida_g2_v5_" + d.lower().replace("-", "_")
    filas = f["filas"]
    gf = geopandas.GeoDataFrame(
        {"id": [r["id"] for r in filas], "subtipo": [r["subtipo"] for r in filas]},
        geometry=[None if r["wkb"] is None else shapely.from_wkb(bytes.fromhex(r["wkb"])) for r in filas],
        crs=crs).rename_geometry("geom")
    estado = {}
    @paso("v5_write_" + d)
    def _():
        e = ore.write(VF, gf); estado["ml"] = e["metadata_location"]; return {"filas": e.get("rows"), "ml": e["metadata_location"]}
    if "ml" not in estado:
        continue
    raiz = "/tmp/v5/" + d + "/tabla"
    @paso("v5_bajar_" + d)
    def _():
        n = bajar(estado["ml"], raiz); return {"ficheros": n}
    v0 = os.path.basename(estado["ml"])[: -len(".metadata.json")]
    @paso("v5_promover_" + d)
    def _():
        p = subprocess.run([sys.executable, "/tmp/m/promover.py", raiz], capture_output=True, text=True)
        return {"salida": (p.stdout + p.stderr)[-300:]}
    v1 = sorted(os.path.basename(m)[: -len(".metadata.json")] for m in glob.glob(raiz + "/metadata/*.metadata.json"))[-1]
    a = (glob.glob(raiz + "/data/*.parquet") or [""])[0]
    args = ["A=" + a, "B=%s@%s" % (raiz, v0), "C=%s@%s" % (raiz, v1)]
    lineas = ""
    for lector in ("duckdb", "duckdb-spatial", "pyarrow", "geopandas"):
        p = subprocess.run([sys.executable, "/tmp/m/lectores.py", lector, "/tmp/m/fijos.json"] + args, capture_output=True, text=True)
        lineas += p.stdout
        if p.stderr.strip():
            di("v5_lector_stderr", lector=lector, stderr=p.stderr[-300:])
    juzgar(crs, lineas)

# ── V6 · la tabla v3 ajena del banco ──────────────────────────────────────────
ajena = {}
@paso("v6_leer")
def _():
    con = ore._duckdb()
    mls = sorted(r[0] for r in con.execute("select filename from glob('%s/metadata/*.metadata.json')" % BANCO).fetchall())
    ml = mls[-1]; ajena["ml"] = ml
    ore._iceberg(ml)
    q = "select id, nombre, categoria, geom from iceberg_scan('%s')" % ml
    tipo = con.execute("select typeof(geom) from (%s) limit 1" % q).fetchone()[0]
    lim = " limit 1000" if ENSAYO else ""
    t = con.execute("select id, nombre, categoria, ST_AsWKB(geom) w from (%s) order by id%s" % (q, lim)).fetch_arrow_table()
    ajena["t"] = t
    ajena["md5"] = hashlib.md5(b"".join(w.as_py() for w in t.column("w") if w.is_valid)).hexdigest()
    return {"ml": ml, "tipo": tipo, "filas": t.num_rows, "md5": ajena["md5"]}
VA = ns + ".medida_g2_ajena"
GEO[VA] = {"geom": {"type": "Geometry", "crs": "OGC:CRS84"}}
@paso("v6_copiar")
def _():
    t = ajena["t"]
    ga = geopandas.GeoDataFrame({"id": t.column("id").to_numpy(), "nombre": t.column("nombre").to_pylist(), "categoria": t.column("categoria").to_pylist()},
                                geometry=shapely.from_wkb(t.column("w").to_numpy(zero_copy_only=False)), crs="OGC:CRS84").rename_geometry("geom")
    e = ore.write(VA, ga); return {"filas": e.get("rows")}
@paso("v6_releer")
def _():
    l = ore.over(VA).sort_values("id")
    md5 = hashlib.md5(b"".join(shapely.to_wkb(v) for v in l.geometry if v is not None)).hexdigest()
    return {"tipo": type(l).__name__, "filas": len(l), "crs": str(l.crs), "md5": md5, "igual": md5 == ajena["md5"]}
di("fin")
'''


def main():
    v5 = S + "v5" + os.sep
    ficheros = {n: empaquetar(v5 + n) for n in ("fijos.json", "lectores.py", "juzgar.py")}
    ficheros["promover.py"] = empaquetar(S + "g24" + os.sep + "promover.py")
    guion = (GUION.replace("__ENSAYO__", repr(ENSAYO)).replace("__BANCO__", BANCO)
             .replace("__FICHEROS__", repr(ficheros)))
    print("guion: %d bytes" % len(guion))
    fecha = time.strftime("%Y%m%d-%H%M")
    log = vara.job(f"geo-g2-{int(time.time()) % 10**6}", ["python3", "-u", "-c", guion],
                   imagen=vara.PUESTO, plazo=1800, memoria="3Gi")
    open(S + f"g2-victor-{fecha}{'-ensayo' if ENSAYO else ''}.log", "w", encoding="utf-8", newline="\n").write(log)
    print("\n".join(l[:300] for l in log.splitlines() if l.startswith(("G2|", "V5|", "|")) or "Traceback" in l))


if __name__ == "__main__":
    main()
