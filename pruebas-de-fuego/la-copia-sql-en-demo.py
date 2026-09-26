#!/usr/bin/env python3
"""
LA COPIA DE UNA VISTA SQL EN DEMO (ADR 0040 paso 4c): lo que `el-lago.sh` 14c no
puede afirmar, porque allí el almacén es un S3 de mentira y no hay Job:

  · el Job de la copia DE VERDAD en `t-demo` (Flux → Kueue → `jobs-p`) con sus
    tres contenedores en fila sobre el mismo clon: `preparar` (ore-drivers)
    vuelca la consulta y lo que lee con `ore-store-gcs` y la identidad del pod;
    `calcular` (puesto-python, SIN el testigo montado) la ejecuta con DuckDB
    como 65532 sobre el `emptyDir`; `copiar` la sella y empuja el puntero a la
    forja del inquilino;
  · la plantilla que llegó a la cola por la convergencia, rendida como la rinde
    `ore-serve` (`cola::rendir_rehacer`).

Todo va POR LA FORJA del inquilino (un túnel `kubectl port-forward` a
`t-demo/forja`, con el testigo `t-demo-forja-token` de Secret Manager, que no se
imprime ni toca el disco): desde W3.7 un agente no escribe en `/arbol` ni pide
`copia/rehacer`.

Fases, en orden:

  mirar     clona el árbol y la cola; dice qué copias hay con puntero hecho (las
            candidatas a entrada) y si la plantilla de la cola ya trae `calcular`
  paquete   el paquete `prueba_sql` (sólo su `package.yaml`), por la forja
  escribir  un Job con el contenedor de Python del puesto (las imágenes de ESTE
            commit) escribe `prueba_sql.entrada` con `write()` —6 filas, `letra`
            y `n`— con el testigo del agente, por `/v1`: la entrada de la copia.
            En demo no había ninguna (las copias de `olist_copia` están en error)
  sembrar   [--entrada=<p.d> --columna=<c>, por defecto prueba_sql.entrada y
            letra]: una View v1alpha14 (`SELECT c, count(*) AS n FROM <entrada>
            GROUP BY c`) y su copia (`Dataset` con `from: { view }`); `ore
            validate` aquí con el `ore` de este commit, y push al árbol
  copiar    encola un Job «rehacer» de `prueba_sql.porColumna` (sólo esa)
  esperar   el Job en `t-demo`, y los registros de sus tres contenedores
  ver       el puntero: copiada, `leidas` = las filas de la entrada, el testigo es
            su snapshot, y la cabecera en el bucket nombra la consulta
  limpiar   retira el paquete y los punteros del árbol y el Job de la cola; los
            objetos del bucket los recoge la pasada siguiente (`--recoger`), no
            esta máquina

Uso:  python pruebas-de-fuego/la-copia-sql-en-demo.py <fase> [--entrada=p.d --columna=c]
Necesita kubectl (el contexto del clúster), gcloud (sesión propia) y git.
⚠️ `escribir` y `copiar` lanzan un Job cada una en `jobs-p` (el pool de pago):
   0 → 1 nodo, que se queda ~10 min.
"""
import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROYECTO = "project-8853a180-450d-47be-b83"
INQUILINO = "demo"
NS = "t-" + INQUILINO
GCLOUD = "gcloud.cmd" if os.name == "nt" else "gcloud"
PAQUETE = "prueba_sql"
VISTA = "porColumna"
COPIA = "porColumnaCopia"
QN_COPIA = "%s.%s" % (PAQUETE, COPIA)
TRABAJO = os.path.join(tempfile.gettempdir(), "ore-copia-sql-en-demo")
REGISTRO = "europe-west1-docker.pkg.dev/%s/ore" % PROYECTO
ENTRADA, COLUMNA = PAQUETE + ".entrada", "letra"
FASE = next((a for a in sys.argv[1:] if not a.startswith("--")), "mirar")


def opcion(nombre):
    for a in sys.argv:
        if a.startswith("--%s=" % nombre):
            return a.split("=", 1)[1]
    return None


def hora():
    return time.strftime("%H:%M:%S")


def ore():
    for d in ("release", "debug"):
        for n in ("ore.exe", "ore"):
            p = os.path.join(RAIZ, "target", d, n)
            if os.path.isfile(p):
                return p
    sys.exit("no hay binario de `ore`: cargo build --release -p ore-cli")


# ── la forja del inquilino, por un túnel, con su testigo en memoria ─────────
def testigo_de_la_forja():
    r = subprocess.run(
        [GCLOUD, "secrets", "versions", "access", "latest",
         "--secret=%s-forja-token" % NS, "--project=%s" % PROYECTO],
        capture_output=True, text=True, encoding="utf-8")
    t = r.stdout.strip()
    if r.returncode != 0 or not t:
        sys.exit("✗ no se pudo leer `%s-forja-token` de Secret Manager (sesión de gcloud)" % NS)
    return t


def puerto_libre():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Forja:
    def __init__(self):
        self.puerto = puerto_libre()
        env = dict(os.environ, MSYS_NO_PATHCONV="1")
        self.tunel = subprocess.Popen(
            ["kubectl", "port-forward", "-n", NS, "svc/forja", "%d:3000" % self.puerto],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env)
        for _ in range(60):
            try:
                socket.create_connection(("127.0.0.1", self.puerto), timeout=1).close()
                break
            except OSError:
                time.sleep(0.5)
        else:
            sys.exit("✗ el túnel a la forja de %s no abrió" % NS)
        # ⛔ El testigo por GIT_CONFIG_* y no en la URL: una URL con testigo la
        #   lee cualquier proceso de la máquina (misma disciplina que el Job).
        self.env = dict(os.environ, GIT_TERMINAL_PROMPT="0", GIT_CONFIG_COUNT="1",
                        GIT_CONFIG_KEY_0="http.extraheader",
                        GIT_CONFIG_VALUE_0="Authorization: token " + testigo_de_la_forja(),
                        GIT_AUTHOR_NAME="prueba-sql", GIT_AUTHOR_EMAIL="prueba-sql@invalido",
                        GIT_COMMITTER_NAME="prueba-sql", GIT_COMMITTER_EMAIL="prueba-sql@invalido")

    def url(self, repo):
        return "http://127.0.0.1:%d/%s/%s.git" % (self.puerto, NS, repo)

    def git(self, *args, cwd=None):
        r = subprocess.run(["git", *args], cwd=cwd, env=self.env, capture_output=True,
                           text=True, encoding="utf-8")
        if r.returncode != 0:
            sys.exit("✗ git %s: %s" % (args[0], (r.stderr or r.stdout).strip()[-400:]))
        return r.stdout

    def clonar(self, repo):
        d = os.path.join(TRABAJO, repo)
        # Los objetos de git son de sólo lectura: en Windows, rmtree no los quita solo.
        if os.path.exists(d):
            shutil.rmtree(d, onerror=lambda f_, p_, _: (os.chmod(p_, 0o700), f_(p_)))
        os.makedirs(TRABAJO, exist_ok=True)
        self.git("clone", "--quiet", self.url(repo), d)
        return d

    def cerrar(self):
        self.tunel.terminate()


def punteros(arbol):
    """Las copias con puntero hecho: `(qn, estado, filas)`."""
    out = []
    base = os.path.join(arbol, "datasets")
    for r, _, fs in os.walk(base):
        for f in fs:
            if not f.endswith(".json"):
                continue
            try:
                j = json.load(open(os.path.join(r, f), encoding="utf-8"))
            except Exception:
                continue
            if j.get("estado") in ("copiada", "al-dia") and j.get("metadata_location"):
                partes = os.path.relpath(os.path.join(r, f[:-5]), base).replace("\\", "/").split("/")
                qn = ".".join(p for i, p in enumerate(partes) if not (i == 1 and p == "default"))
                out.append((qn, j.get("estado"), j.get("filas")))
    return sorted(out, key=lambda x: (x[2] or 0))


def ruta_del_puntero(arbol, qn):
    p, n = qn.split(".", 1)
    return os.path.join(arbol, "datasets", p, "default", n + ".json")


# ── las fases ────────────────────────────────────────────────────────────────
def mirar(f):
    arbol, cola = f.clonar("ontologia"), f.clonar("trabajo")
    print("  %s el árbol de %s: %s" % (hora(), NS, f.git("log", "-1", "--format=%h %s", cwd=arbol).strip()))
    print("  copias con puntero hecho (candidatas a entrada, de menos a más filas):")
    for qn, estado, filas in punteros(arbol)[:12]:
        print("    %-48s %-8s %s filas" % (qn, estado, filas))
    p = os.path.join(cola, "plantilla-copia.txt")
    t = open(p, encoding="utf-8").read() if os.path.isfile(p) else ""
    trae = "- name: calcular" in t and "--calculado /trabajo/calculo" in t
    print("  la plantilla de la cola %s `calcular` (paso 4c)" % ("TRAE" if trae else "NO trae — falta converger"))
    return trae


def sembrar(f):
    entrada, columna = opcion("entrada") or ENTRADA, opcion("columna") or COLUMNA
    arbol = f.clonar("ontologia")
    if not os.path.isfile(ruta_del_puntero(arbol, entrada)):
        sys.exit("✗ `%s` no tiene puntero en el árbol de %s" % (entrada, NS))
    d = os.path.join(arbol, "packages", PAQUETE)
    os.makedirs(os.path.join(d, "views"), exist_ok=True)
    os.makedirs(os.path.join(d, "datasets"), exist_ok=True)
    open(os.path.join(d, "views", VISTA + ".yaml"), "w", encoding="utf-8", newline="\n").write(
        "apiVersion: oos.dev/v1alpha14\nkind: View\n"
        "metadata: { name: %s, namespace: %s }\n"
        "spec:\n  owner: team:%s\n  dialect: duckdb\n  sql: |\n"
        "    SELECT %s, count(*) AS n FROM %s GROUP BY %s\n"
        "  columns:\n    %s: { type: String }\n    n: { type: Integer }\n"
        % (VISTA, PAQUETE, INQUILINO, columna, entrada, columna, columna))
    open(os.path.join(d, "datasets", COPIA + ".yaml"), "w", encoding="utf-8", newline="\n").write(
        "apiVersion: oos.dev/v1alpha12\nkind: Dataset\n"
        "metadata: { name: %s, namespace: %s }\n"
        "spec:\n  owner: team:%s\n  from: { view: %s.%s }\n" % (COPIA, PAQUETE, INQUILINO, PAQUETE, VISTA))
    r = subprocess.run([ore(), "validate", "."], cwd=arbol, capture_output=True, text=True, encoding="utf-8")
    mio = [l for l in (r.stdout + r.stderr).splitlines() if PAQUETE in l]
    if mio:
        print("\n".join("    " + l for l in (r.stdout + r.stderr).splitlines()[-30:]))
        sys.exit("✗ `%s` no compila con el `ore` de este commit" % PAQUETE)
    v = subprocess.run([ore(), "view", "."], cwd=arbol, capture_output=True, text=True, encoding="utf-8")
    bloque = v.stdout.split(QN_COPIA + "\n", 1)[-1].split("\n\n")[0]
    print("  ore view de %s:\n%s" % (QN_COPIA, "\n".join("    " + l for l in bloque.splitlines()[:6])))
    f.git("add", "-A", "packages/" + PAQUETE, cwd=arbol)
    f.git("commit", "-q", "-m", "prueba 0040 4c: una vista SQL y su copia (%s)" % QN_COPIA, cwd=arbol)
    f.git("push", "-q", "origin", "HEAD:main", cwd=arbol)
    print("  %s sembrado y empujado · %s" % (hora(), f.git("log", "-1", "--format=%h", cwd=arbol).strip()))


def paquete(f):
    arbol = f.clonar("ontologia")
    d = os.path.join(arbol, "packages", PAQUETE)
    os.makedirs(d, exist_ok=True)
    open(os.path.join(d, "package.yaml"), "w", encoding="utf-8", newline="\n").write(
        "apiVersion: oos.dev/v1alpha1\nkind: Package\n"
        "metadata: { name: %s, version: 0.1.0, status: active, domain: %s }\n"
        "spec: { owner: team:%s }\n" % (PAQUETE, PAQUETE, INQUILINO))
    f.git("add", "-A", "packages/" + PAQUETE, cwd=arbol)
    if not f.git("status", "--porcelain", cwd=arbol).strip():
        print("  el paquete ya estaba")
        return
    f.git("commit", "-q", "-m", "prueba 0040 4c: el paquete %s" % PAQUETE, cwd=arbol)
    f.git("push", "-q", "origin", "HEAD:main", cwd=arbol)
    print("  %s el paquete %s, empujado" % (hora(), PAQUETE))


# Lo que una celda haría: `write()` con el testigo del agente (como `agente.py`).
ESCRIBIR_PY = """
import json, os, sys, urllib.parse, urllib.request
sys.path.insert(0, "/opt/ore")
import ore, pyarrow as pa
d = os.environ["DIRECCION"].rstrip("/")
cli = open("/puesto/agente-cliente").read().strip(); sec = open("/puesto/agente-secreto").read().strip()
datos = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": cli, "client_secret": sec}).encode()
with urllib.request.urlopen(d + "/realms/rubix/protocol/openid-connect/token", data=datos, timeout=20) as r:
    ore.puesto._cabeceras = {"authorization": "Bearer " + json.load(r)["access_token"]}
ore.puesto.id = ""
t = pa.table({"letra": pa.array(list("abacab"), pa.string()), "n": pa.array(range(1, 7), pa.int64())})
e = ore.write(os.environ["ENTRADA"], t)
print("### " + json.dumps({"filas": e["filas"], "metadata_location": e["metadata_location"]}), flush=True)
"""


def escribir(f):
    # Las imágenes del commit que corre en el inquilino, no las del HEAD de aquí:
    # un commit local sin empujar no tiene imagen (ImagePullBackOff, medido).
    sha = kubectl("get", "deployment", "ore-serve", "-o",
                  r"jsonpath={.spec.template.metadata.annotations.ore\.dev/commit}").strip()[:12]
    if not sha:
        sys.exit("✗ no sé qué commit corre ore-serve en %s" % NS)
    nombre = "prueba-sql-escribir-" + sha[:8]
    cm = {"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": nombre, "namespace": NS},
          "data": {"escribir.py": ESCRIBIR_PY}}
    job = {
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": nombre, "namespace": NS, "labels": {"kueue.x-k8s.io/queue-name": "cola", "ore.dev/tenant": INQUILINO, "ore.dev/rol": "puesto"}},
        "spec": {"backoffLimit": 0, "ttlSecondsAfterFinished": 1800, "activeDeadlineSeconds": 1200, "template": {
            "metadata": {"labels": {"ore.dev/rol": "puesto", "ore.dev/tenant": INQUILINO}},
            "spec": {
                "restartPolicy": "Never", "serviceAccountName": "puesto",
                "volumes": [{"name": "puesto", "emptyDir": {"medium": "Memory"}}, {"name": "trabajo", "emptyDir": {}}, {"name": "guiones", "configMap": {"name": nombre}}],
                "initContainers": [{
                    "name": "traer-el-testigo", "image": REGISTRO + "/ore-drivers:main",
                    "env": [{"name": "HOME", "value": "/tmp"}, {"name": "CLOUDSDK_CONFIG", "value": "/tmp/.gcloud"}],
                    "volumeMounts": [{"name": "puesto", "mountPath": "/puesto"}],
                    "command": ["/bin/sh", "-c"],
                    "args": ["set -e\nfor p in cliente secreto; do gcloud secrets versions access latest --secret=%s-agente-$p --out-file=/puesto/agente-$p; chmod 0444 /puesto/agente-$p; done\necho testigo puesto" % NS],
                    "resources": {"requests": {"cpu": "50m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                    "securityContext": {"allowPrivilegeEscalation": False, "capabilities": {"drop": ["ALL"]}},
                }],
                "containers": [{
                    "name": "python", "image": "%s/puesto-python:%s" % (REGISTRO, sha), "imagePullPolicy": "Always",
                    "env": [{"name": "HOME", "value": "/tmp"},
                            {"name": "ORE_SERVE", "value": "http://ore-serve.%s.svc.cluster.local:8080" % NS},
                            {"name": "DIRECCION", "value": "http://idp-service.identidad.svc.cluster.local:8080"},
                            {"name": "ENTRADA", "value": ENTRADA}],
                    "volumeMounts": [{"name": "puesto", "mountPath": "/puesto", "readOnly": True}, {"name": "guiones", "mountPath": "/guiones", "readOnly": True}, {"name": "trabajo", "mountPath": "/trabajo"}],
                    "workingDir": "/trabajo", "command": ["python3", "/guiones/escribir.py"],
                    "resources": {"requests": {"cpu": "500m", "memory": "1Gi"}, "limits": {"cpu": "2", "memory": "2Gi"}},
                    "securityContext": {"allowPrivilegeEscalation": False, "runAsNonRoot": True, "runAsUser": 65532, "seccompProfile": {"type": "RuntimeDefault"}, "capabilities": {"drop": ["ALL"]}},
                }],
            }}},
    }
    kubectl("delete", "job", nombre, "--ignore-not-found")
    kubectl("delete", "configmap", nombre, "--ignore-not-found")
    env = dict(os.environ, MSYS_NO_PATHCONV="1")
    r = subprocess.run(["kubectl", "-n", NS, "apply", "-f", "-"], input=json.dumps(cm) + "\n---\n" + json.dumps(job),
                       capture_output=True, text=True, encoding="utf-8", env=env)
    if r.returncode != 0:
        sys.exit("✗ " + r.stderr.strip())
    print("  %s Job %s (puesto-python:%s) en %s" % (hora(), nombre, sha, NS))
    ok = seguir(nombre, ("python",), plazo=1200)
    kubectl("delete", "configmap", nombre, "--ignore-not-found")
    return ok


def rendir_rehacer(plantilla, vistas, instante):
    """`cola::rendir_copia` + `cola::rendir_rehacer`, letra por letra."""
    def ocho(t):
        return hashlib.sha256(t.encode("utf-8")).hexdigest()[:8]
    assert "copiar-00000000" in plantilla and 'name: REHACER, value: ""' in plantilla, "no es la plantilla"
    t = plantilla.replace('value: "olist.customers"', 'value: "%s"' % ",".join(vistas))
    t = t.replace("copiar-00000000", "copiar-%s" % ocho(t))
    t = t.replace('name: REHACER, value: ""', 'name: REHACER, value: "%s"' % instante)
    h = ocho(t)
    lineas = []
    for l in t.split("\n")[:-1] if t.endswith("\n") else t.split("\n"):
        lineas.append("  name: copiar-rehacer-%s" % h if l.startswith("  name: copiar-") else l)
    return "48-la-copia-rehacer-%s.yaml" % h, "\n".join(lineas) + "\n", "copiar-rehacer-%s" % h


def copiar(f):
    cola = f.clonar("trabajo")
    plantilla = open(os.path.join(cola, "plantilla-copia.txt"), encoding="utf-8").read()
    if "- name: calcular" not in plantilla:
        sys.exit("✗ la plantilla de la cola no trae `calcular`: falta converger %s" % INQUILINO)
    instante = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    fichero, texto, job = rendir_rehacer(plantilla, [QN_COPIA], instante)
    open(os.path.join(cola, fichero), "w", encoding="utf-8", newline="\n").write(texto)
    f.git("add", fichero, cwd=cola)
    f.git("commit", "-q", "-m", "Rehacer la copia de %s (%s) · prueba 0040 4c" % (QN_COPIA, instante), cwd=cola)
    f.git("push", "-q", "origin", "HEAD:main", cwd=cola)
    open(os.path.join(TRABAJO, "job.txt"), "w").write("%s\n%s\n" % (job, fichero))
    print("  %s encolado `%s` → Job %s" % (hora(), fichero, job))


def kubectl(*args):
    env = dict(os.environ, MSYS_NO_PATHCONV="1")
    r = subprocess.run(["kubectl", "-n", NS, *args], capture_output=True, text=True, encoding="utf-8", env=env)
    return r.stdout


def esperar():
    job = open(os.path.join(TRABAJO, "job.txt")).read().split()[0]
    return seguir(job, ("preparar", "calcular", "copiar"))


def seguir(job, contenedores, plazo=1800):
    print("  %s esperando %s (Kueue → jobs-p; el nodo tarda ~2 min en frío)" % (hora(), job))
    fin, antes = time.time() + plazo, None
    while time.time() < fin:
        try:
            st = json.loads(kubectl("get", "job", job, "-o", "json") or "{}").get("status", {})
        except ValueError:
            st = {}
        estado = ("hecho" if st.get("succeeded") else "fallido" if st.get("failed")
                  else "corriendo" if st.get("active") else "sin crear" if not st else "pendiente")
        if estado != antes:
            print("  %s %s · %s" % (hora(), job, estado))
            antes = estado
        if estado in ("hecho", "fallido"):
            for c in contenedores:
                print("  ── %s ──" % c)
                print("\n".join("    " + l for l in kubectl("logs", "job/" + job, "-c", c, "--tail=40").splitlines()))
            return estado == "hecho"
        time.sleep(20)
    print("  ✗ %s no terminó en %d s" % (job, plazo))
    return False


def ver(f):
    arbol = f.clonar("ontologia")
    p = ruta_del_puntero(arbol, QN_COPIA)
    if not os.path.isfile(p):
        sys.exit("✗ no hay puntero de %s en el árbol" % QN_COPIA)
    j = json.load(open(p, encoding="utf-8"))
    entrada = opcion("entrada") or ENTRADA
    print("  puntero de %s:" % QN_COPIA)
    for k in ("estado", "operacion", "filas", "leidas", "plan", "testigo", "metadata_location", "motivo"):
        if k in j:
            print("    %-18s %s" % (k, json.dumps(j[k], ensure_ascii=False)))
    fallos = []
    if j.get("estado") not in ("copiada", "al-dia"):
        fallos.append("no está copiada")
    if entrada:
        e = json.load(open(ruta_del_puntero(arbol, entrada), encoding="utf-8"))
        if j.get("leidas") != e.get("filas"):
            fallos.append("leidas %s ≠ las %s filas de %s" % (j.get("leidas"), e.get("filas"), entrada))
        if (j.get("testigo") or {}).get("valor") != "%s@%s" % (entrada, e.get("snapshot")):
            fallos.append("el testigo no es el snapshot de %s" % entrada)
    if not str(j.get("metadata_location", "")).startswith("gs://"):
        fallos.append("la copia no vive en el bucket del inquilino")
    autor = f.git("log", "-1", "--format=%an", "--", os.path.relpath(p, arbol), cwd=arbol).strip()
    print("    %-18s %s" % ("empujado por", autor))
    if autor != "copiador":
        fallos.append("el puntero no lo empujó la pasada (`copiador`), sino `%s`" % autor)
    print("  ✓ la copia de la vista SQL en %s" % NS if not fallos else "  ✗ " + " · ".join(fallos))
    return not fallos


def limpiar(f):
    arbol = f.clonar("ontologia")
    quitar = ["packages/" + PAQUETE]
    for qn in (QN_COPIA, ENTRADA):
        p = ruta_del_puntero(arbol, qn)
        if os.path.isfile(p):
            quitar.append(os.path.relpath(p, arbol).replace("\\", "/"))
    hay = [q for q in quitar if os.path.exists(os.path.join(arbol, q))]
    if hay:
        f.git("rm", "-r", "-q", *hay, cwd=arbol)
        f.git("commit", "-q", "-m", "prueba 0040 4c: se retira %s" % PAQUETE, cwd=arbol)
        f.git("push", "-q", "origin", "HEAD:main", cwd=arbol)
        print("  %s retirado del árbol: %s" % (hora(), ", ".join(hay)))
    j = os.path.join(TRABAJO, "job.txt")
    if os.path.isfile(j):
        fichero = open(j).read().split()[1]
        cola = f.clonar("trabajo")
        if os.path.isfile(os.path.join(cola, fichero)):
            f.git("rm", "-q", fichero, cwd=cola)
            f.git("commit", "-q", "-m", "prueba 0040 4c: se retira %s" % fichero, cwd=cola)
            f.git("push", "-q", "origin", "HEAD:main", cwd=cola)
            print("  %s retirado de la cola: %s (Flux se lleva el Job)" % (hora(), fichero))
    print("  los objetos de la copia en el bucket los recoge la pasada siguiente (--recoger)")


def main():
    f = Forja()
    try:
        if FASE == "mirar":
            mirar(f)
        elif FASE == "paquete":
            paquete(f)
        elif FASE == "escribir":
            sys.exit(0 if escribir(f) else 1)
        elif FASE == "sembrar":
            sembrar(f)
        elif FASE == "copiar":
            copiar(f)
        elif FASE == "esperar":
            sys.exit(0 if esperar() else 1)
        elif FASE == "ver":
            sys.exit(0 if ver(f) else 1)
        elif FASE == "limpiar":
            limpiar(f)
        else:
            sys.exit("fase desconocida `%s`: mirar, paquete, escribir, sembrar, copiar, esperar, ver, limpiar" % FASE)
    finally:
        f.cerrar()


if __name__ == "__main__":
    main()
