"""P0 · EL PROYECTO SIN PAQUETE Y LAS FUNCIONES EN SU ESPACIO, MEDIDO.

Antes de decidir que un proyecto deja de ser un paquete (sólo guarda
repositorios, pipelines, maps, lineage) y que una Function vive en su propio
espacio (`functions.<def>`), se mide sobre los árboles de verdad:

  §1  LOS SITIOS       qué proyectos hay, su sitio `packages/<id>` y qué guarda
                       cada sitio: repositorios, Functions, y DATOS (lo que un
                       proyecto no debería tener).
  §2  LAS FUNCIONES    todas las Function del árbol: dónde viven, si su nombre
                       corto se repite entre paquetes (¿cabe `functions.<def>`
                       único?), y si hay un paquete llamado `functions`.
  §3  LOS CHOQUES      paquetes que no son sitio pero se llaman como un
                       proyecto, y punteros de datasets bajo un sitio.
  §4  EL CÓDIGO        qué del código da por hecho `packages/<p>/<repo>`
                       (local, sobre este repositorio).

Sólo lectura: el árbol se trae con el Job de siempre (`traer_arbol`).

    python pruebas-de-fuego/medida-proyecto-sin-paquete.py [victor demo …]
    python pruebas-de-fuego/medida-proyecto-sin-paquete.py --local <dir>
"""
import collections
import importlib.util
import io
import os
import re
import subprocess
import sys
import tarfile
import tempfile

# Con PYTHONIOENCODING=utf-8: el módulo de `traer_arbol` envuelve stdout por su cuenta.
MIO = sys.stdout
_VIVOS = []
AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.dirname(AQUI)
DATOS = {"Table", "View", "Dataset", "MediaCollection", "ObjectTable", "Entity", "Binding", "Schema", "Model"}


def traer(celda, destino):
    if _VIVOS:
        return _VIVOS[0].traer_arbol(celda, destino)
    spec = importlib.util.spec_from_file_location("m", os.path.join(AQUI, "medida-migrar-dataset.py"))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    # El módulo envuelve stdout al importarse; si se recoge, cierra el búfer.
    _VIVOS.append(m)
    return m.traer_arbol(celda, destino)


def raiz_de(destino, r):
    """`traer_arbol` deja un .tgz o una carpeta; devuelve la raíz del árbol."""
    if isinstance(r, str) and os.path.isdir(r):
        return r
    tgz = os.path.join(destino, "arbol.tgz")
    if os.path.isfile(tgz):
        out = os.path.join(destino, "arbol")
        if not os.path.isdir(out):
            with tarfile.open(tgz) as t:
                t.extractall(out)
        for base, dirs, _ in os.walk(out):
            if "packages" in dirs or "proyectos" in dirs:
                return base
        return out
    return None


def frontmatter(texto):
    m = re.match(r"^---\n(.*?)\n---", texto.replace("\r\n", "\n"), re.S)
    return m.group(1) if m else ""


def kind_y_nombre(texto):
    k = re.search(r"^kind:\s*(\w+)", texto, re.M)
    n = re.search(r"name:\s*([\w-]+)", texto)
    ns = re.search(r"namespace:\s*([\w-]+)", texto)
    return (k.group(1) if k else None, n.group(1) if n else None, ns.group(1) if ns else None)


def leer(p):
    try:
        with open(p, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def medir(raiz, celda):
    print("\n══════════ %s ══════════  %s" % (celda, raiz))
    pk = os.path.join(raiz, "packages")
    paquetes = sorted(d for d in os.listdir(pk) if os.path.isfile(os.path.join(pk, d, "package.yaml"))) if os.path.isdir(pk) else []
    pr = os.path.join(raiz, "proyectos")
    proyectos = sorted(d for d in os.listdir(pr) if os.path.isfile(os.path.join(pr, d, "README.md"))) if os.path.isdir(pr) else []

    # Por paquete: repositorios, documentos por kind, Functions.
    repos = collections.defaultdict(list)
    docs = collections.defaultdict(collections.Counter)
    funciones = []  # (paquete, def, ruta)
    for p in paquetes:
        for base, dirs, ficheros in os.walk(os.path.join(pk, p)):
            dirs[:] = [d for d in dirs if not d.startswith(".") and d not in ("node_modules", "__pycache__", ".venv")]
            rel = os.path.relpath(base, os.path.join(pk, p)).replace("\\", "/")
            if "README.md" in ficheros and rel != "." and "plantilla:" in frontmatter(leer(os.path.join(base, "README.md"))):
                pl = re.search(r"plantilla:\s*(\S+)", frontmatter(leer(os.path.join(base, "README.md"))))
                repos[p].append("%s (%s)" % (rel, pl.group(1) if pl else "?"))
            for f in ficheros:
                if not f.endswith((".yaml", ".yml")) or f == "package.yaml":
                    continue
                t = leer(os.path.join(base, f))
                k, n, _ = kind_y_nombre(t)
                if not k:
                    continue
                docs[p][k] += 1
                if k == "Function":
                    funciones.append((p, n, "packages/%s/%s/%s" % (p, rel, f) if rel != "." else "packages/%s/%s" % (p, f)))

    print("\n§1 LOS SITIOS — %d proyectos, %d paquetes" % (len(proyectos), len(paquetes)))
    sitios = []
    for id_ in proyectos:
        fm = frontmatter(leer(os.path.join(pr, id_, "README.md")))
        contiene = re.findall(r"^\s*-\s*(\S+)", fm.split("contiene:", 1)[1], re.M) if "contiene:" in fm else []
        es_sitio = id_ in paquetes
        if es_sitio:
            sitios.append(id_)
        datos = {k: v for k, v in docs.get(id_, {}).items() if k in DATOS}
        print("  · %-24s sitio=%-5s contiene=%s" % (id_, "sí" if es_sitio else "NO", contiene))
        if es_sitio:
            print("      repositorios: %s" % (repos.get(id_) or "—"))
            print("      Functions: %d   documentos: %s" % (docs[id_].get("Function", 0), dict(docs[id_]) or "—"))
            print("      DATOS en el sitio: %s" % (datos or "ninguno"))

    print("\n§2 LAS FUNCIONES — %d en el árbol" % len(funciones))
    por_def = collections.defaultdict(list)
    for p, n, ruta in funciones:
        por_def[n].append(p)
        print("  · %-30s en %-22s %s  %s" % (n, p, "(SITIO de proyecto)" if p in sitios else "", ruta))
    rep = {d: ps for d, ps in por_def.items() if len(ps) > 1}
    print("  nombres repetidos entre paquetes: %s" % (rep or "ninguno"))
    print("  paquete llamado `functions`: %s" % ("SÍ" if "functions" in paquetes else "no"))
    otros_repos = {p: r for p, r in repos.items() if p not in sitios}
    print("  repositorios FUERA de un sitio de proyecto: %s" % (otros_repos or "ninguno"))

    print("\n§3 LOS CHOQUES")
    ds = os.path.join(raiz, "datasets")
    punteros = []
    if os.path.isdir(ds):
        for base, _, fs in os.walk(ds):
            for f in fs:
                rel = os.path.relpath(os.path.join(base, f), ds).replace("\\", "/")
                cab = rel.split("/")[0]
                if any(cab == s or rel.startswith(s + "_") for s in sitios):
                    punteros.append(rel)
    print("  punteros de datasets bajo un sitio: %s" % (punteros or "ninguno"))
    print("  paquetes con datos que se llaman como un proyecto: %s"
          % ([p for p in sitios if any(k in DATOS for k in docs.get(p, {}))] or "ninguno"))
    return {"celda": celda, "sitios": sitios, "funciones": len(funciones), "repetidas": rep}


def codigo():
    print("\n══════════ §4 EL CÓDIGO (local) ══════════")
    pats = [
        ("packages/{paquete}/{carpeta} y afines", r'packages/\{[a-z_]+\}/\{[a-z_]+\}'),
        ("strip_prefix(\"packages/\")", r'strip_prefix\("packages/"\)'),
        ("carpeta_del_paquete", r"carpeta_del_paquete"),
        ("paquetes_publicables", r"paquetes_publicables"),
        ("sitio / sitio_de", r"\bsitio_de\b|\"sitio\""),
    ]
    for nombre, pat in pats:
        r = subprocess.run(["git", "grep", "-n", "-E", pat, "--", "crates", "puesto"], capture_output=True, text=True,
                           encoding="utf-8", errors="replace", cwd=RAIZ)
        lineas = [l for l in r.stdout.splitlines() if "/tests/" not in l]
        ficheros = sorted({l.split(":", 1)[0] for l in lineas})
        print("  · %-40s %3d líneas en %d ficheros" % (nombre, len(lineas), len(ficheros)))
        for f in ficheros:
            print("        %s" % f)


def main():
    args = sys.argv[1:]
    res = []
    if args[:1] == ["--local"]:
        res.append(medir(args[1], "local"))
    else:
        for celda in args or ["victor", "demo"]:
            d = tempfile.mkdtemp(prefix="medida-psp-%s-" % celda)
            r = traer(celda, d)
            raiz = raiz_de(d, r)
            if not raiz:
                print("\n%s: MAL no llegó el árbol" % celda)
                continue
            res.append(medir(raiz, celda))
    codigo()
    print("\n══════════ RESUMEN ══════════")
    for r in res:
        print("  %s: %d sitios, %d Functions, repetidas: %s" % (r["celda"], len(r["sitios"]), r["funciones"], r["repetidas"] or "ninguna"))


if __name__ == "__main__":
    main()
