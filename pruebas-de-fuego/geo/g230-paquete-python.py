"""G2·3·0 (ADR 0067): qué paquete de Python para entregar geometría con su CRS. En la imagen puesto-python."""
import os, subprocess, sys, time, json
OPC = {"A_geopandas": ["geopandas"], "B_shapely": ["shapely"], "C_geoarrow": ["geoarrow-pyarrow"]}
base = set(l.split("==")[0].lower() for l in subprocess.run([sys.executable, "-m", "pip", "freeze"], capture_output=True, text=True).stdout.split())
for o, pk in OPC.items():
    d = f"/tmp/{o}"
    subprocess.run([sys.executable, "-m", "pip", "install", "-q", "--no-cache-dir", "--target", d] + pk, check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    # Solo lo que la imagen no trae ya (pandas, numpy, pyarrow están).
    nuevos = sorted({n.split("-")[0].lower() for n in os.listdir(d) if n.endswith(".dist-info")} - base)
    mb = sum(os.path.getsize(os.path.join(r, f)) for r, _, fs in os.walk(d) for f in fs
             if not any(f"/{x}" in r.lower() for x in ("numpy", "pandas", "pyarrow"))) / 1e6
    print(f"G230|{o}|tamaño|{mb:.0f} MB|paquetes nuevos: {nuevos}", flush=True)

import duckdb
c = duckdb.connect(); c.execute("set extension_directory='/tmp/ext'; install spatial; load spatial")
f = "/salida/ore-geo.parquet"
for crs in ("OGC:CRS84", "EPSG:25830"):
    t0 = time.time()
    t = c.sql(f"select id, ST_GeomFromWKB(ST_AsWKB(geom))::GEOMETRY('{crs}') geom from '{f}'").fetch_arrow_table()
    md = t.schema.field("geom").metadata or {}
    print(f"G230|duckdb→arrow|{crs}|{time.time()-t0:.2f} s|{ {k.decode(): v.decode()[:80] for k, v in md.items()} }", flush=True)
t = c.sql(f"select id, ST_GeomFromWKB(ST_AsWKB(geom))::GEOMETRY('EPSG:25830') geom from '{f}'").fetch_arrow_table()

sys.path.insert(0, "/tmp/A_geopandas")
import geopandas
t0 = time.time(); g = geopandas.GeoDataFrame.from_arrow(t); ta = time.time() - t0
print(f"G230|A_geopandas|from_arrow|{ta:.2f} s|{len(g)} filas · crs {g.crs.to_string() if g.crs else None} · {type(g).__name__}", flush=True)
t0 = time.time(); p = t.to_pandas(); print(f"G230|pandas sin geo|to_pandas|{time.time()-t0:.2f} s|columna {p['geom'].dtype}", flush=True)
sys.path.insert(0, "/tmp/B_shapely")
import shapely
t0 = time.time(); a = shapely.from_wkb(t.column("geom").combine_chunks().storage.to_numpy(zero_copy_only=False) if hasattr(t.column("geom").combine_chunks(), "storage") else t.column("geom").to_numpy()); print(f"G230|B_shapely|from_wkb|{time.time()-t0:.2f} s|{len(a)} geometrías, sin CRS (shapely no lo lleva)", flush=True)
sys.path.insert(0, "/tmp/C_geoarrow")
try:
    import geoarrow.pyarrow as ga
    t0 = time.time(); arr = ga.as_wkb(t.column("geom")); print(f"G230|C_geoarrow|as_wkb|{time.time()-t0:.2f} s|{arr.type} · crs {getattr(arr.type, 'crs', None)}", flush=True)
except Exception as e:
    print(f"G230|C_geoarrow|falla|{str(e)[:160]}")
