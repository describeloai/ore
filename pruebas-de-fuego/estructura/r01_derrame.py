"""R0·1 · Qué operaciones de DuckDB terminan derramando y cuáles fallan (docs/ejecuciones.md §8.10).

Un puesto con la memoria de L (4 GiB, DuckDB con 1 GB) y disco de sobra: cada caso corre en un
proceso hijo (si muere, el siguiente sigue) y escribe su resultado a Parquet, como un
`create or replace dataset`. Por caso: termina o no, con qué error, cuánto tardó, el pico de
memoria del hijo y lo más que derramó. Una línea JSON por caso.
"""
import json
import os
import shutil
import subprocess
import sys
import threading
import time

import duckdb

T = os.environ.get("TRABAJO", "/trabajo")
DB = os.path.join(T, "r01.duckdb")
DERRAME = os.path.join(T, "derrame")
MEMORIA = os.environ.get("MEMORIA", "1GB")
HILOS = int(os.environ.get("HILOS", "2"))
TOPE_S = int(os.environ.get("TOPE_S", "900"))
FILAS = int(os.environ.get("FILAS", "40000000"))


def linea(**d):
    print(json.dumps(d, ensure_ascii=False), flush=True)


def sembrar():
    t0 = time.time()
    c = duckdb.connect(DB)
    c.execute("set threads to 8")
    c.execute("set extension_directory = '/opt/ore/duckdb'")
    c.execute("load spatial")
    c.execute(f"""create or replace table a as
        select (hash(range) % {FILAS // 2})::bigint k, random() v,
               md5(range::varchar)[:16] s
        from range({FILAS})""")
    c.execute(f"create or replace table b as select range::bigint k, random() w from range({FILAS // 2})")
    c.execute("""create or replace table pts as
        select range id, ST_Point(random() * 10, random() * 10) geom from range(4000000)""")
    c.execute("""create or replace table zonas as
        select range id, ST_Buffer(ST_Point(random() * 10, random() * 10), 0.05) geom from range(20000)""")
    c.close()
    linea(evento="sembrado", s=round(time.time() - t0, 1), filas=FILAS,
          mb=round(os.path.getsize(DB) / 1e6))


CASOS = {
    "agrupa_alta": "select k, sum(v) sv, count(*) n from a group by k",
    "junta": "select a.k, a.v, b.w from a join b using (k)",
    "ordena": "select * from a order by v",
    "ventana": "select k, v, row_number() over (partition by k order by v) rn from a",
    "distinto": "select count(distinct s) n from a",
    "lista": "select k, list(v) l from a group by k",
    "string_agg": "select k % 1000000 g, string_agg(s, ',') t from a group by g",
    "mediana": "select k % 1000000 g, median(v) m, quantile_cont(v, 0.9) p90 from a group by g",
    "geo_junta": "select z.id, count(*) n from pts p join zonas z on ST_Intersects(p.geom, z.geom) group by z.id",
    "ctas_geo": "select *, ST_Point(v, v) g from a",
    "tres_bloqueantes": "select x.k, x.sv, b.w from (select k, sum(v) sv from a group by k) x join b using (k) order by x.sv",
    # S0·b (docs/elastic-sessions.md §5): la lista cerrada, ampliada.
    "array_agg": "select k, array_agg(v) l from a group by k",
    "mode": "select k % 1000000 g, mode(v) m from a group by g",
    "histogram": "select k % 100000 g, histogram(s) h from a group by g",
    "arg_max": "select k % 1000000 g, arg_max(s, v) s from a group by g",
    "json_group_array": "select k % 1000000 g, json_group_array(v) j from a group by g",
    "pivot": "pivot (select k % 50 c, k % 1000000 r, v from a) on c using sum(v) group by r",
    "recursiva": "with recursive t(n) as (select 1 union all select n + 1 from t where n < 3000000) select n from t",
    "ventana_marco": "select k, sum(v) over (order by v rows between 1000 preceding and current row) s from a",
    "approx_quantile": "select k % 1000000 g, approx_quantile(v, 0.9) p from a group by g",
    "st_union_agg": "select id % 100 g, ST_Union_Agg(ST_Buffer(geom, 0.01)) u from (select * from pts limit 400000) group by g",
    "st_collect": "select id % 1000 g, ST_Collect(list(geom)) c from pts group by g",
}

HIJO = r"""
import duckdb, sys, time
# si el kernel mata por memoria, que sea a este caso y no al banco entero
open("/proc/self/oom_score_adj", "w").write("1000")
c = duckdb.connect(sys.argv[1], read_only=True)
c.execute("set memory_limit = '%s'" % sys.argv[2])
c.execute("set threads to %s" % sys.argv[3])
c.execute("set temp_directory = '%s'" % sys.argv[4])
c.execute("set max_temp_directory_size = '300GB'")
c.execute("set preserve_insertion_order = false")
c.execute("set extension_directory = '/opt/ore/duckdb'")
c.execute("load spatial")
t0 = time.time()
c.execute("copy (%s) to '%s' (format parquet)" % (sys.argv[5], sys.argv[6]))
print(round(time.time() - t0, 2))
"""


def du(d):
    n = 0
    for r, _, fs in os.walk(d):
        for f in fs:
            try:
                n += os.path.getsize(os.path.join(r, f))
            except OSError:
                pass
    return n


def caso(nombre, q):
    shutil.rmtree(DERRAME, ignore_errors=True)
    os.makedirs(DERRAME, exist_ok=True)
    salida = os.path.join(T, "salida.parquet")
    pico = {"b": 0, "rss": 0}
    fin = threading.Event()
    t0 = time.time()
    p = subprocess.Popen([sys.executable, "-c", HIJO, DB, MEMORIA, str(HILOS), DERRAME, q, salida],
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def mirar():
        while not fin.wait(0.5):
            pico["b"] = max(pico["b"], du(DERRAME))
            try:
                for l in open("/proc/%d/status" % p.pid):
                    if l.startswith("VmHWM:"):
                        pico["rss"] = max(pico["rss"], int(l.split()[1]) * 1024)
            except OSError:
                pass

    h = threading.Thread(target=mirar, daemon=True)
    h.start()
    try:
        out, err = p.communicate(timeout=TOPE_S)
        estado = "ok" if p.returncode == 0 else ("killed" if p.returncode < 0 else "error")
    except subprocess.TimeoutExpired:
        p.kill()
        out, err = p.communicate()
        estado = "tope"
    fin.set()
    h.join(1)
    pico["b"] = max(pico["b"], du(DERRAME))
    lineas = err.decode("utf-8", "replace").strip().splitlines()
    error = next((l for l in lineas if "Error" in l or "Exception" in l), lineas[-1] if lineas else "")[:300] if estado != "ok" else None
    linea(evento="caso", caso=nombre, estado=estado, codigo=p.returncode, s=round(time.time() - t0, 1),
          derrame_mb=round(pico["b"] / 1e6), pico_mb=round(pico["rss"] / 2**20), salida_mb=round(os.path.getsize(salida) / 1e6) if os.path.exists(salida) else None,
          memoria=MEMORIA, hilos=HILOS, error=error)
    try:
        os.remove(salida)
    except OSError:
        pass


def main():
    linea(evento="inicio", duckdb=duckdb.__version__, memoria=MEMORIA, hilos=HILOS,
          cgroup_mb=int(open("/sys/fs/cgroup/memory.max").read().strip()) // 2**20
          if os.path.exists("/sys/fs/cgroup/memory.max") and open("/sys/fs/cgroup/memory.max").read().strip() != "max" else None)
    sembrar()
    solo = [c for c in os.environ.get("CASOS", "").split(",") if c]
    for nombre, q in CASOS.items():
        if not solo or nombre in solo:
            caso(nombre, q)
    linea(evento="fin")


if __name__ == "__main__":
    main()
