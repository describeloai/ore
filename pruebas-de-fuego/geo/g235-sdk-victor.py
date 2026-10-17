"""ADR 0067 G2·3·5 · el SDK de Python con geometría, EN VICTOR, con la imagen del puesto de main.

Un Job en t-victor con `puesto-python:1` y el testigo del agente (como `la-escritura-en-demo.py`):
  · la imagen trae `spatial`, `h3` y geopandas/shapely/pyproj sin red;
  · `ore.write` de un GeoDataFrame de N puntos en EPSG:25830 llega al escritor geo (v3, `ore.geo.geom`);
  · `ore.over` lo devuelve como GeoDataFrame con su CRS;
  · `ore.sql` con `ore.geo.intersects_box` filtra por zona.
Aquí no hay puesto abierto por una persona: `/puestos//datos/<vista>` se resuelve por la ficha del
dataset, y su `geo` se pone a mano (el puntero lo calcula de los tipos del árbol, G2·3·1, y esta tabla
de medida no tiene documento). Deja `<paquete>.medida_geo_py`.

Uso:  python pruebas-de-fuego/geo/g235-sdk-victor.py [N]      deja g235-sdk-<fecha>.log
"""
import os
import sys
import time

S = os.path.dirname(os.path.abspath(__file__)) + os.sep
sys.path.insert(0, S)
import vara  # noqa: E402

N = int(sys.argv[1]) if len(sys.argv) > 1 else 100_000

GUION = r'''
import json, os, sys, time, urllib.parse, urllib.request
def testigo():
    d = os.environ["DIRECCION"].rstrip("/"); realm = os.environ.get("REALM", "rubix")
    cli = open("/puesto/agente-cliente").read().strip(); sec = open("/puesto/agente-secreto").read().strip()
    datos = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": cli, "client_secret": sec}).encode()
    with urllib.request.urlopen(d + "/realms/%s/protocol/openid-connect/token" % realm, data=datos, timeout=20) as r:
        return {"authorization": "Bearer " + json.load(r)["access_token"]}
def di(que, **kw): print("G235|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)
N = __N__
sys.path.insert(0, "/opt/ore")
t0 = time.time()
import duckdb, geopandas, shapely, pyproj, numpy as np
import ore
di("importar", s=round(time.time() - t0, 2), geopandas=geopandas.__version__, shapely=shapely.__version__, pyproj=pyproj.__version__, duckdb=duckdb.__version__)
c = ore._duckdb()
for e in ("spatial", "h3"):
    try:
        c.execute("load " + e); di("extension", nombre=e, ok=True)
    except Exception as ex:
        di("extension", nombre=e, ok=False, error=str(ex)[:200])
ore.session._cabeceras = testigo(); ore.session.id = ""
_pedir = ore.session.pedir
def pedir(metodo, ruta, *a, **kw):
    if ruta.startswith("/puestos//datos/"):
        c, r = _pedir("GET", "/datasets/" + ruta.rsplit("/", 1)[1].replace(".", "/"))
        if isinstance(r, dict):
            r["geo"] = {"geom": {"type": "Geometry<Point, EPSG:25830>", "crs": "EPSG:25830"}}
        return c, r
    return _pedir(metodo, ruta, *a, **kw)
ore.session.pedir = pedir
c_, r = ore.session.pedir("GET", "/v1/namespaces")
ns = sorted(n[0] for n in r["namespaces"])[0]
rng = np.random.default_rng(7)
x = rng.uniform(430000, 450000, N); y = rng.uniform(4465000, 4485000, N)
g = geopandas.GeoDataFrame({"id": np.arange(N, dtype="int64"), "valor": rng.normal(size=N)},
                           geometry=geopandas.points_from_xy(x, y), crs="EPSG:25830").rename_geometry("geom")
t0 = time.time()
try:
    e = ore.write(ns + ".medida_geo_py", g)
    di("write", s=round(time.time() - t0, 2), filas=e.get("rows"), metadata_location=e.get("metadata_location"))
except Exception as ex:
    di("write", s=round(time.time() - t0, 2), error=(type(ex).__name__ + ": " + str(ex))[:400]); raise
c_, f = _pedir("GET", "/datasets/%s/medida_geo_py" % ns)
di("ficha", codigo=c_, claves=sorted(f)[:30] if isinstance(f, dict) else None)
t0 = time.time()
l = ore.over(ns + ".medida_geo_py")
di("over", s=round(time.time() - t0, 2), tipo=type(l).__name__, filas=len(l), crs=str(getattr(l, "crs", None)),
   iguales=bool(isinstance(l, geopandas.GeoDataFrame) and l.sort_values("id").geometry.reset_index(drop=True).geom_equals(g.geometry).all()))
caja = (435000, 4470000, 437000, 4472000)
esperado = int(((x >= caja[0]) & (x <= caja[2]) & (y >= caja[1]) & (y <= caja[3])).sum())
for i in range(3):
    t0 = time.time()
    z = ore.sql("select id, geom from %s.medida_geo_py where %s" % (ns, ore.geo.intersects_box("geom", *caja)))
    di("zona", vez=i, s=round(time.time() - t0, 2), tipo=type(z).__name__, filas=len(z), esperado=esperado, crs=str(getattr(z, "crs", None)))
t0 = time.time()
h = ore.sql("select h3_latlng_to_cell(ST_Y(ST_Transform(geom, 'EPSG:25830', 'EPSG:4326', true)), ST_X(ST_Transform(geom, 'EPSG:25830', 'EPSG:4326', true)), 8) as celda, count(*) n from %s.medida_geo_py group by 1 order by 2 desc limit 3" % ns)
di("h3", s=round(time.time() - t0, 2), celdas=len(h))
di("fin")
'''


def main():
    fecha = time.strftime("%Y%m%d-%H%M")
    log = vara.job(f"geo-g235-{int(time.time()) % 10**6}", ["python3", "-u", "-c", GUION.replace("__N__", str(N))],
                   imagen=vara.PUESTO, plazo=1800, memoria="3Gi")
    open(S + f"g235-sdk-{fecha}.log", "w", encoding="utf-8", newline="\n").write(log)
    print("\n".join(l for l in log.splitlines() if l.startswith("G235|") or "Error" in l or "Traceback" in l))


if __name__ == "__main__":
    main()
