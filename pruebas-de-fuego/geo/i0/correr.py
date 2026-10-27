"""ORE 0072 I0 · lanza un previo de I0 (construir.py, …) en un Job de t-victor, en un nodo del pool pesado, con la
imagen del puesto y la credencial del agente, y un `emptyDir` de 46 GiB en `/trabajo` (como el Job de la
pirámide). Deja el registro en pruebas-de-fuego/geo/i0/<previo>-<fecha>.log.

  python pruebas-de-fuego/geo/i0/correr.py construir [I0_TAMANOS=1000000]
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
        k["resources"] = {"requests": {"cpu": "7", "memory": "40Gi", "ephemeral-storage": "48Gi"},
                          "limits": {"cpu": "8", "memory": "44Gi", "ephemeral-storage": "48Gi"}}
        entrada = json.dumps(d)
    return _sh(args, entrada)


vara.sh = sh
fecha = time.strftime("%Y%m%d-%H%M")
nombre = f"i0-{previo.replace('_', '-')}-{int(time.time()) % 10**6}"
log = vara.job(nombre, ["python3", "-u", "-c", guion], imagen=vara.PUESTO, plazo=5400, memoria="2Gi")
ruta = S + f"{previo}-{fecha}.log"
open(ruta, "w", encoding="utf-8", newline="\n").write(log)
print("\n".join(l for l in log.splitlines() if l.startswith(("I0", "##")) or "Error" in l or "Traceback" in l))
print("registro:", ruta)
