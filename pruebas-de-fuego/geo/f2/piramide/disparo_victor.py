"""ORE 0072 F2·3·2·8 · el disparo en victor, de punta a punta y sin tocar nada a mano: un Job de medida (la
cuenta del puesto, el agente de la celda) pide el descriptor de cada capa geo a `ore-serve`, como la consola,
hasta que su pirámide está `lista` (o `fallida`); por el camino, los Jobs `piramide-*` que encoló el disparo
(Kueue, el nodo, desalojos) y, con ella lista, unas teselas por debajo del suelo (de la pirámide) y por encima
(del crudo).

  python pruebas-de-fuego/geo/f2/piramide/disparo_victor.py [CAPAS=query.default.trip_3m_puntos,…] [MINUTOS=40]

Deja el registro en piramide/disparo-victor-<fecha>.log.
"""
import importlib.util
import json
import os
import subprocess
import sys
import threading
import time

S = os.path.dirname(os.path.abspath(__file__)) + os.sep
G = os.path.dirname(os.path.dirname(S.rstrip(os.sep))) + os.sep
spec = importlib.util.spec_from_file_location("vara", G + "vara.py")
vara = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vara)

env = dict(a.split("=", 1) for a in sys.argv[1:] if "=" in a)
CAPAS = env.get("CAPAS", "query.default.trip_3m_puntos,query.default.trip_3m_lineas,query.default.trip_3m,query.default.trip_30m")
MINUTOS = int(env.get("MINUTOS", "40"))
LOG = open(S + time.strftime("disparo-victor-%Y%m%d-%H%M.log"), "w", encoding="utf-8")


def di(*a):
    t = " ".join(str(x) for x in a)
    print(t, flush=True)
    LOG.write(t + "\n")
    LOG.flush()


JOB = r'''
import json, os, time, urllib.parse, urllib.request
SERVE = "http://ore-serve.%s.svc.cluster.local:8080" % os.environ.get("NS", "t-victor")

def testigo():
    d = os.environ["DIRECCION"].rstrip("/")
    cli = open("/puesto/agente-cliente").read().strip()
    sec = open("/puesto/agente-secreto").read().strip()
    datos = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": cli, "client_secret": sec}).encode()
    with urllib.request.urlopen(d + "/realms/%s/protocol/openid-connect/token" % os.environ.get("REALM", "rubix"), data=datos, timeout=20) as r:
        return json.load(r)["access_token"]

def pedir(url, cab):
    t = time.time()
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers=cab), timeout=60) as r:
            return r.status, r.read(), round((time.time() - t) * 1000)
    except urllib.error.HTTPError as e:
        return e.code, e.read(), round((time.time() - t) * 1000)

def di(que, **kw):
    print("DISPARO|" + que + "|" + json.dumps(kw, ensure_ascii=False), flush=True)

capas = "__CAPAS__".split(",")
hechas = {}
fin = time.time() + __MINUTOS__ * 60
tok, tok_t = None, 0
while time.time() < fin and len(hechas) < len(capas):
    if time.time() - tok_t > 240:
        tok, tok_t = testigo(), time.time()
    for c in capas:
        if c in hechas:
            continue
        cod, b, ms = pedir(SERVE + "/v1/layers/%s/descriptor" % c,
                           {"authorization": "Bearer " + tok, "host": "victor.ore.paladio.io", "x-forwarded-proto": "https"})
        if cod != 200:
            di("descriptor", capa=c, codigo=cod, ms=ms, cuerpo=b.decode("utf-8", "replace")[:300])
            hechas[c] = {"codigo": cod}
            continue
        d = json.loads(b)
        p = d.get("ore", {}).get("piramide", {})
        di("descriptor", capa=c, ms=ms, minzoom=d.get("minzoom"), suelo=d.get("ore", {}).get("suelo"), piramide=p)
        if p.get("estado") in ("lista", "fallida", "no", "ausente"):
            hechas[c] = d
    if len(hechas) < len(capas):
        time.sleep(30)

# con la pirámide lista: una tesela por debajo del suelo y otra por encima, sin sujeto (la firma)
import math
def xy(lon, lat, z):
    n = 2 ** z
    return int((lon + 180) / 360 * n), int((1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n)
for c, d in hechas.items():
    if not isinstance(d, dict) or "tiles" not in d:
        continue
    t = d["tiles"][0].replace("https://victor.ore.paladio.io", SERVE)
    s = d["ore"].get("suelo", 13)
    for z in sorted({max(0, s - 5), s - 1, s, s + 1}):
        x, y = xy(-73.98, 40.75, z)
        cod, b, ms = pedir(t.replace("{z}", str(z)).replace("{x}", str(x)).replace("{y}", str(y)), {})
        cod2, b2, ms2 = pedir(t.replace("{z}", str(z)).replace("{x}", str(x)).replace("{y}", str(y)), {})
        di("tesela", capa=c, z=z, codigo=cod, bytes=len(b), ms=ms, ms_otra_vez=ms2,
           de="pirámide" if z < s and d["ore"]["piramide"].get("estado") == "lista" else "crudo")
'''


def vigilar(parar):
    """Los Jobs de la pirámide, Kueue, el nodo y los desalojos, mientras el Job de medida espera."""
    visto = {}
    desde = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    while not parar.is_set():
        _, o = vara.sh(["kubectl", "-n", vara.NS, "get", "jobs", "-o", "json"])
        try:
            js = [j for j in json.loads(o)["items"] if j["metadata"]["name"].startswith("piramide-")]
        except Exception:
            js = []
        for j in js:
            n, s = j["metadata"]["name"], j.get("status", {})
            e = "hecho" if s.get("succeeded") else "fallido" if any(
                c["type"] == "Failed" and c["status"] == "True" for c in s.get("conditions", [])) else \
                "corriendo" if s.get("active") else "esperando"
            if visto.get(n) != e:
                di("  %s · job %s · %s" % (time.strftime("%H:%M:%S"), n, e))
                visto[n] = e
        _, o = vara.sh(["kubectl", "get", "nodes", "-o", "json"])
        try:
            for nd in json.loads(o)["items"]:
                for c in nd["status"]["conditions"]:
                    if c["type"] in ("DiskPressure", "MemoryPressure") and c["status"] == "True":
                        di("  ✗ %s · %s" % (nd["metadata"]["name"], c["type"]))
        except Exception:
            pass
        parar.wait(15)
    _, o = vara.sh(["kubectl", "get", "events", "-A", "--field-selector", "reason=Evicted", "-o", "json"])
    ev = [x for x in json.loads(o or '{"items":[]}')["items"]
          if (x.get("lastTimestamp") or x.get("eventTime") or "") >= desde]
    di("desalojos en el clúster desde %s: %d" % (desde, len(ev)))
    for x in ev:
        di("  ✗ %s/%s · %s" % (x["involvedObject"].get("namespace"), x["involvedObject"]["name"], x["message"][:160]))
    return visto


def main():
    di("# el disparo en victor (0072 F2·3·2·8) · %s · %s" % (time.strftime("%Y-%m-%d %H:%M"), CAPAS))
    parar = threading.Event()
    hilo = threading.Thread(target=vigilar, args=(parar,), daemon=True)
    hilo.start()
    orden = ["python3", "-c", JOB.replace("__CAPAS__", CAPAS).replace("__MINUTOS__", str(MINUTOS))]
    log = vara.job("disparo-%s" % time.strftime("%H%M%S"), orden, plazo=MINUTOS * 60 + 300, memoria="512Mi")
    parar.set()
    hilo.join(timeout=120)
    for l in log.splitlines():
        if l.startswith("DISPARO|") or l.startswith("## falla") or "Error" in l:
            di(l[:600])


main()
