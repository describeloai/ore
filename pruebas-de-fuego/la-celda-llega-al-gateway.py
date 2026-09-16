# -*- coding: utf-8 -*-
"""ACEPTACIÓN · la celda llega al gateway sin que nadie toque nada (0027 E2).

Lo que la ADR acepta, medido de punta a punta en una celda real y SIN `kubectl`
para nada que sea de la plataforma —ni reglas, ni suscripciones, ni realm—:

    ①  `POST /modelos` desde `curl`, por la puerta pública de la celda, con el
       token de agente → 201: el documento en el árbol y la suscripción en el
       gateway, en el mismo acto (E1 I3)
    ②  la primera llamada de un Job de la celda contesta en < 60 s: el Job
       (`driver`, por la cola) sale por la regla de clase que la plantilla ya
       trae (E2 I1), con su token de agente (`aud modelos`, `rubix_celda`: E2 I3)
    ③  `DELETE /modelos/{n}` → el mismo Job recibe 401 en la siguiente llamada
    ④  la celda de al lado, sin `Model`, → 401 (un Job de `prueba` con SU token)
    ⑤  `--cotejar` de 0026 limpio: la regla no añade pods, el modelo no está en la celda

`kubectl` sólo crea los dos Jobs de medida (lo que la cola haría) y lee sus logs.

    uso:  PYTHONIOENCODING=utf-8 python pruebas-de-fuego/la-celda-llega-al-gateway.py [celda] [vecina]
"""
import json
import os
import subprocess
import sys
import time

CELDA = next((a for a in sys.argv[1:] if not a.startswith("--")), "victor")
VECINA = next((a for a in sys.argv[2:] if not a.startswith("--")), "prueba")
NS = "t-" + CELDA
PROYECTO = "project-8853a180-450d-47be-b83"
REGISTRO = "europe-west1-docker.pkg.dev/%s/ore" % PROYECTO
IDP = "https://login.paladio.io/realms/rubix"
MODELOS = "10.10.0.100"
MODELO = {"name": "v2-lite", "profile": "g1/deepseek-v2-lite", "description": "DeepSeek-V2-Lite en un g1 (0027 E2, la aceptacion)"}

E2_SH = r'''#!/bin/sh
set -e
URL="http://$MODELOS:8000"
DICE=$(curl -sSf "$DIRECCION/realms/$REALM/.well-known/openid-configuration" | python3 -c 'import json,sys;print(json.load(sys.stdin)["issuer"])')
[ "$DICE" = "$EMISOR/realms/$REALM" ] || { echo "MAL en $DIRECCION contesta $DICE"; exit 1; }
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" --data-urlencode "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
echo "== E2 · celda $CELDA · token de $(cat /puesto/agente-cliente) · aud $(echo "$TOK" | cut -d. -f2 | python3 -c 'import base64,json,sys;t=sys.stdin.read().strip();print(json.loads(base64.urlsafe_b64decode(t+"="*(-len(t)%4))).get("aud"))')"
T0=$(python3 -c 'import time;print(time.time())')
COD=$(curl -s -m 8 -o /tmp/r.json -w '%{http_code}' -H "Authorization: Bearer $TOK" "$URL/v1/models")
echo "(1) GET /v1/models → $COD · $(head -c 120 /tmp/r.json)"
if [ "$FASE" = "vecina" ]; then
  [ "$COD" = "401" ] && echo "(4) OK la celda de al lado, sin Model: 401" || { echo "(4) MAL esperaba 401 y salio $COD"; exit 1; }
  exit 0
fi
n=0
while [ "$COD" != "200" ] && [ $n -lt 60 ]; do sleep 2; n=$((n+1)); COD=$(curl -s -m 8 -o /tmp/r.json -w '%{http_code}' -H "Authorization: Bearer $TOK" "$URL/v1/models"); done
T1=$(python3 -c 'import time;print(time.time())')
[ "$COD" = "200" ] || { echo "(2) MAL la celda no ve su modelo (http $COD)"; exit 1; }
echo "(2) OK suscrita: $(python3 -c 'import json;print([m["id"] for m in json.load(open("/tmp/r.json"))["data"]])') · $(python3 -c "print(round($T1-$T0,1))") s desde que el Job empezo a mirar"
CUERPO=$(python3 -c "import json;print(json.dumps({'model':'$MODELO_ID','messages':[{'role':'user','content':'Di hola en una frase corta.'}],'max_tokens':16,'stream':True}))")
echo "(2) primera llamada: http ttft_s total_s = $(curl -s -m 60 -o /dev/null -w '%{http_code} %{time_starttransfer} %{time_total}' -H "Authorization: Bearer $TOK" -H 'content-type: application/json' -d "$CUERPO" "$URL/v1/chat/completions")"
echo "(3) esperando el DELETE..."
n=0
while [ "$COD" != "401" ] && [ $n -lt 90 ]; do sleep 2; n=$((n+1)); COD=$(curl -s -m 8 -o /tmp/r.json -w '%{http_code}' -H "Authorization: Bearer $TOK" "$URL/v1/models"); done
T2=$(python3 -c 'import time;print(time.time())')
[ "$COD" = "401" ] && echo "(3) OK tras el DELETE la celda recibe 401: $(head -c 100 /tmp/r.json)" || { echo "(3) MAL esperaba 401 y sigue $COD"; exit 1; }
echo "== fin"
'''


ARBOL_SH = r"""#!/bin/sh
set -e
export GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=http.extraheader GIT_CONFIG_KEY_1=safe.directory GIT_CONFIG_VALUE_1='*' GIT_TERMINAL_PROMPT=0
export GIT_CONFIG_VALUE_0="Authorization: token $(cat /puesto/forja)"
cd /tmp && git clone --quiet "http://forja.t-$CELDA.svc.cluster.local:3000/t-$CELDA/ontologia.git" arbol && cd arbol
case "$FASE" in
  retirar)
    if [ -f functions/segmentar.yaml ] && ! grep -q "model: modelo/" functions/segmentar.yaml; then
      git rm -q functions/segmentar.yaml
      git -c user.name=e2 -c user.email=e2@invalido commit -q -m "E2 (0027) · la Function de E0 se retira: era la forma de v1alpha8 (entrypoint + x-ore-prompt) y v1alpha9 ya tiene la suya, que llega con el Model"
      git push -q origin HEAD:main; echo "(arbol) retirada la Function de E0 · $(git rev-parse --short HEAD)"
    else echo "(arbol) nada que retirar · $(git rev-parse --short HEAD)"; fi ;;
  funcion)
    [ -f modelos/v2-lite.yaml ] || { echo "(arbol) MAL no hay modelos/v2-lite.yaml: POST /modelos no lo escribio"; exit 1; }
    printf '%s\n' 'apiVersion: oos.dev/v1alpha9' 'kind: Function' 'metadata: { name: segmentar, namespace: ventas }' 'spec:' '  runtime: model' '  model: modelo/v2-lite' '  prompt: "Clasifica la actividad de este cliente en un segmento de una palabra: pyme, corporativo, publico o particular. Contesta solo con la palabra."' '  effects:' '    - writes: ventas.Cliente.segmento' > functions/segmentar.yaml
    ore validate . | sed 's/^/    /'
    git add functions/segmentar.yaml
    git -c user.name=e2 -c user.email=e2@invalido commit -q -m "E2 (0027) · la Function de E1: runtime: model, model: modelo/v2-lite, prompt — nombra el Model que POST /modelos escribio"
    git push -q origin HEAD:main; echo "(arbol) la Function de E1 nombra modelo/v2-lite · $(git rev-parse --short HEAD)" ;;
esac
"""


def job(nombre, ns, celda, fase):
    return {
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": nombre, "namespace": ns, "labels": {"kueue.x-k8s.io/queue-name": "cola", "ore.dev/tenant": celda, "ore.dev/rol": "driver", "ore.dev/e2": fase}},
        "spec": {"backoffLimit": 0, "ttlSecondsAfterFinished": 3600, "activeDeadlineSeconds": 900,
                 "template": {"metadata": {"labels": {"ore.dev/rol": "driver", "ore.dev/tenant": celda, "ore.dev/e2": fase}},
                              "spec": {"restartPolicy": "Never", "serviceAccountName": "driver",
                                       "volumes": [{"name": "puesto", "emptyDir": {"medium": "Memory"}},
                                                   {"name": "e2", "configMap": {"name": "e2", "defaultMode": 0o555}}],
                                       "initContainers": [{"name": "traer-el-agente", "image": REGISTRO + "/ore-drivers:main",
                                                           "env": [{"name": "HOME", "value": "/tmp"}, {"name": "CLOUDSDK_CONFIG", "value": "/tmp/.gcloud"}],
                                                           "volumeMounts": [{"name": "puesto", "mountPath": "/puesto"}],
                                                           "command": ["/bin/sh", "-c"],
                                                           "args": ["set -e\nfor p in cliente secreto; do gcloud secrets versions access latest --secret=%s-agente-$p --out-file=/puesto/agente-$p; done\ngcloud secrets versions access latest --secret=%s-forja-token --out-file=/puesto/forja\nchmod 0400 /puesto/*\necho \"agente y testigo puestos · $(cat /puesto/agente-cliente)\"\n" % (ns, ns)],
                                                           "resources": {"requests": {"cpu": "50m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                                                           "securityContext": {"allowPrivilegeEscalation": False, "capabilities": {"drop": ["ALL"]}}}],
                                       "containers": [{"name": "e2", "image": REGISTRO + "/ore-drivers:main", "imagePullPolicy": "Always",
                                                       "env": [{"name": k, "value": v} for k, v in [
                                                           ("HOME", "/tmp"), ("CELDA", celda), ("FASE", fase), ("MODELOS", MODELOS), ("MODELO_ID", "deepseek-ai/DeepSeek-V2-Lite"),
                                                           ("EMISOR", "https://login.paladio.io"), ("DIRECCION", "http://idp-service.identidad.svc.cluster.local:8080"), ("REALM", "rubix")]],
                                                       "volumeMounts": [{"name": "puesto", "mountPath": "/puesto", "readOnly": True}, {"name": "e2", "mountPath": "/e2", "readOnly": True}],
                                                       "command": ["/bin/sh", "/e2/arbol.sh" if fase in ("retirar", "funcion") else "/e2/e2.sh"],
                                                       "resources": {"requests": {"cpu": "100m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                                                       "securityContext": {"allowPrivilegeEscalation": False, "capabilities": {"drop": ["ALL"]}}}]}}},
    }


def sh(cmd):
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, encoding="utf-8")
    return r.stdout.strip() if r.returncode == 0 else ""


def kubectl(*args, entrada=None):
    r = subprocess.run(["kubectl", *args], input=entrada, capture_output=True, text=True, encoding="utf-8", env={**os.environ, "MSYS_NO_PATHCONV": "1"})
    return r.stdout


def token_del_agente(celda):
    ns = "t-" + celda
    cli = sh("gcloud secrets versions access latest --secret=%s-agente-cliente --project=%s" % (ns, PROYECTO))
    sec = sh("gcloud secrets versions access latest --secret=%s-agente-secreto --project=%s" % (ns, PROYECTO))
    r = subprocess.run(["curl", "-s", "-m", "10", "-X", "POST", IDP + "/protocol/openid-connect/token", "-d", "grant_type=client_credentials",
                        "-d", "client_id=" + cli, "--data-urlencode", "client_secret=" + sec], capture_output=True, text=True, encoding="utf-8")
    return json.loads(r.stdout)["access_token"]


def serve(metodo, camino, tok, cuerpo=None):
    args = ["curl", "-s", "-m", "60", "-o", "-", "-w", "\n%{http_code} %{time_total}", "-X", metodo, "https://%s.ore.paladio.io%s" % (CELDA, camino), "-H", "authorization: Bearer " + tok]
    if cuerpo is not None:
        args += ["-H", "content-type: application/json", "-d", json.dumps(cuerpo)]
    r = subprocess.run(args, capture_output=True, text=True, encoding="utf-8")
    cuerpo, _, cod = (r.stdout or "").rpartition("\n")
    return cod.split()[0] if cod else "000", (cod.split()[1] if len(cod.split()) > 1 else "?"), cuerpo.strip()


def log_de(nombre, ns):
    return kubectl("logs", "-n", ns, "job/" + nombre, "-c", "e2") or ""


def terminado(nombre, ns):
    j = kubectl("get", "job", nombre, "-n", ns, "-o", "json")
    st = json.loads(j)["status"] if j else {}
    return bool(st.get("succeeded")), bool(st.get("failed"))


filas = []


def fila(n, que, res):
    filas.append((n, que, res))
    print("  %s  %-52s %s" % (n, que, res))


print("\n  ═══ E2 · LA CELDA LLEGA AL GATEWAY SIN QUE NADIE TOQUE NADA · %s (vecina %s) ═══\n" % (CELDA, VECINA))
tok = token_del_agente(CELDA)
# partida limpia: sin Model en el árbol (si quedó uno, se retira por el verbo)
cod, _, _ = serve("DELETE", "/modelos/" + MODELO["name"], tok)
print("  ⓪ DELETE previo de `%s` → %s (partida limpia) · reglas en la celda: %s" % (
    MODELO["name"], cod, " ".join(l.split()[0] for l in kubectl("get", "netpol", "-n", NS).splitlines() if "modelo" in l)))
for n, ns in (("e2-celda", NS), ("e2-vecina", "t-" + VECINA), ("e2-arbol-retirar", NS), ("e2-arbol-funcion", NS)):
    kubectl("delete", "job", n, "-n", ns, "--ignore-not-found", "--wait=true")
    kubectl("apply", "-f", "-", entrada=json.dumps({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "e2", "namespace": ns}, "data": {"e2.sh": E2_SH, "arbol.sh": ARBOL_SH}}))

# ── ⓪ el árbol: la Function de E0 (forma de v1alpha8) se retira; la de E1 llega con el Model ──
def job_arbol(fase):
    n = "e2-arbol-" + fase
    kubectl("apply", "-f", "-", entrada=json.dumps(job(n, NS, CELDA, fase)))
    t0 = time.time()
    while time.time() - t0 < 900:
        ok, ko = terminado(n, NS)
        if ok or ko:
            break
        time.sleep(5)
    salida = log_de(n, NS)
    for l in salida.splitlines():
        print("     │ " + l)
    return next((l for l in salida.splitlines() if l.startswith("(arbol)")), "✗ " + salida[-120:])


fila("⓪", "el árbol de la celda, antes", job_arbol("retirar")[8:])

# ── ① POST /modelos ──────────────────────────────────────────────────────────
t_post = time.time()
cod, seg, cuerpo = serve("POST", "/modelos", tok, MODELO)
fila("①", "POST /modelos por la puerta pública, con el token de agente", "%s en %s s · %s" % (cod, seg, cuerpo[:150]))
if cod != "201":
    print("  ✗ sin 201 no hay E2. Lo que dijo: %s" % cuerpo)
    sys.exit(1)
cod, _, ficha = serve("GET", "/modelos/" + MODELO["name"], tok)
fila("①", "GET /modelos/v2-lite resuelve", "%s · %s" % (cod, ficha[:150]))

# ── ② el Job de la celda, y ③ el DELETE mientras mira ───────────────────────
kubectl("apply", "-f", "-", entrada=json.dumps(job("e2-celda", NS, CELDA, "celda")))
kubectl("apply", "-f", "-", entrada=json.dumps(job("e2-vecina", "t-" + VECINA, VECINA, "vecina")))
borrado = None
t0 = time.time()
while time.time() - t0 < 900:
    time.sleep(3)
    log = log_de("e2-celda", NS)
    if borrado is None and "(3) esperando el DELETE" in log:
        cod, seg, cuerpo = serve("DELETE", "/modelos/" + MODELO["name"], tok)
        borrado = (cod, seg, cuerpo)
        print("     DELETE /modelos/%s → %s en %s s" % (MODELO["name"], cod, seg))
    ok, ko = terminado("e2-celda", NS)
    if ok or ko:
        break
log = log_de("e2-celda", NS)
for l in log.splitlines():
    print("     │ " + l)
lineas = log.splitlines()
l2 = next((l for l in lineas if l.startswith("(2) OK")), "")
l2b = next((l for l in lineas if l.startswith("(2) primera")), "")
l3 = next((l for l in lineas if l.startswith("(3) OK")), "")
fila("②", "el Job de la celda ve su modelo, sin que nadie toque nada", ("a la primera llamada (%s)" % l2.split("·")[-1].strip()) if l2 else "✗")
fila("②", "primera llamada · http ttft_s total_s", l2b.split("= ")[-1] if l2b else "✗")
fila("③", "DELETE /modelos → el Job recibe 401", ("%s → %s" % (borrado[0], l3[7:80])) if (borrado and l3) else "✗ %s" % (borrado,))
# la vecina
t0 = time.time()
while time.time() - t0 < 600:
    ok, ko = terminado("e2-vecina", "t-" + VECINA)
    if ok or ko:
        break
    time.sleep(5)
lv = log_de("e2-vecina", "t-" + VECINA)
for l in lv.splitlines():
    print("     │ " + l)
l4 = next((l for l in lv.splitlines() if l.startswith("(4)")), "")
fila("④", "la celda de al lado (%s), sin Model, con SU token" % VECINA, l4[4:] if l4 else "✗")
# --cotejar
r = subprocess.run([sys.executable, os.path.join(os.path.dirname(__file__), "medida-el-estado-de-la-celda.py"), CELDA, "--cotejar"], capture_output=True, text=True, encoding="utf-8", env={**os.environ, "PYTHONIOENCODING": "utf-8"})
ult = [l for l in (r.stdout or "").splitlines() if l.strip()]
fila("⑤", "--cotejar de 0026 con la regla en la plantilla", ult[-1].strip() if ult else "✗")
cod, _, lista = serve("GET", "/modelos", tok)
fila("⑤", "GET /modelos después del DELETE", "%s · %s" % (cod, lista[:80]))
# ── ⑥ el estado final: el Model otra vez, la Function de E1 que lo nombra, y el DELETE que se niega ──
cod, seg, cuerpo = serve("POST", "/modelos", tok, MODELO)
fila("⑥", "POST /modelos otra vez (el estado final de la celda)", "%s en %s s" % (cod, seg))
fila("⑥", "la Function de E1 en el árbol (runtime: model, model: modelo/v2-lite)", job_arbol("funcion")[8:])
cod, seg, cuerpo = serve("DELETE", "/modelos/" + MODELO["name"], tok)
fila("⑥", "DELETE /modelos/v2-lite con la Function nombrándolo", "%s · %s" % (cod, cuerpo[:110]))
cod, _, lista = serve("GET", "/modelos", tok)
fila("⑥", "GET /modelos, final", "%s · %s" % (cod, lista[:120]))
for n, ns in (("e2-celda", NS), ("e2-vecina", "t-" + VECINA)):
    kubectl("delete", "configmap", "e2", "-n", ns, "--ignore-not-found")
print("\n  ═══ la tabla ═══")
for n, q, res in filas:
    print("  %s  %-52s %s" % (n, q, res))
print()
