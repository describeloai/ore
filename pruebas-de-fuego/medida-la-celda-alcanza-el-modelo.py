# -*- coding: utf-8 -*-
"""MEDIDA · la celda alcanza el modelo (0027 E0).

Antes de escribir `kind: Model` (E1): medir desde DENTRO de una celda lo que el
ADR sólo afirma. Un Job de la celda —el mismo que corre un catálogo: `driver`,
por la cola— contra el gateway de Bastion (B3) en una máquina de la misma VPC,
que es la figura de 0027 ④ («ninguna celda alcanza una máquina de modelo
directamente») y de 0024 ② («misma VPC, otra máquina»):

    (a)  con la NetworkPolicy de hoy el Job NO alcanza :8000 — el timeout, con
         código —; con la regla de clase hacia el modelo (MODELOS/32:8000, la
         misma figura que MAESTRO en 47) sí
    (b)  latencia y TTFT vistos desde la celda, 1 y 4 llamadas concurrentes
    (c)  el mismo curl con el token de agente de la celda: 401 mientras la celda
         no está suscrita, 200 cuando `ore-serve` (aquí: esta medida, por el
         plano de control del gateway) la suscribe — y cuántos segundos tarda
         en verse desde la celda
    (d)  una Function del árbol que nombra `modelo/v2-lite` y cuya salida
         aterriza en la ontología: el Job la ejecuta a mano (F4 no existe),
         hace la Propuesta, `ore verify` la coteja y la empuja al árbol con el
         commit que la trajo
    (e)  `--cotejar` de 0026 no ve nada nuevo en la celda: el modelo no está en ella

Hoy el modelo detrás del gateway es UN vLLM DE MENTIRA (`bastion/env/e0`):
contesta `pyme` a un prompt de segmentación, una palabra cada 20 ms. Vale para
(a), (c), (d) y (e) —red, identidad, árbol—; los números de (b) se imprimen y
se llaman «de mentira». El mismo comando, con un g1 registrado en este gateway,
mide (b) de verdad sin tocar nada de lo demás.

    uso:  PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-la-celda-alcanza-el-modelo.py [celda] [--dejar]

    --dejar   no retirar la regla de clase al acabar (E2 la llevará a la plantilla)

Lo que deja: el paquete `ventas` y `functions/segmentar.yaml` en el árbol de la
celda (un commit), y `propuestas/segmentar-C-0001.json` (otro, del modelo). La
suscripción en el gateway se retira antes de medir y se deja puesta al acabar.
"""
import json
import os
import subprocess
import sys
import tempfile
import time

CELDA = next((a for a in sys.argv[1:] if not a.startswith("--")), "victor")
NS = "t-" + CELDA
DEJAR = "--dejar" in sys.argv

PROYECTO = "project-8853a180-450d-47be-b83"
REGISTRO = "europe-west1-docker.pkg.dev/%s/ore" % PROYECTO
IDP = "https://login.paladio.io/realms/rubix"

# ── La puerta a los modelos: una constante nombrada, como MAESTRO en 47 ──────
#
# La IP interna reservada `modelos` (10.10.0.100, subred ore-mesh-europe-west1),
# donde corre el gateway de Bastion: 8000 es el plano de datos (OpenAI /v1),
# 9000 el de control (/admin), que sólo la VPC alcanza. Cambia si se recrea la
# reserva: `gcloud compute addresses describe modelos --region europe-west1`.
MODELOS = "10.10.0.100"
PUERTA = 8000
ADMIN = 9000
MODELO_ID = "deepseek-ai/DeepSeek-V2-Lite"   # lo que el perfil g1/deepseek-v2-lite sirve; `modelo/v2-lite` resuelve a esto
# Qué hay detrás del gateway: por defecto el vLLM de mentira. Con un g1 real registrado,
# `E0_ETIQUETA="g1 real: …"` y las filas (b) y (d) dejan de llamarse «de mentira».
ETIQUETA = os.environ.get("E0_ETIQUETA", "modelo de mentira: los numeros no son de un g1")
DE_MENTIRA = ETIQUETA.startswith("modelo de mentira")

# ── El árbol de E0: un paquete mínimo y la Function que nombra el modelo ─────
#
# Lo que hizo falta para que compile, y es lo que E1 tiene que saber:
#   · `runtime: model` y `entrypoint: modelo/v2-lite` pasan la gramática de hoy
#     (el valor de `runtime` no se comprueba); el prompt va en una extensión
#     `x-ore-prompt` porque `Function.spec` es cerrado y ésa es la puerta que deja
#   · la salida de un modelo sin endoso es `untrusted`: `OOS7002` rechaza que
#     escriba una propiedad que exija `inferred`. La propiedad y el conducto de
#     materialización se declaran `untrusted` a propósito — es lo que un modelo
#     sin revisar ES en el retículo, y el árbol lo dice en vez de callarlo
ARBOL = {
    "packages/ventas/package.yaml": """apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: ventas, version: 0.1.0, status: active, domain: ventas }
spec: { owner: team:ventas }
""",
    "packages/ventas/tables/clientes.yaml": """apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: clientes, namespace: ventas }
spec:
  datasource: erp
  object: 'public.clientes'
  columns:
    cliente_id: { physicalType: 'varchar(16)' }
    nombre: { physicalType: 'varchar(80)' }
    actividad: { physicalType: 'text' }
    segmento: { physicalType: 'varchar(32)' }
  reads: { predicatePushdown: [eq], fullScan: cheap }
  changes: { mode: retract, witness: log, key: [cliente_id] }
""",
    "packages/ventas/views/clientes.yaml": """apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: clientes, namespace: ventas }
spec:
  owner: team:ventas
  from: { table: ventas.clientes }
  fields:
    clienteId: cliente_id
    nombre: nombre
    actividad: actividad
    segmento: segmento
  # con escrituras se materializa (OOS2025): la copia es donde una edición se sostiene
  materialized: { datasource: lago, table: 'cache.ventas_clientes' }
""",
    "packages/ventas/entities/Cliente.yaml": """apiVersion: oos.dev/v1alpha1
kind: Entity
metadata: { name: Cliente, namespace: ventas }
spec:
  nature: entity
  primaryKey: [clienteId]
  backedBy: ventas.clientes
  properties:
    clienteId: { type: String }
    nombre: { type: String }
    actividad: { type: String }
    # lo escribe un modelo sin endoso: el mínimo del retículo, dicho
    segmento:
      type: String
      labels: { acme.assurance: untrusted }
""",
    "lattices/assurance.yaml": """apiVersion: oos.dev/v1alpha2
kind: Lattice
metadata: { name: assurance, namespace: acme }
spec:
  axis: integrity
  levels: [untrusted, inferred, reviewed, attested]
""",
    "conduits.yaml": """apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: ventas }
spec:
  owner: team:security
  conduits:
    # la copia admite lo que un modelo sin revisar produce
    materialization.payload:
      acme.assurance: untrusted
""",
    "functions/segmentar.yaml": """apiVersion: oos.dev/v1alpha8
kind: Function
metadata: { name: segmentar, namespace: ventas }
spec:
  # 0027 ⑤: lo que una Function invoca es `modelo/<nombre>`, no una URL.
  # E0: la gramática admite `runtime: model`; E1 fija la forma en `oos`.
  runtime: model
  entrypoint: modelo/v2-lite
  x-ore-prompt: "Clasifica la actividad de este cliente en un segmento de una palabra: pyme, corporativo, publico o particular. Contesta solo con la palabra."
  effects:
    - writes: ventas.Cliente.segmento
""",
}

# ── El Job: lo que corre DENTRO de la celda ──────────────────────────────────
#
# POSIX sh sobre `ore-drivers:main` (alpine: gcloud, git, curl, python3 con yaml,
# `ore`). Sin heredocs: el árbol llega por un ConfigMap montado en /e0/arbol.
E0_SH = r'''#!/bin/sh
set -e
URL="http://$MODELOS:$PUERTA"
echo "== E0 · fase $FASE · celda $CELDA · gateway $URL · nodo $(cat /etc/hostname 2>/dev/null)"

# ── (a) ¿alcanza un Job de la celda :8000? ───────────────────────────────────
# curl rc 28 = plazo agotado: una NetworkPolicy no rechaza, TIRA EL PAQUETE.
RC=0
COD=$(curl -s -m 8 -o /dev/null -w '%{http_code} %{time_total}' "$URL/v1/models" 2>/dev/null) || RC=$?
echo "(a) GET $URL/v1/models sin token → http/segundos [$COD] · curl rc $RC"
if [ "$FASE" = "sin-regla" ]; then
  if [ "$RC" = "28" ]; then
    echo "(a) OK la celda NO alcanza el modelo: deny-all-egress tira el paquete (rc 28 a los 8 s)"
  else
    echo "(a) MAL esperaba rc 28 (timeout) y salio rc $RC [$COD]"
    exit 1
  fi
  exit 0
fi
if [ "$RC" != "0" ]; then
  echo "(a) MAL con la regla de clase la celda tenia que alcanzar :8000 (rc $RC)"
  exit 1
fi
echo "(a) OK con salida-al-modelo ($MODELOS/32:$PUERTA) la celda alcanza el gateway: http $(echo $COD | cut -d' ' -f1) (401 = la puerta pide identidad)"

# ── el token de agente de la celda, por el camino de los Jobs de catalogo (44) ─
DICE=$(curl -sSf "$DIRECCION/realms/$REALM/.well-known/openid-configuration" | python3 -c 'import json,sys;print(json.load(sys.stdin)["issuer"])')
[ "$DICE" = "$EMISOR/realms/$REALM" ] || { echo "MAL en $DIRECCION contesta $DICE"; exit 1; }
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" --data-urlencode "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
echo "    token de agente acunado: $(cat /puesto/agente-cliente) · $(echo "$TOK" | cut -c1-12)... (${#TOK} bytes)"

# ── (c) el mismo curl con el token: 401 sin suscripcion, 200 con ella ────────
COD=$(curl -s -m 8 -o /tmp/r.json -w '%{http_code}' -H "Authorization: Bearer $TOK" "$URL/v1/models")
echo "(c) GET /v1/models con el token de agente → http $COD · $(head -c 160 /tmp/r.json)"
if [ "$COD" = "401" ]; then
  echo "(c) OK sin Model en el arbol la celda no esta suscrita: 401 (esperando la suscripcion...)"
  T0=$(python3 -c 'import time;print(time.time())')
  n=0
  while [ "$COD" != "200" ] && [ $n -lt 90 ]; do
    sleep 2; n=$((n+1))
    COD=$(curl -s -m 8 -o /tmp/r.json -w '%{http_code}' -H "Authorization: Bearer $TOK" "$URL/v1/models")
  done
  T1=$(python3 -c 'import time;print(time.time())')
  [ "$COD" = "200" ] || { echo "(c) MAL la suscripcion no llego en 180 s (http $COD)"; exit 1; }
  echo "(c) OK suscrita: la celda ve $(python3 -c 'import json;print([m["id"] for m in json.load(open("/tmp/r.json"))["data"]])') · vista desde la celda $(python3 -c "print(round($T1-$T0,1))") s despues de empezar a esperar"
elif [ "$COD" = "200" ]; then
  echo "(c) ~ ya estaba suscrita (la medida no la retiro antes): $(head -c 120 /tmp/r.json)"
else
  echo "(c) MAL http $COD"; exit 1
fi

# ── (b) latencia y TTFT vistos desde la celda: 1 y 4 llamadas concurrentes ───
CUERPO=$(python3 -c "import json;print(json.dumps({'model':'$MODELO_ID','messages':[{'role':'user','content':'Di hola en una frase corta.'}],'max_tokens':16,'stream':True}))")
llamada() {
  curl -s -m 60 -o /dev/null -w '%{http_code} %{time_starttransfer} %{time_total}\n' -H "Authorization: Bearer $TOK" -H 'content-type: application/json' -d "$CUERPO" "$URL/v1/chat/completions"
}
echo "(b) 1 llamada (stream, 16 tokens): http ttft_s total_s = $(llamada)  [$ETIQUETA]"
for i in 1 2 3 4; do llamada > /tmp/b$i & done; wait
echo "(b) 4 concurrentes: $(cat /tmp/b1 /tmp/b2 /tmp/b3 /tmp/b4 | tr '\n' '|')  [$ETIQUETA]"

# ── (d) la Function del arbol que nombra modelo/v2-lite, y el eco ────────────
export GIT_CONFIG_VALUE_0="Authorization: token $(cat /puesto/forja)"
cd /trabajo
git clone --quiet "http://forja.$NS.svc.cluster.local:3000/$NS/ontologia.git" arbol
cd arbol
if [ ! -f functions/segmentar.yaml ]; then
  cp -rL /e0/arbol/. .
  # el manifiesto de la celda no declara fuentes: E0 pone las dos que el paquete nombra
  if ! grep -q '^datasources:' ontology.config.yaml; then
    printf '\ndatasources:\n  - { name: erp, type: postgres, connectionEnv: ERP_URL }\n  - { name: lago, type: iceberg, connectionEnv: LAGO_URL }\n' >> ontology.config.yaml
  fi
  ore validate .
  git add -A
  git -c user.name=e0 -c user.email=e0@invalido commit -q -m "E0 (0027) · el paquete ventas y la Function segmentar, que nombra modelo/v2-lite" -m "Un paquete minimo —Table, View materializada, Entity— y una Function con runtime: model, entrypoint: modelo/v2-lite y effects: writes ventas.Cliente.segmento. La propiedad y el conducto son untrusted: es lo que la salida de un modelo sin endoso es en el reticulo (OOS7002)."
  git push -q origin HEAD:main
  echo "(d) el arbol tiene el paquete y la Function · $(git rev-parse --short HEAD)"
else
  echo "(d) el arbol ya tenia functions/segmentar.yaml · $(git rev-parse --short HEAD)"
fi
ENTRADA=$(python3 -c 'import yaml;print(yaml.safe_load(open("functions/segmentar.yaml"))["spec"]["entrypoint"])')
PROMPT=$(python3 -c 'import yaml;print(yaml.safe_load(open("functions/segmentar.yaml"))["spec"]["x-ore-prompt"])')
echo "(d) ventas.segmentar invoca $ENTRADA → hoy no hay documento Model (E1): la plataforma da la puerta ($URL) y el perfil g1/deepseek-v2-lite da el id ($MODELO_ID)"
# la funcion es pura: recibe valores. La fila, dada:
FILA=C-0001
ACTIVIDAD="Taller mecanico familiar con seis empleados en Ourense"
CUERPO=$(python3 -c "import json,sys;print(json.dumps({'model':'$MODELO_ID','messages':[{'role':'system','content':sys.argv[1]},{'role':'user','content':'Cliente $FILA. Actividad: $ACTIVIDAD'}],'max_tokens':8,'temperature':0}))" "$PROMPT")
MED=$(curl -s -m 60 -o /tmp/d.json -w '%{http_code} %{time_starttransfer} %{time_total}' -H "Authorization: Bearer $TOK" -H 'content-type: application/json' -d "$CUERPO" "$URL/v1/chat/completions")
VALOR=$(python3 -c 'import json;print(json.load(open("/tmp/d.json"))["choices"][0]["message"]["content"].strip().lower())')
USO=$(python3 -c 'import json;u=json.load(open("/tmp/d.json"))["usage"];print("%s+%s tokens"%(u["prompt_tokens"],u["completion_tokens"]))')
echo "(d) el modelo contesta [$MED] · $USO · segmento = '$VALOR'  [$ETIQUETA]"
# la Propuesta: las dos identidades que se saben sin abrir nada, preguntadas; las tres delegadas, dichas
BUNDLE=$(ore compile . | python3 -c 'import json,sys;print(json.load(sys.stdin)["digest"]["bundle"])')
VISTA=$(ore view . 2>/dev/null | grep -o 'plan      sha256:[0-9a-f]*' | head -1 | awk '{print $2}')
mkdir -p propuestas
python3 -c "import json,sys;json.dump({'funcion':'ventas.segmentar','bajo':{'bundle':sys.argv[1],'plan':'sha256:'+'0'*64,'testigos':{'ventas.clientes':'0'},'topologia':'0','vista':sys.argv[2]},'edits':[{'escribe':'ventas.Cliente.segmento','fila':{'clienteId':'$FILA'},'valor':sys.argv[3]}]},open('propuestas/segmentar-$FILA.json','w'),indent=2)" "$BUNDLE" "$VISTA" "$VALOR"
ore verify propuestas/segmentar-$FILA.json . | sed 's/^/    /'
git add propuestas
git -c user.name=ventas.segmentar -c user.email="modelo-v2-lite@$CELDA.invalido" commit -q -m "ventas.segmentar · modelo/v2-lite · ventas.Cliente.segmento [clienteId=$FILA] ← $VALOR" -m "La Propuesta de la Function, cotejada con ore verify contra este mismo arbol (bundle $BUNDLE). El modelo: $MODELO_ID por el gateway ($URL), con el token de agente de la celda ($ETIQUETA). Aplicarla por la vista sobre la copia es F5 (functions.md), que no existe: lo que aterriza es la propuesta y quien la trajo."
git push -q origin HEAD:main
echo "(d) OK la salida del modelo aterriza en la ontologia: commit $(git rev-parse --short HEAD) · $(git log -1 --format='%an <%ae>') · propuestas/segmentar-$FILA.json"
echo "== E0 · fin"
'''

REGLA = """apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: salida-al-modelo
  namespace: %(ns)s
  labels:
    ore.dev/tenant: %(celda)s
    ore.dev/e0: a-mano
spec:
  # 0027 ④: los Jobs de la celda salen SOLO al gateway (MODELOS/32:puerto).
  # E0 la aplica a mano; E2 la lleva a 13-el-inquilino-reconciliado.yaml.
  podSelector:
    matchLabels:
      ore.dev/rol: driver
  policyTypes: [Egress]
  egress:
    - to:
        - ipBlock:
            cidr: %(modelos)s/32
      ports:
        - { protocol: TCP, port: %(puerta)d }
""" % {"ns": NS, "celda": CELDA, "modelos": MODELOS, "puerta": PUERTA}


def job(nombre, fase):
    return {
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": nombre, "namespace": NS,
                     "labels": {"kueue.x-k8s.io/queue-name": "cola", "ore.dev/tenant": CELDA, "ore.dev/rol": "driver", "ore.dev/e0": fase}},
        "spec": {"backoffLimit": 0, "ttlSecondsAfterFinished": 3600, "activeDeadlineSeconds": 900,
                 "template": {
                     "metadata": {"labels": {"ore.dev/rol": "driver", "ore.dev/tenant": CELDA, "ore.dev/e0": fase}},
                     "spec": {
                         "restartPolicy": "Never", "serviceAccountName": "driver",
                         "volumes": [{"name": "puesto", "emptyDir": {"medium": "Memory"}}, {"name": "trabajo", "emptyDir": {}},
                                     {"name": "e0", "configMap": {"name": "e0", "defaultMode": 0o555,
                                                                   "items": [{"key": "e0.sh", "path": "e0.sh"}] + [{"key": k.replace("/", "__"), "path": "arbol/" + k} for k in ARBOL]}}],
                         "initContainers": [{
                             "name": "traer-el-testigo", "image": REGISTRO + "/ore-drivers:main",
                             "env": [{"name": "HOME", "value": "/tmp"}, {"name": "CLOUDSDK_CONFIG", "value": "/tmp/.gcloud"}],
                             "volumeMounts": [{"name": "puesto", "mountPath": "/puesto"}],
                             "command": ["/bin/sh", "-c"],
                             "args": ["set -e\n"
                                      "gcloud secrets versions access latest --secret=%s-forja-token --out-file=/puesto/forja\n"
                                      "for p in cliente secreto; do gcloud secrets versions access latest --secret=%s-agente-$p --out-file=/puesto/agente-$p; done\n"
                                      "chmod 0400 /puesto/*\n"
                                      "echo \"testigo y agente puestos · $(cat /puesto/agente-cliente)\"\n" % (NS, NS)],
                             "resources": {"requests": {"cpu": "50m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                             "securityContext": {"allowPrivilegeEscalation": False, "capabilities": {"drop": ["ALL"]}},
                         }],
                         "containers": [{
                             "name": "e0", "image": REGISTRO + "/ore-drivers:main", "imagePullPolicy": "Always",
                             "env": [{"name": k, "value": v} for k, v in [
                                 ("HOME", "/tmp"), ("CLOUDSDK_CONFIG", "/tmp/.gcloud"), ("GIT_TERMINAL_PROMPT", "0"),
                                 ("GIT_CONFIG_COUNT", "2"), ("GIT_CONFIG_KEY_0", "http.extraheader"), ("GIT_CONFIG_KEY_1", "safe.directory"), ("GIT_CONFIG_VALUE_1", "*"),
                                 ("CELDA", CELDA), ("NS", NS), ("FASE", fase), ("MODELOS", MODELOS), ("PUERTA", str(PUERTA)), ("MODELO_ID", MODELO_ID),
                                 ("ETIQUETA", ETIQUETA),
                                 ("EMISOR", "https://login.paladio.io"), ("DIRECCION", "http://idp-service.identidad.svc.cluster.local:8080"), ("REALM", "rubix")]],
                             "volumeMounts": [{"name": "puesto", "mountPath": "/puesto", "readOnly": True}, {"name": "trabajo", "mountPath": "/trabajo"}, {"name": "e0", "mountPath": "/e0", "readOnly": True}],
                             "command": ["/bin/sh", "/e0/e0.sh"],
                             "resources": {"requests": {"cpu": "200m", "memory": "512Mi"}, "limits": {"cpu": "1000m", "memory": "1Gi"}},
                             "securityContext": {"allowPrivilegeEscalation": False, "capabilities": {"drop": ["ALL"]}},
                         }],
                     }}},
    }


def sh(cmd, ok_codes=(0,)):
    """`gcloud` en Windows es un .cmd: va como UNA cadena con shell=True."""
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, encoding="utf-8")
    return r.stdout.strip() if r.returncode in ok_codes else None


def kubectl(*args, entrada=None, silencio=False):
    r = subprocess.run(["kubectl", *args], input=entrada, capture_output=True, text=True, encoding="utf-8",
                       env={**os.environ, "MSYS_NO_PATHCONV": "1"})
    if r.returncode != 0 and not silencio:
        print("  ✗ kubectl %s: %s" % (" ".join(args[:4]), (r.stderr or r.stdout).strip()[:300]))
    return r.stdout


def aplicar(doc):
    return kubectl("apply", "-f", "-", entrada=doc if isinstance(doc, str) else json.dumps(doc))


def admin(metodo, ruta, cuerpo=None):
    """Una llamada al plano de control del gateway desde la VPC: un pod efímero en `default`
    (sin NetworkPolicy). Es lo que `ore-serve` hará desde `/modelos` en E1."""
    nombre = "e0-admin-%d" % int(time.time() * 1000 % 100000)
    args = ["curl", "-s", "-m", "10", "-o", "/tmp/o", "-w", "%{http_code}", "-X", metodo, "http://%s:%d%s" % (MODELOS, ADMIN, ruta)]
    if cuerpo is not None:
        args += ["-H", "content-type: application/json", "-d", json.dumps(cuerpo)]
    args += [";", "echo", ";", "cat", "/tmp/o"]
    out = kubectl("run", nombre, "-n", "default", "--rm", "-i", "--restart=Never", "--quiet",
                  "--image=" + REGISTRO + "/ore-drivers:main", "--command", "--", "sh", "-c", " ".join("'%s'" % a if " " in a or "{" in a else a for a in args))
    lineas = [l for l in (out or "").splitlines() if l.strip()]
    return (lineas[0].strip(), lineas[1].strip() if len(lineas) > 1 else "") if lineas else ("", "")


def esperar_job(nombre, plazo=600):
    """Espera a que el Job termine; devuelve (ok, log, segundos hasta arrancar)."""
    t0 = time.time()
    arranco = None
    while time.time() - t0 < plazo:
        j = kubectl("get", "job", nombre, "-n", NS, "-o", "json", silencio=True)
        st = json.loads(j)["status"] if j else {}
        if arranco is None and st.get("startTime") and st.get("active"):
            pods = json.loads(kubectl("get", "pods", "-n", NS, "-l", "job-name=" + nombre, "-o", "json", silencio=True) or '{"items":[]}')["items"]
            if pods and pods[0]["status"].get("phase") in ("Running", "Succeeded", "Failed"):
                arranco = time.time() - t0
        if st.get("succeeded") or st.get("failed"):
            log = kubectl("logs", "-n", NS, "job/" + nombre, "-c", "e0", silencio=True)
            return bool(st.get("succeeded")), log or "", arranco
        time.sleep(3)
    return False, kubectl("logs", "-n", NS, "job/" + nombre, "-c", "e0", silencio=True) or "(sin registro)", arranco


def token_del_agente():
    cli = sh("gcloud secrets versions access latest --secret=%s-agente-cliente --project=%s" % (NS, PROYECTO))
    sec = sh("gcloud secrets versions access latest --secret=%s-agente-secreto --project=%s" % (NS, PROYECTO))
    if not cli or not sec:
        return None
    r = subprocess.run(["curl", "-s", "-m", "10", "-X", "POST", IDP + "/protocol/openid-connect/token",
                        "-d", "grant_type=client_credentials", "-d", "client_id=" + cli, "--data-urlencode", "client_secret=" + sec],
                       capture_output=True, text=True, encoding="utf-8")
    try:
        return json.loads(r.stdout)["access_token"]
    except (ValueError, KeyError):
        return None


filas = []


def fila(letra, que, resultado):
    filas.append((letra, que, resultado))
    print("  %s  %-58s %s" % (letra, que, resultado))


print()
print("  ═══ E0 · LA CELDA ALCANZA EL MODELO · %s · gateway %s:%d ═══" % (CELDA, MODELOS, PUERTA))

# ── 0 · lo que hay antes de tocar nada ───────────────────────────────────────
pol = json.loads(kubectl("get", "netpol", "-n", NS, "-o", "json") or '{"items":[]}')["items"]
nombres = [p["metadata"]["name"] for p in pol]
print("\n  ⓪ la celda hoy: %d NetworkPolicy · deny-all-egress %s · salida-al-modelo %s" % (
    len(pol), "sí" if "deny-all-egress" in nombres else "NO", "ya estaba" if "salida-al-modelo" in nombres else "no"))
if "salida-al-modelo" in nombres:
    kubectl("delete", "netpol", "salida-al-modelo", "-n", NS)
    print("     retirada para medir (a) desde cero")
cod, cuerpo = admin("GET", "/admin/health")
print("     gateway /admin/health desde la VPC → %s %s" % (cod, cuerpo[:160]))
if cod != "200":
    print("  ✗ el gateway no contesta en %s:%d — ¿está la máquina `modelos` encendida?" % (MODELOS, ADMIN))
    sys.exit(1)
cod, _ = admin("DELETE", "/admin/tenants/%s/models/%s" % (CELDA, MODELO_ID.replace("/", "%2F")))
print("     suscripción de %s retirada antes de medir → %s" % (CELDA, cod))

# el ConfigMap con el guion y el árbol de E0
cm = {"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "e0", "namespace": NS, "labels": {"ore.dev/tenant": CELDA, "ore.dev/e0": "a-mano"}},
      "data": {"e0.sh": E0_SH, **{k.replace("/", "__"): v for k, v in ARBOL.items()}}}
aplicar(cm)
for n in ("e0-sin-regla", "e0-con-regla"):
    kubectl("delete", "job", n, "-n", NS, "--ignore-not-found", "--wait=true")

# ── 1 · (a) sin la regla: el Job no alcanza :8000 ────────────────────────────
print("\n  ① Job `e0-sin-regla` por la cola (driver, Kueue)…")
aplicar(job("e0-sin-regla", "sin-regla"))
ok, log, arranco = esperar_job("e0-sin-regla")
for l in log.splitlines():
    print("     │ " + l)
fila("(a)", "sin regla: un Job de la celda alcanza :8000", ("NO — timeout rc 28, deny-all-egress tira el paquete" if ok else "✗ " + log.strip().splitlines()[-1] if log.strip() else "✗"))
print("     (el Job arrancó a los %s s: la cola y el pool `jobs`)" % (round(arranco) if arranco else "?"))

# ── 2 · la regla de clase, a mano ────────────────────────────────────────────
aplicar(REGLA)
print("\n  ② `salida-al-modelo` aplicada a mano: driver → %s/32:%d, y nada más" % (MODELOS, PUERTA))

# ── 3 · (a)(c)(b)(d) con la regla ────────────────────────────────────────────
print("\n  ③ Job `e0-con-regla` por la cola…")
aplicar(job("e0-con-regla", "con-regla"))
# la suscripción llega mientras el Job espera con 401: es POST /modelos de E1, hecha aquí
t0 = time.time()
suscrito_en = None
while time.time() - t0 < 600:
    time.sleep(3)
    log = kubectl("logs", "-n", NS, "job/e0-con-regla", "-c", "e0", silencio=True) or ""
    if suscrito_en is None and "esperando la suscripcion" in log:
        cod, cuerpo = admin("POST", "/admin/tenants/%s/models" % CELDA, {"model": MODELO_ID})
        suscrito_en = time.time()
        print("     POST /admin/tenants/%s/models {%s} → %s %s" % (CELDA, MODELO_ID, cod, cuerpo[:120]))
    j = kubectl("get", "job", "e0-con-regla", "-n", NS, "-o", "json", silencio=True)
    st = json.loads(j)["status"] if j else {}
    if st.get("succeeded") or st.get("failed"):
        break
ok, log, arranco = esperar_job("e0-con-regla", plazo=60)
for l in log.splitlines():
    print("     │ " + l)
lineas = log.splitlines()


def linea(prefijo):
    return next((l for l in lineas if l.startswith(prefijo)), "")


def entre(texto, desde, hasta=None):
    """Lo que hay entre `desde` y `hasta` (o el final); vacío si no está."""
    if desde not in texto:
        return ""
    resto = texto.split(desde, 1)[1]
    return resto.split(hasta, 1)[0].strip() if hasta and hasta in resto else resto.strip()


fila("(a)", "con la regla de clase: el Job alcanza :8000", ("SÍ — " + entre(linea("(a) OK con"), "(a) OK con ", " (401")) if linea("(a) OK con") else "✗")
fila("(c)", "el mismo curl con el token de agente", ("401 «not subscribed» → 200 suscrita" if linea("(c) OK suscrita") else "✗ " + (linea("(c) MAL") or linea("(c) ~"))))
if suscrito_en and linea("(c) OK suscrita"):
    fila("(c)", "segundos entre POST /modelos y verlo desde la celda", "≤ " + entre(linea("(c) OK suscrita"), "vista desde la celda ", " despues"))
fila("(b)", "1 llamada · http ttft_s total_s", (entre(linea("(b) 1 llamada"), "= ", "  [") or "✗") + ("  · de mentira" if DE_MENTIRA else "  · " + ETIQUETA))
fila("(b)", "4 concurrentes", (entre(linea("(b) 4 concurrentes"), "concurrentes: ", "  [") or "✗") + ("  · de mentira" if DE_MENTIRA else "  · " + ETIQUETA))
fila("(d)", "la Function nombra modelo/v2-lite y el modelo contesta", (entre(linea("(d) el modelo contesta"), "contesta ", "  [") or "✗"))
fila("(d)", "la salida aterriza en la ontología (commit)", (entre(linea("(d) OK"), "ontologia: ") or "✗"))

# ── 4 · el árbol lo nombra: visto desde fuera, por ore-serve ─────────────────
tok = token_del_agente()
if tok:
    r = subprocess.run(["curl", "-s", "-m", "15", "-H", "authorization: Bearer " + tok, "https://%s.ore.paladio.io/paquetes" % CELDA],
                       capture_output=True, text=True, encoding="utf-8")
    try:
        paquetes = json.loads(r.stdout)
        nombres = ["%s %s" % (p.get("name"), p.get("version")) for p in paquetes.get("packages", [])]
    except ValueError:
        nombres = ["✗ " + (r.stdout or "")[:80]]
    fila("(d)", "GET /paquetes de ore-serve (el árbol, desde fuera)", ", ".join(str(n) for n in nombres) or "(vacío)")

# ── 5 · (e) el informador no ve nada nuevo ───────────────────────────────────
r = subprocess.run([sys.executable, os.path.join(os.path.dirname(__file__), "medida-el-estado-de-la-celda.py"), CELDA, "--cotejar"],
                   capture_output=True, text=True, encoding="utf-8", env={**os.environ, "PYTHONIOENCODING": "utf-8"})
ult = [l for l in (r.stdout or "").splitlines() if l.strip()]
fila("(e)", "--cotejar de 0026: nada nuevo en la celda", (ult[-1].strip() if ult else "✗ " + (r.stderr or "")[:120]))
pods = json.loads(kubectl("get", "pods", "-n", NS, "-o", "json") or '{"items":[]}')["items"]
roles = sorted({p["metadata"]["labels"].get("ore.dev/rol", "?") for p in pods if p["status"].get("phase") == "Running"})
fila("(e)", "pods vivos en la celda, por rol", ", ".join(roles) + " — ninguno sirve un modelo")

# ── 6 · recoger ──────────────────────────────────────────────────────────────
kubectl("delete", "configmap", "e0", "-n", NS, "--ignore-not-found")
if DEJAR:
    print("\n  la regla `salida-al-modelo` se queda (--dejar); los Jobs caducan solos en 1 h")
else:
    kubectl("delete", "netpol", "salida-al-modelo", "-n", NS, "--ignore-not-found")
    print("\n  regla `salida-al-modelo` retirada (E2 la lleva a la plantilla); los Jobs caducan solos en 1 h")

print("\n  ═══ la tabla ═══")
for l, q, res in filas:
    print("  %s  %-58s %s" % (l, q, res))
print()
