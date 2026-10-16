"""LA VARA GEO (ADR 0067 G0·3): lo que es del formato geo, medido en victor, con un informe por fecha.

Cada caso dice qué paso del plan de 0067 lo pone en verde. Corre contra victor (la memoria «medir en
prod»), en Jobs del inquilino: las URLs de las fuentes salen del cofre dentro del Job y no tocan este PC.

  V1 · declarar una columna geo y compilarla                      → G1
  V2 · la inducción sobre las fuentes del banco                   → G1·4
  V3 · escribir N geometrías al lago (filas/s, MB, las marcas)     → G2·1
  V4 · leer por zona: descarte y latencia con 1, 8 y 32 lectores   → G2·2
  V5 · la matriz de adopción del fichero que escribe ORE           → G2·5
  V6 · abrir una tabla Iceberg v3 ajena con `geometry`             → G2·4
  V7 · cómo llega la geometría de cada origen por su driver        → G3

Los casos que corren en la imagen de drivers (V1, V2, V7) van en `vara-celda.sh`; V4 en `vara-v4.py`
con la imagen del puesto. V3, V5 y V6 no tienen hoy nada que medir (ORE no escribe geometría y la tabla
v3 ajena está pendiente): se dicen en rojo con su motivo, y se miden en cuanto su paso exista.

Uso:  python pruebas-de-fuego/geo/vara.py      deja el informe en pruebas-de-fuego/geo/vara-<fecha>.md
"""
import json
import os
import subprocess
import sys
import time

S = os.path.dirname(os.path.abspath(__file__)) + os.sep
SALUD = os.path.join(os.path.dirname(S.rstrip(os.sep)), "salud") + os.sep
NS = "t-victor"
PUESTO = "europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/puesto-python:1"


def sh(args, entrada=None):
    p = subprocess.run(args, input=entrada, capture_output=True, text=True, encoding="utf-8")
    return p.returncode, (p.stdout or "") + (p.stderr or "")


def job(nombre, orden, imagen=None, cuenta=None, plazo=1800, memoria="3Gi"):
    """Un Job de medida en victor (la base de las medidas de salud), y su registro al acabar."""
    d = json.load(open(SALUD + "job-base.json", encoding="utf-8"))
    d["metadata"]["name"] = nombre
    s = d["spec"]["template"]["spec"]
    k = s["containers"][0]
    k["command"] = orden
    k["imagePullPolicy"] = "Always"
    k["env"] = [e for e in k["env"] if e["name"] not in ("NS", "PYTHONUNBUFFERED")] + [
        {"name": "NS", "value": NS}, {"name": "PYTHONUNBUFFERED", "value": "1"}]
    # La base es de 256 MiB: una lectura entera en Arrow de BigQuery no cabe (el Job muere sin decirlo).
    k["resources"] = {"requests": {"cpu": "1", "memory": memoria}, "limits": {"cpu": "4", "memory": memoria}}
    if imagen:
        k["image"] = imagen
    if cuenta:
        # Otra cuenta no trae la credencial del agente: sin el contenedor que la pone ni su volumen.
        s["serviceAccountName"] = cuenta
        s.pop("initContainers", None)
        k.pop("volumeMounts", None)
        s.pop("volumes", None)
    d["spec"]["activeDeadlineSeconds"] = plazo
    d["spec"]["backoffLimit"] = 0
    c, o = sh(["kubectl", "create", "-f", "-"], json.dumps(d))
    if c != 0:
        return "## falla: kubectl create: " + o[-300:]
    fin = time.time() + plazo + 120
    while time.time() < fin:
        _, o = sh(["kubectl", "-n", NS, "get", "job", nombre, "-o", "jsonpath={.status.succeeded}/{.status.failed}"])
        if o.startswith("1/") or o.endswith("/1"):
            break
        time.sleep(10)
    _, log = sh(["kubectl", "-n", NS, "logs", "job/" + nombre, "-c", k["name"]])
    _, estado = sh(["kubectl", "-n", NS, "get", "job", nombre, "-o", "jsonpath={.status.succeeded}/{.status.failed}"])
    if not estado.startswith("1/"):
        _, d2 = sh(["kubectl", "-n", NS, "describe", "pods", "-l", "job-name=" + nombre])
        log += "\n## falla: el Job no acabó bien (" + estado + ")\n" + "\n".join(
            l for l in d2.splitlines() if any(x in l for x in ("Reason", "Exit Code", "State", "Message", "OOM", "Error")))
    sh(["kubectl", "-n", NS, "delete", "job", nombre, "--wait=false"])
    return log


def casos(log, prefijo):
    return [l.split("|", 4) for l in log.splitlines() if l.startswith(prefijo + "|")]


def main():
    fecha = time.strftime("%Y%m%d-%H%M")
    marca = int(time.time()) % 10 ** 6
    _, commit = sh(["kubectl", "-n", NS, "get", "deploy", "ore-serve", "-o",
                    "jsonpath={.spec.template.metadata.annotations.ore\\.dev/commit}"])
    print(f"vara geo {fecha} · ore-serve en {commit[:8]}", flush=True)
    t0 = time.time()
    guion = open(S + "vara-celda.sh", encoding="utf-8").read().replace(
        "__V2_JUZGAR__", open(S + "v2-juzgar.py", encoding="utf-8").read())
    celda = job(f"vara-geo-celda-{marca}", ["/bin/sh", "-c", guion])
    print("  celda hecha", flush=True)
    v4 = job(f"vara-geo-v4-{marca}", ["python3", "-c", open(S + "vara-v4.py", encoding="utf-8").read()],
             imagen=PUESTO, cuenta="puesto")
    print("  V4 hecho", flush=True)
    log = celda + "\n" + v4
    filas = {v: casos(log, v) for v in ("V1", "V2", "V3", "V4", "V5", "V6", "V7")}
    hoy = {
        "V3": "rojo · ORE no escribe geometría: no hay tipo (G1) ni escritor geo (G2·1)",
        "V5": "rojo · no hay fichero geo escrito por ORE que dar a leer (G2·1)",
        "V6": "rojo · falta la tabla del banco: pyiceberg 0.12 tiene el tipo pero no escribe v3; "
              "se escribe con Spark + Iceberg Java (G2·4). ORE no tiene aún cómo abrirla",
    }
    o = [f"# Vara geo · {fecha}", "",
         f"ADR 0067 G0·3. ore-serve en `{commit[:8]}`. {time.time() - t0:.0f} s.", ""]
    for v, titulo, paso in (("V1", "Declarar una columna geo", "G1"), ("V2", "La inducción", "G1·4"),
                            ("V3", "Escribir geometrías al lago", "G2·1"), ("V4", "Leer por zona", "G2·2"),
                            ("V5", "La matriz de adopción", "G2·5"), ("V6", "Una tabla Iceberg v3 ajena", "G2·4"),
                            ("V7", "La geometría de cada origen, por su driver", "G3")):
        o += [f"## {v} · {titulo} (lo pone en verde {paso})", ""]
        if v in hoy:
            o += [hoy[v], ""]
        if filas[v]:
            o += ["| fuente | objeto | columna | lo visto |", "|---|---|---|---|"]
            o += [f"| {a} | {b} | {c} | {d.replace('|', '/')} |" for _, a, b, c, d in [f + [""] * (5 - len(f)) for f in filas[v]]]
            o += [""]
        elif v not in hoy:
            o += ["sin líneas: mirar el registro del Job", ""]
    fallos = [l for l in log.splitlines() if "## falla" in l or "Traceback" in l or "Error" in l]
    if fallos:
        o += ["## Fallos del guion", ""] + [f"- `{l[:200]}`" for l in fallos] + [""]
    ruta = S + f"vara-{fecha}.md"
    open(ruta, "w", encoding="utf-8", newline="\n").write("\n".join(o))
    open(S + f"vara-{fecha}.log", "w", encoding="utf-8", newline="\n").write(log)
    print("informe:", ruta)


if __name__ == "__main__":
    sys.exit(main())
