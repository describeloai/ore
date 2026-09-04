# -*- coding: utf-8 -*-
"""El precio de «lo que se sirve se debe materializar», contra el corpus.

La regla candidata diria: una vista que respalda a una entidad -o sea, que se
SIRVE- debe declarar `materialized`. Esto cuenta que dejaria de compilar.

Y cuenta la segunda mitad, que nadie habia contado: materializar no es anadir
una linea. Si algun campo de la vista lleva clasificacion, hace falta ademas un
`ConduitPolicy` que autorice `materialization.payload` — es `OOS4011`, y es lo
que mordio al escribir los casos de F0a.

Nota de implementacion, que costo una ejecucion colgada: NO usar una expresion
con `(?:\\s+.*\\n)*?` para encontrar propiedades etiquetadas. Retrocede
catastroficamente sobre un documento largo y no termina. Se parte el bloque a
mano.
"""
import collections
import pathlib
import re
import sys

RAIZ = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\ORE\vendor\oos")


def documentos(raiz):
    for f in raiz.rglob("*.yaml"):
        txt = f.read_text(encoding="utf-8", errors="replace")
        for d in re.split(r"^---\s*$", txt, flags=re.M):
            if "kind:" in d:
                yield d, f


def nombre(d):
    m = re.search(r"name:\s*([\w.-]+)", d)
    return m.group(1) if m else "?"


def espacio(d):
    m = re.search(r"namespace:\s*([\w.-]+)", d)
    return m.group(1) if m else ""


def etiquetadas(d):
    """Que propiedades de una entidad llevan `labels`."""
    if "properties:" not in d:
        return set()
    lineas = d.split("properties:", 1)[1].splitlines()
    sangria, actual, trozo, out = None, None, [], set()
    for ln in lineas:
        if not ln.strip():
            continue
        s = len(ln) - len(ln.lstrip())
        if sangria is None:
            sangria = s
        if s < sangria:
            break
        m = re.match(r"\s+(\w+)\s*:", ln) if s == sangria else None
        if m:
            if actual and any("labels" in x for x in trozo):
                out.add(actual)
            actual, trozo = m.group(1), [ln]
        else:
            trozo.append(ln)
    if actual and any("labels" in x for x in trozo):
        out.add(actual)
    return out


def campos(d):
    if "fields:" not in d:
        return {}
    cuerpo = re.split(
        r"^\s*(where|materialized|freshness):",
        d.split("fields:", 1)[1],
        maxsplit=1,
        flags=re.M,
    )[0]
    return dict(re.findall(r"^\s+(\w+)\s*:\s*(\S+)\s*$", cuerpo, re.M))


def caso_de(f):
    """El caso de conformidad al que pertenece un fichero, si pertenece a uno."""
    p = f.as_posix()
    m = re.search(
        r"conformance/(?:(v1alpha\d)/)?(valid|invalid|canonical|diff)/([^/]+)/", p
    )
    if m:
        return ((m.group(1) or "v1alpha1"), m.group(2), m.group(3))
    m = re.search(r"examples/([^/]+)/", p)
    return ("ejemplo", "ejemplo", m.group(1)) if m else None


vistas, entidades, conductos = {}, [], set()

for d, f in documentos(RAIZ):
    c = caso_de(f)
    if "kind: View" in d:
        desde = re.search(r"from:\s*\{?\s*view:\s*([\w.]+)", d)
        vistas[(c, espacio(d) + "." + nombre(d))] = {
            "materializada": "materialized:" in d,
            "campos": campos(d),
            "espacio": espacio(d),
            "sobre": desde.group(1) if desde else None,
        }
    elif "kind: Entity" in d:
        bb = re.search(r"backedBy:\s*(\S+)", d)
        if bb:
            n = bb.group(1)
            if "." not in n:
                n = espacio(d) + "." + n
            entidades.append((c, espacio(d) + "." + nombre(d), n, etiquetadas(d)))
    elif "kind: ConduitPolicy" in d and "materialization.payload" in d:
        conductos.add(c)

tot = collections.Counter()
rompen = []
for c, eqn, vqn, con_etiqueta in entidades:
    v = vistas.get((c, vqn))
    if v is None:
        tot["backedBy que no resuelve"] += 1
        continue
    tot["vistas que SIRVEN a una entidad"] += 1
    # La raiz de LECTURA: ella misma si se materializa, o la primera copia de su
    # cadena hacia abajo. Una vista virtual sobre una materializada YA se sirve
    # de una copia — lo decide `vistas::raiz_de_lectura`, y la primera version
    # de esta medida lo pasaba por alto.
    actual, sirve_de_copia, saltos = v, False, 0
    while actual is not None and saltos < 12:
        if actual["materializada"]:
            sirve_de_copia = True
            break
        if not actual["sobre"]:
            break
        n2 = actual["sobre"]
        if "." not in n2:
            n2 = actual["espacio"] + "." + n2
        actual, saltos = vistas.get((c, n2)), saltos + 1
    if sirve_de_copia:
        tot["  YA se sirven de una copia"] += 1
        if not v["materializada"]:
            tot["    (de una copia mas abajo)"] += 1
        continue
    tot["  VIRTUALES · dejarian de compilar"] += 1
    expone_clasificado = bool(con_etiqueta & set(v["campos"]))
    if expone_clasificado and c not in conductos:
        tot["    y ademas necesitan conducto"] += 1
        remedio = "materialized + ConduitPolicy"
    else:
        remedio = "materialized"
    rompen.append((c, eqn, vqn, remedio))

print("== corpus:", RAIZ, "==")
for k, v in tot.items():
    print("  %-38s %3d" % (k, v))

if rompen:
    print()
    print("  que dejaria de compilar:")
    for c, eqn, vqn, r in rompen:
        print("    %-30s %-22s %-18s -> %s" % ("/".join(c[1:]), eqn, vqn, r))
