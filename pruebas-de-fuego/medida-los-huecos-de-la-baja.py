#!/usr/bin/env python3
"""
MEDIDA · los dos huecos de la baja (18 de septiembre)

Al retirar una database (`DELETE /paquetes/{n}`) o una conexión (`DELETE
/fuentes/{n}`) quedan dos cosas vivas que nadie retira:

  A. las COPIAS de la base retirada: sus recibos en `copias/<n>_<v>.json` del
     árbol, y sus objetos en el bucket (recibo bajo `ore/v1/plan/<plan>/…` y
     artefacto `ore/v1/<digest>`). `recoger` sólo borra copias SUPERADAS de una
     vista que sigue existiendo.
  B. la CREDENCIAL de la conexión retirada, en el custodio (`fuente-<n>`): el
     cofre no tiene verbo de baja.

Se mide sobre lo real:

  §A  por inquilino: qué planes reclama el árbol (los recibos `copias/*.json`
      de vistas que siguen existiendo) frente a qué recibos hay en el bucket
      → huérfanas, con sus bytes; y recibos en el árbol de vistas que ya no
      están
  §B  por inquilino: fuentes declaradas alguna vez (historia de
      `ontology.config.yaml` en la forja) frente a las declaradas hoy → las
      credenciales que siguen en el custodio sin conexión

Uso:
  python pruebas-de-fuego/medida-los-huecos-de-la-baja.py \\
      --inquilino demo --arbol <dir> --bucket <nombre> --historial <git log -p de ontology.config.yaml> \\
      [--inquilino victor --arbol … --bucket … --historial …]

`--arbol` es un `git archive` de la forja; `--historial` es la salida de
`git log -p --format='COMMIT %h' -- ontology.config.yaml`; el bucket se lista
con `gcloud storage` (hace falta sesión).
"""
import glob
import json
import os
import re
import subprocess
import sys


def fila(k, v, nota=""):
    print("  %-52s %-26s %s" % (k, v, nota))


def grupos():
    a = sys.argv[1:]
    out, cur = [], None
    i = 0
    while i < len(a):
        if a[i] == "--inquilino":
            cur = {"nombre": a[i + 1]}
            out.append(cur)
            i += 2
        elif a[i] in ("--arbol", "--bucket", "--historial") and cur is not None:
            cur[a[i][2:]] = a[i + 1]
            i += 2
        else:
            i += 1
    return out


def gcloud(*args):
    r = subprocess.run(["gcloud", "storage"] + list(args), capture_output=True, text=True, encoding="utf-8", errors="replace", shell=(os.name == "nt"))
    return r.stdout if r.returncode == 0 else ""


def seccion_a(g):
    nombre, arbol, bucket = g["nombre"], g.get("arbol"), g.get("bucket")
    print("\n§A · %s: las copias que nadie reclama" % nombre)
    if not arbol or not bucket:
        fila("sin --arbol o --bucket", "—")
        return
    vistas = set()
    for p in glob.glob(os.path.join(arbol, "packages", "*", "views", "*.yaml")):
        t = open(p, encoding="utf-8").read()
        # `metadata:` en bloque o en línea (`{ name: x, namespace: y }`)
        m = re.search(r"name:\s*([A-Za-z0-9_]+)", t)
        ns = re.search(r"namespace:\s*([A-Za-z0-9_]+)", t)
        if m and ns and "materialized:" in t:
            vistas.add("%s.%s" % (ns.group(1), m.group(1)))
    recibos_arbol = {}
    for p in glob.glob(os.path.join(arbol, "copias", "*.json")):
        try:
            d = json.loads(open(p, encoding="utf-8").read())
        except Exception:
            continue
        recibos_arbol[os.path.basename(p)] = d
    vigentes = {d.get("plan", "").replace("sha256:", "") for d in recibos_arbol.values() if d.get("vista") in vistas and d.get("plan")}
    huerfanos_arbol = [k for k, d in recibos_arbol.items() if d.get("vista") not in vistas]
    fila("vistas con copia en el árbol", "%d" % len(vistas), " ".join(sorted(vistas)))
    fila("recibos copias/*.json", "%d · %d de vistas que ya no están" % (len(recibos_arbol), len(huerfanos_arbol)), " ".join(huerfanos_arbol))

    # el bucket
    listado = gcloud("ls", "-l", "-r", "gs://%s/ore/v1/**" % bucket)
    tam = {}
    for linea in listado.splitlines():
        m = re.match(r"\s*(\d+)\s+\S+\s+gs://[^/]+/(.+)$", linea)
        if m:
            tam[m.group(2)] = int(m.group(1))
    recibos = [k for k in tam if k.startswith("ore/v1/plan/")]
    artefactos = [k for k in tam if not k.startswith("ore/v1/plan/")]
    huerfanos = []
    apuntados = set()
    for r in recibos:
        plan = r.split("/")[3]
        arte = gcloud("cat", "gs://%s/%s" % (bucket, r)).strip()
        apuntados.add(arte)
        if plan not in vigentes:
            huerfanos.append((r, arte, tam.get(arte, 0)))
    sueltos = [a for a in artefactos if a not in apuntados]
    bytes_h = sum(b for _, _, b in huerfanos)
    bytes_t = sum(tam.values())
    fila("bucket %s" % bucket, "%d recibos · %d artefactos · %.0f KB" % (len(recibos), len(artefactos), bytes_t / 1024))
    fila("recibos de planes que el árbol NO reclama", "%d · %.0f KB" % (len(huerfanos), bytes_h / 1024), "%.0f %% de los bytes" % (100.0 * bytes_h / max(bytes_t, 1)))
    for r, a, b in huerfanos:
        fila("  · plan %s…" % r.split("/")[3][:12], "%d B" % b, "→ %s…" % a.split("/")[-1][:12])
    fila("artefactos sin recibo (subidas cortadas)", "%d · %.0f KB" % (len(sueltos), sum(tam[a] for a in sueltos) / 1024))


def seccion_b(g):
    nombre, arbol, hist = g["nombre"], g.get("arbol"), g.get("historial")
    print("\n§B · %s: las credenciales sin conexión" % nombre)
    if not arbol or not hist:
        fila("sin --arbol o --historial", "—")
        return
    declaradas_hoy = set()
    cfg = os.path.join(arbol, "ontology.config.yaml")
    if os.path.exists(cfg):
        en = False
        for l in open(cfg, encoding="utf-8"):
            if l.startswith("datasources:"):
                en = True
                continue
            if en and l and not l.startswith(" ") and not l.startswith("-"):
                en = False
            m = re.search(r"name:\s*([A-Za-z0-9_]+)", l) if en else None
            if m:
                declaradas_hoy.add(m.group(1))
    alguna_vez = set()
    for l in open(hist, encoding="utf-8", errors="replace"):
        if l.startswith("+") and not l.startswith("+++"):
            m = re.search(r"name:\s*([A-Za-z0-9_]+)", l)
            if m and m.group(1) not in ("ore", nombre):
                alguna_vez.add(m.group(1))
    # sólo las que pasaron por el custodio: las dadas de alta por la consola
    # llevan connectionEnv <ORG>_<NOMBRE>_URL desde el 2026-09-10
    sin_conexion = sorted(alguna_vez - declaradas_hoy)
    fila("declaradas hoy", "%d" % len(declaradas_hoy), " ".join(sorted(declaradas_hoy)))
    fila("declaradas alguna vez", "%d" % len(alguna_vez))
    fila("retiradas → credencial que sigue en el custodio", "%d" % len(sin_conexion), " ".join(sin_conexion))
    print("    (cota superior: las anteriores al 2026-09-10 no pasaron por el custodio; las de después, todas)")


def main():
    gs = grupos()
    print("medida · los dos huecos de la baja")
    if not gs:
        print("  uso: --inquilino <n> --arbol <dir> --bucket <b> --historial <f> …")
        return
    for g in gs:
        seccion_a(g)
        seccion_b(g)
    print("""
Lectura:
  · A: retirar una base deja sus recibos en el árbol y sus copias en el bucket; `recoger` no
    las ve porque busca superadas BAJO un plan vigente, y estas ya no tienen plan
  ⇒ `DELETE /paquetes` retira también `copias/<n>_*.json`, y el Job de la copia recoge lo
    huérfano: `ore-store recoger-huerfanas {planes vigentes}` borra recibo y artefacto de todo
    plan que ninguna vista del árbol reclame (y los artefactos sin recibo). Se encola aunque
    no quede ninguna vista con copia: es la pasada que limpia.
  · B: cada conexión retirada deja su credencial en el custodio, viva y sin dueño
  ⇒ `DELETE /organizaciones/{org}/secretos/{n}` en el cofre (potestad `secreto:retirar`, con su
    huella) y `DELETE /fuentes/{n}` lo pide con el testigo de quien pulsa, como el alta
""")


if __name__ == "__main__":
    main()
