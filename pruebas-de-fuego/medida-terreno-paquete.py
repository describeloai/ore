# -*- coding: utf-8 -*-
"""El terreno de «un paquete es un conjunto de vistas», antes de construir nada.

La vision esta afilada -`docs/ontologia-como-repositorio.md` §6 y §7- y dice que
lo que falta no es naturaleza sino una DECLARACION: que el manifiesto diga de que
vistas se compone, en vez de heredarlo del directorio.

Esto mide el terreno de esa declaracion, y solo eso. Cinco preguntas:

  A. EL TAMANO      cuantas vistas tendria que listar cada paquete
  B. EL DEFECTO     si el directorio sigue siendo el defecto, cuantos
                    `package.yaml` NO habria que tocar
  C. LO QUE ROMPE   cuantos paquetes tienen vistas de las que nadie tira, y
                    cuantas entidades apuntan a una vista de otro paquete
  D. LA DEUDA       directorios con pinta de paquete y sin manifiesto
  E. EL DIFF        cuantos pares `before/after` cambiarian de conjunto, que es
                    la superficie diferenciable nueva que esto crea

Lector heredado de `medida-entidad.py` con sus dos correcciones: `metadata: {}`
en linea -el 91 % del corpus- y no contar la clave propia como anotada.
"""
import collections
import pathlib
import re
import sys

RAIZ = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\ORE\vendor\oos")


def campo(d, k):
    m = re.search(r"(?:^|[{,\s])%s:\s*([\w.\-/]+)" % k, d, re.M)
    return m.group(1) if m else None


def metadatos(d):
    if "metadata:" not in d:
        return ""
    return re.split(r"^\s*spec:", d.split("metadata:", 1)[1], maxsplit=1, flags=re.M)[0]


def documentos(raiz):
    for f in sorted(raiz.rglob("*.yaml")):
        txt = f.read_text(encoding="utf-8", errors="replace")
        for d in re.split(r"^---\s*$", txt, flags=re.M):
            if "kind:" in d:
                yield d, f


def paquete_de(f):
    d = f.parent
    for _ in range(6):
        if (d / "package.yaml").exists():
            return d
        d = d.parent
    return None


def arbol_de(f):
    """El arbol cargable: el directorio con `ontology.config.yaml`."""
    d = f.parent
    for _ in range(8):
        if (d / "ontology.config.yaml").exists():
            return d
        d = d.parent
    return None


# --------------------------------------------------------------------------
vistas_de = collections.defaultdict(list)   # paquete -> [qn de vista]
entidades_de = collections.defaultdict(list)
backed = []                                  # (paquete, entidad, vista)
desde = []                                   # (paquete, vista, vista de origen)
paquetes = {}

for d, f in documentos(RAIZ):
    m = re.search(r"^kind:\s*(\w+)", d, re.M)
    if not m:
        continue
    kind, p = m.group(1), paquete_de(f)
    qn = (campo(metadatos(d), "namespace") or "") + "." + (campo(metadatos(d), "name") or "?")
    if kind == "Package" and p is not None:
        paquetes[p] = qn
    if p is None:
        continue
    if kind == "View":
        vistas_de[p].append(qn)
        mv = re.search(r"from:\s*\{?\s*view:\s*([\w.]+)", d)
        if mv:
            desde.append((p, qn, mv.group(1)))
    elif kind == "Entity":
        entidades_de[p].append(qn)
        bb = campo(d, "backedBy")
        if bb:
            backed.append((p, qn, bb))

print("== corpus:", RAIZ, "==")
print("   paquetes:", len(paquetes))

# --------------------------------------------------------------------------
print()
print("A - EL TAMANO de la lista que habria que escribir")
tam = collections.Counter(len(vistas_de.get(p, [])) for p in paquetes)
for n in sorted(tam):
    print("   %2d vista(s)  %4d paquetes  %s" % (n, tam[n], "#" * min(40, tam[n])))
total_v = sum(len(v) for v in vistas_de.values())
con = sum(1 for p in paquetes if vistas_de.get(p))
print("   %-40s %4d" % ("vistas en total", total_v))
print("   %-40s %4d" % ("paquetes con al menos una", con))
if con:
    print("   %-40s %.1f" % ("media por paquete que tiene", 1.0 * total_v / con))

# --------------------------------------------------------------------------
print()
print("B - EL DEFECTO: si el directorio sigue siendo el defecto")
print("   Es el precedente de `workspace.members`, que declara `packages/*` y")
print("   dice que «el valor por defecto lo aplica el COMPILADOR al normalizar».")
print("   %-40s %4d  %3d%%" % ("package.yaml que NO hay que tocar", len(paquetes),
                               100))
print("   %-40s %4d" % ("  ...porque el directorio ya los agrupa", con))
print("   %-40s %4d" % ("  y los que no tienen vistas no listan nada", len(paquetes) - con))

# --------------------------------------------------------------------------
print()
print("C - LO QUE LA REGLA DESTAPARIA")
def corto(x):
    return x.rsplit(".", 1)[-1]


# `backedBy: empleados` y `backedBy: hr.empleados` son la misma referencia: el
# espacio se hereda. Se compara SIEMPRE por el nombre corto ademas del
# cualificado; la primera version tiraba las formas cortas y daba 51 huerfanas
# de 55, que era el sintoma de estar midiendo el parser y no el corpus.
tiradas = collections.defaultdict(set)
for p, e, bb in backed:
    tiradas[p].add(corto(bb))
for p, v, orig in desde:
    tiradas[p].add(corto(orig))

huerfanas, cruzadas = 0, []
for p, vs in vistas_de.items():
    usadas = tiradas.get(p, set())
    for v in vs:
        if corto(v) not in usadas:
            huerfanas += 1
print("   %-40s %4d" % ("vistas de las que nadie tira", huerfanas))

indice = {}
for p, vs in vistas_de.items():
    a = arbol_de(p / "package.yaml")
    for v in vs:
        indice[(a, v)] = p
        indice.setdefault((a, v.rsplit(".", 1)[-1]), p)
for p, e, bb in backed:
    a = arbol_de(p / "package.yaml")
    d = indice.get((a, bb))
    if d is not None and d != p:
        cruzadas.append((e, bb, p.name, d.name))
print("   %-40s %4d" % ("entidades respaldadas por vista AJENA", len(cruzadas)))
for e, bb, o, d in cruzadas[:8]:
    print("     %-22s -> %-16s  %s => %s" % (e, bb, o, d))

# --------------------------------------------------------------------------
print()
print("D - LA DEUDA: directorios con pinta de paquete y sin manifiesto")
deuda = []
for cfg in sorted(RAIZ.rglob("ontology.config.yaml")):
    pk = cfg.parent / "packages"
    if not pk.is_dir():
        continue
    for sub in sorted(pk.iterdir()):
        if sub.is_dir() and not (sub / "package.yaml").exists():
            n = len(list(sub.rglob("*.yaml")))
            deuda.append((sub, n))
print("   %-40s %4d" % ("directorios bajo packages/ sin manifiesto", len(deuda)))
for s, n in deuda[:8]:
    print("     %-56s %d yaml" % (s.as_posix()[-56:], n))

# --------------------------------------------------------------------------
print()
print("E - EL DIFF: la superficie diferenciable nueva")
pares = collections.Counter()
for cfg in sorted(RAIZ.rglob("ontology.config.yaml")):
    if cfg.parent.name in ("before", "after"):
        pares[cfg.parent.parent] += 1
completos = [k for k, v in pares.items() if v == 2]
print("   %-40s %4d" % ("casos diff con before y after", len(completos)))
cambian = 0
for caso in completos:
    def conj(lado):
        out = set()
        for f in (caso / lado).rglob("*.yaml"):
            t = f.read_text(encoding="utf-8", errors="replace")
            if re.search(r"^kind:\s*View", t, re.M):
                out.add((campo(metadatos(t), "namespace") or "") + "."
                        + (campo(metadatos(t), "name") or "?"))
        return out
    if conj("before") != conj("after"):
        cambian += 1
print("   %-40s %4d" % ("  en los que el conjunto de vistas cambia", cambian))
print("   (cada uno seria una entrada nueva en `ore diff`: anadir o quitar una")
print("    vista de un paquete es un cambio que su consumidor tiene que ver)")
