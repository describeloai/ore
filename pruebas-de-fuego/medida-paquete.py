# -*- coding: utf-8 -*-
"""La novena medida - ¿es el `Package` la unidad ontologica?

Si la unidad de la capa semantica es un conjunto versionado de vistas -que es a
lo que converge la industria- la pregunta es si `Package` YA es esa unidad, o si
falta una pieza entre la vista y el paquete.

"Proyectable" se mide con cuatro preguntas, y ninguna es de opinion:

  A. QUE CONTIENE      un paquete hoy, documento a documento
  B. ¿ES un conjunto de vistas?   cuantos paquetes tienen vistas, y cuantos no
  C. ¿CIERRA el limite?           referencias que cruzan de un paquete a otro:
                                  `backedBy`, `from`, `relations.target`
  D. QUE ATRIBUTOS DE REPO tiene ya: version, estado, dueno, dependencias con
                                  rango semver, politica de cambio rompedor

Reutiliza el lector de `medida-entidad.py`, incluidas sus dos correcciones: el
`metadata: { name: X }` en linea -el 91 % del corpus- y no contar el nombre de
la propia clave como clave anotada.
"""
import collections
import pathlib
import re
import sys

RAIZ = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\ORE\vendor\oos")


def campo(d, k):
    m = re.search(r"(?:^|[{,\s])%s:\s*([\w.\-^~<>=*/]+)" % k, d, re.M)
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
    """A que paquete pertenece un fichero: el directorio que tiene package.yaml.

    Dos disposiciones conviven en el corpus:
      - plana        <caso>/{package.yaml, views/, entities/, tables/}
      - multipaquete <caso>/packages/<n>/{package.yaml, views/, ...}
    Se sube por el arbol hasta encontrar el `package.yaml`, que es la definicion
    operativa de "estar dentro de un paquete".
    """
    d = f.parent
    for _ in range(6):
        if (d / "package.yaml").exists():
            return d
        d = d.parent
    return None


# --------------------------------------------------------------------------
contenido = collections.defaultdict(collections.Counter)
paquetes = {}
docs_por_paquete = collections.defaultdict(list)
sueltos = collections.Counter()

for d, f in documentos(RAIZ):
    m = re.search(r"^kind:\s*(\w+)", d, re.M)
    if not m:
        continue
    kind = m.group(1)
    p = paquete_de(f)
    if p is None:
        sueltos[kind] += 1
        continue
    contenido[p][kind] += 1
    docs_por_paquete[p].append((kind, d, f))
    if kind == "Package":
        paquetes[p] = d

print("== corpus:", RAIZ, "==")
print()
print("A - QUE CONTIENE UN PAQUETE")
print("   %-34s %4d" % ("paquetes (directorios con package.yaml)", len(paquetes)))
tot = collections.Counter()
for p, c in contenido.items():
    tot.update(c)
for k, v in tot.most_common(14):
    print("     %-30s %5d" % (k, v))
if sueltos:
    print("   documentos FUERA de todo paquete:")
    for k, v in sueltos.most_common(8):
        print("     %-30s %5d" % (k, v))

# --------------------------------------------------------------------------
print()
print("B - ¿ES UN PAQUETE UN CONJUNTO DE VISTAS?".replace("¿", ""))
b = collections.Counter()
sin_vistas_con_entidades = []
for p in paquetes:
    c = contenido[p]
    v, e, t = c.get("View", 0), c.get("Entity", 0), c.get("Table", 0)
    if v:
        b["con al menos una vista"] += 1
        if v > 1:
            b["  y con mas de una"] += 1
    elif e:
        b["SIN vistas, pero CON entidades"] += 1
        sin_vistas_con_entidades.append((p, e, t))
    else:
        b["ni vistas ni entidades"] += 1
for k, v in b.items():
    print("   %-40s %4d" % (k, v))
print("   %-40s %4d" % ("total de paquetes", len(paquetes)))
if sin_vistas_con_entidades:
    print("   ejemplos de paquete con entidades y sin vistas:")
    for p, e, t in sin_vistas_con_entidades[:6]:
        print("     %-56s ent=%d tab=%d" % (p.as_posix()[-56:], e, t))

# --------------------------------------------------------------------------
print()
print("C - CIERRA EL LIMITE? referencias que cruzan de paquete a paquete")
# Se indexa por (caso, nombre cualificado) -> paquete, y luego se mira si el
# referente y el referido caen en el mismo directorio de paquete.
def caso_de(p):
    """El caso o ejemplo que contiene a este paquete.

    Sin esto el indice es GLOBAL y `hr.Employee` de un caso de conformidad
    resuelve al paquete `hr` de acme-retail: cruces falsos, y todos en la misma
    direccion. Es el mismo error que ya mordio en `medida-servir.py`.
    """
    s = p.as_posix()
    m = re.search(r"(conformance/(?:v1alpha\d/)?\w+/[^/]+)", s)
    if m:
        return m.group(1)
    m = re.search(r"(examples/[^/]+)", s)
    return m.group(1) if m else s


donde = {}
for p, docs in docs_por_paquete.items():
    k0 = caso_de(p)
    for kind, d, f in docs:
        n = campo(metadatos(d), "name")
        ns = campo(metadatos(d), "namespace")
        if n:
            donde[(k0, kind, (ns or "") + "." + n)] = p
            donde.setdefault((k0, kind, n), p)

c = collections.Counter()
cruces = []


def mirar(kind_destino, ref, p_origen, etiqueta, quien):
    if not ref:
        return
    k0 = caso_de(p_origen)
    p2 = donde.get((k0, kind_destino, ref))
    if p2 is None:
        c["%-26s no resuelve" % etiqueta] += 1
        return
    c["%-26s total" % etiqueta] += 1
    if p2 == p_origen:
        c["%-26s   dentro" % etiqueta] += 1
    else:
        c["%-26s   CRUZA" % etiqueta] += 1
        cruces.append((etiqueta, quien, ref, p_origen, p2))


for p, docs in docs_por_paquete.items():
    for kind, d, f in docs:
        quien = (campo(metadatos(d), "namespace") or "") + "." + (campo(metadatos(d), "name") or "?")
        if kind == "Entity":
            mirar("View", campo(d, "backedBy"), p, "backedBy -> View", quien)
            for tgt in re.findall(r"target:\s*([\w.]+)", d):
                mirar("Entity", tgt, p, "relations.target -> Ent", quien)
        elif kind == "View":
            mv = re.search(r"from:\s*\{?\s*view:\s*([\w.]+)", d)
            mt = re.search(r"from:\s*\{?\s*table:\s*([\w.]+)", d)
            if mv:
                mirar("View", mv.group(1), p, "from.view -> View", quien)
            if mt:
                mirar("Table", mt.group(1), p, "from.table -> Table", quien)

for k, v in c.items():
    print("   %-40s %4d" % (k, v))
if cruces:
    print("   los cruces:")
    vistos = set()
    for et, quien, ref, p1, p2 in cruces:
        k = (et, quien, ref)
        if k in vistos:
            continue
        vistos.add(k)
        print("     %-26s %-20s -> %-18s  %s => %s"
              % (et, quien, ref, p1.name, p2.name))
        if len(vistos) >= 12:
            break

# --------------------------------------------------------------------------
print()
print("D - QUE ATRIBUTOS DE REPOSITORIO TIENE YA")
d4 = collections.Counter()
rangos = collections.Counter()
for p, doc in paquetes.items():
    md = metadatos(doc)
    d4["paquetes"] += 1
    if False:
        pass
for p, doc in paquetes.items():
    md = metadatos(doc)
    for k, etiqueta in [("version", "version semver"),
                        ("status", "estado (vocabulario ODCS)"),
                        ("domain", "dominio")]:
        if campo(md, k):
            d4[etiqueta] += 1
    if re.search(r"^\s*owner:", doc, re.M):
        d4["dueno (handle, alinea CODEOWNERS)"] += 1
    if re.search(r"^\s*dependencies:", doc, re.M):
        d4["dependencias declaradas"] += 1
        for r in re.findall(r"version:\s*[\"']?([\^~><=\d][^\"'\s},]*)", doc):
            rangos[r[:1]] += 1
    if "breakingChangePolicy" in doc:
        d4["politica de cambio rompedor"] += 1
    if "limitations" in doc:
        d4["limitations (donde NO usarlo)"] += 1
    if re.search(r"^\s*sla:", doc, re.M):
        d4["sla"] += 1
del d4[""]
n = d4.pop("paquetes")
print("   sobre %d paquetes:" % n)
for k, v in sorted(d4.items(), key=lambda x: -x[1]):
    print("     %-38s %4d  %3d%%" % (k, v, round(100.0 * v / max(1, n))))
if rangos:
    print("   forma de los rangos de dependencia: %s"
          % dict(sorted(rangos.items(), key=lambda x: -x[1])))
