"""Query Q1 · La ventana que llenó el nodo, otra vez, con el tope de disco de la sesión.

Corre con la forma de la sesión nueva (`/trabajo` con `sizeLimit: 4Gi`, `ORE_DISCO_MB=4096`) y el
`ore/__init__.py` de la rama montado sobre el de la imagen. La misma ventana sobre 600 M filas que el
2026-10-23 derramó 10 GiB y puso el nodo del sistema en `DiskPressure`. Lo esperado: un error que lo
dice al llegar a ~2 GiB de derrame, el pod vivo y el nodo sin presión. Vigila el derrame cada segundo.
"""
import json
import os
import threading
import time
import sys
import urllib.request

sys.path.insert(0, "/opt/ore")  # lo que hace el agente
import ore  # noqa: E402

T0 = time.time()
TRIP = "https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo/spatialbench/sf100/trip.parquet"


def di(**k):
    k["t"] = round(time.time() - T0, 1)
    print(json.dumps(k, ensure_ascii=False), flush=True)


def ocupado_mb(d="/trabajo"):
    t = 0
    for r, _, fs in os.walk(d):
        for f in fs:
            try:
                t += os.path.getsize(os.path.join(r, f))
            except OSError:
                pass
    return t // 2**20


pico = [0]
parar = threading.Event()


def vigia():
    while not parar.is_set():
        pico[0] = max(pico[0], ocupado_mb())
        time.sleep(1)


req = urllib.request.Request("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token",
                             headers={"Metadata-Flavor": "Google"})
tok = json.load(urllib.request.urlopen(req, timeout=10))["access_token"]
con = ore._duckdb()
con.execute("load httpfs")
con.execute("create secret banco (type http, bearer_token '%s')" % tok)
di(evento="motor", memoria=con.execute("select current_setting('memory_limit')").fetchone()[0],
   derrame_max=con.execute("select current_setting('max_temp_directory_size')").fetchone()[0],
   derrame_en=con.execute("select current_setting('temp_directory')").fetchone()[0])
threading.Thread(target=vigia, daemon=True).start()
t = time.time()
try:
    r = ore.sql("select count(*) n from (select t_fare - lag(t_fare) over (partition by t_custkey order by t_pickuptime) d "
                "from read_parquet('%s')) where d > 0" % TRIP)
    di(consulta="ventana_600M", s=round(time.time() - t, 1), resultado=str(r.iloc[0, 0]))
except Exception as e:  # noqa: BLE001
    di(consulta="ventana_600M", s=round(time.time() - t, 1), error=str(e)[:400])
parar.set()
time.sleep(1.5)
di(evento="fin", pico_trabajo_mb=pico[0], ahora_trabajo_mb=ocupado_mb(), vivo=True)
