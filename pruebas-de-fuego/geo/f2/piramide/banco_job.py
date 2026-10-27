"""ORE 0072 F2·3·2·4 · el banco del Job del sistema: la plantilla `malla/59-la-piramide.yaml` rendida para
victor (`gen-inquilino.py victor`) y con la petición que pone `ore-serve` (`cola::rendir_piramide`, aquí en
Python, letra por letra), lanzada tal cual en `t-victor` —la cuenta `driver`, `cola-pesada`, su disco— con la
imagen de la rama (`ore-piramide:<etiqueta>` en Artifact Registry). Vigila Kueue, el nodo (`DiskPressure`) y
los desalojos de todo el clúster mientras construye; después la relanza para ver que sale en seco (el
manifiesto ya está) y limpia: los Jobs, las pirámides del lago y la etiqueta de la imagen.

  python pruebas-de-fuego/geo/f2/piramide/banco_job.py ETIQUETA=banco-f2324 [CAPAS=nombre=gs://…metadata.json,…]
  python …/banco_job.py ETIQUETA=banco-f2324 LIMPIAR=0      # deja las pirámides (para mirarlas)

Deja el registro en piramide/banco-job-<fecha>.log.
"""
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time

S = os.path.dirname(os.path.abspath(__file__)) + os.sep
RAIZ = os.path.abspath(S + "../../../..") + os.sep
REG = "europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/ore-piramide"
NS = "t-victor"
LAGO = "gs://project-8853a180-450d-47be-b83-t-victor-copia/ore/v2/catalogo/query/default/"
env = dict(a.split("=", 1) for a in sys.argv[1:] if "=" in a)
ETIQUETA = env["ETIQUETA"]
LIMPIAR = env.get("LIMPIAR", "1") == "1"
# ID=id: la petición con la columna id (la de ore-serve no la lleva), a otro destino, para cotejar
ID = env.get("ID")
CAPAS = env.get("CAPAS") or ",".join(
    "%s=%s%s/metadata/%s" % (n, LAGO, n, m) for n, m in [
        ("trip_3m_puntos", "00000-d940fd1a-9b8f-42b1-a553-a773adc4a4c8.metadata.json"),
        ("trip_3m_lineas", "00000-a06aa467-c7df-4539-92ac-119bb445e864.metadata.json"),
    ])
SUELO = {"trip_3m_lineas": 14}
LOG = open(S + time.strftime("banco-job-%Y%m%d-%H%M.log"), "w", encoding="utf-8")


def di(*a):
    t = " ".join(str(x) for x in a)
    print(t, flush=True)
    LOG.write(t + "\n")
    LOG.flush()


def sh(c, ok=True, entrada=None):
    p = subprocess.run(c, shell=True, capture_output=True, text=True, input=entrada, encoding="utf-8")
    if ok and p.returncode != 0:
        raise SystemExit("✗ %s\n%s" % (c, p.stderr[-800:]))
    return p.stdout


def jcs(v):
    return json.dumps(v, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def ubicacion(ml):
    tabla, fichero = ml.rsplit("/metadata/", 1)
    return tabla, "%s/_ore_piramides/%s-r1.pmtiles" % (tabla, fichero.removesuffix(".metadata.json"))


def rendir(plantilla, tabla, peticion):
    """`cola::rendir_piramide`."""
    modelo = "value: '{\"modelo\":\"piramide\"}'"
    assert "piramide-00000000" in plantilla and modelo in plantilla
    t = plantilla.replace(modelo, "value: '%s'" % peticion)
    h = hashlib.sha256(t.encode()).hexdigest()[:8]
    r = hashlib.sha256(tabla.encode()).hexdigest()[:12]
    return "59-la-piramide-%s.yaml" % r, t.replace("piramide-00000000", "piramide-%s" % h)


def vigilar(jobs, desde):
    """Hasta que acaben: el estado de cada Job, el nodo donde cae y lo que pase en el clúster."""
    visto, nodos, t0 = {}, set(), time.time()
    while True:
        fin = True
        for j in jobs:
            s = json.loads(sh("kubectl -n %s get job %s -o json" % (NS, j)))["status"]
            falla = any(c["type"] == "Failed" and c["status"] == "True" for c in s.get("conditions", []))
            e = "hecho" if s.get("succeeded") else "fallido" if falla else \
                "corriendo" if s.get("active") else "esperando"
            pod = sh("kubectl -n %s get pods -l job-name=%s -o jsonpath={.items[*].spec.nodeName}" % (NS, j), ok=False).strip()
            if pod:
                nodos.update(pod.split())
            if visto.get(j) != (e, pod):
                di("  %5.0f s · %s · %s · %s" % (time.time() - t0, j, e, pod or "-"))
                visto[j] = (e, pod)
            fin &= e in ("hecho", "fallido")
        for n in nodos:
            c = json.loads(sh("kubectl get node %s -o json" % n, ok=False) or "{}")
            for cond in c.get("status", {}).get("conditions", []):
                if cond["type"] in ("DiskPressure", "MemoryPressure") and cond["status"] == "True":
                    di("  ✗ %s · %s" % (n, cond["type"]))
        if fin:
            break
        time.sleep(10)
    ev = sh("kubectl get events -A --field-selector reason=Evicted -o json", ok=False)
    desalojos = [x for x in json.loads(ev or '{"items":[]}')["items"]
                 if (x.get("lastTimestamp") or x.get("eventTime") or "") >= desde]
    di("  desalojos en el clúster desde %s: %d" % (desde, len(desalojos)))
    for x in desalojos:
        di("    ✗ %s/%s · %s" % (x["involvedObject"].get("namespace"), x["involvedObject"]["name"], x["message"][:160]))
    return nodos


def lineas(j):
    out = sh("kubectl -n %s logs job/%s" % (NS, j), ok=False)
    return [l for l in out.splitlines() if l.startswith("PIRAMIDE|")]


def main():
    imagen = "%s:%s" % (REG, ETIQUETA)
    di("# banco del Job de la pirámide (0072 F2·3·2·4) · %s · %s" % (time.strftime("%Y-%m-%d %H:%M"), imagen))
    sh("docker tag ore-piramide:local %s" % imagen)
    sh("docker push -q %s" % imagen)
    di("imagen empujada")
    d = tempfile.mkdtemp()
    sh('python "%smalla/gen-inquilino.py" victor --a "%s"' % (RAIZ, d))
    plantilla = open(os.path.join(d, "plantilla-piramide.txt"), encoding="utf-8").read()
    plantilla = plantilla.replace("%s:main" % REG, imagen)
    jobs, destinos = [], []
    for c in CAPAS.split(","):
        n, ml = c.split("=", 1)
        tabla, u = ubicacion(ml)
        fuente = {"crs": "EPSG:4326", "geometria": "geom", "metadata_location": ml}
        if ID:
            fuente["id"] = ID
            u = u.replace(".pmtiles", "-con-id.pmtiles")
        pet = jcs({"capa": n, "destino": {"ubicacion": u},
                   "fuente": fuente,
                   "limites": {"segundos": 3000}, "suelo": SUELO.get(n, 13), "trabajo": "/trabajo"})
        f, t = rendir(plantilla, tabla, pet)
        open(os.path.join(d, f), "w", encoding="utf-8").write(t)
        j = next(l.split("name: ", 1)[1] for l in t.splitlines() if l.startswith("  name: piramide-"))
        jobs.append(j)
        destinos.append(u)
        di("%s · %s · %s" % (n, f, j))
    desde = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    for f in os.listdir(d):
        if f.startswith("59-la-piramide-"):
            sh('kubectl apply -f "%s"' % os.path.join(d, f))
    t = time.time()
    nodos = vigilar(jobs, desde)
    di("construidas en %.0f s · nodos %s" % (time.time() - t, ", ".join(sorted(nodos))))
    for j in jobs:
        for l in lineas(j):
            di("  " + l)
    # ⭐ lo que hace Flux al caducar el TTL: el mismo Job otra vez. Sale en seco.
    for j in jobs:
        sh("kubectl -n %s delete job %s --wait=true" % (NS, j))
    t = time.time()
    for f in os.listdir(d):
        if f.startswith("59-la-piramide-"):
            sh('kubectl apply -f "%s"' % os.path.join(d, f))
    vigilar(jobs, desde)
    di("relanzadas en %.0f s" % (time.time() - t))
    for j in jobs:
        ya = [l for l in lineas(j) if l.startswith("PIRAMIDE|ya|")]
        di("  %s · %s" % (j, "en seco ✓" if ya else "✗ NO salió en seco: " + " / ".join(lineas(j))[:300]))
    for j in jobs:
        sh("kubectl -n %s delete job %s --wait=false" % (NS, j), ok=False)
    if LIMPIAR:
        for u in destinos:
            sh('gcloud storage rm "%s" "%s.json"' % (u, u), ok=False)
        sh("gcloud artifacts docker images delete %s --quiet" % imagen, ok=False)
        di("limpio: Jobs, pirámides y la etiqueta %s" % ETIQUETA)


main()
