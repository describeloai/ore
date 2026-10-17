"""ORE 0067 G2·2: las variantes de los 34 M de edificios para V4. GEOMETRY nativo + clave geo (BOTH)."""
import duckdb, os, time, shutil
c = duckdb.connect()
c.execute("install spatial; load spatial; install h3 from community; load h3")
c.execute("set threads=8; set memory_limit='6GB'; set temp_directory='/tmp/duck'; set preserve_insertion_order=true")
E = "/salida/edificios_es.parquet"
# Sin tabla intermedia: cada copia lee el origen y ordena con desbordamiento a disco.
c.execute(f"""create or replace view e as
  select row_number() over () id, geometry geom, bbox,
         ST_Hilbert(ST_Centroid(geometry), ST_Extent(ST_MakeEnvelope(-9.6, 35.8, 4.6, 44.0))) hilbert,
         h3_latlng_to_cell(ST_Y(ST_Centroid(geometry)), ST_X(ST_Centroid(geometry)), 8) h3_r8,
         h3_latlng_to_cell(ST_Y(ST_Centroid(geometry)), ST_X(ST_Centroid(geometry)), 3) h3_r3
  from '{E}'""")
V = "/salida/g22"
shutil.rmtree(V, ignore_errors=True); os.makedirs(V)
def copia(nombre, sql, rg, extra=""):
    t0 = time.time()
    c.execute(f"copy ({sql}) to '{V}/{nombre}' (format parquet, geoparquet_version 'BOTH', compression zstd, row_group_size {rg}{extra})")
    print(nombre, f"{time.time()-t0:.0f} s", flush=True)
copia("a_tal_cual.parquet", "select id, geom, bbox from e", 100_000)
copia("b_hilbert_100k.parquet", "select id, geom, bbox, h3_r8 from e order by hilbert", 100_000)
copia("c_hilbert_25k.parquet", "select id, geom, bbox from e order by hilbert", 25_000)
copia("d_hilbert_400k.parquet", "select id, geom, bbox from e order by hilbert", 400_000)
copia("e_h3r3", "select id, geom, bbox, h3_r3 from e order by h3_r3, hilbert", 100_000, ", partition_by (h3_r3)")
for r, _, fs in os.walk(V):
    pass
print("tamaños:", {n: round(sum(os.path.getsize(os.path.join(r, f)) for r, _, fs in os.walk(os.path.join(V, n)) for f in fs) / 1e9, 2) if os.path.isdir(os.path.join(V, n)) else round(os.path.getsize(os.path.join(V, n)) / 1e9, 2) for n in os.listdir(V)})
print("particiones e:", len(os.listdir(f"{V}/e_h3r3")))
