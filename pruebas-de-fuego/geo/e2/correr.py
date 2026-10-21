"""ORE 0072 E2·0 · Lanza un previo (previo_a.py, …) en un Job de t-victor con la imagen del puesto y la
credencial del agente (como g2-victor.py). Deja el registro en pruebas-de-fuego/geo/e2/<previo>-<fecha>.log.

  python pruebas-de-fuego/geo/e2/correr.py previo_a --ensayo     # N pequeño, en el pool de sistema
  python pruebas-de-fuego/geo/e2/correr.py previo_a              # la medida, en un nodo del pool pesado
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
ensayo = "--ensayo" in sys.argv
guion = open(S + previo + ".py", encoding="utf-8").read()
env = {"E2_N": "200000", "E2_N_EDIF": "20000"} if ensayo else {}
env.update(a.split("=", 1) for a in sys.argv[2:] if "=" in a)  # K=V: al entorno del Job
pesado = not ensayo

# vara.job con dos cambios: el entorno del previo y, en la medida, un nodo del pool pesado
# (8 vCPU, 64 GiB, spot): no medir en el nodo de sistema, donde vive ore-serve.
_sh = vara.sh


def sh(args, entrada=None):
    if args[:2] == ["kubectl", "create"] and entrada:
        d = json.loads(entrada)
        s = d["spec"]["template"]["spec"]
        k = s["containers"][0]
        k["env"] += [{"name": n, "value": v} for n, v in env.items()]
        if pesado:
            # Kueue admite el Job en `cola-pesada` (cq-<celda>-pesado: 7 CPU, 48 GiB) y su sabor lo
            # lleva al pool pesado; en `cola` (la de la base) se queda suspendido.
            d["metadata"].setdefault("labels", {})["kueue.x-k8s.io/queue-name"] = "cola-pesada"
            s.pop("nodeSelector", None)  # el de la base (system) lo pone el sabor: pesado
            k["resources"] = {"requests": {"cpu": "7", "memory": "40Gi", "ephemeral-storage": "20Gi"},
                              "limits": {"cpu": "8", "memory": "44Gi", "ephemeral-storage": "40Gi"}}
        entrada = json.dumps(d)
    return _sh(args, entrada)


vara.sh = sh
fecha = time.strftime("%Y%m%d-%H%M")
nombre = f"e2-{previo.replace('_', '-')}-{int(time.time()) % 10**6}"
log = vara.job(nombre, ["python3", "-u", "-c", guion], imagen=vara.PUESTO, plazo=5400 if pesado else 1200,
               memoria="2Gi")
ruta = S + f"{previo}-{fecha}{'-ensayo' if ensayo else ''}.log"
open(ruta, "w", encoding="utf-8", newline="\n").write(log)
print("\n".join(l for l in log.splitlines() if l.startswith(("E2", "##")) or "Error" in l or "Traceback" in l))
print("registro:", ruta)
