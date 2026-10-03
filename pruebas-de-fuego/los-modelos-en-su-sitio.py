#!/usr/bin/env python3
"""LOS MODELOS EN SU SITIO (2026-10-03): fuera los `Model` de antes del paradigma.

Un `Model` de hoy vive en `base.schema.nombre` (0041 Model Registry), declara
v1alpha21 y lleva `owner` de quien lo da de alta (0052 Ownership). Los de antes
—v1alpha9 en la raíz del árbol, o v1alpha15 sin dueño— se limpian:

  · `victor`: `modelos/deepseek-v2-lite.yaml` (v1alpha9, raíz) y
    `packages/foreign_test/modelos/qwen3-vl-8b.yaml` (v1alpha15, sin dueño) se
    retiran —ninguna `Function` los nombra—, y con ellos sus suscripciones en
    la puerta (`DELETE /admin/tenants/victor/models/<id>`, lo mismo que
    `DELETE /modelos`).
  · `demo`: `modelos/v2-lite.yaml` (v1alpha9, raíz, creado por el agente de la
    celda) lo usa `olist_copia.traducirCategoria` (`modelo/v2-lite`), así que se
    rehace en el paradigma: `packages/olist_copia/modelos/v2-lite.yaml`,
    v1alpha21, `owner: user:victor` (quien lo pide); el de la raíz se va. La
    función lo resuelve en su mismo schema, y la suscripción es la misma (el
    id servido no cambia).

Un Job por celda (imagen `ore-drivers`, el token de la forja de Secret Manager,
nada impreso): `ore validate` antes y después —ni un diagnóstico más— y, con
`--empujar`, un commit en `main`. La retirada de la suscripción va en otro Job
con `rol: control`, el único que llega al plano de control de la puerta.

    python pruebas-de-fuego/los-modelos-en-su-sitio.py [victor demo]           # ensayo
    python pruebas-de-fuego/los-modelos-en-su-sitio.py victor demo --empujar

Vuelta atrás: `git revert` del commit; en `victor`, volver a suscribir
(`POST /modelos` de nuevo, o la puerta).
"""
import io
import json
import os
import subprocess
import sys
import time

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
PROYECTO = "project-8853a180-450d-47be-b83"
NOMBRE = "los-modelos-en-su-sitio"
PUERTA = "10.10.0.100:9000"

FUERA = {
    "victor": ["modelos/deepseek-v2-lite.yaml", "packages/foreign_test/modelos/qwen3-vl-8b.yaml"],
    "demo": ["modelos/v2-lite.yaml"],
}
NUEVOS = {
    "demo": {
        "packages/olist_copia/modelos/v2-lite.yaml": (
            "apiVersion: oos.dev/v1alpha21\n"
            "kind: Model\n"
            "metadata:\n"
            "  name: v2-lite\n"
            "  namespace: olist_copia\n"
            "spec:\n"
            "  owner: user:victor\n"
            "  profile: g1/deepseek-v2-lite\n"
            "  tier: shared\n"
            "  task: chat\n"
        ),
    },
}
# Las suscripciones que se retiran (el id servido de cada perfil que se va).
DESUSCRIBIR = {
    "victor": ["deepseek-ai/DeepSeek-V2-Lite", "Qwen/Qwen3-VL-8B-Instruct-FP8"],
}


def kubectl(*args, entrada=None):
    r = subprocess.run(["kubectl", *args], input=entrada, capture_output=True, text=True, encoding="utf-8",
                       env={**os.environ, "MSYS_NO_PATHCONV": "1", "MSYS2_ARG_CONV_EXCL": "*"})
    if r.returncode != 0 and "NotFound" not in (r.stderr or ""):
        print("     kubectl", " ".join(args[:3]), "→", (r.stderr or "").strip()[:200])
    return r.stdout


GUION_SH = r"""#!/bin/sh
set -e
export GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=http.extraheader GIT_CONFIG_KEY_1=safe.directory GIT_CONFIG_VALUE_1='*' GIT_TERMINAL_PROMPT=0
export GIT_CONFIG_VALUE_0="Authorization: token $(cat /puesto/forja)"
cd /tmp && git clone --quiet "http://forja.t-$CELDA.svc.cluster.local:3000/t-$CELDA/ontologia.git" arbol && cd arbol
echo "(arbol) commit=$(git rev-parse --short HEAD)"
ore validate . > /tmp/v0.txt 2>&1 || true
python3 /guion/cambiar.py
ore validate . > /tmp/v1.txt 2>&1 || true
D0=$(grep -c '^error\[' /tmp/v0.txt || true); D1=$(grep -c '^error\[' /tmp/v1.txt || true)
echo "(validate) diagnósticos $D0 → $D1"
if [ "$D1" -gt "$D0" ]; then echo "(validate) ✗ más diagnósticos que antes:"; grep -A3 '^error\[' /tmp/v1.txt | head -20; exit 1; fi
git add -A && git status --short | sed 's/^/(cambia) /'
if [ -z "$EMPUJAR" ]; then echo "(ensayo) nada empujado"; exit 0; fi
git -c user.name='ore 0041' -c user.email=modelos@invalido \
  commit -q -m "0041 · los modelos en su sitio: fuera los de antes del paradigma (v1alpha9 en la raiz, v1alpha15 sin dueno)"
git push -q origin HEAD:main
echo "(empujado) $(git rev-parse --short HEAD)"
"""

CAMBIAR_PY = r'''
import json, os
cfg = json.load(open("/guion/cfg.json"))
for f in cfg["fuera"]:
    if os.path.exists(f):
        os.remove(f); print("(fuera) " + f)
    else:
        print("(fuera) " + f + " · ya no estaba")
for f, texto in cfg["nuevos"].items():
    os.makedirs(os.path.dirname(f), exist_ok=True)
    open(f, "w", encoding="utf-8", newline="").write(texto); print("(nuevo) " + f)
'''

DESUSCRIBIR_SH = r"""#!/bin/sh
set -e
for id in $IDS; do
  c=$(curl -s -o /tmp/r -w '%{http_code}' -X DELETE "http://$PUERTA/admin/tenants/$CELDA/models/$(echo "$id" | sed 's#/#%2F#g')")
  echo "(puerta) DELETE $CELDA · $id → $c"
done
curl -s "http://$PUERTA/admin/tenants" | python3 -c 'import json,sys,os
for t in json.load(sys.stdin):
    if t["id"]==os.environ["CELDA"]: print("(puerta) suscripciones de", t["id"], "→", t.get("allowed_models"))'
"""


def job(ns, celda, guion, rol, env):
    img = "europe-west1-docker.pkg.dev/%s/ore/ore-drivers:main" % PROYECTO
    secretos = ("set -e\ngcloud secrets versions access latest --secret=%s-forja-token --out-file=/puesto/forja\n"
                "chmod 0400 /puesto/*\n") % ns
    return {
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": NOMBRE, "namespace": ns, "labels": {"kueue.x-k8s.io/queue-name": "cola"}},
        "spec": {"backoffLimit": 0, "ttlSecondsAfterFinished": 600, "template": {"metadata": {"labels": {"ore.dev/rol": rol, "ore.dev/tenant": celda, "ore.dev/medida": NOMBRE}}, "spec": {
            "restartPolicy": "Never", "serviceAccountName": "driver",
            "initContainers": [{"name": "puesto", "image": img, "command": ["/bin/sh", "-c"],
                                "env": [{"name": "HOME", "value": "/tmp"}, {"name": "CLOUDSDK_CONFIG", "value": "/tmp/.gcloud"}, {"name": "CLOUDSDK_CORE_PROJECT", "value": PROYECTO}],
                                "args": [secretos],
                                "resources": {"requests": {"cpu": "100m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                                "volumeMounts": [{"name": "puesto", "mountPath": "/puesto"}]}],
            "containers": [{"name": "guion", "image": img, "imagePullPolicy": "Always",
                            "command": ["/bin/sh", "/guion/" + guion],
                            "env": [{"name": "CELDA", "value": celda}, {"name": "HOME", "value": "/tmp"}] +
                                   [{"name": k, "value": v} for k, v in env.items()],
                            "resources": {"requests": {"cpu": "200m", "memory": "512Mi"}, "limits": {"cpu": "1000m", "memory": "1Gi"}},
                            "volumeMounts": [{"name": "puesto", "mountPath": "/puesto", "readOnly": True}, {"name": "guion", "mountPath": "/guion"}]}],
            "volumes": [{"name": "puesto", "emptyDir": {"medium": "Memory"}}, {"name": "guion", "configMap": {"name": NOMBRE, "defaultMode": 0o555}}],
        }}},
    }


def correr(celda, guion, rol, env):
    ns = "t-" + celda
    cfg = {"fuera": FUERA.get(celda, []), "nuevos": NUEVOS.get(celda, {})}
    datos = {"guion.sh": GUION_SH, "cambiar.py": CAMBIAR_PY, "desuscribir.sh": DESUSCRIBIR_SH, "cfg.json": json.dumps(cfg)}
    kubectl("apply", "-f", "-", entrada=json.dumps({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": NOMBRE, "namespace": ns}, "data": datos}))
    kubectl("delete", "job", NOMBRE, "-n", ns, "--ignore-not-found", "--wait=true")
    kubectl("apply", "-f", "-", entrada=json.dumps(job(ns, celda, guion, rol, env)))
    t0, st = time.time(), {}
    while time.time() - t0 < 900:
        j = kubectl("get", "job", NOMBRE, "-n", ns, "-o", "json")
        st = json.loads(j)["status"] if j else {}
        if st.get("succeeded") or st.get("failed"):
            break
        time.sleep(5)
    salida = kubectl("logs", "-n", ns, "job/" + NOMBRE, "-c", "guion") or ""
    kubectl("delete", "job", NOMBRE, "-n", ns, "--ignore-not-found", "--wait=true")
    kubectl("delete", "configmap", NOMBRE, "-n", ns, "--ignore-not-found")
    for l in salida.splitlines():
        print("     " + l)
    ok = bool(st.get("succeeded"))
    print("     %s · %.0f s" % ("✓" if ok else "✗ el Job no terminó bien", time.time() - t0))
    return ok


def main():
    args = sys.argv[1:]
    empujar = "--empujar" in args
    celdas = [a for a in args if not a.startswith("--")] or ["victor", "demo"]
    bien = True
    for c in celdas:
        print("los modelos en su sitio · %s · %s" % (c, "EMPUJAR" if empujar else "ENSAYO"))
        bien &= correr(c, "guion.sh", "driver", {"EMPUJAR": "1" if empujar else ""})
        if bien and empujar and DESUSCRIBIR.get(c):
            bien &= correr(c, "desuscribir.sh", "control", {"IDS": " ".join(DESUSCRIBIR[c]), "PUERTA": PUERTA})
    print("  (el clúster, como estaba: los Jobs y su ConfigMap se retiraron)")
    return 0 if bien else 1


if __name__ == "__main__":
    sys.exit(main())
