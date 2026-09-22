#!/usr/bin/env python3
"""
MEDIDA · W3.7 gobierno ②b, la credencial de lectura (22 de septiembre), antes
de abordarla. En `t-demo`, con `jobs-p` 0 → 1 → 0 (con go).

② puso el conducto de la lectura en `datos_del_puesto`: lo que un dataset lleva
se coteja con `contextSurface.workspace` y se niega con el OOS. Pero en el
clúster `over()` lee el bucket **con el token del pod** (W3.5b, camino (b)), y la
cuenta `ore-puesto-<n>` es `objectViewer` del bucket entero: una celda que
tenga un `metadata_location` (o lo liste) lee lo que quiera sin pasar por
`datos`. Aquí se mide, desde un pod con la identidad del puesto de verdad:

  §1  EL AGUJERO      el token del pod lee el metadata.json de un dataset, otro
                      dataset cualquiera, y la capa (`ore/puesto/`): lo que hoy
                      alcanza
  §2  LA PRESTADA     un token acotado (CAB) con `objectViewer` sobre la tabla:
                      lee la suya y no otra; cuánto tarda STS en acotarlo; y
                      DuckDB `iceberg_scan` con él como bearer (el camino real
                      del SDK)
  §3  LA CUENTA       la misma acotación sobre `ore/puesto/` como lo que será la
                      condición IAM de `ore-puesto-<n>`: la capa se lee, los
                      datasets no
  y de paso, cuántos punteros de demo siguen siendo sobres (`clave` sin
  `metadata_location`), que la prestada no cubriría.

Uso:  python pruebas-de-fuego/medida-w3-credencial.py [--sha <12 hex>] [--celda t-victor] [--cuenta] [--capa <id>]
  --cuenta: sólo §5, con la condición IAM ya puesta en `ore-puesto-<n>` (lo que
  el token del pod alcanza entonces).
  --capa <id>: sólo §6, el init `traer-la-capa` de la plantilla bajo esa
  condición (la capa por su nombre, sin listar), y si /capa queda instalada.
Necesita kubectl (ore-mesh) y la imagen puesto-python:<sha> en el registro.
Deja el nodo a 0 y no escribe nada en el bucket ni en el árbol.
No imprime ningún token.
"""
import json
import os
import subprocess
import sys
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__))).replace("\\", "/")
PROYECTO = "project-8853a180-450d-47be-b83"
REGISTRO = "europe-west1-docker.pkg.dev/%s/ore" % PROYECTO
NS = sys.argv[sys.argv.index("--celda") + 1] if "--celda" in sys.argv else "t-demo"
INQUILINO = NS[2:] if NS.startswith("t-") else NS
SHA = sys.argv[sys.argv.index("--sha") + 1] if "--sha" in sys.argv else subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True, cwd=RAIZ).stdout.strip()[:12]


def fila(a, b="", c=""):
    print("  %-58s %-20s %s" % (a, b, c))


def ms(t):
    return int((time.time() - t) * 1000)


def k(*args, entrada=None):
    env = dict(os.environ, MSYS_NO_PATHCONV="1", MSYS2_ARG_CONV_EXCL="*")
    r = subprocess.run(["kubectl", "-n", NS, *args], input=entrada, capture_output=True, text=True, encoding="utf-8", env=env)
    return r.returncode, r.stdout, r.stderr


GUION = r'''
import json, os, time, urllib.parse, urllib.request, urllib.error
T0 = time.time()
def di(que, **kw): print("### " + json.dumps(dict(que=que, ms=int((time.time() - T0) * 1000), **kw)), flush=True)
def testigo():
    d = os.environ["DIRECCION"].rstrip("/"); realm = os.environ.get("REALM", "rubix")
    cli = open("/puesto/agente-cliente").read().strip(); sec = open("/puesto/agente-secreto").read().strip()
    datos = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": cli, "client_secret": sec}).encode()
    with urllib.request.urlopen(d + "/realms/%s/protocol/openid-connect/token" % realm, data=datos, timeout=20) as r:
        return json.load(r)["access_token"]
def pide(ruta, tok):
    q = urllib.request.Request(os.environ["ORE_SERVE"] + ruta, headers={"authorization": "Bearer " + tok, "accept": "application/json"})
    with urllib.request.urlopen(q, timeout=60) as r:
        return json.load(r)
def token_del_pod():
    q = urllib.request.Request("http://169.254.169.254/computeMetadata/v1/instance/service-accounts/default/token", headers={"Metadata-Flavor": "Google"})
    with urllib.request.urlopen(q, timeout=10) as r:
        return json.load(r)["access_token"]
def gcs(objeto, tok):
    url = "https://storage.googleapis.com/storage/v1/b/%s/o/%s?alt=media" % (BUCKET, urllib.parse.quote(objeto, safe=""))
    q = urllib.request.Request(url, headers={"authorization": "Bearer " + tok})
    try:
        with urllib.request.urlopen(q, timeout=30) as r:
            return r.status, len(r.read())
    except urllib.error.HTTPError as e:
        return e.code, 0
def lista(prefijo, tok, n=3):
    url = "https://storage.googleapis.com/storage/v1/b/%s/o?prefix=%s&maxResults=%d" % (BUCKET, urllib.parse.quote(prefijo, safe=""), n)
    q = urllib.request.Request(url, headers={"authorization": "Bearer " + tok})
    try:
        with urllib.request.urlopen(q, timeout=30) as r:
            return r.status, [i["name"] for i in json.load(r).get("items", [])]
    except urllib.error.HTTPError as e:
        return e.code, []
def cab(tok, prefijo, roles):
    regla = {"accessBoundary": {"accessBoundaryRules": [{"availableResource": "//storage.googleapis.com/projects/_/buckets/" + BUCKET,
             "availablePermissions": ["inRole:roles/storage." + r for r in roles],
             "availabilityCondition": {"expression": "resource.name.startsWith('projects/_/buckets/%s/objects/%s')" % (BUCKET, prefijo)}}]}}
    cuerpo = urllib.parse.urlencode({"grant_type": "urn:ietf:params:oauth:grant-type:token-exchange", "subject_token_type": "urn:ietf:params:oauth:token-type:access_token",
                                     "requested_token_type": "urn:ietf:params:oauth:token-type:access_token", "subject_token": tok, "options": json.dumps(regla)}).encode()
    t = time.time()
    with urllib.request.urlopen(urllib.request.Request("https://sts.googleapis.com/v1/token", data=cuerpo, headers={"content-type": "application/x-www-form-urlencoded"}), timeout=20) as r:
        j = json.load(r)
    return j["access_token"], int((time.time() - t) * 1000), j.get("expires_in")

BUCKET = os.environ["BUCKET"]
di("python arrancado")
ag = testigo(); di("testigo del agente")
pod = token_del_pod(); di("token del pod")
ds = pide("/datasets", ag)
items = ds.get("datasets") if isinstance(ds, dict) else ds
con_ml = [d for d in items if d.get("metadata_location", "").startswith("gs://")]
sobres = [d for d in items if not d.get("metadata_location") and d.get("clave")]
di("datasets del inquilino", total=len(items), iceberg=len(con_ml), sobres=len(sobres))
if len(con_ml) < 2:
    di("fin", motivo="hacen falta dos datasets Iceberg"); raise SystemExit(0)
a, b = con_ml[0], con_ml[1]
def partes(ml):
    obj = ml[len("gs://" + BUCKET + "/"):]
    return obj, obj.rsplit("/metadata/", 1)[0] + "/"
obj_a, pref_a = partes(a["metadata_location"]); obj_b, pref_b = partes(b["metadata_location"])
di("dos datasets", a=a.get("dataset") or a.get("nombre"), b=b.get("dataset") or b.get("nombre"))

# §1 · el agujero
c1, n1 = gcs(obj_a, pod); c2, n2 = gcs(obj_b, pod)
c3, capa = lista("ore/puesto/", pod)
c4, todo = lista("ore/v2/datasets/", pod, 5)
di("§1 pod lee metadata.json de a", http=c1, bytes=n1)
di("§1 pod lee metadata.json de b", http=c2, bytes=n2)
di("§1 pod lista ore/puesto/ (la capa)", http=c3, objetos=len(capa))
di("§1 pod lista ore/v2/datasets/", http=c4, objetos=len(todo))

# §2 · la prestada: objectViewer sobre la tabla a
tok_a, sts_ms, exp = cab(pod, pref_a, ["objectViewer"])
di("§2 STS acota el token a la tabla a", sts_ms=sts_ms, expires_in=exp)
di("§2 prestada lee a", http=gcs(obj_a, tok_a)[0])
di("§2 prestada lee b", http=gcs(obj_b, tok_a)[0])
di("§2 prestada lista ore/puesto/", http=lista("ore/puesto/", tok_a)[0])
# el camino real: DuckDB iceberg_scan con la prestada como bearer
try:
    import duckdb
    con = duckdb.connect()
    con.execute("set autoinstall_known_extensions=false"); con.execute("set autoload_known_extensions=false")
    con.execute("set extension_directory='/opt/ore/duckdb'")
    for e in ("json", "icu", "avro", "iceberg", "httpfs"):
        con.execute("load " + e)
    con.execute("create or replace secret ore_gcs (type http, bearer_token '%s')" % tok_a.replace("'", "''"))
    raiz = "https://storage.googleapis.com/%s/%s" % (BUCKET, pref_a.rstrip("/"))
    version = obj_a.rsplit("/", 1)[1][:-len(".metadata.json")]
    t = time.time()
    n = con.execute("select count(*) from iceberg_scan('%s', version='%s', allow_moved_paths=true)" % (raiz, version)).fetchone()[0]
    di("§2 duckdb iceberg_scan de a con la prestada", filas=n, duckdb_ms=int((time.time() - t) * 1000))
    raiz_b = "https://storage.googleapis.com/%s/%s" % (BUCKET, pref_b.rstrip("/"))
    version_b = obj_b.rsplit("/", 1)[1][:-len(".metadata.json")]
    try:
        con.execute("select count(*) from iceberg_scan('%s', version='%s', allow_moved_paths=true)" % (raiz_b, version_b)).fetchone()
        di("§2 duckdb iceberg_scan de b con la prestada de a", resultado="LEYÓ")
    except Exception as e:
        di("§2 duckdb iceberg_scan de b con la prestada de a", resultado="no: " + str(e)[:90])
except Exception as e:
    di("§2 duckdb", error=str(e)[:160])

# §3 · la cuenta del puesto como será: objectViewer sólo bajo ore/puesto/
tok_p, sts_ms, _ = cab(pod, "ore/puesto/", ["objectViewer"])
di("§3 STS acota el token a ore/puesto/", sts_ms=sts_ms)
di("§3 acotado lista ore/puesto/ (la capa)", http=lista("ore/puesto/", tok_p)[0])
if capa:
    di("§3 acotado lee un fichero de la capa", http=gcs(capa[0], tok_p)[0])
di("§3 acotado lee metadata.json de a", http=gcs(obj_a, tok_p)[0])
di("§3 acotado lista ore/v2/datasets/", http=lista("ore/v2/datasets/", tok_p)[0])
di("fin")
'''


# §5 · la cuenta del puesto con la CONDICIÓN IAM puesta (`--cuenta`): lo que el
# token del pod alcanza cuando `ore-puesto-<n>` sólo es objectViewer bajo
# `ore/puesto/` — lista la capa con prefijo, lee un fichero suyo, y no lista ni
# lee datasets.
GUION_CUENTA = GUION[:GUION.index("BUCKET = os.environ")] + r'''
BUCKET = os.environ["BUCKET"]
di("python arrancado")
pod = token_del_pod(); di("token del pod")
c, capa = lista("ore/puesto/", pod)
di("§5 pod lista ore/puesto/ con prefijo", http=c, objetos=len(capa))
if capa:
    di("§5 pod lee un fichero de la capa", http=gcs(capa[0], pod)[0])
if os.environ.get("OBJETO_CAPA"):
    di("§5 pod lee un fichero de la capa por su nombre", http=gcs(os.environ["OBJETO_CAPA"], pod)[0])
c, todo = lista("ore/v2/datasets/", pod)
di("§5 pod lista ore/v2/datasets/", http=c, objetos=len(todo))
c, todo = lista("", pod)
di("§5 pod lista el bucket entero", http=c, objetos=len(todo))
di("§5 pod lee un metadata.json cualquiera", http=gcs("ore/v2/datasets/no_existe/metadata/v1.metadata.json", pod)[0])
di("fin")
'''


# §6 · la capa por su nombre (`--capa <id>`): el init `traer-la-capa` de
# `51-el-puesto.yaml` tal cual, bajo la condición IAM, y el guion mira /capa.
def capa_init(capa, bucket):
    plantilla = open(RAIZ + "/malla/51-el-puesto.yaml", encoding="utf-8").read()
    i = plantilla.index("        - name: traer-la-capa")
    a = plantilla.index("            - |\n", i) + len("            - |\n")
    b = plantilla.index("          resources:", a)
    mando = "\n".join(l[14:] for l in plantilla[a:b].rstrip().splitlines())
    return {"name": "traer-la-capa", "image": "%s/puesto-python:%s" % (REGISTRO, SHA), "imagePullPolicy": "Always",
            "env": [{"name": "HOME", "value": "/tmp"}, {"name": "BUCKET", "value": bucket}, {"name": "CAPA", "value": capa}],
            "volumeMounts": [{"name": "capa", "mountPath": "/capa"}, {"name": "trabajo", "mountPath": "/trabajo"}], "command": ["/bin/sh", "-c"], "args": [mando],
            "resources": {"requests": {"cpu": "500m", "memory": "1Gi"}, "limits": {"cpu": "2", "memory": "2Gi"}},
            "securityContext": {"allowPrivilegeEscalation": False, "runAsNonRoot": True, "runAsUser": 65532, "seccompProfile": {"type": "RuntimeDefault"}, "capabilities": {"drop": ["ALL"]}}}


GUION_CAPA = r'''
import json, os, time
T0 = time.time()
def di(que, **kw): print("### " + json.dumps(dict(que=que, ms=int((time.time() - T0) * 1000), **kw)), flush=True)
hay = os.path.isdir("/capa") and any(n.startswith("polars") for n in os.listdir("/capa"))
di("§6 la capa instalada en /capa por su nombre", polars=hay, entradas=len(os.listdir("/capa")) if os.path.isdir("/capa") else 0)
di("fin")
'''


def testigo_init():
    plantilla = open(RAIZ + "/malla/51-el-puesto.yaml", encoding="utf-8").read()
    i = plantilla.index("              import base64, json, os, time, urllib.request")
    j = plantilla.index("          resources:", i)
    codigo = "\n".join(l[14:] for l in plantilla[i:j].rstrip().splitlines())
    return {"name": "traer-el-testigo", "image": "%s/puesto-python:%s" % (REGISTRO, SHA), "imagePullPolicy": "Always",
            "env": [{"name": "HOME", "value": "/tmp"}, {"name": "PROYECTO", "value": PROYECTO}, {"name": "CELDA", "value": NS}],
            "volumeMounts": [{"name": "puesto", "mountPath": "/puesto"}], "command": ["python3", "-c"], "args": [codigo],
            "resources": {"requests": {"cpu": "50m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
            "securityContext": {"allowPrivilegeEscalation": False, "runAsNonRoot": True, "runAsUser": 65532, "seccompProfile": {"type": "RuntimeDefault"}, "capabilities": {"drop": ["ALL"]}}}


def job(nombre, bucket):
    cont = {
        "name": "python", "image": "%s/puesto-python:%s" % (REGISTRO, SHA), "imagePullPolicy": "Always",
        "env": [{"name": "HOME", "value": "/tmp"}, {"name": "ORE_SERVE", "value": "http://ore-serve.%s.svc.cluster.local:8080" % NS},
                {"name": "DIRECCION", "value": "http://idp-service.identidad.svc.cluster.local:8080"}, {"name": "REALM", "value": "rubix"},
                {"name": "BUCKET", "value": bucket}, {"name": "OBJETO_CAPA", "value": os.environ.get("OBJETO_CAPA", "")}],
        "volumeMounts": [{"name": "puesto", "mountPath": "/puesto", "readOnly": True}, {"name": "guiones", "mountPath": "/guiones", "readOnly": True}, {"name": "trabajo", "mountPath": "/trabajo"}],
        "workingDir": "/trabajo", "command": ["python3", "/guiones/guion.py"],
        "resources": {"requests": {"cpu": "500m", "memory": "1Gi"}, "limits": {"cpu": "2", "memory": "2Gi"}},
        "securityContext": {"allowPrivilegeEscalation": False, "runAsNonRoot": True, "runAsUser": 65532, "seccompProfile": {"type": "RuntimeDefault"}, "capabilities": {"drop": ["ALL"]}},
    }
    j = {"apiVersion": "batch/v1", "kind": "Job",
         "metadata": {"name": nombre, "namespace": NS, "labels": {"kueue.x-k8s.io/queue-name": "cola", "ore.dev/tenant": INQUILINO, "ore.dev/rol": "puesto"}},
         "spec": {"backoffLimit": 0, "ttlSecondsAfterFinished": 1800, "activeDeadlineSeconds": 1200,
                  "template": {"metadata": {"labels": {"ore.dev/rol": "puesto", "ore.dev/tenant": INQUILINO}},
                               "spec": {"restartPolicy": "Never", "serviceAccountName": "puesto",
                                        "volumes": [{"name": "puesto", "emptyDir": {"medium": "Memory"}}, {"name": "trabajo", "emptyDir": {}}, {"name": "guiones", "configMap": {"name": nombre}}],
                                        "initContainers": [testigo_init()],
                                        "containers": [cont]}}}}
    if "--capa" in sys.argv:
        capa = sys.argv[sys.argv.index("--capa") + 1]
        j["spec"]["template"]["spec"]["initContainers"] = [capa_init(capa, bucket)]
        j["spec"]["template"]["spec"]["volumes"].append({"name": "capa", "emptyDir": {}})
        cont["volumeMounts"].append({"name": "capa", "mountPath": "/capa", "readOnly": True})
    guion = GUION_CAPA if "--capa" in sys.argv else GUION_CUENTA if "--cuenta" in sys.argv else GUION
    cm = {"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": nombre, "namespace": NS}, "data": {"guion.py": guion}}
    return json.dumps(cm) + "\n---\n" + json.dumps(j)


def esperar(nombre, plazo=1200):
    t0 = time.time(); fase = ""
    while time.time() - t0 < plazo:
        c, out, _ = k("get", "job", nombre, "-o", "jsonpath={.status.succeeded}/{.status.failed}")
        ok, mal = (out.split("/") + [""])[:2]
        if ok == "1":
            return "ok", int(time.time() - t0)
        if mal and mal != "0":
            return "falló", int(time.time() - t0)
        c, out, _ = k("get", "pods", "-l", "job-name=" + nombre, "-o", "jsonpath={.items[0].status.phase}")
        if out != fase:
            fase = out; fila("  el pod", fase or "(esperando nodo)", "%ds" % int(time.time() - t0))
        time.sleep(5)
    return "plazo", int(time.time() - t0)


def main():
    c, out, _ = k("get", "cm", "ore-serve", "-o", "jsonpath={.data.ORE_GCS_BUCKET}")
    bucket = out.strip() or "%s-%s-copia" % (PROYECTO, NS)
    print("§1–§3 · en %s, con la identidad del puesto (SA puesto), puesto-python:%s, bucket %s" % (NS, SHA, bucket))
    nombre = "medida-w37-cred-" + SHA[:8]
    k("delete", "job", nombre, "--ignore-not-found"); k("delete", "configmap", nombre, "--ignore-not-found")
    t0 = time.time()
    c, out, err = k("apply", "-f", "-", entrada=job(nombre, bucket))
    if c:
        print("  no se pudo crear el Job:", err[:300]); return
    fila("Job creado", "%d ms" % ms(t0), nombre)
    estado, seg = esperar(nombre)
    fila("el Job", estado, "%d s" % seg)
    if "--capa" in sys.argv:
        c, log, _ = k("logs", "job/" + nombre, "-c", "traer-la-capa")
        fila("  traer-la-capa dijo", "", " | ".join(l for l in log.strip().splitlines() if l.strip())[-200:] or "(nada)")
    c, log, _ = k("logs", "job/" + nombre, "-c", "python")
    for l in log.splitlines():
        if l.startswith("### "):
            j = json.loads(l[4:]); fila("  " + j.pop("que"), "%d ms" % j.pop("ms"), json.dumps(j, ensure_ascii=False) if j else "")
    raro = [l for l in log.splitlines() if not l.startswith("### ") and l.strip()]
    if raro:
        fila("  el guion dijo además", "", " | ".join(raro[-4:])[:240])
    k("delete", "job", nombre, "--ignore-not-found"); k("delete", "configmap", nombre, "--ignore-not-found")
    c, out, _ = k("get", "jobs", "-o", "name")
    fila("jobs que quedan en " + NS, str(len(out.split())) if out.strip() else "0", out.replace("\n", " ")[:120])


if __name__ == "__main__":
    main()
