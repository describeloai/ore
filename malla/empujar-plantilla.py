#!/usr/bin/env python3
"""
Empuja a la cola de un inquilino (`t-<celda>/trabajo.git`) las plantillas que
`gen-inquilino.py` rinde —hoy `plantilla-puesto.txt` y `plantilla-capa.txt`—
sin pasar por el aprovisionador entero: un Job en la celda, con la imagen de
los drivers y el token de la forja de Secret Manager (como `48-la-copia`),
clona la cola, sustituye los ficheros y empuja si cambió algo.

Es lo que hace falta cuando cambia lo que un puesto corre al arrancar (0031
W3.7 gobierno ②b: la capa se baja por su nombre) y no hay que reaprovisionar
nada más. Nada de esto toca los puestos abiertos: la plantilla se lee al
rendir uno nuevo.

Uso:  python malla/empujar-plantilla.py <celda> [<celda>…] [--seco]
"""
import json
import os
import subprocess
import sys
import tempfile
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__))).replace("\\", "/")
PROYECTO = "project-8853a180-450d-47be-b83"
FICHEROS = ("plantilla-puesto.txt", "plantilla-capa.txt")
SECO = "--seco" in sys.argv


def kubectl(*args, entrada=None):
    env = dict(os.environ, MSYS_NO_PATHCONV="1", MSYS2_ARG_CONV_EXCL="*")
    r = subprocess.run(["kubectl", *args], input=entrada, capture_output=True, text=True, encoding="utf-8", env=env)
    return r.returncode, r.stdout, r.stderr


GUION = r'''set -e
export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=http.extraheader
export GIT_CONFIG_VALUE_0="Authorization: token $(cat /puesto/forja)"
cd /tmp && git clone --quiet "http://forja.t-$CELDA.svc.cluster.local:3000/t-$CELDA/trabajo.git" cola && cd cola
for f in /plantillas/*; do cp "$f" "./$(basename "$f")"; done
if git status --porcelain | grep -q .; then
  git -c user.name=ore -c user.email=ore@sujeto.invalid add -A
  git -c user.name=ore -c user.email=ore@sujeto.invalid commit -q -m "La plantilla del puesto, rendida de nuevo (${QUE:-sin motivo dicho})"
  [ -n "$SECO" ] && { echo "(seco) cambiaria: $(git show --stat --oneline HEAD | tail -n +2 | tr '\n' ' ')"; exit 0; }
  git push -q origin HEAD:main && echo "empujado: $(git rev-parse --short HEAD)"
else
  echo "ya estaba: nada que empujar"
fi
'''


def job(ns, celda, nombre):
    img = "europe-west1-docker.pkg.dev/%s/ore/ore-drivers:main" % PROYECTO
    return {
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": nombre, "namespace": ns, "labels": {"kueue.x-k8s.io/queue-name": "cola"}},
        "spec": {"backoffLimit": 0, "ttlSecondsAfterFinished": 600, "template": {"metadata": {"labels": {"ore.dev/rol": "driver", "ore.dev/tenant": celda}}, "spec": {
            "restartPolicy": "Never", "serviceAccountName": "driver",
            "initContainers": [{"name": "puesto", "image": img, "command": ["/bin/sh", "-c"],
                                "env": [{"name": "HOME", "value": "/tmp"}, {"name": "CLOUDSDK_CONFIG", "value": "/tmp/.gcloud"}, {"name": "CLOUDSDK_CORE_PROJECT", "value": PROYECTO}],
                                "args": ["set -e\ngcloud secrets versions access latest --secret=%s-forja-token --out-file=/puesto/forja\nchmod 0400 /puesto/*\n" % ns],
                                "resources": {"requests": {"cpu": "100m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                                "volumeMounts": [{"name": "puesto", "mountPath": "/puesto"}]}],
            "containers": [{"name": "empujar", "image": img, "command": ["/bin/sh", "/guion/empujar.sh"],
                            "env": [{"name": "CELDA", "value": celda}, {"name": "HOME", "value": "/tmp"}, {"name": "SECO", "value": "1" if SECO else ""},
                                    {"name": "QUE", "value": "0031 W3.7 gobierno 2b: la capa por su nombre"}],
                            "resources": {"requests": {"cpu": "100m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                            "volumeMounts": [{"name": "puesto", "mountPath": "/puesto", "readOnly": True}, {"name": "guion", "mountPath": "/guion"}, {"name": "plantillas", "mountPath": "/plantillas"}]}],
            "volumes": [{"name": "puesto", "emptyDir": {"medium": "Memory"}}, {"name": "guion", "configMap": {"name": nombre, "defaultMode": 0o555}}, {"name": "plantillas", "configMap": {"name": nombre + "-plantillas"}}],
        }}},
    }


def main():
    celdas = [a for a in sys.argv[1:] if not a.startswith("--")]
    if not celdas:
        print(__doc__); sys.exit(2)
    for celda in celdas:
        ns = "t-" + celda
        nombre = "empujar-plantilla"
        d = tempfile.mkdtemp(prefix="ore-plantilla-").replace("\\", "/")
        r = subprocess.run([sys.executable, RAIZ + "/malla/gen-inquilino.py", celda, "--a", d], capture_output=True, text=True)
        if r.returncode:
            print("  no se pudo rendir", celda, r.stderr[-300:]); continue
        datos = {f: open(os.path.join(d, f), encoding="utf-8").read() for f in FICHEROS if os.path.exists(os.path.join(d, f))}
        print("  %s · %s" % (celda, ", ".join("%s (%d B)" % (f, len(t)) for f, t in datos.items())))
        kubectl("apply", "-f", "-", entrada=json.dumps({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": nombre, "namespace": ns}, "data": {"empujar.sh": GUION}}))
        kubectl("apply", "-f", "-", entrada=json.dumps({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": nombre + "-plantillas", "namespace": ns}, "data": datos}))
        kubectl("delete", "job", nombre, "-n", ns, "--ignore-not-found", "--wait=true")
        c, out, err = kubectl("apply", "-f", "-", entrada=json.dumps(job(ns, celda, nombre)))
        if c:
            print("  no se pudo crear el Job:", err[:300]); continue
        t0 = time.time()
        estado = "plazo"
        while time.time() - t0 < 300:
            c, out, _ = kubectl("get", "job", nombre, "-n", ns, "-o", "jsonpath={.status.succeeded}/{.status.failed}")
            ok, mal = (out.split("/") + [""])[:2]
            if ok == "1":
                estado = "ok"; break
            if mal and mal != "0":
                estado = "falló"; break
            time.sleep(4)
        c, log, _ = kubectl("logs", "job/" + nombre, "-n", ns, "-c", "empujar")
        print("  %s · %s · %s" % (celda, estado, (log.strip().splitlines() or ["(sin salida)"])[-1][:160]))
        kubectl("delete", "job", nombre, "-n", ns, "--ignore-not-found")
        kubectl("delete", "cm", nombre, nombre + "-plantillas", "-n", ns, "--ignore-not-found")


if __name__ == "__main__":
    main()
