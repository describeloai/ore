"""ORE 0072 F2·3 · La latencia de GCS desde un puesto de victor: lo que cuesta un GET por rango (en serie, con la
conexión viva, como el `ureq::Agent` del motor) y una oleada en paralelo, sobre un fichero de `trip_3m_puntos`
con la credencial que presta el catálogo. Fija el presupuesto de F2·3: cuántas idas a GCS caben en 300 ms.

  python pruebas-de-fuego/geo/f2/sonda_gcs.py
"""
import importlib.util
import os

S = os.path.dirname(os.path.abspath(__file__)) + os.sep
G = os.path.dirname(S.rstrip(os.sep)) + os.sep
spec = importlib.util.spec_from_file_location("vara", G + "vara.py")
vara = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vara)

SONDA = r'''
import http.client, json, random, statistics, threading, time, urllib.parse
ml, cfg = del_catalogo("query", "default", "trip_3m_puntos")
tok = next(v for k, v in cfg.items() if "token" in k and "expires" not in k)
bucket, pre = ml[5:].split("/", 1)
pre = pre.rsplit("/metadata/", 1)[0] + "/data/"
H = {"authorization": "Bearer " + tok}

def con():
    return http.client.HTTPSConnection("storage.googleapis.com", timeout=30)

# la credencial prestada no lista (acotada a leer): el fichero, de fuera
nombre, largo = pre + "7c7aa780-0576-45c9-a7f4-645a781c9e06.parquet", 141223430
ruta = "/storage/v1/b/%s/o/%s?alt=media" % (bucket, urllib.parse.quote(nombre, safe=""))
di("fichero", nombre=nombre.rsplit("/", 1)[1], mb=round(largo / 2**20, 1))

def get(c, a, n):
    t0 = time.time()
    c.request("GET", ruta, headers=dict(H, range="bytes=%d-%d" % (a, a + n - 1)))
    r = c.getresponse(); b = r.read()
    assert r.status == 206 and len(b) == n, (r.status, len(b))
    return (time.time() - t0) * 1000

def resumen(ms):
    ms = sorted(ms)
    return {"n": len(ms), "p50": round(statistics.median(ms)), "p95": round(ms[int(len(ms) * .95) - 1]),
            "max": round(ms[-1])}

t0 = time.time(); c = con(); c.connect(); di("conexion_nueva_ms", ms=round((time.time() - t0) * 1000))
for kb in (8, 64, 256, 1024, 4096):
    n = kb << 10
    ms = [get(c, random.randrange(0, largo - n), n) for _ in range(30)]
    di("serie", kb=kb, **resumen(ms))
for hilos in (4, 16, 32):
    n, res = 256 << 10, []
    def uno():
        cc = con()
        for _ in range(4):
            res.append(get(cc, random.randrange(0, largo - n), n))
    t0 = time.time()
    hs = [threading.Thread(target=uno) for _ in range(hilos)]
    [h.start() for h in hs]; [h.join() for h in hs]
    di("paralelo", hilos=hilos, kb=256, oleada_ms=round((time.time() - t0) * 1000 / 4), **resumen(res))
'''

fuente = open(S + "banco_victor.py", encoding="utf-8").read().rsplit("\nmain()", 1)[0]
log = vara.job("f2-sonda-%d" % (__import__("time").time() % 10**6), ["python3", "-u", "-c", fuente + SONDA],
               imagen=vara.PUESTO, plazo=900, memoria="1Gi")
print("\n".join(l for l in log.splitlines() if l.startswith(("BANCO", "##")) or "Error" in l or "Traceback" in l))
