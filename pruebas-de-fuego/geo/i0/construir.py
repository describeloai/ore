"""ORE 0072 I0·d · construir el índice de una capa entero, sobre los viajes de SpatialBench sf100 (600 M, sin orden).

Lo que hará el Job de la capa (I2), con DuckDB como motor de partida: leer la fuente, pasar a 3857, la caja, la
clave Hilbert sobre la extensión de los datos (rejilla congelada), ordenar fuera de memoria y escribir Parquet
con grupos de 100 000 filas. Para cada tamaño: tiempo, filas/s, disco temporal de pico, bytes escritos y la
profundidad del resultado (cuántos grupos cubren, de media, un punto con datos). Emite «I0D|…».

Se lanza con `correr.py` (nodo pesado: 8 vCPU, 44 GiB, disco efímero de 48 GiB).
"""
import json
import os
import shutil
import subprocess
import sys
import threading
import time

sys.path.insert(0, "/opt/ore")
import ore  # noqa: E402

BANCO = "https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo"
TRIP = BANCO + "/spatialbench/sf100/trip.parquet"
TAMANOS = [int(x) for x in os.environ.get("I0_TAMANOS", "60000000,240000000").split(",")]
TRABAJO = "/trabajo"
TEMP = TRABAJO + "/temp"


def di(que, **kw):
    print("I0D|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)


def du(ruta):
    try:
        return int(subprocess.run(["du", "-sb", ruta], capture_output=True, text=True).stdout.split()[0])
    except Exception:
        return 0


class Pico(threading.Thread):
    """El disco de `/trabajo` (temporal de la ordenación + salida) cada 2 s, y su pico."""

    def __init__(self):
        super().__init__(daemon=True)
        self.pico, self.parar = 0, threading.Event()

    def run(self):
        while not self.parar.is_set():
            self.pico = max(self.pico, du(TRABAJO))
            self.parar.wait(2)


os.makedirs(TEMP, exist_ok=True)
con = ore._duckdb()
for e in ("spatial", "httpfs"):
    con.execute("load " + e)
con.execute("set geometry_always_xy = true")
con.execute("set memory_limit = '36GB'")
con.execute("set threads = 8")
con.execute(f"set temp_directory = '{TEMP}'")
con.execute("set preserve_insertion_order = false")
con.execute("create or replace secret banco (type http, bearer_token '%s', scope '%s')" % (ore._token_de_google(), BANCO))

cols = [n for n, *_ in con.execute(f"describe select * from read_parquet('{TRIP}')").fetchall()]
di("esquema", columnas=cols)
ID = "t_tripkey" if "t_tripkey" in cols else None
PROPS = [c for c in ("t_fare", "t_tip", "t_pickuptime") if c in cols]

for n in TAMANOS:
    fuente = f"(select * from read_parquet('{TRIP}') limit {n})"
    salida = f"{TRABAJO}/indice-{n}.parquet"
    pico = Pico()
    pico.start()
    t0 = time.time()
    # 1 · la extensión (la rejilla congelada): una pasada sobre la fuente
    ext = con.execute(f"""
      select min(x), min(y), max(x), max(y) from (
        select ST_X(g) x, ST_Y(g) y from (
          select ST_Transform(ST_GeomFromWKB(t_pickuploc), 'EPSG:4326', 'EPSG:3857') g from {fuente}))""").fetchone()
    t_ext = time.time() - t0
    # 2 · el índice: 3857, caja, clave Hilbert, orden fuera de memoria, grupos de 100 000
    t1 = time.time()
    caja = "ST_MakeBox2D(ST_Point(%r, %r), ST_Point(%r, %r))" % ext
    sel_id = f"{ID} id" if ID else "row_number() over () id"
    props = "".join(f", {c}" for c in PROPS)
    con.execute(f"""
      copy (
        select id, ST_AsWKB(g) geom, {{'xmin': ST_X(g), 'ymin': ST_Y(g), 'xmax': ST_X(g), 'ymax': ST_Y(g)}} geom_bbox{props}
        from (select {sel_id}, ST_Transform(ST_GeomFromWKB(t_pickuploc), 'EPSG:4326', 'EPSG:3857') g{props} from {fuente})
        order by ST_Hilbert(g, {caja})
      ) to '{salida}' (format parquet, row_group_size 100000, compression zstd)""")
    t_ind = time.time() - t1
    pico.parar.set()
    pico.join()
    # 3 · lo que salió: grupos, bytes y profundidad (cajas de los grupos frente a 2 000 puntos de la salida)
    con.execute(f"""
      create or replace temp table grupos as
      select row_group_id g, min(try_cast(stats_min_value as double)) filter (where path_in_schema = 'geom_bbox, xmin') x0,
             max(try_cast(stats_max_value as double)) filter (where path_in_schema = 'geom_bbox, xmax') x1,
             min(try_cast(stats_min_value as double)) filter (where path_in_schema = 'geom_bbox, ymin') y0,
             max(try_cast(stats_max_value as double)) filter (where path_in_schema = 'geom_bbox, ymax') y1
      from parquet_metadata('{salida}') group by 1""")
    ng, prof = con.execute(f"""
      with q as (select geom_bbox.xmin x, geom_bbox.ymin y from read_parquet('{salida}') using sample 2000)
      select (select count(*) from grupos),
             avg(c) from (select count(*) c from q join grupos on x between x0 and x1 and y between y0 and y1 group by q.x, q.y)""").fetchone()
    filas = con.execute(f"select count(*), sum(hash(id)) from read_parquet('{salida}')").fetchone()
    di("construido", filas=filas[0], suma_ids=str(filas[1]), s_extension=round(t_ext, 1), s_indice=round(t_ind, 1),
       filas_s=round(n / (t_ext + t_ind)), filas_s_indice=round(n / t_ind), grupos=ng, profundidad=round(prof or 0, 2),
       bytes=os.path.getsize(salida), disco_pico=pico.pico, extension=ext)
    os.remove(salida)
    shutil.rmtree(TEMP, ignore_errors=True)
    os.makedirs(TEMP, exist_ok=True)
