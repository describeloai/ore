#!/usr/bin/env python3
"""
MEDIDA · W3 · el puesto (19 de septiembre)

Antes de construir la sesión viva (0031), cuánto cuesta cada pieza sobre el
clúster real, con la imagen `puesto-python:1` (python 3.12 + pandas + pyarrow
+ duckdb + GCS) y la identidad `driver` del inquilino:

  §1  EL SITIO    el pool `jobs-p` (máquina, min/max, taint) y qué GPU hay en la
                  zona y en la región — CONSULTADO, no lanzado
  §2  EL FRÍO     un puesto de sesión (un Job de Kueue que se queda vivo) en
                  frío (pool a cero → nodo) y otro en caliente (el nodo ya está):
                  creado → nodo → programado → imagen (pull) → trabajando
  §3  LA COPIA    desde DENTRO del puesto: la copia más grande del bucket del
                  inquilino, bajada, desenvuelta (ORECOPY1) y en un DataFrame;
                  y una agregación con duckdb sobre el mismo Parquet
  §4  LA COLA     qué dice Kueue de un puesto que no termina: la Workload, la
                  cuota que ocupa, y qué pasa al pedir más de lo que hay

Uso:
  python pruebas-de-fuego/medida-w3-el-puesto.py --inquilino victor

Hace falta `kubectl` con el contexto de la malla y `gcloud` con sesión. Crea
dos Jobs `puesto-medida-*` en el namespace del inquilino y los borra al acabar
(y también si algo falla). No lanza nada de pago fuera del clúster: `jobs-p`
es el pool on-demand que ya existe y escala 0→1→0.
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

os.environ["MSYS_NO_PATHCONV"] = "1"
os.environ["MSYS2_ARG_CONV_EXCL"] = "*"

REGISTRO = "europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore"
IMAGEN = f"{REGISTRO}/puesto-python:1"
ZONA = "europe-west1-b"
REGION = "europe-west1"


def fila(k, v, nota=""):
    print("  %-46s %-24s %s" % (k, v, nota))


def sh(*args, entrada=None):
    exe = shutil.which(args[0]) or args[0]
    r = subprocess.run((exe,) + tuple(args[1:]), input=entrada.encode("utf-8") if entrada else None,
                       capture_output=True)
    return r.stdout.decode("utf-8", "replace") + r.stderr.decode("utf-8", "replace")


def k(*args, ns=None, entrada=None):
    a = ["kubectl"] + (["-n", ns] if ns else []) + list(args)
    return sh(*a, entrada=entrada)


def fecha(s):
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00"))


def segundos(a, b):
    return int((fecha(b) - fecha(a)).total_seconds())


# ── lo que corre DENTRO del puesto ─────────────────────────────────────────
GUION = r'''
import io, json, os, sys, time
T0 = time.time()
def dice(**kw):
    kw["t"] = round(time.time() - T0, 2); print("MEDIDA " + json.dumps(kw), flush=True)
dice(paso="arranca", python=sys.version.split()[0])
import pandas, pyarrow, pyarrow.parquet as pq, duckdb
from google.cloud import storage
dice(paso="importado", pandas=pandas.__version__, pyarrow=pyarrow.__version__, duckdb=duckdb.__version__)
t = time.time(); cli = storage.Client(); b = cli.bucket(os.environ["BUCKET"])
objs = [o for o in cli.list_blobs(b, prefix="ore/v1/") if not o.name.startswith("ore/v1/plan/")]
dice(paso="listado", objetos=len(objs), ms=round((time.time() - t) * 1000))
if not objs:
    dice(paso="sin-copias"); time.sleep(int(os.environ.get("VIVO", "60"))); sys.exit(0)
o = max(objs, key=lambda x: x.size or 0)
t = time.time(); crudo = o.download_as_bytes(); ms_bajar = round((time.time() - t) * 1000)
assert crudo[:8] == b"ORECOPY1", crudo[:8]
n = int.from_bytes(crudo[8:12], "little"); cab = json.loads(crudo[12:12 + n]); carga = crudo[12 + n:]
dice(paso="bajada", clave=o.name, bytes=len(crudo), ms=ms_bajar, campos=list(cab.keys()))
t = time.time(); tabla = pq.read_table(io.BytesIO(carga)); df = tabla.to_pandas()
dice(paso="dataframe", filas=len(df), columnas=len(df.columns), ms=round((time.time() - t) * 1000), mb_en_memoria=round(df.memory_usage(deep=True).sum() / 1e6, 1))
t = time.time(); con = duckdb.connect(); con.register("copia", tabla)
c = df.columns[0]
r = con.execute(f'select count(*), count(distinct "{c}") from copia').fetchone()
dice(paso="duckdb", filas=r[0], distintos_en_primera=r[1], ms=round((time.time() - t) * 1000))
# ¿alcanza internet? (no debería: nodos privados sin NAT + deny-all-egress)
import socket
t = time.time()
try:
    socket.create_connection(("pypi.org", 443), timeout=5).close(); fuera = "SI"
except Exception as e:
    fuera = "no (%s)" % type(e).__name__
dice(paso="internet", alcanza=fuera, ms=round((time.time() - t) * 1000))
dice(paso="vivo", segundos=int(os.environ.get("VIVO", "60")))
time.sleep(int(os.environ.get("VIVO", "60")))
dice(paso="fin")
'''


def job_yaml(ns, inq, nombre, bucket, vivo):
    return f"""apiVersion: batch/v1
kind: Job
metadata:
  name: {nombre}
  namespace: {ns}
  labels:
    kueue.x-k8s.io/queue-name: cola
    ore.dev/tenant: {inq}
    ore.dev/rol: driver
    ore.dev/medida: w3
spec:
  backoffLimit: 0
  ttlSecondsAfterFinished: 600
  activeDeadlineSeconds: 900
  template:
    metadata:
      labels:
        ore.dev/rol: driver
        ore.dev/tenant: {inq}
    spec:
      restartPolicy: Never
      serviceAccountName: driver
      volumes:
        - name: guion
          configMap: {{ name: {nombre} }}
      containers:
        - name: puesto
          image: {IMAGEN}
          imagePullPolicy: IfNotPresent
          command: ["python3", "/guion/main.py"]
          env:
            - {{ name: BUCKET, value: "{bucket}" }}
            - {{ name: VIVO, value: "{vivo}" }}
            - {{ name: HOME, value: /tmp }}
          volumeMounts: [{{ name: guion, mountPath: /guion, readOnly: true }}]
          resources:
            requests: {{cpu: "1", memory: 2Gi}}
            limits:   {{cpu: "2", memory: 4Gi}}
          securityContext:
            allowPrivilegeEscalation: false
            capabilities: {{ drop: [ALL] }}
"""


def crear_puesto(ns, inq, nombre, bucket, vivo):
    with tempfile.TemporaryDirectory() as d:
        p = os.path.join(d, "main.py")
        with open(p, "w", encoding="utf-8", newline="\n") as f:
            f.write(GUION)
        k("create", "configmap", nombre, f"--from-file=main.py={p}", ns=ns)
    print(k("apply", "-f", "-", ns=ns, entrada=job_yaml(ns, inq, nombre, bucket, vivo)).strip())


def borrar_puesto(ns, nombre):
    k("delete", "job", nombre, "--ignore-not-found", "--wait=false", ns=ns)
    k("delete", "configmap", nombre, "--ignore-not-found", ns=ns)


def pod_de(ns, job):
    ps = json.loads(k("get", "pods", "-l", f"job-name={job}", "-o", "json", ns=ns) or "{}").get("items", [])
    return ps[0] if ps else None


def esperar(ns, job, fase, limite):
    t0 = time.time()
    while time.time() - t0 < limite:
        p = pod_de(ns, job)
        if p:
            if fase == "corriendo" and p["status"].get("phase") in ("Running", "Succeeded", "Failed"):
                return p
            if fase == "medido":
                logs = k("logs", p["metadata"]["name"], ns=ns)
                if "MEDIDA {\"paso\": \"vivo\"" in logs or "MEDIDA {\"paso\": \"sin-copias\"" in logs or p["status"].get("phase") in ("Succeeded", "Failed"):
                    return p
        time.sleep(3)
    return pod_de(ns, job)


def linea_de_tiempos(ns, job, pod):
    ev = json.loads(k("get", "events", "--field-selector", f"involvedObject.name={pod}", "-o", "json", ns=ns) or "{}").get("items", [])
    evs = sorted((e.get("lastTimestamp") or e.get("eventTime") or "", e.get("reason", ""), e.get("message", "")) for e in ev)
    j = json.loads(k("get", "job", job, "-o", "json", ns=ns) or "{}")
    creado = j.get("metadata", {}).get("creationTimestamp")
    pedido = next((t for t, r, _ in evs if r == "TriggeredScaleUp"), None)
    programado = next((t for t, r, _ in evs if r == "Scheduled"), None)
    pulled = [(t, m) for t, r, m in evs if r == "Pulled"]
    arranque = next((t for t, r, _ in evs if r == "Started"), None)
    partes = []
    if pedido and programado:
        partes.append("nodo %ds" % segundos(pedido, programado))
    elif programado and creado:
        partes.append("nodo ya estaba (programado a los %ds)" % segundos(creado, programado))
    if pulled:
        t, m = pulled[-1]
        pull = re.search(r"in ([\d.]+m?s)", m)
        tam = re.search(r"\(([^)]*B)[^)]*\)", m)
        partes.append("pull %s%s" % (pull.group(1) if pull else "?", " · " + tam.group(1) if tam else ""))
        if "already present" in m:
            partes.append("imagen ya estaba")
    total = segundos(creado, arranque) if creado and arranque else None
    return total, " · ".join(partes), evs


def medidas_de(ns, pod):
    out = {}
    for l in k("logs", pod, ns=ns).splitlines():
        if l.startswith("MEDIDA "):
            try:
                d = json.loads(l[7:]); out[d.get("paso")] = d
            except ValueError:
                pass
    return out


# ── §1 ─────────────────────────────────────────────────────────────────────
def seccion_1():
    print("\n§1 · EL SITIO — el pool de sesiones y qué GPU hay (consultado, no lanzado)")
    pool = sh("gcloud", "container", "node-pools", "describe", "jobs-p", "--cluster", "ore-mesh", "--zone", ZONA, "--format=json")
    try:
        p = json.loads(pool); a = p.get("autoscaling", {})
        fila("jobs-p", "%s · spot=%s" % (p["config"]["machineType"], p["config"].get("spot", False)),
             "min %s · max %s · taint %s · disco %s GB" % (a.get("minNodeCount", 0), a.get("maxNodeCount"),
                                                            ",".join(t["key"] for t in p["config"].get("taints", [])), p["config"].get("diskSizeGb")))
    except (ValueError, KeyError):
        fila("jobs-p", "?", pool.strip()[:100])
    nodos = json.loads(k("get", "nodes", "-l", "ore.dev/pool=jobs", "-o", "json") or "{}").get("items", [])
    fila("nodos del pool ahora", str(len(nodos)), "0 = el primer puesto paga el frío")
    acc = sh("gcloud", "compute", "accelerator-types", "list", "--filter=zone:(%s)" % " ".join(f"{REGION}-{z}" for z in "bcd"),
             "--format=value(name,zone,maximumCardsPerInstance)")
    por_zona = {}
    for l in acc.splitlines():
        t = l.split("\t") if "\t" in l else l.split()
        if len(t) >= 2:
            por_zona.setdefault(t[1].rsplit("/", 1)[-1], []).append(t[0])
    for z in sorted(por_zona):
        fila("GPU en %s" % z, "%d tipos" % len(por_zona[z]), ", ".join(sorted(por_zona[z])))
    if not por_zona:
        fila("GPU en la región", "?", acc.strip()[:120])
    maq = sh("gcloud", "compute", "machine-types", "list", "--filter=zone:%s AND (name~^a2 OR name~^g2 OR name~^a3 OR name~^n1)" % ZONA,
             "--format=value(name,guestCpus,memoryMb)")
    fams = {}
    for l in maq.splitlines():
        t = l.split()
        if t:
            fams.setdefault(t[0].split("-")[0], []).append(t[0])
    for f in sorted(fams):
        fila("máquinas %s en %s" % (f, ZONA), "%d" % len(fams[f]), ", ".join(sorted(fams[f])[:6]) + (" …" if len(fams[f]) > 6 else ""))
    fila("precio", "lista pública", "cloud.google.com/compute/gpus-pricing — no se consulta por API sin habilitar Billing Catalog")


# ── §2 + §3 ────────────────────────────────────────────────────────────────
def seccion_2_3(ns, inq, bucket):
    print("\n§2 · EL FRÍO — un puesto en frío y otro en caliente")
    creados = []
    try:
        frio = "puesto-medida-frio"
        borrar_puesto(ns, frio)
        time.sleep(2)
        t0 = time.time()
        crear_puesto(ns, inq, frio, bucket, vivo=240); creados.append(frio)
        p = esperar(ns, frio, "corriendo", 600)
        if not p:
            fila("puesto en frío", "no hay pod", k("get", "job", frio, ns=ns).strip()); return creados
        pod = p["metadata"]["name"]
        fila("pod en frío", pod, "%s a los %ds" % (p["status"].get("phase"), int(time.time() - t0)))
        p = esperar(ns, frio, "medido", 300)
        total, partes, evs = linea_de_tiempos(ns, frio, pod)
        fila("en frío: creado → trabajando", "%ss" % total if total is not None else "?", partes)
        m = medidas_de(ns, pod)
        if "importado" in m:
            fila("  python arranca + import pandas/pyarrow/duckdb", "%.1fs" % m["importado"]["t"], "pandas %s · pyarrow %s · duckdb %s" % (m["importado"]["pandas"], m["importado"]["pyarrow"], m["importado"]["duckdb"]))
        # el caliente: el nodo ya está (el frío sigue vivo)
        cal = "puesto-medida-caliente"
        borrar_puesto(ns, cal); time.sleep(2)
        t1 = time.time()
        crear_puesto(ns, inq, cal, bucket, vivo=20); creados.append(cal)
        p2 = esperar(ns, cal, "medido", 300)
        if p2:
            pod2 = p2["metadata"]["name"]
            total2, partes2, _ = linea_de_tiempos(ns, cal, pod2)
            fila("en caliente: creado → trabajando", "%ss" % total2 if total2 is not None else "?", partes2)
            m2 = medidas_de(ns, pod2)
            if "importado" in m2:
                fila("  python arranca + imports", "%.1fs" % m2["importado"]["t"])
        else:
            fila("puesto en caliente", "no hay pod", "")
        print("\n§3 · LA COPIA — desde dentro del puesto (identidad `driver`, bucket del inquilino)")
        if "listado" in m:
            fila("listar `ore/v1/`", "%d objeto(s) · %d ms" % (m["listado"]["objetos"], m["listado"]["ms"]))
        if "bajada" in m:
            fila("bajar la copia más grande", "%.1f MB · %d ms" % (m["bajada"]["bytes"] / 1e6, m["bajada"]["ms"]), m["bajada"]["clave"][:52])
        if "dataframe" in m:
            fila("Parquet → DataFrame", "%d filas × %d col · %d ms" % (m["dataframe"]["filas"], m["dataframe"]["columnas"], m["dataframe"]["ms"]), "%s MB en memoria" % m["dataframe"]["mb_en_memoria"])
        if "duckdb" in m:
            fila("duckdb count/distinct sobre el Parquet", "%d ms" % m["duckdb"]["ms"], "%d filas · %d distintos" % (m["duckdb"]["filas"], m["duckdb"]["distintos_en_primera"]))
        if "internet" in m:
            fila("¿alcanza pypi.org:443?", m["internet"]["alcanza"], "%d ms — debe ser «no»" % m["internet"]["ms"])
        if "sin-copias" in m:
            fila("copias en el bucket", "ninguna", "el inquilino no tiene vistas con copia")
        if not m:
            fila("el guion no dijo nada", "", k("logs", pod, "--tail=20", ns=ns).strip()[:300])
        print("\n§4 · LA COLA — Kueue con un puesto que no termina")
        wl = json.loads(k("get", "workloads", "-o", "json", ns=ns) or "{}").get("items", [])
        for w in wl:
            n = w["metadata"].get("ownerReferences", [{}])[0].get("name", w["metadata"]["name"])
            if not n.startswith("puesto-medida"):
                continue
            adm = w.get("status", {}).get("admission", {})
            conds = {c["type"]: c["status"] for c in w.get("status", {}).get("conditions", [])}
            fila("workload %s" % n, "admitida=%s" % conds.get("Admitted", "?"), "cola %s · sabor %s" % (adm.get("clusterQueue"), ",".join(sorted({f for ps in adm.get("podSetAssignments", []) for f in ps.get("flavors", {}).values()}))))
        cq = json.loads(k("get", "clusterqueue", f"cq-{inq}", "-o", "json") or "{}")
        uso = cq.get("status", {}).get("flavorsUsage") or cq.get("status", {}).get("flavorsReservation") or []
        for f in uso:
            fila("cq-%s · sabor %s" % (inq, f.get("name")), ", ".join("%s %s" % (r["name"], r["total"]) for r in f.get("resources", [])), "cuota cpu %s · admitidas %s · pendientes %s" % (
                cq["spec"]["resourceGroups"][0]["flavors"][0]["resources"][0]["nominalQuota"], cq.get("status", {}).get("admittedWorkloads"), cq.get("status", {}).get("pendingWorkloads")))
        fila("un puesto de larga vida", "es un Job que no termina", "ocupa su cuota mientras viva; el TTL/tope lo pone `activeDeadlineSeconds`")
    finally:
        for n in creados:
            borrar_puesto(ns, n)
        fila("limpieza", "Jobs y ConfigMaps borrados", ", ".join(creados))
    return creados


def main():
    inq = "victor"
    if "--inquilino" in sys.argv:
        inq = sys.argv[sys.argv.index("--inquilino") + 1]
    ns = f"t-{inq}"
    bucket = f"project-8853a180-450d-47be-b83-t-{inq}-copia"
    print("MEDIDA · W3 · el puesto · %s · %s" % (inq, dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%MZ")))
    fila("imagen", IMAGEN.rsplit("/", 1)[-1], "python:3.12-slim + pandas + pyarrow + duckdb + GCS")
    seccion_1()
    seccion_2_3(ns, inq, bucket)
    nodos = json.loads(k("get", "nodes", "-l", "ore.dev/pool=jobs", "-o", "json") or "{}").get("items", [])
    fila("nodos del pool al acabar", str(len(nodos)), "el autoescalado lo baja solo en ~10 min")


if __name__ == "__main__":
    main()
