"""ORE 0072 I4·0 · lanza el previo de I4 (previo.py) en un Job de t-victor, en un nodo del pool pesado, con la
imagen del puesto y la credencial del agente, y un `emptyDir` de 46 GiB en `/trabajo` (como el Job de la
pirámide). Deja el registro en pruebas-de-fuego/geo/i4/<previo>-<fecha>.log.

  python pruebas-de-fuego/geo/i4/correr.py previo [I40_FILAS=3000000]
  python pruebas-de-fuego/geo/i4/correr.py victor I46_BINARIOS=gs://…/i4/bin/<marca>/      (I4·6)

Con `victor`, además, DuckDB con sus extensiones de la imagen del Job de la pirámide (`ore-piramide:main`), copiado
a `/herr` por un initContainer; los binarios de la rama los trae el guion del banco.
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

PIRAMIDE = "europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/ore-piramide:main"
previo = sys.argv[1]
guion = open(S + previo + ".py", encoding="utf-8").read()
env = dict(a.split("=", 1) for a in sys.argv[2:] if "=" in a)
_sh = vara.sh


def sh(args, entrada=None):
    if args[:2] == ["kubectl", "create"] and entrada:
        d = json.loads(entrada)
        s = d["spec"]["template"]["spec"]
        k = s["containers"][0]
        k["env"] += [{"name": n, "value": v} for n, v in env.items()]
        d["metadata"].setdefault("labels", {})["kueue.x-k8s.io/queue-name"] = "cola-pesada"
        s.pop("nodeSelector", None)
        s.setdefault("volumes", []).append({"name": "trabajo", "emptyDir": {"sizeLimit": "46Gi"}})
        k.setdefault("volumeMounts", []).append({"name": "trabajo", "mountPath": "/trabajo"})
        if previo == "victor":
            s["volumes"].append({"name": "herr", "emptyDir": {}})
            k["volumeMounts"].append({"name": "herr", "mountPath": "/herr"})
            s.setdefault("initContainers", []).append({
                "name": "duckdb", "image": PIRAMIDE, "imagePullPolicy": "Always",
                "command": ["cp", "-a", "/opt/duckdb", "/herr/"],
                "volumeMounts": [{"name": "herr", "mountPath": "/herr"}]})
        k["resources"] = {"requests": {"cpu": "7", "memory": "40Gi", "ephemeral-storage": "48Gi"},
                          "limits": {"cpu": "8", "memory": "44Gi", "ephemeral-storage": "48Gi"}}
        entrada = json.dumps(d)
    return _sh(args, entrada)


vara.sh = sh
fecha = time.strftime("%Y%m%d-%H%M")
nombre = f"i4-{previo.replace('_', '-')}-{int(time.time()) % 10**6}"
log = vara.job(nombre, ["python3", "-u", "-c", guion], imagen=vara.PUESTO.replace("puesto-python:", "puesto-python-notebooks:"), plazo=5400, memoria="2Gi")
ruta = S + f"{previo}-{fecha}.log"
open(ruta, "w", encoding="utf-8", newline="\n").write(log)
print("\n".join(l for l in log.splitlines() if l.startswith(("I40", "I46", "##")) or "Error" in l or "Traceback" in l))
print("registro:", ruta)
