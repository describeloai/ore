"""ORE 0072 F2·3·2·2 · el banco de `ore-piramide`: un Job de victor (la imagen del puesto, la cuenta `driver`,
la cola del sistema) con tippecanoe de conda-forge y el binario de la rama copiado (`kubectl cp`), que construye
las pirámides de `trip_3m_puntos` y `trip_3m_lineas` (por loadTable, credencial prestada) y las sube a
`gs://<lago de victor>/piramides/banco/` con la identidad del pod, como lo hará el Job del sistema (paso 4).
Compara con la prueba de concepto (pc-*.log) y deja el registro en piramide/banco-piramide-<fecha>.log.

  python pruebas-de-fuego/geo/f2/piramide/banco_piramide.py [BANCO_FUENTES=trip_3m_puntos,trip_3m_lineas]
  python …/banco_piramide.py BANCO_FUENTES=trip_3m_puntos=gs://…/metadata/00000-….metadata.json   # sin catálogo

Al acabar, borrar `piramides/banco/` del lago (lo dice el registro).
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

BINARIO = os.path.join(os.path.dirname(S.rstrip(os.sep)), "motor", "ore-piramide")
LAGO = "project-8853a180-450d-47be-b83-t-victor-copia"
env = dict(a.split("=", 1) for a in sys.argv[1:] if "=" in a)
FUENTES = env.get("BANCO_FUENTES", "trip_3m_puntos,trip_3m_lineas")

JOB = r'''
import json, os, subprocess, sys, tarfile, io, time, urllib.parse, urllib.request
sys.path.insert(0, "/opt/ore")
os.environ.setdefault("ORE_SERVE", "http://ore-serve.%s.svc.cluster.local:8080" % os.environ.get("NS", "t-victor"))
import ore

def di(que, **kw):
    print("BANCO|" + que + "|" + json.dumps(kw, ensure_ascii=False), flush=True)

t0 = time.time()
url = "https://micro.mamba.pm/api/micromamba/linux-64/latest"
tarfile.open(fileobj=io.BytesIO(urllib.request.urlopen(url).read()), mode="r:bz2").extract("bin/micromamba", "/tmp")
subprocess.run(["/tmp/bin/micromamba", "create", "-y", "-q", "-p", "/tmp/tip", "-c", "conda-forge", "tippecanoe=2.79.0"],
               check=True, capture_output=True, env=dict(os.environ, MAMBA_ROOT_PREFIX="/tmp/mamba"))
di("tippecanoe", s=round(time.time() - t0, 1))
while not os.path.exists("/tmp/ore-piramide.listo"):
    time.sleep(2)

def testigo():
    d = os.environ["DIRECCION"].rstrip("/")
    cli = open("/puesto/agente-cliente").read().strip()
    sec = open("/puesto/agente-secreto").read().strip()
    datos = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": cli, "client_secret": sec}).encode()
    with urllib.request.urlopen(d + "/realms/%s/protocol/openid-connect/token" % os.environ.get("REALM", "rubix"), data=datos, timeout=20) as r:
        return {"authorization": "Bearer " + json.load(r)["access_token"]}

for f in "__FUENTES__".split(","):
    # `nombre` (por loadTable, credencial prestada) o `nombre=gs://…metadata.json` (sin catálogo: la
    # identidad del pod lee el lago, como el Job del sistema)
    n, _, ml = f.partition("=")
    cred = {}
    if not ml:
        ore.session._cabeceras = testigo()
        ore.session.id = ""
        c, r = ore.session.pedir("GET", "/v1/query/namespaces/default/tables/%s" % n,
                                 cabeceras={"x-iceberg-access-delegation": "vended-credentials"})
        if c != 200:
            di("falla", fuente=n, paso="loadTable", error=str(r)[:300])
            continue
        ml = r["metadata-location"]
        cred = {k: str(v) for k, v in r.get("config", {}).items() if k == "gcs.oauth2.token"}
    pet = {"fuente": {"metadata_location": ml, "geometria": "geom", "id": "id", "crs": "EPSG:4326",
                      "credencial": cred},
           "capa": n, "suelo": "14" if "lineas" in n else "13",
           "destino": {"ubicacion": "gs://__LAGO__/piramides/banco/%s.pmtiles" % n},
           "limites": {"segundos": "1800"}, "trabajo": "/tmp/piramide-" + n}
    t = time.time()
    p = subprocess.run(["/tmp/ore-piramide"], input=json.dumps(pet).encode(), capture_output=True,
                       env=dict(os.environ, ORE_TIPPECANOE="/tmp/tip/bin/tippecanoe"))
    for l in p.stdout.decode("utf-8", "replace").splitlines():
        print(l, flush=True)
    di("hecho", fuente=n, codigo=p.returncode, s=round(time.time() - t, 1), stderr=p.stderr.decode("utf-8", "replace")[-500:])
di("fin")
'''.replace("__FUENTES__", FUENTES).replace("__LAGO__", LAGO)

nombre = "banco-piramide-%d" % (int(time.time()) % 10**6)


def copiar():
    """El binario al pod en cuanto corra (desde su carpeta: `kubectl cp` lee `C:` como un pod)."""
    while True:
        time.sleep(5)
        _, pod = vara.sh(["kubectl", "-n", vara.NS, "get", "pods", "-l", "job-name=" + nombre, "-o",
                          "jsonpath={.items[0].metadata.name}{' '}{.items[0].status.phase}"])
        if pod.endswith("Running"):
            pod = pod.split()[0]
            c = subprocess.run(["kubectl", "-n", vara.NS, "cp", "-c", "medir", "ore-piramide", "%s:/tmp/.p" % pod],
                               cwd=os.path.dirname(BINARIO), capture_output=True, text=True)
            vara.sh(["kubectl", "-n", vara.NS, "exec", pod, "-c", "medir", "--", "sh", "-c",
                     "chmod 755 /tmp/.p && mv /tmp/.p /tmp/ore-piramide && touch /tmp/ore-piramide.listo"])
            print("## binario copiado " + (c.stderr[-200:] if c.returncode else ""), flush=True)
            return


threading.Thread(target=copiar, daemon=True).start()
fecha = time.strftime("%Y%m%d-%H%M")
log = vara.job(nombre, ["python3", "-u", "-c", JOB], imagen=vara.PUESTO, plazo=3600, memoria="3584Mi")
ruta = S + "banco-piramide-%s.log" % fecha
open(ruta, "w", encoding="utf-8", newline="\n").write(log)
for l in log.splitlines():
    if l.startswith(("BANCO|", "PIRAMIDE|", "##")) or "Traceback" in l:
        print(l[:700])
print("registro:", ruta)
print("al acabar: gcloud storage rm -r gs://%s/piramides/banco/" % LAGO)
