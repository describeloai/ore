# -*- coding: utf-8 -*-
"""0027 E3 I5 — LA ACEPTACIÓN EN LA CELDA, A CUATRO MANOS.

Lo que la ADR acepta de E3: desde el Hub, *Use in this cluster* → una fila en
*provisioning* con la hora → *running* con la máquina; el `Model` está en el árbol
con el autor de la sesión; el 409 de *Retire* en pantalla no toca nada; retirado,
la fila desaparece y el Job de la celda recibe 401.

La consola la pulsa UNA PERSONA con su sesión (esto no se puede fingir: el autor del
commit es su `sub`). Este guion hace lo de alrededor, por fases, y mira el resultado
donde se puede mirar sin la consola: el árbol (`git log`), `GET /modelos` con el token
de agente, un Job en la celda, y la máquina de modelos.

    python pruebas-de-fuego/deployments-tiene-filas.py <fase> [celda]

    preparar         enciende `modelos-e0`, espera al gateway y enseña GET /modelos
    retirar-funcion  Job en la celda: `functions/segmentar.yaml` fuera (para que Retire pase)
    ver              GET /modelos ahora, con la hora (se llama entre pulsación y pulsación)
    parar-backend    en la máquina: `docker stop de-mentira` → el gateway lo da por caído
    arrancar-backend `docker start de-mentira` → arriba otra vez
    job-401          Job en la celda: /v1/models con el token de agente (espera 401 o 200)
    autor            `git log -1 -- modelos/v2-lite.yaml` en el árbol: quién lo firmó
    funcion          Job: la Function de E1 vuelve a nombrar modelo/v2-lite (estado final)
    apagar           apaga `modelos-e0` y comprueba que nada queda encendido
"""
import io
import json
import os
import subprocess
import sys
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")

FASE = next((a for a in sys.argv[1:] if not a.startswith("--")), "ver")
CELDA = next((a for a in sys.argv[2:] if not a.startswith("--")), "victor")
NS = "t-" + CELDA
PROYECTO = "project-8853a180-450d-47be-b83"
ZONA = "europe-west1-b"
VM = "modelos-e0"
IDP = "https://login.paladio.io/realms/rubix"
MODELO = next((a.split("=", 1)[1] for a in sys.argv if a.startswith("--modelo=")), "deepseek-v2-lite")
GCLOUD = "gcloud.cmd" if os.name == "nt" else "gcloud"


def sh(cmd):
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, encoding="utf-8")
    return r.stdout.strip() if r.returncode == 0 else ""


def kubectl(*args, entrada=None):
    r = subprocess.run(["kubectl", *args], input=entrada, capture_output=True, text=True, encoding="utf-8", env={**os.environ, "MSYS_NO_PATHCONV": "1"})
    return r.stdout


def gcloud(*args):
    r = subprocess.run([GCLOUD, *args, "--project", PROYECTO, "--quiet"], capture_output=True, text=True, encoding="utf-8")
    return r.returncode, (r.stdout + r.stderr).strip()


def token_del_agente(celda):
    ns = "t-" + celda
    cli = sh("%s secrets versions access latest --secret=%s-agente-cliente --project=%s" % (GCLOUD, ns, PROYECTO))
    sec = sh("%s secrets versions access latest --secret=%s-agente-secreto --project=%s" % (GCLOUD, ns, PROYECTO))
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


def hora():
    return time.strftime("%H:%M:%S")


def ver(tok):
    cod, seg, cuerpo = serve("GET", "/modelos", tok)
    try:
        d = json.loads(cuerpo)
    except ValueError:
        print("  %s GET /modelos → %s en %s s · %s" % (hora(), cod, seg, cuerpo[:200]))
        return None
    g = d.get("gateway", {})
    print("  %s GET /modelos → %s en %s s · gateway contesta=%s backends_arriba=%s%s" % (
        hora(), cod, seg, g.get("contesta"), g.get("backends_arriba"), (" · " + str(g.get("motivo"))[:90]) if g.get("motivo") else ""))
    for m in d.get("modelos", []):
        e = m.get("estado", {})
        u = m.get("uso") or {}
        print("     · %-10s %-13s backends=%-18s autor=%s desde=%s uso.hoy=%s%s" % (
            m.get("name"), e.get("fase"), e.get("backends"), str(m.get("autor"))[-28:], str(m.get("desde"))[:19], json.dumps(u.get("hoy")),
            (" · " + str(e.get("motivo"))[:80]) if e.get("motivo") else ""))
    return d


# ── los Jobs en la celda (la misma figura que E2: el agente y el testigo desde Secret Manager) ──

ARBOL_SH = r"""#!/bin/sh
set -e
export GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=http.extraheader GIT_CONFIG_KEY_1=safe.directory GIT_CONFIG_VALUE_1='*' GIT_TERMINAL_PROMPT=0
export GIT_CONFIG_VALUE_0="Authorization: token $(cat /puesto/forja)"
cd /tmp && git clone --quiet "http://forja.t-$CELDA.svc.cluster.local:3000/t-$CELDA/ontologia.git" arbol && cd arbol
case "$FASE" in
  retirar-funcion)
    if [ -f functions/segmentar.yaml ]; then
      git rm -q functions/segmentar.yaml
      git -c user.name=e3 -c user.email=e3@invalido commit -q -m "E3 I5 (0027) · la Function segmentar se retira para que Retire pase: el 409 ya se vio en pantalla"
      git push -q origin HEAD:main; echo "(arbol) retirada functions/segmentar.yaml · $(git rev-parse --short HEAD)"
    else echo "(arbol) no habia Function que retirar · $(git rev-parse --short HEAD)"; fi ;;
  autor)
    if [ -f modelos/$MODELO.yaml ]; then
      echo "(arbol) modelos/$MODELO.yaml · $(git log -1 --format='autor=%an <%ae> committer=%cn fecha=%aI commit=%h' -- modelos/$MODELO.yaml)"
      echo "(arbol) asunto: $(git log -1 --format=%s -- modelos/$MODELO.yaml)"
      sed 's/^/    | /' modelos/$MODELO.yaml
    else echo "(arbol) MAL no hay modelos/$MODELO.yaml"; exit 1; fi ;;
  funcion)
    [ -f modelos/$MODELO.yaml ] || { echo "(arbol) MAL no hay modelos/$MODELO.yaml"; exit 1; }
    printf '%s\n' 'apiVersion: oos.dev/v1alpha9' 'kind: Function' 'metadata: { name: segmentar, namespace: ventas }' 'spec:' '  runtime: model' "  model: modelo/$MODELO" '  prompt: "Clasifica la actividad de este cliente en un segmento de una palabra: pyme, corporativo, publico o particular. Contesta solo con la palabra."' '  effects:' '    - writes: ventas.Cliente.segmento' > functions/segmentar.yaml
    ore validate . | sed 's/^/    /'
    git add functions/segmentar.yaml
    git -c user.name=e3 -c user.email=e3@invalido commit -q -m "E3 I5 (0027) · la Function de E1 vuelve a nombrar modelo/$MODELO: el estado final de la celda"
    git push -q origin HEAD:main; echo "(arbol) la Function nombra modelo/$MODELO · $(git rev-parse --short HEAD)" ;;
esac
"""

JOB_SH = r"""#!/bin/sh
TOK=$(curl -sSf -X POST "https://login.paladio.io/realms/rubix/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" --data-urlencode "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
n=0; COD=000
while [ $n -lt 45 ]; do
  COD=$(curl -s -m 8 -o /tmp/r.json -w '%{http_code}' -H "Authorization: Bearer $TOK" "http://10.10.0.100:8000/v1/models")
  echo "$(date +%H:%M:%S) GET /v1/models → $COD · $(head -c 100 /tmp/r.json)"
  [ "$COD" = "$ESPERA" ] && { echo "(job) OK la celda recibe $ESPERA"; exit 0; }
  n=$((n+1)); sleep 4
done
echo "(job) MAL esperaba $ESPERA y sigue $COD"; exit 1
"""


def job(nombre, fase, espera="401"):
    arbol = fase in ("retirar-funcion", "autor", "funcion")
    return {
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": nombre, "namespace": NS, "labels": {"kueue.x-k8s.io/queue-name": "cola"}},
        "spec": {"backoffLimit": 0, "ttlSecondsAfterFinished": 3600, "template": {"metadata": {"labels": {"ore.dev/rol": "driver", "ore.dev/tenant": CELDA, "ore.dev/e3": fase}}, "spec": {
            "restartPolicy": "Never", "serviceAccountName": "driver",
            "initContainers": [{"name": "puesto", "image": "europe-west1-docker.pkg.dev/%s/ore/ore-drivers:main" % PROYECTO, "command": ["/bin/sh", "-c"],
                                "env": [{"name": "HOME", "value": "/tmp"}, {"name": "CLOUDSDK_CONFIG", "value": "/tmp/.gcloud"}, {"name": "CLOUDSDK_CORE_PROJECT", "value": PROYECTO}],
                                "args": ["set -e\nfor p in cliente secreto; do gcloud secrets versions access latest --secret=%s-agente-$p --out-file=/puesto/agente-$p; done\ngcloud secrets versions access latest --secret=%s-forja-token --out-file=/puesto/forja\nchmod 0400 /puesto/*\necho 'agente y testigo puestos'\n" % (NS, NS)],
                                "resources": {"requests": {"cpu": "100m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                                "volumeMounts": [{"name": "puesto", "mountPath": "/puesto"}]}],
            "containers": [{"name": "e3", "image": "europe-west1-docker.pkg.dev/%s/ore/ore-drivers:main" % PROYECTO, "imagePullPolicy": "Always",
                            "command": ["/bin/sh", "/e3/arbol.sh" if arbol else "/e3/job.sh"],
                            "env": [{"name": "CELDA", "value": CELDA}, {"name": "FASE", "value": fase}, {"name": "MODELO", "value": MODELO}, {"name": "ESPERA", "value": espera}],
                            "resources": {"requests": {"cpu": "100m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                            "volumeMounts": [{"name": "puesto", "mountPath": "/puesto", "readOnly": True}, {"name": "e3", "mountPath": "/e3"}]}],
            "volumes": [{"name": "puesto", "emptyDir": {"medium": "Memory"}}, {"name": "e3", "configMap": {"name": "e3", "defaultMode": 0o555}}],
        }}},
    }


def correr_job(nombre, fase, espera="401", plazo=600):
    kubectl("apply", "-f", "-", entrada=json.dumps({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "e3", "namespace": NS}, "data": {"arbol.sh": ARBOL_SH, "job.sh": JOB_SH}}))
    kubectl("delete", "job", nombre, "-n", NS, "--ignore-not-found", "--wait=true")
    kubectl("apply", "-f", "-", entrada=json.dumps(job(nombre, fase, espera)))
    t0 = time.time()
    while time.time() - t0 < plazo:
        j = kubectl("get", "job", nombre, "-n", NS, "-o", "json")
        st = json.loads(j)["status"] if j else {}
        if st.get("succeeded") or st.get("failed"):
            break
        time.sleep(5)
    salida = kubectl("logs", "-n", NS, "job/" + nombre, "-c", "e3") or ""
    for l in salida.splitlines():
        print("     │ " + l)
    return salida


def en_la_maquina(cmd):
    return gcloud("compute", "ssh", VM, "--zone", ZONA, "--tunnel-through-iap", "--command", cmd)


print("\n  ═══ E3 I5 · %s · %s · %s ═══\n" % (FASE, CELDA, hora()))

if FASE == "preparar":
    rc, out = gcloud("compute", "instances", "describe", VM, "--zone", ZONA, "--format=value(status)")
    print("  %s %s está %s" % (hora(), VM, out))
    if out != "RUNNING":
        rc, out = gcloud("compute", "instances", "start", VM, "--zone", ZONA)
        print("  %s start → rc=%s" % (hora(), rc))
    tok = token_del_agente(CELDA)
    t0 = time.time()
    while True:
        d = ver(tok)
        if d and d.get("gateway", {}).get("contesta") and d.get("gateway", {}).get("backends_arriba", 0) > 0:
            print("  %s el gateway contesta con un backend arriba · %d s desde el start" % (hora(), time.time() - t0))
            break
        if time.time() - t0 > 600:
            print("  ✗ el gateway no ha contestado en 10 min")
            sys.exit(1)
        time.sleep(15)

elif FASE == "ver":
    ver(token_del_agente(CELDA))

elif FASE in ("retirar-funcion", "autor", "funcion"):
    correr_job("e3-arbol-" + FASE, FASE)
    ver(token_del_agente(CELDA))

elif FASE == "job-401":
    espera = "200" if "--200" in sys.argv else "401"
    correr_job("e3-job", "job", espera, plazo=300)

elif FASE == "parar-backend":
    rc, out = en_la_maquina("sudo docker stop de-mentira && sudo docker ps --format '{{.Names}} {{.Status}}'")
    print("  %s docker stop de-mentira → rc=%s\n     %s" % (hora(), rc, out.replace("\n", "\n     ")))
    tok = token_del_agente(CELDA)
    for _ in range(12):
        d = ver(tok)
        if d and d.get("gateway", {}).get("backends_arriba") == 0:
            break
        time.sleep(5)

elif FASE == "arrancar-backend":
    rc, out = en_la_maquina("sudo docker start de-mentira && sudo docker ps --format '{{.Names}} {{.Status}}'")
    print("  %s docker start de-mentira → rc=%s\n     %s" % (hora(), rc, out.replace("\n", "\n     ")))
    tok = token_del_agente(CELDA)
    for _ in range(12):
        d = ver(tok)
        if d and d.get("gateway", {}).get("backends_arriba", 0) > 0:
            break
        time.sleep(5)

elif FASE == "apagar":
    rc, out = gcloud("compute", "instances", "stop", VM, "--zone", ZONA)
    print("  %s stop %s → rc=%s" % (hora(), VM, rc))
    rc, out = gcloud("compute", "instances", "list", "--format=value(name,status)")
    print("  máquinas del proyecto:\n     " + out.replace("\n", "\n     "))
    kubectl("delete", "configmap", "e3", "-n", NS, "--ignore-not-found")

else:
    print(__doc__)
