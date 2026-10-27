"""Query Q0 · la escalera: las mismas consultas, a 6 M, 60 M y 600 M filas, en un motor DuckDB
configurado como el puesto (memoria, hilos, derrame, extensiones sin red), leyendo el Parquet del
banco por HTTPS con la identidad del pod. Una línea JSON por consulta y corrida.

  ESCALON=sesion|grande  MEMORIA_MB=…  [HILOS=…]  [LIMITE=300]  [ESCALAS=6,60,600]  python3 consultas.py

Además de medir el tiempo, calcula lo que un enrutador sabría ANTES de correr: los bytes que la
consulta va a leer, sacados de los metadatos del Parquet (columnas que proyecta × grupos de filas que
el filtro deja). Es la cifra con la que se decidiría el motor; aquí se cruza con el tiempo real.
"""
import json
import os
import shutil
import sys
import threading
import time
import urllib.request

import duckdb

B = "https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo/"
TRIP = B + "spatialbench/sf100/trip.parquet"
CUSTOMER = B + "spatialbench/sf100/customer.parquet"
LUGARES = B + "vectorial/overture/2026-09-23.1/lugares_es.parquet"
EDIFICIOS = B + "g22/a_tal_cual.parquet"
ESCALON = os.environ.get("ESCALON", "sesion")
LIMITE = int(os.environ.get("LIMITE", "300"))
ESCALAS = [int(x) for x in os.environ.get("ESCALAS", "6,60,600").split(",")]
DERRAME = os.environ.get("DERRAME", "/tmp/derrame")

# (nombre, columnas que lee de trip, sql con {T} = la vista de trip a la escala)
CONSULTAS = [
    ("contar", [], "select count(*) from {T}"),
    ("punto", ["*"], "select * exclude (t_pickuploc, t_dropoffloc) from {T} where t_tripkey = 4242"),
    ("agregar", ["t_custkey", "t_fare", "t_distance"],
     "select t_custkey % 100 g, count(*) n, sum(t_fare) f, avg(t_distance) d from {T} group by 1 order by 1"),
    ("top_n", ["t_tripkey", "t_custkey", "t_totalamount", "t_pickuptime"],
     "select t_tripkey, t_custkey, t_totalamount, t_pickuptime from {T} order by t_totalamount desc limit 100"),
    ("muchos_grupos", ["t_custkey", "t_fare"],
     "select t_custkey, sum(t_fare) f from {T} group by 1 order by f desc limit 10"),
    ("distintos", ["t_driverkey", "t_vehiclekey"],
     "select count(*) from (select distinct t_driverkey, t_vehiclekey from {T})"),
    ("junta", ["t_custkey", "t_fare"],
     "select c_nation, count(*) n, sum(t_fare) f from {T} t join customer c on c.c_custkey = t.t_custkey group by 1 order by 1"),
    ("ventana", ["t_custkey", "t_pickuptime", "t_fare"],
     "select count(*) from (select t_custkey, t_fare - lag(t_fare) over (partition by t_custkey order by t_pickuptime) d from {T}) where d > 0"),
    ("geo_caja", ["t_pickuploc"],
     "select count(*) from {T} where ST_Intersects(ST_GeomFromWKB(t_pickuploc), ST_MakeEnvelope(-74.05, 40.6, -73.9, 40.8))"),
]


def token():
    if os.environ.get("TOKEN"):  # fuera del clúster (la prueba del guion en local)
        return os.environ["TOKEN"]
    req = urllib.request.Request("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token",
                                 headers={"Metadata-Flavor": "Google"})
    return json.load(urllib.request.urlopen(req, timeout=10))["access_token"]


def motor():
    """Como `ore._duckdb()` del puesto: tope de memoria del pod, derrame a disco, sin red para extensiones."""
    c = duckdb.connect()
    if os.environ.get("HILOS"):
        c.execute("set threads to %d" % int(os.environ["HILOS"]))
    c.execute("set memory_limit='%dMB'" % int(os.environ["MEMORIA_MB"]))
    os.makedirs(DERRAME, exist_ok=True)
    c.execute("set temp_directory='%s'" % DERRAME)
    c.execute("set autoinstall_known_extensions = false")
    c.execute("set TimeZone = 'UTC'")
    if os.path.isdir("/opt/ore/duckdb"):
        c.execute("set extension_directory = '/opt/ore/duckdb'")
    for e in ("httpfs", "spatial"):
        c.execute("load %s" % e)
    c.execute("set geometry_always_xy = true")
    c.execute("create secret banco (type http, bearer_token '%s')" % token())
    c.execute("create view customer as select * from read_parquet('%s')" % CUSTOMER)
    return c


def grupos(c):
    """Los grupos de filas de trip con su rango de t_tripkey y lo que pesa cada columna."""
    filas = c.execute("""select row_group_id, path_in_schema, total_compressed_size,
                                stats_min_value, stats_max_value
                         from parquet_metadata('%s')""" % TRIP).fetchall()
    g = {}
    for rg, col, peso, mn, mx in filas:
        d = g.setdefault(rg, {"cols": {}})
        d["cols"][col] = peso
        if col == "t_tripkey":
            d["min"], d["max"] = int(mn), int(mx)
    return g


def bytes_estimados(g, k, cols):
    """Lo que el enrutador sabría antes: grupos con min(t_tripkey) <= k, columnas que lee."""
    vivos = [d for d in g.values() if d["min"] <= k]
    if cols == ["*"]:
        vivos = [d for d in vivos if d["min"] <= 4242 <= d["max"]]
        return sum(sum(d["cols"].values()) for d in vivos)
    return sum(sum(d["cols"].get(x, 0) for x in cols) for d in vivos)


def pico_mb():
    for f in ("/sys/fs/cgroup/memory.peak", "/sys/fs/cgroup/memory/memory.max_usage_in_bytes"):
        try:
            return int(open(f).read()) // 2**20
        except OSError:
            pass
    return None


def derramado_mb():
    t = 0
    for r, _, fs in os.walk(DERRAME):
        for f in fs:
            try:
                t += os.path.getsize(os.path.join(r, f))
            except OSError:
                pass
    return t // 2**20


def correr(c, sql):
    corte = threading.Timer(LIMITE, c.interrupt)
    corte.start()
    t0 = time.time()
    try:
        r = c.execute(sql).fetchall()
        return time.time() - t0, len(r), None
    except Exception as e:  # noqa: BLE001
        return time.time() - t0, 0, ("limite" if time.time() - t0 >= LIMITE - 1 else str(e).splitlines()[0][:200])
    finally:
        corte.cancel()


def main():
    t0 = time.time()
    c = motor()
    print(json.dumps({"escalon": ESCALON, "evento": "motor", "s": round(time.time() - t0, 2),
                      "hilos": c.execute("select current_setting('threads')").fetchone()[0],
                      "memoria": c.execute("select current_setting('memory_limit')").fetchone()[0],
                      "cpus": os.cpu_count(), "duckdb": duckdb.__version__}), flush=True)
    # El suelo: lo que cuesta una consulta trivial y una lectura pequeña del bucket.
    for nombre, sql in (("uno", "select 1"), ("lugares_contar", "select count(*) from read_parquet('%s')" % LUGARES),
                        ("lugares_agregar", "select confidence > 0.8, count(*) from read_parquet('%s') group by 1" % LUGARES),
                        ("edificios_caja", "select count(*) from read_parquet('%s') where bbox.xmin > -3.8 and bbox.xmax < -3.6 and bbox.ymin > 40.3 and bbox.ymax < 40.5" % EDIFICIOS)):
        for vez in (1, 2):
            s, n, e = correr(c, sql)
            print(json.dumps({"escalon": ESCALON, "consulta": nombre, "vez": vez, "s": round(s, 3), "filas": n, "error": e}), flush=True)
    t0 = time.time()
    g = grupos(c)
    print(json.dumps({"escalon": ESCALON, "evento": "metadatos", "s": round(time.time() - t0, 2), "grupos": len(g),
                      "ordenado": all(g[i]["max"] < g[i + 1]["min"] for i in range(len(g) - 1))}), flush=True)
    solo = os.environ.get("CONSULTAS")
    for k_m in ESCALAS:
        k = k_m * 1_000_000
        c.execute("create or replace view trip as select * from read_parquet('%s') where t_tripkey <= %d" % (TRIP, k))
        fallo = set()
        for nombre, cols, sql in CONSULTAS:
            if solo and nombre not in solo.split(","):
                continue
            # Lo que no acabó a una escala no se lanza a la siguiente (como C0).
            if nombre in fallidas.get(ESCALON, set()):
                print(json.dumps({"escalon": ESCALON, "consulta": nombre, "millones": k_m, "saltada": True}), flush=True)
                continue
            shutil.rmtree(DERRAME, ignore_errors=True)
            os.makedirs(DERRAME, exist_ok=True)
            veces = 2 if k_m <= 60 else 1
            for vez in range(1, veces + 1):
                s, n, e = correr(c, sql.format(T="trip"))
                print(json.dumps({"escalon": ESCALON, "consulta": nombre, "millones": k_m, "vez": vez, "s": round(s, 2),
                                  "filas": n, "error": e, "gb_estimados": round(bytes_estimados(g, k, cols) / 1e9, 3),
                                  "pico_mb": pico_mb(), "derrame_mb": derramado_mb()}), flush=True)
                if e:
                    fallo.add(nombre)
                    break
        fallidas.setdefault(ESCALON, set()).update(fallo)


fallidas = {}

if __name__ == "__main__":
    sys.exit(main())
