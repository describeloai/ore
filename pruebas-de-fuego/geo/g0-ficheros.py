"""G0·2 (ADR 0067): los ficheros y las cargas de las fuentes de prueba, desde el Overture del banco.

- ficheros/: los municipios de la Comunidad de Madrid en GeoJSON, Shapefile (zip), GeoPackage, KML,
  FlatGeobuf, File Geodatabase y GeoParquet; los lugares de Madrid como CSV con lat/lon, como CSV con
  dirección y como CSV con WKT.
- carga/: lo que se carga en PostGIS (EWKB en hexadecimal, en su CRS) y en BigQuery (WKT).
"""
import glob
import os
import shutil
import time
import zipfile

import duckdb

S = "/salida"
F, C = f"{S}/ficheros", f"{S}/carga"
for d in (F, C, f"{S}/shp"):
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
c = duckdb.connect()
c.execute("install spatial; load spatial")
t0 = time.time()
MADRID = "bbox.xmin between -4.6 and -3.0 and bbox.ymin between 39.8 and 41.2"
# Municipios de la Comunidad de Madrid (y la comunidad), de las divisiones de Overture.
c.execute(f"""create table muni as
  select id, names.primary nombre, subtype tipo, ST_Multi(geometry) geom
  from '{S}/divisiones_es.parquet'
  where subtype in ('locality', 'county', 'region') and region = 'ES-MD'""")
print("municipios de Madrid:", c.execute("select count(*), string_agg(distinct tipo, ',') from muni").fetchone(), flush=True)
# Overture 2026-09: `categories` pasó a `basic_category` + `taxonomy`; `addresses` puede ser lista o struct.
tipo_dir = dict(c.execute(f"select column_name, column_type from (describe select * from '{S}/lugares_es.parquet')").fetchall())["addresses"]
a = "addresses[1]" if tipo_dir.endswith("[]") else "addresses"
c.execute(f"""create table lug as
  select id, names.primary nombre, basic_category categoria,
         {a}.freeform direccion, {a}.locality localidad, {a}.postcode cp,
         ST_Y(geometry) lat, ST_X(geometry) lon, geometry geom
  from '{S}/lugares_es.parquet' where {MADRID}""")
print("lugares de Madrid:", c.execute("select count(*) from lug").fetchone()[0], flush=True)

# ── ficheros/ ──
for drv, nombre in (("GeoJSON", "municipios_madrid.geojson"), ("GPKG", "municipios_madrid.gpkg"),
                    ("KML", "municipios_madrid.kml"), ("FlatGeobuf", "municipios_madrid.fgb"),
                    ("OpenFileGDB", "municipios_madrid.gdb")):
    opc = ", geometry_type 'MULTIPOLYGON'" if drv == "OpenFileGDB" else ""
    c.execute(f"copy (select id, nombre, tipo, geom from muni) to '{F}/{nombre}' (format gdal, driver '{drv}'{opc})")
c.execute(f"copy (select id, nombre, tipo, geom from muni) to '{S}/shp/municipios_madrid.shp' (format gdal, driver 'ESRI Shapefile')")
with zipfile.ZipFile(f"{F}/municipios_madrid_shp.zip", "w", zipfile.ZIP_DEFLATED) as z:
    for f in glob.glob(f"{S}/shp/municipios_madrid.*"):
        z.write(f, os.path.basename(f))
c.execute(f"copy (select id, nombre, tipo, geom from muni) to '{F}/municipios_madrid.parquet' (format parquet)")
c.execute(f"copy (select id, nombre, categoria, lat, lon from lug) to '{F}/lugares_madrid_latlon.csv' (header)")
c.execute(f"copy (select id, nombre, categoria, direccion, localidad, cp from lug where direccion is not null) to '{F}/lugares_madrid_direcciones.csv' (header)")
c.execute(f"copy (select id, nombre, categoria, ST_AsText(geom) wkt from lug) to '{F}/lugares_madrid_wkt.csv' (header)")

# ── carga/ PostGIS: hex WKB (municipios en ETRS89/UTM 30N, como los daría una IDE española) ──
c.execute(f"""copy (select id, nombre, tipo,
    ST_AsHEXWKB(ST_Transform(geom, 'OGC:CRS84', 'EPSG:25830')) geom_25830
  from muni) to '{C}/pg_municipios.csv' (header)""")
c.execute(f"copy (select id, nombre, categoria, ST_AsHEXWKB(geom) geom_4326 from lug) to '{C}/pg_lugares.csv' (header)")
c.execute(f"""copy (select id, coalesce(names.primary, '') nombre, class clase, ST_AsHEXWKB(geometry) geom_4326
  from '{S}/carreteras_es.parquet' where {MADRID}) to '{C}/pg_carreteras.csv' (header)""")
# ── carga/ BigQuery: WKT (GEOGRAPHY) ──
c.execute(f"copy (select id, nombre, categoria, ST_AsText(geom) geo from lug) to '{C}/bq_lugares.csv' (header)")
c.execute(f"copy (select id, nombre, tipo, ST_AsText(geom) geo from muni) to '{C}/bq_municipios.csv' (header)")
for d in (F, C):
    for f in sorted(os.listdir(d)):
        p = f"{d}/{f}"
        tam = sum(os.path.getsize(os.path.join(r, x)) for r, _, xs in os.walk(p) for x in xs) if os.path.isdir(p) else os.path.getsize(p)
        print(f"  {d.split('/')[-1]}/{f}: {tam / 1e6:.1f} MB")
print(f"hecho en {time.time() - t0:.0f} s")
