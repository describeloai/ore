"""ORE 0072 I2·4 · el Job del índice en victor, como lo lanzará producción: la plantilla de la malla
(`malla/59-la-piramide.yaml`, rendida por `gen-inquilino.py victor`) con `"tarea": "indice"`, la cuenta `driver`
(su identidad, sin credenciales en la petición), `cola-pesada` y su disco, y la imagen de la rama
(`ore-piramide:<etiqueta>`). Vigila Kueue, el nodo y los desalojos; relanza para ver que sale en seco; limpia.

  python pruebas-de-fuego/geo/i2/banco_indice.py ETIQUETA=banco-i24 [CASOS=trip_30m,trip_600m] [LIMPIAR=1]
  … RECURSOS=7,40Gi,46Gi      cpu, memoria y disco del contenedor (por defecto, los de la plantilla)
  … TRAMO=25000000            filas por tramo (`limites.filas_por_tramo`)

Deja el registro en i2/banco-indice-<fecha>.log.
"""
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time

sys.stdout.reconfigure(encoding="utf-8")

S = os.path.dirname(os.path.abspath(__file__)) + os.sep
RAIZ = os.path.abspath(S + "../../..") + os.sep
REG = "europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/ore-piramide"
NS = "t-victor"
LAGO = "gs://project-8853a180-450d-47be-b83-t-victor-copia/ore/v2/"
env = dict(a.split("=", 1) for a in sys.argv[1:] if "=" in a)
ETIQUETA = env["ETIQUETA"]
LIMPIAR = env.get("LIMPIAR", "1") == "1"
RECURSOS = env.get("RECURSOS")
TRAMO = env.get("TRAMO")  # limites.filas_por_tramo (por defecto, el del Job)
# nombre → (metadata_location, geometría, columnas, destino del índice: la ubicación de la tabla, o una de prueba)
TODOS = {
    "trip_30m": (LAGO + "catalogo/query/default/trip_30m/metadata/%s", "t_pickup",
                 ["t_tripkey", "t_fare", "t_pickuptime"], LAGO + "catalogo/query/default/trip_30m"),
    "trip_600m": ("gs://project-8853a180-450d-47be-b83-banco-geo/i2/tablas/trip_600m/metadata/"
                  "00001-47c15f2d-8e25-4701-8bfd-bedae7deae58.metadata.json", "t_pickuploc",
                  ["t_tripkey", "t_fare", "t_pickuptime"], LAGO + "banco-i2/trip_600m"),
}
CASOS = env.get("CASOS", "trip_30m,trip_600m").split(",")
LOG = open(S + time.strftime("banco-indice-%Y%m%d-%H%M.log"), "w", encoding="utf-8")


def di(*a):
    t = " ".join(str(x) for x in a)
    print(t, flush=True)
    LOG.write(t + "\n")
    LOG.flush()


def sh(c, ok=True):
    p = subprocess.run(c, shell=True, capture_output=True, text=True, encoding="utf-8")
    if ok and p.returncode != 0:
        raise SystemExit("✗ %s\n%s" % (c, p.stderr[-800:]))
    return p.stdout


def ultima(tabla_metadata):
    """El metadata.json más nuevo de una tabla (el `%s` de la ubicación)."""
    d = tabla_metadata.rsplit("/", 1)[0]
    return sorted(l for l in sh('gcloud storage ls "%s/"' % d).split() if l.endswith(".metadata.json"))[-1]


def rendir(plantilla, nombre, peticion):
    """Lo de `cola::rendir_piramide`, con un nombre de Job propio del banco."""
    modelo = "value: '{\"modelo\":\"piramide\"}'"
    assert "piramide-00000000" in plantilla and modelo in plantilla
    t = plantilla.replace(modelo, "value: '%s'" % peticion)
    if RECURSOS:
        cpu, mem, disco = RECURSOS.split(",")
        t = t.replace('requests: {cpu: "2", memory: 4Gi, ephemeral-storage: 48Gi}',
                      'requests: {cpu: "%s", memory: %s, ephemeral-storage: 48Gi}' % (cpu, mem))
        t = t.replace('limits:   {cpu: "4", memory: 8Gi, ephemeral-storage: 48Gi}',
                      'limits:   {cpu: "%s", memory: %s, ephemeral-storage: 48Gi}' % (cpu, mem))
        t = t.replace("emptyDir: {sizeLimit: 40Gi}", "emptyDir: {sizeLimit: %s}" % disco)
    h = hashlib.sha256(t.encode()).hexdigest()[:8]
    return t.replace("piramide-00000000", "indice-%s-%s" % (nombre.replace("_", "-"), h))


def estado(j):
    s = json.loads(sh("kubectl -n %s get job %s -o json" % (NS, j)))["status"]
    if s.get("succeeded"):
        return "hecho"
    if any(c["type"] == "Failed" and c["status"] == "True" for c in s.get("conditions", [])):
        return "fallido"
    return "corriendo" if s.get("active") else "esperando"


def vigilar(j, desde):
    visto, t0, pico, ultimas, disco = None, time.time(), {}, [], {}
    while True:
        e = estado(j)
        pod = sh("kubectl -n %s get pods -l job-name=%s -o jsonpath={.items[*].metadata.name}" % (NS, j), ok=False).strip()
        nodo = sh("kubectl -n %s get pods -l job-name=%s -o jsonpath={.items[*].spec.nodeName}" % (NS, j), ok=False).strip()
        if (e, nodo) != visto:
            di("  %5.0f s · %s · %s · %s" % (time.time() - t0, j, e, nodo or "-"))
            visto = (e, nodo)
        if pod and e == "corriendo":
            top = sh("kubectl -n %s top pod %s --no-headers" % (NS, pod.split()[0]), ok=False).split()
            if len(top) >= 3:
                pico["cpu"] = max(pico.get("cpu", 0), int(top[1].rstrip("m") or 0))
                pico["mem"] = max(pico.get("mem", 0), int(top[2].rstrip("Mi") or 0))
            # lo que el Job lleva dicho y lo que ocupa su disco: si lo desalojan, sus logs se van con él
            l = [x for x in sh("kubectl -n %s logs %s" % (NS, pod.split()[0]), ok=False).splitlines()
                 if x.startswith("INDICE|")]
            for x in l[len(ultimas):]:
                di("    · %5.0f s · %s" % (time.time() - t0, x[:300]))
            ultimas = l if len(l) >= len(ultimas) else ultimas
            du = sh("kubectl -n %s exec %s -- du -sm /trabajo" % (NS, pod.split()[0]), ok=False).split()
            if du and du[0].isdigit():
                mb = int(du[0])
                if mb // 4096 != disco.get("ultimo", -1):
                    di("    · %5.0f s · /trabajo %.1f GiB" % (time.time() - t0, mb / 1024))
                    disco["ultimo"] = mb // 4096
                pico["disco"] = max(pico.get("disco", 0), mb)
        if nodo:
            c = json.loads(sh("kubectl get node %s -o json" % nodo, ok=False) or "{}")
            for cond in c.get("status", {}).get("conditions", []):
                if cond["type"] in ("DiskPressure", "MemoryPressure") and cond["status"] == "True":
                    di("  ✗ %s · %s" % (nodo, cond["type"]))
        if e in ("hecho", "fallido"):
            break
        time.sleep(10)
    ev = sh("kubectl get events -A --field-selector reason=Evicted -o json", ok=False)
    desalojos = [x for x in json.loads(ev or '{"items":[]}')["items"]
                 if (x.get("lastTimestamp") or x.get("eventTime") or "") >= desde]
    di("  pico (kubectl top): cpu %sm · memoria %s Mi · /trabajo %s MiB · desalojos en el clúster: %d"
       % (pico.get("cpu", "?"), pico.get("mem", "?"), pico.get("disco", "?"), len(desalojos)))
    for x in desalojos:
        di("    ✗ %s/%s · %s" % (x["involvedObject"].get("namespace"), x["involvedObject"]["name"], x["message"][:200]))
    if e == "fallido":
        d2 = sh("kubectl -n %s describe pods -l job-name=%s" % (NS, j), ok=False)
        for l in d2.splitlines():
            if any(k in l for k in ("Reason", "Exit Code", "Message", "OOM", "ephemeral")):
                di("    " + l.strip()[:200])
    return e, time.time() - t0


def lineas(j):
    return [l for l in sh("kubectl -n %s logs job/%s" % (NS, j), ok=False).splitlines() if l.startswith("INDICE|")]


def main():
    imagen = "%s:%s" % (REG, ETIQUETA)
    di("# banco del Job del índice (0072 I2·4) · %s · %s · recursos %s · tramo %s"
       % (time.strftime("%Y-%m-%d %H:%M"), imagen, RECURSOS or "los de la plantilla", TRAMO or "el del Job"))
    sh("docker tag ore-piramide:local %s" % imagen)
    sh("docker push -q %s" % imagen)
    di("imagen empujada")
    d = tempfile.mkdtemp()
    sh('python "%smalla/gen-inquilino.py" victor --a "%s"' % (RAIZ, d))
    plantilla = open(os.path.join(d, "plantilla-piramide.txt"), encoding="utf-8").read()
    plantilla = plantilla.replace("%s:main" % REG, imagen)
    destinos = []
    for n in CASOS:
        ml, geometria, columnas, destino = TODOS[n]
        if "%s" in ml:
            ml = ultima(ml)
        pet = json.dumps({"tarea": "indice", "capa": "banco.%s" % n, "suelo": 13, "trabajo": "/trabajo",
                          "fuente": {"metadata_location": ml, "geometria": geometria,
                                     "crs": "EPSG:4326", "columnas": columnas},
                          "destino": {"ubicacion": destino},
                          **({"limites": {"filas_por_tramo": int(TRAMO)}} if TRAMO else {})},
                         sort_keys=True, separators=(",", ":"))
        t = rendir(plantilla, n, pet)
        f = os.path.join(d, "indice-%s.yaml" % n)
        open(f, "w", encoding="utf-8").write(t)
        j = next(l.split("name: ", 1)[1] for l in t.splitlines() if l.startswith("  name: indice-"))
        destinos.append(destino + "/_ore_capa/")
        di("\n## %s · %s · %s" % (n, j, ml.rsplit("/", 1)[-1]))
        desde = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        sh('kubectl apply -f "%s"' % f)
        e, s = vigilar(j, desde)
        di("  %s en %.0f s" % (e, s))
        for l in lineas(j):
            di("  " + l[:600])
        sh("kubectl -n %s delete job %s --wait=true" % (NS, j), ok=False)
        if e == "hecho":
            # lo que hace Flux al caducar el TTL: el mismo Job otra vez. Sale en seco.
            sh('kubectl apply -f "%s"' % f)
            e2, s2 = vigilar(j, desde)
            ya = [l for l in lineas(j) if l.startswith("INDICE|ya|")]
            di("  relanzado: %s en %.0f s · %s" % (e2, s2, "en seco ✓" if ya else "✗ no salió en seco"))
            sh("kubectl -n %s delete job %s --wait=false" % (NS, j), ok=False)
    if LIMPIAR:
        for u in destinos:
            sh('gcloud storage rm -r "%s"' % u, ok=False)
        sh("gcloud artifacts docker images delete %s --quiet" % imagen, ok=False)
        di("\nlimpio: Jobs, índices (%s) y la etiqueta %s" % (", ".join(destinos), ETIQUETA))


main()
