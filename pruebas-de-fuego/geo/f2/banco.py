"""ORE 0072 F2·3 · Lanza el banco en victor (`banco_victor.py`): un Job de t-victor (vara.job, la credencial del
agente) con `ore-teselas` de al lado —contenedor nativo de al lado (initContainer con `restartPolicy: Always`): vive
mientras la carga y muere con ella—, de la imagen de `ore-serve` en `main` (o `BANCO_IMAGEN`) y con los MISMOS
límites que la malla (`malla/59-las-teselas.yaml`: 1 CPU, 2 GiB; pide 50m/128Mi) y en el mismo pool (system).
Deja el registro en pruebas-de-fuego/geo/f2/banco-<fecha>.log.

  python pruebas-de-fuego/geo/f2/banco.py                        # todo
  python pruebas-de-fuego/geo/f2/banco.py BANCO_EDIFICIOS=0      # sin escribir los edificios
"""
import importlib.util
import json
import os
import sys
import time

S = os.path.dirname(os.path.abspath(__file__)) + os.sep
G = os.path.dirname(S.rstrip(os.sep)) + os.sep
spec = importlib.util.spec_from_file_location("vara", G + "vara.py")
vara = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vara)

IMAGEN = os.environ.get("BANCO_IMAGEN", "europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/ore-serve:main")
env = dict(a.split("=", 1) for a in sys.argv[1:] if "=" in a)
_sh = vara.sh


def sh(args, entrada=None):
    if args[:2] == ["kubectl", "create"] and entrada:
        d = json.loads(entrada)
        s = d["spec"]["template"]["spec"]
        s["containers"][0]["env"] += [{"name": n, "value": v} for n, v in env.items()]
        # la cola del pool system admite 1 CPU y 4 GiB en total: el generador pide poco y el motor, lo de la malla
        s["containers"][0]["resources"] = {"requests": {"cpu": "500m", "memory": "3584Mi"},
                                           "limits": {"cpu": "2", "memory": "6Gi"}}
        s.setdefault("initContainers", []).append({
            "name": "ore-teselas",
            "image": IMAGEN,
            "imagePullPolicy": "Always",
            "command": ["/usr/local/bin/ore-teselas"],
            "env": [{"name": "ORE_TESELAS_PUERTO", "value": "8099"}],
            "restartPolicy": "Always",
            "resources": {"requests": {"cpu": "50m", "memory": "128Mi"}, "limits": {"cpu": "1", "memory": "2Gi"}},
            "securityContext": {"allowPrivilegeEscalation": False, "readOnlyRootFilesystem": True,
                                "runAsNonRoot": True, "runAsUser": 65532, "capabilities": {"drop": ["ALL"]}},
        })
        entrada = json.dumps(d)
    return _sh(args, entrada)


vara.sh = sh
fecha = time.strftime("%Y%m%d-%H%M")
nombre = "f2-banco-%d" % (int(time.time()) % 10**6)
log = vara.job(nombre, ["python3", "-u", "-c", open(S + "banco_victor.py", encoding="utf-8").read()],
               imagen=vara.PUESTO, plazo=3600)
ruta = S + "banco-%s.log" % fecha
open(ruta, "w", encoding="utf-8", newline="\n").write(log)
print("\n".join(l for l in log.splitlines() if l.startswith(("BANCO", "##")) or "Error" in l or "Traceback" in l))
print("registro:", ruta)
