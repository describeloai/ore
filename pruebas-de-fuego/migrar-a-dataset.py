"""0033 · LA MIGRACIÓN A `kind: Dataset` DE UN ÁRBOL REAL (docs/dataset.md, paso 4).

Lo que `medida-migrar-dataset.py` midió, aplicado: un Job por inquilino clona
el árbol de su forja (con el token de Secret Manager, como la copia de 48),
corre `ore migrate v1alpha12 .` DENTRO —el `ore` de la imagen `ore-drivers`,
el mismo que copia—, comprueba que compila igual o mejor y que `ore datasets`
lista lo mismo, y lo empuja como un commit firmado `ore migrate`. Sin
`--empujar` es un ensayo: el Job hace todo menos el push y cuenta lo que
cambiaría. Sin instancias; el Job y su ConfigMap se retiran al final.

    python pruebas-de-fuego/migrar-a-dataset.py [demo victor …] [--empujar]
"""
import io
import json
import os
import subprocess
import sys
import time

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
PROYECTO = "project-8853a180-450d-47be-b83"


def kubectl(*args, entrada=None):
    r = subprocess.run(["kubectl", *args], input=entrada, capture_output=True, text=True, encoding="utf-8",
                       env={**os.environ, "MSYS_NO_PATHCONV": "1", "MSYS2_ARG_CONV_EXCL": "*"})
    if r.returncode != 0 and "NotFound" not in (r.stderr or ""):
        print("     kubectl", " ".join(args[:3]), "→", (r.stderr or "").strip()[:200])
    return r.stdout


# El guion que corre dentro. `EMPUJAR` vacío = ensayo. Todo lo que dice va a
# los logs, que es el informe.
MIGRAR_SH = r"""#!/bin/sh
set -e
export GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=http.extraheader GIT_CONFIG_KEY_1=safe.directory GIT_CONFIG_VALUE_1='*' GIT_TERMINAL_PROMPT=0
export GIT_CONFIG_VALUE_0="Authorization: token $(cat /puesto/forja)"
cd /tmp && git clone --quiet "http://forja.t-$CELDA.svc.cluster.local:3000/t-$CELDA/ontologia.git" arbol && cd arbol
echo "(arbol) commit=$(git rev-parse --short HEAD) ficheros=$(git ls-files | wc -l) ore=$(ore --version 2>/dev/null | head -1)"
cuenta() { # <fichero> → cuántos error[/warning[
  grep -c '^\(error\|warning\)\[' "$1" 2>/dev/null || true
}
lista() { # → los nombres de `ore datasets --json`, ordenados
  ore datasets . --json 2>/dev/null | tail -1 | python3 -c 'import json,sys
try:
    j = json.load(sys.stdin); ds = j.get("datasets") if isinstance(j, dict) else j
    print(",".join(sorted((d.get("nombre") or d.get("dataset") or json.dumps(d)) for d in (ds or []))))
except Exception:
    print("?")'
}
ore validate . > /tmp/v0.txt 2>&1 && V0=ok || V0=mal
D0=$(cuenta /tmp/v0.txt); L0=$(lista)
echo "(antes) validate=$V0 diagnosticos=$D0 datasets=$(echo "$L0" | tr ',' '\n' | grep -c . || true)"
if ! ore migrate v1alpha12 . > /tmp/m.txt 2>&1; then
  echo "(migrate) FALLO"; cat /tmp/m.txt; exit 1
fi
sed 's/^/(migrate) /' /tmp/m.txt
ore validate . > /tmp/v1.txt 2>&1 && V1=ok || V1=mal
D1=$(cuenta /tmp/v1.txt); L1=$(lista)
echo "(despues) validate=$V1 diagnosticos=$D1 datasets=$(echo "$L1" | tr ',' '\n' | grep -c . || true)"
if [ "$D1" -gt "$D0" ]; then echo "(despues) MAS DIAGNOSTICOS QUE ANTES:"; head -20 /tmp/v1.txt; exit 1; fi
if [ "$L0" != "$L1" ]; then echo "(despues) LA LISTA DE DATASETS CAMBIO: antes=$L0 despues=$L1"; exit 1; fi
QUEDAN=$(find . -name '*.yaml' -not -path './.git/*' | xargs grep -l -e '^  materialized:' -e 'datasource: lago' 2>/dev/null | wc -l)
echo "(despues) materialized/lago que quedan: $QUEDAN"
git add -A
echo "(cambios) $(git diff --cached --stat | tail -1)"
git diff --cached --name-status | sed 's/^/(cambios) /'
if [ -z "$EMPUJAR" ]; then echo "(ensayo) nada empujado"; exit 0; fi
git -c user.name='ore migrate' -c user.email=migrate@invalido \
  commit -q -m "ore migrate v1alpha12 (0033): las vistas con materialized pasan a ser datasets con su plan; los punteros, a datasets/"
git push -q origin HEAD:main
echo "(empujado) $(git rev-parse --short HEAD)"
"""


def job(ns, celda, nombre, empujar):
    img = "europe-west1-docker.pkg.dev/%s/ore/ore-drivers:main" % PROYECTO
    return {
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": nombre, "namespace": ns, "labels": {"kueue.x-k8s.io/queue-name": "cola"}},
        "spec": {"backoffLimit": 0, "ttlSecondsAfterFinished": 600, "template": {"metadata": {"labels": {"ore.dev/rol": "driver", "ore.dev/tenant": celda, "ore.dev/medida": "migrar-a-dataset"}}, "spec": {
            "restartPolicy": "Never", "serviceAccountName": "driver",
            "initContainers": [{"name": "puesto", "image": img, "command": ["/bin/sh", "-c"],
                                "env": [{"name": "HOME", "value": "/tmp"}, {"name": "CLOUDSDK_CONFIG", "value": "/tmp/.gcloud"}, {"name": "CLOUDSDK_CORE_PROJECT", "value": PROYECTO}],
                                "args": ["set -e\ngcloud secrets versions access latest --secret=%s-forja-token --out-file=/puesto/forja\nchmod 0400 /puesto/*\n" % ns],
                                "resources": {"requests": {"cpu": "100m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                                "volumeMounts": [{"name": "puesto", "mountPath": "/puesto"}]}],
            "containers": [{"name": "migrar", "image": img, "command": ["/bin/sh", "/migrar/migrar.sh"],
                            "env": [{"name": "CELDA", "value": celda}, {"name": "EMPUJAR", "value": "1" if empujar else ""}, {"name": "HOME", "value": "/tmp"}],
                            "resources": {"requests": {"cpu": "200m", "memory": "256Mi"}, "limits": {"cpu": "1000m", "memory": "512Mi"}},
                            "volumeMounts": [{"name": "puesto", "mountPath": "/puesto", "readOnly": True}, {"name": "migrar", "mountPath": "/migrar"}]}],
            "volumes": [{"name": "puesto", "emptyDir": {"medium": "Memory"}}, {"name": "migrar", "configMap": {"name": "migrar-a-dataset", "defaultMode": 0o555}}],
        }}},
    }


def migrar(celda, empujar):
    ns = "t-" + celda
    nombre = "migrar-a-dataset"
    print("  %s%s" % (celda, "" if empujar else " (ensayo)"))
    kubectl("apply", "-f", "-", entrada=json.dumps({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": nombre, "namespace": ns}, "data": {"migrar.sh": MIGRAR_SH}}))
    kubectl("delete", "job", nombre, "-n", ns, "--ignore-not-found", "--wait=true")
    kubectl("apply", "-f", "-", entrada=json.dumps(job(ns, celda, nombre, empujar)))
    t0 = time.time()
    st = {}
    while time.time() - t0 < 600:
        j = kubectl("get", "job", nombre, "-n", ns, "-o", "json")
        st = json.loads(j)["status"] if j else {}
        if st.get("succeeded") or st.get("failed"):
            break
        time.sleep(5)
    salida = kubectl("logs", "-n", ns, "job/" + nombre, "-c", "migrar") or ""
    kubectl("delete", "job", nombre, "-n", ns, "--ignore-not-found", "--wait=true")
    kubectl("delete", "configmap", nombre, "-n", ns, "--ignore-not-found")
    for l in salida.splitlines():
        print("     " + l)
    ok = bool(st.get("succeeded"))
    print("     %s · %.0f s" % ("✓" if ok else "✗ el Job no terminó bien", time.time() - t0))
    return ok


def main():
    args = sys.argv[1:]
    empujar = "--empujar" in args
    celdas = [a for a in args if not a.startswith("--")] or ["demo", "victor"]
    print("0033 · la migración a `kind: Dataset`%s" % ("" if empujar else " · ENSAYO (sin --empujar nada cambia)"))
    bien = all([migrar(c, empujar) for c in celdas])
    print("  (el clúster, como estaba: el Job y su ConfigMap se retiraron)")
    return 0 if bien else 1


if __name__ == "__main__":
    sys.exit(main())
