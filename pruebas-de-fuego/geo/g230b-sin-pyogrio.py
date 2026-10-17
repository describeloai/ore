import os, subprocess, sys, time
def mb(d, excl=("numpy", "pandas", "pyarrow")):
    return sum(os.path.getsize(os.path.join(r, f)) for r, _, fs in os.walk(d) for f in fs if not any(f"/{x}" in r.lower() for x in excl)) / 1e6
subprocess.run([sys.executable, "-m", "pip", "install", "-q", "--no-cache-dir", "--target", "/tmp/A", "geopandas"], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
for p in sorted(os.listdir("/tmp/A")):
    if os.path.isdir(f"/tmp/A/{p}") and not p.endswith((".dist-info", "__pycache__")):
        t = mb(f"/tmp/A/{p}", ())
        if t > 1: print(f"G230|A|{p}|{t:.0f} MB")
subprocess.run([sys.executable, "-m", "pip", "install", "-q", "--no-cache-dir", "--target", "/tmp/A2", "--no-deps", "geopandas", "shapely", "pyproj", "packaging"], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
print(f"G230|A2_geopandas_sin_pyogrio|tamaño|{mb('/tmp/A2'):.0f} MB")
sys.path.insert(0, "/tmp/A2")
import duckdb, geopandas
c = duckdb.connect(); c.execute("set extension_directory='/tmp/ext'; install spatial; load spatial")
t = c.sql("select id, ST_GeomFromWKB(ST_AsWKB(geom))::GEOMETRY('EPSG:25830') geom from '/salida/ore-geo.parquet'").fetch_arrow_table()
t0 = time.time(); g = geopandas.GeoDataFrame.from_arrow(t)
print(f"G230|A2|from_arrow|{time.time()-t0:.2f} s|{len(g)} filas · crs {g.crs.to_string()} · area {g.geometry.iloc[:3].area.sum()}")
try:
    geopandas.read_file("/salida/ore-geo.parquet")
except Exception as e:
    print(f"G230|A2|read_file sin pyogrio|{type(e).__name__}: {str(e)[:120]}")
t0 = time.time(); out = g.to_arrow(geometry_encoding="WKB"); import pyarrow as pa
tab = pa.table(out)
print(f"G230|A2|to_arrow|{time.time()-t0:.2f} s|{ {k.decode(): v.decode()[:60] for k, v in (tab.schema.field('geom').metadata or {}).items()} }")
