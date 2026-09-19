#!/usr/bin/env python3
"""
MEDIDA · W3.2 · la capa (19 de septiembre)

0031 §3 dice que las dependencias se DECLARAN en el árbol y las RESUELVE la
plataforma en una capa sobre la imagen base, y que el puesto no tiene `pip`
como verdad. Antes de decidir CÓMO es esa capa (¿una imagen en el registro, o
una caja de ruedas en el bucket que el puesto instala al arrancar?), cuánto
cuesta cada paso, sobre el clúster real y con `polars` de ejemplo:

  §1  RESOLVER      un Job con la red del DRIVER (alcanza PyPI): `pip download
                    --only-binary` de `polars` → cuántas ruedas, cuántos MB,
                    cuántos segundos; y subirlas al bucket del inquilino
  §2  INSTALAR      un Job con la red del PUESTO (sin internet): bajar la caja
                    del bucket, `pip install --no-index --find-links --target`,
                    `import polars`, y leer la copia más grande con polars
  §3  EL TAMAÑO     lo que ocupa la capa instalada frente a las ruedas, y qué
                    pesaría como imagen (para decidir registro vs bucket)

Uso:
  python pruebas-de-fuego/medida-w3-la-capa.py --inquilino victor [--paquete polars]

Crea dos Jobs `capa-medida-*` en el namespace del inquilino y los borra al
acabar; deja y luego retira `ore/puesto/medida/` en el bucket. No lanza nada
de pago fuera del clúster.
"""
import datetime as dt
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
os.environ["MSYS_NO_PATHCONV"] = "1"
os.environ["MSYS2_ARG_CONV_EXCL"] = "*"

REGISTRO = "europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore"
IMAGEN = f"{REGISTRO}/puesto-python:1"
PREFIJO = "ore/puesto/medida"


def fila(k, v, nota=""):
    print("  %-46s %-24s %s" % (k, v, nota))


def sh(*args, entrada=None):
    exe = shutil.which(args[0]) or args[0]
    r = subprocess.run((exe,) + tuple(args[1:]), input=entrada.encode("utf-8") if entrada else None, capture_output=True)
    return r.stdout.decode("utf-8", "replace") + r.stderr.decode("utf-8", "replace")


def k(*args, ns=None, entrada=None):
    return sh(*(["kubectl"] + (["-n", ns] if ns else []) + list(args)), entrada=entrada)


RESOLVER = r'''
import json, os, subprocess, sys, time
T0 = time.time()
def dice(**kw):
    kw["t"] = round(time.time() - T0, 2); print("MEDIDA " + json.dumps(kw), flush=True)
paq = os.environ["PAQUETE"]; d = "/trabajo/ruedas"; os.makedirs(d, exist_ok=True)
t = time.time()
r = subprocess.run([sys.executable, "-m", "pip", "download", "--only-binary=:all:", "--dest", d, "--disable-pip-version-check", "-q", paq], capture_output=True, text=True)
ruedas = sorted(os.listdir(d)) if r.returncode == 0 else []
dice(paso="resuelto", ok=r.returncode == 0, ms=round((time.time() - t) * 1000), ruedas=ruedas, mb=round(sum(os.path.getsize(os.path.join(d, f)) for f in ruedas) / 1e6, 1), error=r.stderr[-400:] if r.returncode else "")
if r.returncode:
    sys.exit(1)
from google.cloud import storage
cli = storage.Client(); b = cli.bucket(os.environ["BUCKET"])
t = time.time()
for f in ruedas:
    b.blob(os.environ["PREFIJO"] + "/" + f).upload_from_filename(os.path.join(d, f))
b.blob(os.environ["PREFIJO"] + "/requirements.lock").upload_from_string("\n".join(ruedas) + "\n")
dice(paso="subido", ms=round((time.time() - t) * 1000), objetos=len(ruedas) + 1)
'''

INSTALAR = r'''
import io, json, os, subprocess, sys, time
T0 = time.time()
def dice(**kw):
    kw["t"] = round(time.time() - T0, 2); print("MEDIDA " + json.dumps(kw), flush=True)
from google.cloud import storage
cli = storage.Client(); b = cli.bucket(os.environ["BUCKET"]); pre = os.environ["PREFIJO"] + "/"
d = "/trabajo/ruedas"; os.makedirs(d, exist_ok=True)
t = time.time(); n = 0; mb = 0
for o in cli.list_blobs(b, prefix=pre):
    o.download_to_filename(os.path.join(d, o.name[len(pre):])); n += 1; mb += (o.size or 0) / 1e6
dice(paso="bajado", objetos=n, mb=round(mb, 1), ms=round((time.time() - t) * 1000))
capa = "/trabajo/capa"
t = time.time()
r = subprocess.run([sys.executable, "-m", "pip", "install", "--no-index", "--find-links", d, "--target", capa, "--disable-pip-version-check", "-q", os.environ["PAQUETE"]], capture_output=True, text=True)
tam = sum(os.path.getsize(os.path.join(dp, f)) for dp, _, fs in os.walk(capa) for f in fs) / 1e6 if r.returncode == 0 else 0
dice(paso="instalado", ok=r.returncode == 0, ms=round((time.time() - t) * 1000), mb_capa=round(tam, 1), error=r.stderr[-400:] if r.returncode else "")
if r.returncode:
    sys.exit(1)
sys.path.insert(0, capa)
t = time.time(); import polars as pl; dice(paso="importado", version=pl.__version__, ms=round((time.time() - t) * 1000))
objs = [o for o in cli.list_blobs(b, prefix="ore/v1/") if not o.name.startswith("ore/v1/plan/")]
if objs:
    o = max(objs, key=lambda x: x.size or 0); crudo = o.download_as_bytes()
    n = int.from_bytes(crudo[8:12], "little"); carga = crudo[12 + n:]
    t = time.time(); df = pl.read_parquet(io.BytesIO(carga)); dice(paso="polars", filas=df.height, columnas=df.width, ms=round((time.time() - t) * 1000))
import socket
try:
    socket.create_connection(("pypi.org", 443), timeout=5).close(); fuera = "SI"
except Exception as e:
    fuera = "no (%s)" % type(e).__name__
dice(paso="internet", alcanza=fuera)
'''


def job_yaml(ns, inq, nombre, rol, bucket, paquete, guion_env):
    return f"""apiVersion: batch/v1
kind: Job
metadata:
  name: {nombre}
  namespace: {ns}
  labels:
    kueue.x-k8s.io/queue-name: cola
    ore.dev/tenant: {inq}
    ore.dev/rol: {rol}
    ore.dev/medida: w3-capa
spec:
  backoffLimit: 0
  ttlSecondsAfterFinished: 600
  activeDeadlineSeconds: 900
  template:
    metadata:
      labels:
        ore.dev/rol: {rol}
        ore.dev/tenant: {inq}
    spec:
      restartPolicy: Never
      serviceAccountName: driver
      volumes:
        - name: guion
          configMap: {{ name: {nombre} }}
        - name: trabajo
          emptyDir: {{}}
      containers:
        - name: medida
          image: {IMAGEN}
          imagePullPolicy: IfNotPresent
          command: ["python3", "/guion/main.py"]
          env:
            - {{ name: BUCKET, value: "{bucket}" }}
            - {{ name: PAQUETE, value: "{paquete}" }}
            - {{ name: PREFIJO, value: "{PREFIJO}" }}
            - {{ name: HOME, value: /tmp }}
            - {{ name: PIP_CACHE_DIR, value: /tmp/pip }}
          volumeMounts:
            - {{ name: guion, mountPath: /guion, readOnly: true }}
            - {{ name: trabajo, mountPath: /trabajo }}
          resources:
            requests: {{cpu: "1", memory: 2Gi}}
            limits:   {{cpu: "2", memory: 4Gi}}
          securityContext:
            allowPrivilegeEscalation: false
            runAsNonRoot: true
            runAsUser: 65532
            seccompProfile: {{ type: RuntimeDefault }}
            capabilities: {{ drop: [ALL] }}
"""


def correr(ns, inq, nombre, rol, bucket, paquete, guion):
    k("delete", "job", nombre, "--ignore-not-found", "--wait=false", ns=ns)
    k("delete", "configmap", nombre, "--ignore-not-found", ns=ns)
    time.sleep(2)
    with tempfile.TemporaryDirectory() as d:
        p = os.path.join(d, "main.py")
        with open(p, "w", encoding="utf-8", newline="\n") as f:
            f.write(guion)
        k("create", "configmap", nombre, f"--from-file=main.py={p}", ns=ns)
    k("apply", "-f", "-", ns=ns, entrada=job_yaml(ns, inq, nombre, rol, bucket, paquete, ""))
    t0 = time.time()
    pod = None
    while time.time() - t0 < 600:
        ps = json.loads(k("get", "pods", "-l", f"job-name={nombre}", "-o", "json", ns=ns) or "{}").get("items", [])
        if ps:
            pod = ps[0]["metadata"]["name"]
            if ps[0]["status"].get("phase") in ("Succeeded", "Failed"):
                break
        time.sleep(3)
    logs = k("logs", pod, ns=ns) if pod else ""
    out = {}
    for l in logs.splitlines():
        if l.startswith("MEDIDA "):
            try:
                d = json.loads(l[7:]); out[d.get("paso")] = d
            except ValueError:
                pass
    if not out:
        fila("  el Job no dijo nada", "", logs.strip()[-300:])
    return out, int(time.time() - t0)


def main():
    inq, paquete = "victor", "polars"
    if "--inquilino" in sys.argv:
        inq = sys.argv[sys.argv.index("--inquilino") + 1]
    if "--paquete" in sys.argv:
        paquete = sys.argv[sys.argv.index("--paquete") + 1]
    ns = f"t-{inq}"
    bucket = f"project-8853a180-450d-47be-b83-t-{inq}-copia"
    print("MEDIDA · W3.2 · la capa · %s · %s · %s" % (inq, paquete, dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%MZ")))
    creados = []
    try:
        print("\n§1 · RESOLVER — con la red del driver (PyPI), y subir la caja al bucket")
        m, total = correr(ns, inq, "capa-medida-resolver", "driver", bucket, paquete, RESOLVER); creados.append("capa-medida-resolver")
        if "resuelto" in m:
            r = m["resuelto"]
            fila("pip download --only-binary %s" % paquete, "%s · %d ms" % ("ok" if r["ok"] else "FALLO", r["ms"]), "%d ruedas · %s MB" % (len(r["ruedas"]), r["mb"]) if r["ok"] else r["error"][-200:])
            for w in r["ruedas"]:
                fila("  rueda", w[:60], "")
        if "subido" in m:
            fila("subir la caja al bucket", "%d objetos · %d ms" % (m["subido"]["objetos"], m["subido"]["ms"]), "ore/puesto/medida/")
        fila("el Job entero", "%d s" % total, "creado → terminado (con el frío del nodo si lo hubo)")
        print("\n§2 · INSTALAR — con la red del puesto (sin internet): la caja del bucket → la capa")
        m2, total2 = correr(ns, inq, "capa-medida-instalar", "puesto", bucket, paquete, INSTALAR); creados.append("capa-medida-instalar")
        if "bajado" in m2:
            fila("bajar la caja", "%d objetos · %s MB · %d ms" % (m2["bajado"]["objetos"], m2["bajado"]["mb"], m2["bajado"]["ms"]))
        if "instalado" in m2:
            i = m2["instalado"]
            fila("pip install --no-index --target", "%s · %d ms" % ("ok" if i["ok"] else "FALLO", i["ms"]), "%s MB instalados" % i["mb_capa"] if i["ok"] else i["error"][-200:])
        if "importado" in m2:
            fila("import polars", "%d ms" % m2["importado"]["ms"], "polars %s" % m2["importado"]["version"])
        if "polars" in m2:
            fila("polars lee la copia más grande", "%d × %d · %d ms" % (m2["polars"]["filas"], m2["polars"]["columnas"], m2["polars"]["ms"]))
        if "internet" in m2:
            fila("¿alcanza pypi.org?", m2["internet"]["alcanza"], "debe ser «no»")
        fila("el Job entero", "%d s" % total2, "")
        print("\n§3 · EL TAMAÑO — qué pesa la capa")
        if "resuelto" in m and "instalado" in m2:
            fila("ruedas en el bucket", "%s MB" % m["resuelto"]["mb"], "lo que viaja")
            fila("capa instalada", "%s MB" % m2["instalado"]["mb_capa"], "lo que ocupa en el emptyDir")
            fila("como imagen sería", "base (~200 MB) + %s MB" % m2["instalado"]["mb_capa"], "y un push/pull por versión, con permisos de registro por inquilino")
    finally:
        for n in creados:
            k("delete", "job", n, "--ignore-not-found", "--wait=false", ns=ns)
            k("delete", "configmap", n, "--ignore-not-found", ns=ns)
        borrado = sh("gcloud", "storage", "rm", "-r", f"gs://{bucket}/{PREFIJO}/**", "--quiet")
        fila("limpieza", "Jobs, ConfigMaps y ore/puesto/medida/ fuera", borrado.strip()[-80:])


if __name__ == "__main__":
    main()
