"""Simula la política de limpieza de Artifact Registry (ADR 0060 A2) sobre lo que hay.

    gcloud artifacts docker images list <registro>/<repo> --include-tags --format=json > lista.json
    ORE_TOKEN=$(gcloud auth print-access-token) python malla/simular-limpieza.py malla/registro-limpieza.json lista.json         [--dentro-de 8] [--alias 1=en-uso] [--manifiestos malla]

`--dentro-de N`: cómo quedaría dentro de N días sin construir nada más (lo que hoy salva solo
`olderThan`, se va). `--alias 1=en-uso`: como si la versión con la etiqueta `1` llevara también
`en-uso` (lo que el despliegue pondrá; ADR 0060 A2). `--manifiestos DIR`: comprueba que cada
imagen de este repositorio que nombra un fichero de DIR —todos, estén o no en una
kustomization: el proxy de 0058 o lo comentado también— se queda; si no, sale con error.

Aplica las reglas como las aplica Google (docs de cleanup policies, 2026-10-10): una versión que
casa con una Keep se queda aunque case con la Delete; `mostRecentVersions` guarda las N más
nuevas de cada paquete; y lo que un índice (multi-arquitectura) nombra no se borra mientras el
índice siga. Lo que no casa con ninguna Delete, se queda.

Lo que no dice la documentación se simula de las dos maneras y se avisa si cambia el resultado:
si `mostRecentVersions` cuenta también las versiones hijas de un índice.

El tamaño que quedaría se calcula como lo cobra el registro: cada capa una vez, aunque la usen
cien imágenes. Para eso lee los manifiestos de lo que se queda (el token va por la variable de
entorno y no se escribe en ningún sitio).
"""
import json
import os
import re
import sys
import urllib.request
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta, timezone

INDICES = ("application/vnd.oci.image.index.v1+json", "application/vnd.docker.distribution.manifest.list.v2+json")
ACEPTA = ", ".join(INDICES + ("application/vnd.oci.image.manifest.v1+json", "application/vnd.docker.distribution.manifest.v2+json"))


def fecha(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00"))


def plazo(s):
    m = re.fullmatch(r"(\d+)([smhd])", s)
    n, u = int(m.group(1)), m.group(2)
    return timedelta(**{{"s": "seconds", "m": "minutes", "h": "hours", "d": "days"}[u]: n})


def manifiesto(paquete, digest, token):
    host, ruta = paquete.split("/", 1)
    req = urllib.request.Request(f"https://{host}/v2/{ruta}/manifests/{digest}",
                                 headers={"Authorization": f"Bearer {token}", "Accept": ACEPTA})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def casa(cond, v):
    tags = v["tags"]
    estado = cond.get("tagState", "any").lower()
    if estado == "tagged" and not tags:
        return False
    if estado == "untagged" and tags:
        return False
    if "tagPrefixes" in cond and not any(t.startswith(p) for t in tags for p in cond["tagPrefixes"]):
        return False
    if "packageNamePrefixes" in cond and not any(v["nombre"].startswith(p) for p in cond["packageNamePrefixes"]):
        return False
    if "versionNamePrefixes" in cond and not any(v["version"].startswith(p) for p in cond["versionNamePrefixes"]):
        return False
    edad = AHORA - v["creada"]
    if "olderThan" in cond and edad <= plazo(cond["olderThan"]):
        return False
    if "newerThan" in cond and edad >= plazo(cond["newerThan"]):
        return False
    return True


def simular(reglas, versiones, hijos, cuenta_hijas):
    por_paquete = defaultdict(list)
    todas_las_hijas = {h for hs in hijos.values() for h in hs}
    for v in versiones:
        if cuenta_hijas or (v["paquete"], v["version"]) not in todas_las_hijas:
            por_paquete[v["paquete"]].append(v)
    recientes = {}
    for r in reglas:
        if "mostRecentVersions" in r:
            m = r["mostRecentVersions"]
            for p, vs in por_paquete.items():
                if any(vs[0]["nombre"].startswith(x) for x in m.get("packageNamePrefixes", [""])):
                    for v in sorted(vs, key=lambda v: v["creada"], reverse=True)[: m["keepCount"]]:
                        recientes.setdefault((p, v["version"]), r["name"])
    queda, porque = set(), {}
    for v in versiones:
        k = (v["paquete"], v["version"])
        guarda = recientes.get(k) or next((r["name"] for r in reglas if r["action"]["type"] == "Keep"
                                             and "condition" in r and casa(r["condition"], v)), None)
        borra = any(r["action"]["type"] == "Delete" and casa(r["condition"], v) for r in reglas)
        if guarda or not borra:
            queda.add(k)
            porque[k] = guarda or "ninguna Delete"
    # Lo que nombra un índice que se queda, se queda.
    for k in list(queda):
        for h in hijos.get(k, ()):
            if h not in queda:
                queda.add(h)
                porque[h] = "hija de un índice"
    return queda, porque


def main():
    global AHORA
    args = sys.argv[1:]
    dias, alias, manifiestos = 0, {}, None
    while len(args) > 2:
        op, val = args.pop(2), args.pop(2)
        if op == "--dentro-de":
            dias = int(val)
        elif op == "--alias":
            a, b = val.split("=")
            alias[a] = b
        elif op == "--manifiestos":
            manifiestos = val
    reglas = json.load(open(args[0], encoding="utf-8"))
    crudo = json.load(open(args[1], encoding="utf-8"))
    token = os.environ["ORE_TOKEN"]
    AHORA = datetime.now(timezone.utc) + timedelta(days=dias)
    versiones = [{
        "paquete": e["package"], "nombre": e["package"].rsplit("/", 1)[1], "version": e["version"],
        "tags": (e.get("tags") or []) + [alias[t] for t in (e.get("tags") or []) if t in alias], "creada": fecha(e["createTime"]),
        "tipo": e["metadata"].get("mediaType", ""),
    } for e in crudo]

    def leer(v):
        return (v["paquete"], v["version"]), manifiesto(v["paquete"], v["version"], token)

    with ThreadPoolExecutor(16) as ex:
        indices = dict(ex.map(leer, [v for v in versiones if v["tipo"] in INDICES]))
    hijos = {k: [(k[0], m["digest"]) for m in idx.get("manifests", [])] for k, idx in indices.items()}

    a, porque = simular(reglas, versiones, hijos, cuenta_hijas=True)
    b, _ = simular(reglas, versiones, hijos, cuenta_hijas=False)
    if a != b:
        print(f"⚠️  depende de si mostRecentVersions cuenta las hijas: {len(a)} contándolas, {len(b)} sin contarlas;"
              " abajo, la que guarda MENOS")
        if len(b) < len(a):
            a = b
    queda = a

    # Lo que costaría: cada blob una vez.
    leidos = {}

    def blobs(k):
        m = indices.get(k) or manifiesto(k[0], k[1], token)
        if m.get("mediaType") in INDICES or "manifests" in m:
            return k, []
        return k, [(l["digest"], l["size"]) for l in m.get("layers", [])] + [(m["config"]["digest"], m["config"]["size"])]

    with ThreadPoolExecutor(16) as ex:
        leidos = dict(ex.map(blobs, sorted(queda)))
    unicos = {}
    for bs in leidos.values():
        unicos.update(bs)
    # Lo que pesa cada paquete: sus capas, cada una una vez (las compartidas cuentan en cada uno).
    peso = defaultdict(dict)
    for k, bs in leidos.items():
        peso[k[0].rsplit("/", 1)[1]].update(bs)

    tabla = defaultdict(lambda: [0, 0, set()])
    for v in versiones:
        t = tabla[v["nombre"]]
        t[0] += 1
        if (v["paquete"], v["version"]) in queda:
            t[1] += 1
            t[2].update(v["tags"] or ["(sin etiqueta)"])
    print(f"{'paquete':26} {'hay':>5} {'queda':>5} {'GB':>6}  etiquetas que quedan")
    for n in sorted(tabla):
        hay, q, tags = tabla[n]
        shas = sum(1 for t in tags if re.fullmatch(r"[0-9a-f]{7,40}", t))
        otras = sorted(t for t in tags if not re.fullmatch(r"[0-9a-f]{7,40}", t))
        print(f"{n:26} {hay:5} {q:5} {sum(peso[n].values()) / 1e9:6.1f}  {', '.join(otras)}{f' + {shas} commits' if shas else ''}")
    print(f"\nversiones: {len(versiones)} → {len(queda)}; quedaría {sum(unicos.values()) / 1e9:.1f} GB (cada capa una vez)")
    razones = defaultdict(int)
    for k in queda:
        razones[porque[k]] += 1
    print("por qué se queda: " + ", ".join(f"{r} {n}" for r, n in sorted(razones.items())))
    if manifiestos:
        sys.exit(comprobar(manifiestos, versiones, queda))


def comprobar(raiz, versiones, queda):
    """Cada `<repo>/<paquete>:<tag>` o `@<digest>` que nombran los ficheros de `raiz`, ¿se queda?"""
    repo = versiones[0]["paquete"].rsplit("/", 1)[0]
    patron = re.compile(re.escape(repo) + r"/([a-z0-9._-]+)([:@][A-Za-z0-9._:-]+)")
    citas = set()
    for dirpath, _, ficheros in os.walk(raiz):
        for f in ficheros:
            try:
                texto = open(os.path.join(dirpath, f), encoding="utf-8").read()
            except (UnicodeDecodeError, OSError):
                continue
            for m in patron.finditer(texto):
                if "$" not in m.group(2):
                    citas.add((m.group(1), m.group(2), os.path.relpath(os.path.join(dirpath, f), raiz)))
    mal = 0
    for paquete, ref, donde in sorted(citas):
        v = next((v for v in versiones if v["nombre"] == paquete and
                  (ref[1:] in v["tags"] if ref[0] == ":" else v["version"] == ref[1:])), None)
        if v is None:
            print(f"?  {paquete}{ref} ({donde}): no está en el registro")
        elif (v["paquete"], v["version"]) not in queda:
            print(f"✗  {paquete}{ref} ({donde}): la regla la BORRARÍA")
            mal = 1
    print(f"manifiestos: {len(citas)} citas de imágenes en {raiz}/, " + ("alguna se borraría" if mal else "todas se quedan"))
    return mal


if __name__ == "__main__":
    main()
