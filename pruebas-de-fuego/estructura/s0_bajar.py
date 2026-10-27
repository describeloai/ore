"""S0·c · ¿Se puede bajar en caliente? (docs/elastic-sessions.md §7).

El pod nace pequeño, pide crecer (`pide-crecer`, el lanzador lo lleva a 3 GiB), DuckDB usa ~2 GB
en una agrupación, y luego se intenta devolver la memoria: bajar `memory_limit`, vaciar el
asignador, cerrar la conexión. En cada paso, la memoria residente. Al final pide bajar el pod
(`pide-bajar`, a 1 GiB): Kubernetes no baja por debajo de lo que se usa.
"""
import gc
import json
import os
import time

import duckdb

T = os.environ.get("TRABAJO", "/trabajo")
CG = "/sys/fs/cgroup/memory.max"


def linea(**d):
    print(json.dumps(d, ensure_ascii=False), flush=True)


def cgroup_mb():
    v = open(CG).read().strip()
    return None if v == "max" else int(v) // 2**20


def rss_mb():
    return int(open("/proc/self/statm").read().split()[1]) * os.sysconf("SC_PAGE_SIZE") // 2**20


def cgroup_uso_mb():
    try:
        return int(open("/sys/fs/cgroup/memory.current").read()) // 2**20
    except OSError:
        return None


def esperar_cgroup(cond, s=90):
    t0 = time.time()
    while time.time() - t0 < s:
        if cond(cgroup_mb()):
            return round(time.time() - t0, 1)
        time.sleep(0.5)
    return None


def foto(paso):
    linea(evento="memoria", paso=paso, rss_mb=rss_mb(), cgroup_uso_mb=cgroup_uso_mb(), cgroup_mb=cgroup_mb())


def main():
    linea(evento="inicio", duckdb=duckdb.__version__, cgroup_mb=cgroup_mb())
    base = cgroup_mb()
    linea(evento="pide-crecer")
    linea(evento="crecio", s=esperar_cgroup(lambda c: c and c > base), cgroup_mb=cgroup_mb())
    c = duckdb.connect()
    c.execute("set memory_limit = '2GB'")
    c.execute("set threads to 2")
    c.execute("set temp_directory = '%s'" % os.path.join(T, "derrame"))
    c.execute("set preserve_insertion_order = false")
    t0 = time.time()
    c.execute("""copy (select (hash(range) %% 8000000)::bigint k, sum(random()) sv, count(*) n
                 from range(30000000) group by k) to '%s' (format parquet)""" % os.path.join(T, "s.parquet"))
    linea(evento="consulta", s=round(time.time() - t0, 1))
    foto("tras la consulta")
    c.execute("set memory_limit = '256MB'")
    time.sleep(5)
    foto("memory_limit 256MB, 5 s")
    try:
        c.execute("set allocator_flush_threshold = '1MB'")
        c.execute("select 1").fetchall()
        time.sleep(3)
        foto("allocator_flush_threshold 1MB")
    except Exception as e:  # noqa: BLE001
        linea(evento="sin-flush", error=str(e)[:200])
    c.close()
    del c
    gc.collect()
    time.sleep(3)
    foto("conexión cerrada")
    linea(evento="pide-bajar")
    s = esperar_cgroup(lambda m: m and m <= 1024, 90)
    linea(evento="bajo", s=s, cgroup_mb=cgroup_mb())
    foto("tras bajar")
    linea(evento="fin")


if __name__ == "__main__":
    main()
