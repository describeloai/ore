"""ORE 0072 F2·3 · Lanza el banco en victor (`banco_victor.py`): un Job de t-victor (vara.job, la credencial del
agente) con `ore-teselas` de al lado —contenedor nativo de al lado (initContainer con `restartPolicy: Always`): vive
mientras la carga y muere con ella—, de la imagen de `ore-serve` en `main` (o `BANCO_IMAGEN`) y con los MISMOS
límites que la malla (`malla/59-las-teselas.yaml`: 1 CPU, 2 GiB; pide 50m/128Mi) y en el mismo pool (system).
Deja el registro en pruebas-de-fuego/geo/f2/banco-<fecha>.log.

  python pruebas-de-fuego/geo/f2/banco.py                        # todo
  python pruebas-de-fuego/geo/f2/banco.py BANCO_EDIFICIOS=0      # sin escribir los edificios
  python pruebas-de-fuego/geo/f2/banco.py BANCO_BINARIO=<ruta>   # el ore-teselas de la rama (musl estático)
  python pruebas-de-fuego/geo/f2/banco.py ORE_TESELAS_CACHE_MB=0 # al motor: sin caché de páginas, todo en frío

Con `BANCO_BINARIO`, el motor de al lado corre en la imagen del puesto y espera a que el lanzador le copie el
binario (`kubectl cp` al volumen compartido): la rama se mide sin imagen ni `main`.
"""
import importlib.util
import json
import os
import subprocess
import sys
import threading
import time

S = os.path.dirname(os.path.abspath(__file__)) + os.sep
G = os.path.dirname(S.rstrip(os.sep)) + os.sep
spec = importlib.util.spec_from_file_location("vara", G + "vara.py")
vara = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vara)

IMAGEN = os.environ.get("BANCO_IMAGEN", "europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/ore-serve:main")
env = dict(a.split("=", 1) for a in sys.argv[1:] if "=" in a)
BINARIO = env.pop("BANCO_BINARIO", None)
# lo que es del motor (`ORE_TESELAS_CACHE_MB=0`: todo en frío), al motor
MOTOR = {k: env.pop(k) for k in list(env) if k.startswith("ORE_TESELAS_")}
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
            "env": [{"name": "ORE_TESELAS_PUERTO", "value": "8099"}] + [{"name": k, "value": v} for k, v in MOTOR.items()],
            "restartPolicy": "Always",
            "resources": {"requests": {"cpu": "50m", "memory": "128Mi"}, "limits": {"cpu": "1", "memory": "2Gi"}},
            "securityContext": {"allowPrivilegeEscalation": False, "readOnlyRootFilesystem": True,
                                "runAsNonRoot": True, "runAsUser": 65532, "capabilities": {"drop": ["ALL"]}},
        })
        if BINARIO:
            m = s["initContainers"][-1]
            m["image"] = vara.PUESTO
            m["command"] = ["sh", "-c", "until [ -x /banco/ore-teselas ]; do sleep 1; done; exec /banco/ore-teselas"]
            s.setdefault("volumes", []).append({"name": "banco", "emptyDir": {}})
            for k in (m, s["containers"][0]):
                k.setdefault("volumeMounts", []).append({"name": "banco", "mountPath": "/banco"})
            threading.Thread(target=copiar, args=(d["metadata"]["name"],), daemon=True).start()
        entrada = json.dumps(d)
    return _sh(args, entrada)


def copiar(nombre):
    """El binario de la rama al volumen del pod, en cuanto el generador corre (el motor lo espera)."""
    for _ in range(120):
        _, o = _sh(["kubectl", "-n", vara.NS, "get", "pods", "-l", "job-name=" + nombre, "-o",
                    "jsonpath={.items[0].metadata.name} {.items[0].status.containerStatuses[0].state.running}"])
        if "startedAt" in o:
            pod = o.split()[0]
            break
        time.sleep(3)
    else:
        print("## falla: el pod no arrancó para copiar el binario")
        return
    # `kubectl cp` lee `C:` como un pod: desde su carpeta, por el nombre
    c = subprocess.run(["kubectl", "-n", vara.NS, "cp", os.path.basename(BINARIO), pod + ":/banco/.t", "-c", "medir"],
                       capture_output=True, text=True, cwd=os.path.dirname(os.path.abspath(BINARIO)))
    c2 = subprocess.run(["kubectl", "-n", vara.NS, "exec", pod, "-c", "medir", "--", "sh", "-c",
                         "chmod 755 /banco/.t && mv /banco/.t /banco/ore-teselas"], capture_output=True, text=True)
    print("## binario copiado" if c.returncode == 0 and c2.returncode == 0 else
          "## falla al copiar: " + (c.stderr + c2.stderr)[-300:])


vara.sh = sh
fecha = time.strftime("%Y%m%d-%H%M")
nombre = "f2-banco-%d" % (int(time.time()) % 10**6)
log = vara.job(nombre, ["python3", "-u", "-c", open(S + "banco_victor.py", encoding="utf-8").read()],
               imagen=vara.PUESTO, plazo=3600)
ruta = S + "banco-%s.log" % fecha
open(ruta, "w", encoding="utf-8", newline="\n").write(log)
print("\n".join(l for l in log.splitlines() if l.startswith(("BANCO", "##")) or "Error" in l or "Traceback" in l))
print("registro:", ruta)
