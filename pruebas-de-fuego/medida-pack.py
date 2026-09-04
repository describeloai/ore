# -*- coding: utf-8 -*-
"""El precio de acotar `ore pack` por miembro, contra el corpus.

`identidad()` coge el primer `Package` que encuentra. La correccion tiene forma
conocida -acotar con `solo(pkg, miembros, sitio)` como ya hace `sync`- pero
acotar CAMBIA que documentos entran, y eso hay que contarlo antes de escribirlo:

  A. cuantos arboles tienen VARIOS miembros   -> los que hoy publican corrupto
  B. cuantos tienen documentos FUERA de todo miembro
                                              -> los que hoy publican de mas
  C. y en el ejemplo real, cuanto cambia

Un miembro es un directorio con `package.yaml` (`link::miembros`). Un documento
pertenece al miembro cuyo directorio es el prefijo MAS LARGO de su ruta
(`link::miembro_de`); si ninguno lo es, no pertenece a ningun paquete.
"""
import collections
import pathlib
import re
import sys

RAIZ = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\ORE\vendor\oos")


def arboles(raiz):
    """Cada arbol cargable: el directorio que tiene `ontology.config.yaml`.

    Es lo que se le pasa a `ore pack`, y por eso es la unidad de esta medida y
    no el caso de conformidad: un caso `diff` tiene dos arboles.
    """
    for f in sorted(raiz.rglob("ontology.config.yaml")):
        yield f.parent


def miembros_de(arbol):
    return sorted(p.parent for p in arbol.rglob("package.yaml"))


def miembro_de(miembros, doc):
    """El prefijo mas largo, que es lo que hace `link::miembro_de`."""
    dentro = [m for m in miembros if m == doc.parent or m in doc.parents]
    return max(dentro, key=lambda m: len(m.parts)) if dentro else None


def kind(f):
    try:
        t = f.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    m = re.search(r"^kind:\s*(\w+)", t, re.M)
    return m.group(1) if m else None


tot = collections.Counter()
varios, fuera = [], []

for a in arboles(RAIZ):
    ms = miembros_de(a)
    tot["arboles (con ontology.config.yaml)"] += 1
    if not ms:
        tot["  sin ningun package.yaml"] += 1
        continue
    if len(ms) > 1:
        tot["  con VARIOS miembros"] += 1
        varios.append((a, ms))
    else:
        tot["  con un solo miembro"] += 1

    huerfanos = []
    for f in sorted(a.rglob("*.yaml")):
        if f.name in ("ontology.config.yaml", "case.yaml"):
            continue
        k = kind(f)
        if k is None or k == "OntologyConfig":
            continue
        if miembro_de(ms, f) is None:
            huerfanos.append((f, k))
    if huerfanos:
        tot["  con documentos FUERA de todo miembro"] += 1
        fuera.append((a, huerfanos))

print("== corpus:", RAIZ, "==")
print()
print("A/B - EL PRECIO DE ACOTAR")
for k, v in tot.items():
    print("   %-42s %4d" % (k, v))

if varios:
    print()
    print("   los arboles con VARIOS miembros -- hoy publican un .oob corrupto:")
    for a, ms in varios[:12]:
        print("     %-58s %s" % (a.as_posix()[-58:], [m.name for m in ms]))

if fuera:
    print()
    print("   los que tienen documentos fuera de todo miembro -- hoy publican de mas:")
    for a, hs in fuera[:12]:
        ks = collections.Counter(k for _, k in hs)
        print("     %-52s %d docs %s" % (a.as_posix()[-52:], len(hs), dict(ks)))
