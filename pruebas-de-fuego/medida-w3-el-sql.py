#!/usr/bin/env python3
"""
MEDIDA · W3.3 · SQL sobre el bucket (19 de septiembre)

Un fichero `.sql` del árbol pregunta a las COPIAS (Parquet en el bucket del
inquilino) por el nombre de sus vistas: `select … from hr.espanoles`. Antes de
decidir dónde corre —en el puesto (la sesión, un nodo: DuckDB) o como trabajo
encolado— cuánto cuesta cada cosa, con la red y los recursos del puesto
(2 CPU · 4 GiB, rol `puesto`, sin internet):

  §1  LA COPIA REAL   la copia más grande del inquilino: bajar, DuckDB sobre el
                      Parquet en memoria: count, group by, join consigo misma
  §2  A ESCALA        un Parquet SINTÉTICO de N filas (20 M, 100 M, 200 M) escrito
                      en el disco del pod: escribirlo, `count(*)`, `group by` con
                      agregados, `where` selectivo — el techo de UNA sesión
  §3  EL BUCKET       ese Parquet grande subido al bucket y bajado otra vez: el
                      caudal real de GCS desde el puesto (private.googleapis.com)

Uso:
  python pruebas-de-fuego/medida-w3-el-sql.py --inquilino victor [--filas 200000000]

Crea un Job `sql-medida` en el namespace del inquilino y lo borra al acabar;
deja y retira `ore/puesto/medida-sql/` en el bucket. Nada de pago fuera del clúster.
"""
import datetime as dt
import json
import os
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
PREFIJO = "ore/puesto/medida-sql"


def fila(k, v, nota=""):
    print("  %-46s %-26s %s" % (k, v, nota))


def sh(*args, entrada=None):
    exe = shutil.which(args[0]) or args[0]
    r = subprocess.run((exe,) + tuple(args[1:]), input=entrada.encode("utf-8") if entrada else None, capture_output=True)
    return r.stdout.decode("utf-8", "replace") + r.stderr.decode("utf-8", "replace")


def k(*args, ns=None, entrada=None):
    return sh(*(["kubectl"] + (["-n", ns] if ns else []) + list(args)), entrada=entrada)


GUION = r'''
import io, json, os, sys, time
T0 = time.time()
def dice(**kw):
    kw["t"] = round(time.time() - T0, 2); print("MEDIDA " + json.dumps(kw), flush=True)
import duckdb, pyarrow.parquet as pq
from google.cloud import storage
con = duckdb.connect(); con.execute("set threads to 2; set memory_limit = '3GB'")
cli = storage.Client(); b = cli.bucket(os.environ["BUCKET"])
# §1 la copia real
objs = [o for o in cli.list_blobs(b, prefix="ore/v1/") if not o.name.startswith("ore/v1/plan/")]
if objs:
    o = max(objs, key=lambda x: x.size or 0)
    t = time.time(); crudo = o.download_as_bytes(); n = int.from_bytes(crudo[8:12], "little"); carga = crudo[12 + n:]
    open("/trabajo/copia.parquet", "wb").write(carga)
    dice(paso="copia-bajada", mb=round(len(crudo) / 1e6, 1), ms=round((time.time() - t) * 1000))
    cols = [f.name for f in pq.read_schema("/trabajo/copia.parquet")]
    t = time.time(); r = con.execute("select count(*) from read_parquet('/trabajo/copia.parquet')").fetchone(); dice(paso="copia-count", filas=r[0], ms=round((time.time() - t) * 1000))
    c = cols[-1]
    t = time.time(); r = con.execute(f'select "{c}", count(*) from read_parquet(\'/trabajo/copia.parquet\') group by 1 order by 2 desc limit 5').fetchall(); dice(paso="copia-groupby", grupos=len(r), ms=round((time.time() - t) * 1000), columna=c)
    c0 = cols[0]
    t = time.time(); r = con.execute(f'select count(*) from read_parquet(\'/trabajo/copia.parquet\') a join read_parquet(\'/trabajo/copia.parquet\') b on a."{c0}" = b."{c0}"').fetchone(); dice(paso="copia-join", filas=r[0], ms=round((time.time() - t) * 1000))
# §2 a escala: un parquet sintetico
for n in [int(x) for x in os.environ.get("FILAS", "20000000,100000000,200000000").split(",")]:
    f = "/trabajo/grande.parquet"
    t = time.time()
    con.execute(f"copy (select i as id, i % 1000 as cliente, (i * 7919) % 100000 as importe, chr(cast(65 + i % 26 as integer)) as pais from range({n}) t(i)) to '{f}' (format parquet, row_group_size 1000000)")
    mb = os.path.getsize(f) / 1e6
    dice(paso="escrito", filas=n, mb=round(mb, 1), ms=round((time.time() - t) * 1000))
    t = time.time(); r = con.execute(f"select count(*) from read_parquet('{f}')").fetchone(); dice(paso="count", filas=n, ms=round((time.time() - t) * 1000))
    t = time.time(); r = con.execute(f"select pais, count(*), sum(importe), avg(importe) from read_parquet('{f}') group by pais order by 2 desc").fetchall(); dice(paso="groupby", filas=n, grupos=len(r), ms=round((time.time() - t) * 1000))
    t = time.time(); r = con.execute(f"select count(*) from read_parquet('{f}') where cliente = 7 and importe > 50000").fetchone(); dice(paso="where", filas=n, resultado=r[0], ms=round((time.time() - t) * 1000))
    t = time.time(); r = con.execute(f"select cliente, count(*) c from read_parquet('{f}') group by cliente order by c desc limit 10").fetchall(); dice(paso="topn", filas=n, ms=round((time.time() - t) * 1000))
    ultimo = (n, f, mb)
# §3 el bucket: subir y bajar el grande
n, f, mb = ultimo
pre = os.environ["PREFIJO"] + "/grande.parquet"
t = time.time(); b.blob(pre).upload_from_filename(f, timeout=600); dice(paso="subido", mb=round(mb, 1), ms=round((time.time() - t) * 1000))
os.remove(f)
t = time.time(); b.blob(pre).download_to_filename(f, timeout=600); dice(paso="bajado", mb=round(mb, 1), ms=round((time.time() - t) * 1000))
t = time.time(); r = con.execute(f"select count(*) from read_parquet('{f}')").fetchone(); dice(paso="count-tras-bajar", filas=r[0], ms=round((time.time() - t) * 1000))
b.blob(pre).delete()
dice(paso="fin")
'''


def job_yaml(ns, inq, nombre, bucket, filas):
    return f"""apiVersion: batch/v1
kind: Job
metadata:
  name: {nombre}
  namespace: {ns}
  labels:
    kueue.x-k8s.io/queue-name: cola
    ore.dev/tenant: {inq}
    ore.dev/rol: puesto
    ore.dev/medida: w3-sql
spec:
  backoffLimit: 0
  ttlSecondsAfterFinished: 600
  activeDeadlineSeconds: 1800
  template:
    metadata:
      labels:
        ore.dev/rol: puesto
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
            - {{ name: PREFIJO, value: "{PREFIJO}" }}
            - {{ name: FILAS, value: "{filas}" }}
            - {{ name: HOME, value: /tmp }}
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


def main():
    inq, filas = "victor", "20000000,100000000,200000000"
    if "--inquilino" in sys.argv:
        inq = sys.argv[sys.argv.index("--inquilino") + 1]
    if "--filas" in sys.argv:
        filas = sys.argv[sys.argv.index("--filas") + 1]
    ns = f"t-{inq}"
    bucket = f"project-8853a180-450d-47be-b83-t-{inq}-copia"
    nombre = "sql-medida"
    print("MEDIDA · W3.3 · SQL sobre el bucket · %s · %s" % (inq, dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%MZ")))
    fila("el puesto", "2 CPU · 4 GiB · rol puesto", "DuckDB %s threads=2, memory_limit 3GB" % "")
    k("delete", "job", nombre, "--ignore-not-found", "--wait=false", ns=ns)
    k("delete", "configmap", nombre, "--ignore-not-found", ns=ns)
    time.sleep(2)
    try:
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "main.py")
            with open(p, "w", encoding="utf-8", newline="\n") as f:
                f.write(GUION)
            k("create", "configmap", nombre, f"--from-file=main.py={p}", ns=ns)
        k("apply", "-f", "-", ns=ns, entrada=job_yaml(ns, inq, nombre, bucket, filas))
        t0 = time.time(); pod = None; vistos = set()
        while time.time() - t0 < 1800:
            ps = json.loads(k("get", "pods", "-l", f"job-name={nombre}", "-o", "json", ns=ns) or "{}").get("items", [])
            if ps:
                pod = ps[0]["metadata"]["name"]
                for l in k("logs", pod, ns=ns).splitlines():
                    if l.startswith("MEDIDA ") and l not in vistos:
                        vistos.add(l)
                        try:
                            m = json.loads(l[7:])
                        except ValueError:
                            continue
                        paso = m.pop("paso"); t = m.pop("t", 0)
                        fila("  %s" % paso, " · ".join("%s %s" % (a, b) for a, b in m.items() if a != "ms"), "%s ms" % m.get("ms", "") if "ms" in m else "")
                if ps[0]["status"].get("phase") in ("Succeeded", "Failed"):
                    fila("el Job", ps[0]["status"].get("phase"), "%d s en total" % int(time.time() - t0))
                    if ps[0]["status"].get("phase") == "Failed":
                        print(k("logs", pod, "--tail=15", ns=ns))
                    break
            time.sleep(5)
    finally:
        k("delete", "job", nombre, "--ignore-not-found", "--wait=false", ns=ns)
        k("delete", "configmap", nombre, "--ignore-not-found", ns=ns)
        sh("gcloud", "storage", "rm", "-r", f"gs://{bucket}/{PREFIJO}/**", "--quiet")
        fila("limpieza", "Job, ConfigMap y ore/puesto/medida-sql/ fuera")


if __name__ == "__main__":
    main()
