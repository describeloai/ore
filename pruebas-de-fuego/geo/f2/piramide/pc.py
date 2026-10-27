"""ORE 0072 F2·3·2 · lanza la prueba de concepto de la pirámide (`pc_job.py`) en un Job de victor.

  python pruebas-de-fuego/geo/f2/piramide/pc.py                      # el previo: 100 000 filas por fuente
  python pruebas-de-fuego/geo/f2/piramide/pc.py PC_FILAS=0           # PC1: todas
  python pruebas-de-fuego/geo/f2/piramide/pc.py PC_FUENTES=edificios PC_OPCIONES='{...}'

Deja el registro en pruebas-de-fuego/geo/f2/piramide/pc-<fecha>.log e imprime las líneas `PC|`.
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
codigo = "".join("import os; os.environ[%r] = %r\n" % kv for kv in env.items())
codigo += open(S + "pc_job.py", encoding="utf-8").read()
fecha = time.strftime("%Y%m%d-%H%M")
nombre = "pc-piramide-%d" % (int(time.time()) % 10**6)
BAJADOS = os.path.join(S, "pmtiles")
BUCKET = "gs://project-8853a180-450d-47be-b83-banco-geo/piramide/"
FILAS = env.get("PC_FILAS", "100000")


def entregar():
    """Cada `PC|listo` del pod: copia su `.pmtiles` aquí, lo sube al bucket y lo suelta; hasta `PC|fin`."""
    ns, hechos = vara.NS, set()
    while True:
        time.sleep(10)
        _, pod = vara.sh(["kubectl", "-n", ns, "get", "pods", "-l", "job-name=" + nombre, "-o",
                          "jsonpath={.items[0].metadata.name}"])
        if not pod:
            continue
        _, log = vara.sh(["kubectl", "-n", ns, "logs", pod, "-c", "medir"])
        for l in log.splitlines():
            if not l.startswith("PC|listo|"):
                continue
            f = json.loads(l.split("|", 2)[2])["fichero"]
            if f in hechos:
                continue
            hechos.add(f)
            os.makedirs(BAJADOS, exist_ok=True)
            base = os.path.basename(f)
            t0 = time.time()
            c = subprocess.run(["kubectl", "-n", ns, "cp", "-c", "medir", "%s:%s" % (pod, f), base],
                               cwd=BAJADOS, capture_output=True, text=True)
            n = base[: -len(".pmtiles")]
            destino = BUCKET + "%s-%s.pmtiles" % (n, env.get("PC_FILAS_" + n, FILAS))
            c2 = subprocess.run(["gcloud", "storage", "cp", base, destino], cwd=BAJADOS, capture_output=True,
                                text=True, shell=True)
            print("PC|subido|%s · %.0f s · %s%s" % (destino, time.time() - t0, c.stderr[-200:],
                                                     c2.returncode and c2.stderr[-200:] or ""), flush=True)
            vara.sh(["kubectl", "-n", ns, "exec", pod, "-c", "medir", "--", "touch", f + ".liberar"])
        if "PC|fin|" in log or "Terminated" in log:
            return


threading.Thread(target=entregar, daemon=True).start()
memoria = env.pop("PC_MEMORIA", "3584Mi")
log = vara.job(nombre, ["python3", "-u", "-c", codigo],
               imagen=vara.PUESTO, plazo=int(env.get("PC_PLAZO", "3600")), memoria=memoria)
ruta = S + "pc-%s.log" % fecha
open(ruta, "w", encoding="utf-8", newline="\n").write(log)
for l in log.splitlines():
    if l.startswith("PC|piramide|"):
        _, _, d = l.split("|", 2)
        d = json.loads(d)
        print("PC|piramide|%s · %s s · %s MB pico · %s MB · %s" % (d["fuente"], d["s"], d["rss_mb"], d["mb_pmtiles"], d["orden"]))
        for z in d["zooms"]:
            print("    z%-2d %6d teselas · media %7.1f KB · p95 %7.1f KB · max %7.1f KB"
                  % (z["z"], z["teselas"], z["kb_media"], z["kb_p95"], z["kb_max"]))
    elif l.startswith("PC|") or l.startswith("##") or "Traceback" in l:
        print(l[:600])
print("registro:", ruta)
