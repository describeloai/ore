"""ORE 0068 C0·1 · El banco de cómputo: Parquet compacto (geometría en WKB) que leen los dos motores.

    python preparar-banco.py <overture_dir> <ais_dir_con_csv> <salida>

Sale:
  edificios.parquet   id, altura, geom (CRS84), utm (EPSG:25830)      34 M polígonos
  lugares.parquet     id, categoria, geom, utm                         2,2 M puntos
  municipios.parquet  id, nombre, geom                                 8 655 polígonos (Overture `locality`)
  ais.parquet         mmsi, t, sog, geom                               puntos AIS (lo que haya en <ais_dir>)
Ordenado por la curva de Hilbert, como lo dejaría el lago (0067 G2·2).
"""
import sys, time
import duckdb

ov, ais, out = sys.argv[1:4]
c = duckdb.connect()
c.execute("set extension_directory = '/opt/ore/duckdb'")
c.execute("load spatial")
c.execute("set preserve_insertion_order = false")
c.execute(f"set temp_directory = '{out}/tmp'")
c.execute("set memory_limit = '4GB'")


def paso(nombre, sql):
    import os
    if os.path.exists(f"{out}/{nombre}.parquet"):
        print(f"{nombre}: ya está", flush=True)
        return
    t = time.time()
    c.execute(f"copy ({sql}) to '{out}/{nombre}.parquet' (format parquet, compression zstd, row_group_size 122880)")
    n = c.sql(f"select count(*) from '{out}/{nombre}.parquet'").fetchone()[0]
    print(f"{nombre}: {n:,} filas en {time.time() - t:.1f} s", flush=True)


EXT = "ST_Extent(ST_MakeEnvelope(-10, 35, 5, 44))"
paso("municipios", f"""
  select id, names.primary as nombre, ST_AsWKB(geometry) as geom
  from '{ov}/divisiones_es.parquet' where subtype = 'locality'""")
paso("lugares", f"""
  select id, basic_category as categoria, ST_AsWKB(geometry) as geom,
         ST_AsWKB(ST_Transform(geometry, 'OGC:CRS84', 'EPSG:25830')) as utm
  from '{ov}/lugares_es.parquet'
  order by ST_Hilbert(geometry, {EXT})""")
paso("edificios", f"""
  select id, height as altura, ST_AsWKB(geometry) as geom,
         ST_AsWKB(ST_Transform(geometry, 'OGC:CRS84', 'EPSG:25830')) as utm
  from '{ov}/edificios_es.parquet'
  order by ST_Hilbert(geometry, {EXT})""")
paso("ais", f"""
  select "MMSI" as mmsi, strptime("# Timestamp", '%d/%m/%Y %H:%M:%S') as t, "SOG" as sog,
         ST_AsWKB(ST_Point("Longitude", "Latitude")) as geom
  from read_csv('{ais}/*.csv', header = true, types = {{'# Timestamp': 'VARCHAR'}}, ignore_errors = true)
  where "Latitude" between -90 and 90 and "Longitude" between -180 and 180
  order by ST_Hilbert(ST_Point("Longitude", "Latitude"), ST_Extent(ST_MakeEnvelope(0, 50, 20, 60)))""")
