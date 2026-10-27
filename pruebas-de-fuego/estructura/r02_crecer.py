"""R0·2 · ¿Un puesto puede crecer mientras corre? (docs/ejecuciones.md §8.10).

Tres pasadas de la misma consulta (agrupar 120 M filas en 60 M grupos, que en L derrama):
  A · fija baja: DuckDB con el 50 % del cgroup de L (4 GiB).
  B · crece: al empezar pide crecer (línea `pide-crecer`); el lanzador redimensiona el pod en
      caliente (`kubectl patch --subresource resize`) y un hilo de aquí, al ver subir
      `memory.max`, sube `memory_limit` desde otra conexión. ¿Lo usa la consulta en marcha?
  C · fija alta: con la memoria nueva desde el principio (lo ideal).
Al final pide crecer a lo imposible (`pide-imposible`): ¿qué pasa si el nodo no tiene sitio?
"""
import json
import os
import threading
import time

import duckdb

T = os.environ.get("TRABAJO", "/trabajo")
DERRAME = os.path.join(T, "derrame")
FILAS = int(os.environ.get("FILAS", "120000000"))
CG = "/sys/fs/cgroup/memory.max"
Q = "select k, sum(v) sv, count(*) n, max(s) s from g group by k"


def linea(**d):
    print(json.dumps(d, ensure_ascii=False), flush=True)


def cgroup_mb():
    try:
        v = open(CG).read().strip()
    except OSError:
        v = "max"
    return int(os.environ.get("CGROUP_MB", "4096")) if v == "max" else int(v) // 2**20


def rss_mb():
    return int(open("/proc/self/statm").read().split()[1]) * os.sysconf("SC_PAGE_SIZE") // 2**20


def du(d):
    n = 0
    for r, _, fs in os.walk(d):
        for f in fs:
            try:
                n += os.path.getsize(os.path.join(r, f))
            except OSError:
                pass
    return n // 10**6


def pasada(c, nombre, limite_mb, crecer=False):
    # El directorio de derrame es uno para toda la vida de DuckDB (no se cambia una vez
    # usado): lo pone `main`, y DuckDB lo vacía al acabar cada consulta.
    c.execute("set memory_limit = '%dMB'" % limite_mb)
    t0 = time.time()
    fin = threading.Event()
    tl = {"pico_rss": 0, "pico_derrame": 0, "muestras": [], "crecio": None}
    otra = c.cursor()

    def mirar():
        cg0 = cgroup_mb()
        while not fin.wait(0.2):
            r, d = rss_mb(), du(DERRAME)
            tl["pico_rss"] = max(tl["pico_rss"], r)
            tl["pico_derrame"] = max(tl["pico_derrame"], d)
            s = round(time.time() - t0, 1)
            if not tl["muestras"] or s - tl["muestras"][-1][0] >= 5:
                tl["muestras"].append((s, r, d))
            cg = cgroup_mb()
            if crecer and tl["crecio"] is None and cg and cg0 and cg > cg0:
                nuevo = int(cg * 0.6)
                try:
                    otra.execute("set memory_limit = '%dMB'" % nuevo)
                    ok = otra.execute("select current_setting('memory_limit')").fetchone()[0]
                except Exception as e:  # noqa: BLE001
                    ok = "error: %s" % e
                tl["crecio"] = {"s": s, "cgroup_mb": cg, "memory_limit": ok, "rss_mb": r, "derrame_mb": d}
                linea(evento="crecio", pasada=nombre, **tl["crecio"])

    h = threading.Thread(target=mirar, daemon=True)
    h.start()
    if crecer:
        linea(evento="pide-crecer", pasada=nombre)
    salida = os.path.join(T, "salida.parquet")
    try:
        c.execute("copy (%s) to '%s' (format parquet)" % (Q, salida))
        estado, error = "ok", None
    except Exception as e:  # noqa: BLE001
        estado, error = "error", str(e)[:300]
    fin.set()
    h.join(1)
    linea(evento="pasada", pasada=nombre, estado=estado, error=error, s=round(time.time() - t0, 1),
          memory_limit_mb=limite_mb, cgroup_mb=cgroup_mb(), pico_rss_mb=tl["pico_rss"],
          pico_derrame_mb=tl["pico_derrame"], crecio=tl["crecio"], muestras=tl["muestras"])
    try:
        os.remove(salida)
    except OSError:
        pass


def main():
    linea(evento="inicio", duckdb=duckdb.__version__, cgroup_mb=cgroup_mb())
    c = duckdb.connect(os.path.join(T, "r02.duckdb"))
    c.execute("set threads to %s" % os.environ.get("HILOS", "2"))
    c.execute("set max_temp_directory_size = '300GB'")
    c.execute("set preserve_insertion_order = false")
    c.execute("set memory_limit = '%dMB'" % (cgroup_mb() // 2))
    c.execute("set temp_directory = '%s'" % DERRAME)
    t0 = time.time()
    c.execute(f"""create or replace table g as
        select (hash(range) % {FILAS // 2})::bigint k, random() v, md5(range::varchar)[:8] s
        from range({FILAS})""")
    linea(evento="sembrado", s=round(time.time() - t0, 1), filas=FILAS)
    base = cgroup_mb()
    pasada(c, "A_fija_baja", base // 2)
    pasada(c, "B_crece", base // 2, crecer=True)
    pasada(c, "C_fija_alta", int(cgroup_mb() * 0.6))
    antes = cgroup_mb()
    linea(evento="pide-imposible")
    for _ in range(60):
        time.sleep(1)
        if cgroup_mb() != antes:
            break
    linea(evento="imposible", cgroup_antes_mb=antes, cgroup_despues_mb=cgroup_mb())
    linea(evento="fin")


if __name__ == "__main__":
    main()
