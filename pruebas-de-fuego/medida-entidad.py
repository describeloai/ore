# -*- coding: utf-8 -*-
"""La octava medida · la naturaleza de la entidad, contra el corpus.

`02-entity.md` nacio el 2026-08-29, el mismo dia que `03-binding.md`, y cruzo la
frontera del paradigma de vistas con UN campo anadido (`backedBy`). La pregunta
no es si sobrevive -sobrevive- sino QUE PARTES de ella siguen siendo suyas y
cuales las sabe ya el sustrato.

Se miden cinco cosas:

  A. las seis partes de §1.3, cuanto se usan de verdad
  B. IDENTIDAD  -- `primaryKey` frente a `changes.key` de la tabla raiz
  C. CONEXION   -- si el `via` de cada relacion ya es campo de la vista
  D. el texto de `02-entity.md` -- cuanto nombra al binding
  E. la migracion -- cuantas entidades tienen `backedBy` y cuantas no

Nota de implementacion, heredada de `medida-servir.py` y que costo una ejecucion
colgada: NO usar expresiones con `(?:\\s+.*\\n)*?` sobre documentos largos.
Retroceden catastroficamente. Los bloques anidados se parten a mano por sangria.
"""
import collections
import pathlib
import re
import sys

RAIZ = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\ORE\vendor\oos")
SPEC = pathlib.Path(r"C:\ORE\vendor\oos\spec\v1alpha1\02-entity.md")


# --------------------------------------------------------------------------
# lectura del corpus
# --------------------------------------------------------------------------
def documentos(raiz):
    for f in sorted(raiz.rglob("*.yaml")):
        txt = f.read_text(encoding="utf-8", errors="replace")
        for d in re.split(r"^---\s*$", txt, flags=re.M):
            if "kind:" in d:
                yield d, f


def campo(d, k):
    """Un escalar, en cualquiera de las dos formas.

    263 de las 290 entidades del corpus escriben `metadata: { name: X,
    namespace: Y }` EN LINEA. Anclar el valor al fin de linea -que es lo que
    hacia la primera version de esta medida- deja ciego al parser sobre el 91 %
    del corpus, y la medida sale con un denominador falso sin dar ningun error.
    """
    m = re.search(r"(?:^|[{,\s])%s:\s*([\w.-]+)" % k, d, re.M)
    return m.group(1) if m else None


def metadatos(d):
    """El trozo de `metadata`, para no confundir su `name` con otro de mas abajo."""
    if "metadata:" not in d:
        return ""
    resto = d.split("metadata:", 1)[1]
    return re.split(r"^\s*spec:", resto, maxsplit=1, flags=re.M)[0]


def nombre(d):
    return campo(metadatos(d), "name") or "?"


def espacio(d):
    return campo(metadatos(d), "namespace") or ""


def caso_de(f):
    """El caso de conformidad al que pertenece un fichero. Aisla los espacios de
    nombres: dos casos distintos pueden tener ambos `hr.Employee`."""
    p = f.as_posix()
    m = re.search(
        r"conformance/(?:(v1alpha\d)/)?(valid|invalid|canonical|diff)/([^/]+)/", p
    )
    if m:
        return ((m.group(1) or "v1alpha1"), m.group(2), m.group(3))
    m = re.search(r"examples/([^/]+)/", p)
    return ("ejemplo", "ejemplo", m.group(1)) if m else ("suelto", "suelto", p)


def bloque(d, clave):
    """Devuelve {subclave: [lineas]} del mapa anidado bajo `clave:`.

    Se parte por sangria a mano. `clave:` debe estar a nivel de `spec`.
    """
    if not re.search(r"^\s*%s:\s*$" % clave, d, re.M):
        return {}
    cuerpo = re.split(r"^\s*%s:\s*$" % clave, d, maxsplit=1, flags=re.M)[1]
    sangria, actual, trozo, out = None, None, [], {}
    for ln in cuerpo.splitlines():
        if not ln.strip() or ln.lstrip().startswith("#"):
            continue
        s = len(ln) - len(ln.lstrip())
        if sangria is None:
            sangria = s
        if s < sangria:
            break
        m = re.match(r"\s+([\w.-]+)\s*:", ln) if s == sangria else None
        if m:
            if actual:
                out[actual] = trozo
            actual, trozo = m.group(1), [ln]
        elif actual:
            trozo.append(ln)
    if actual:
        out[actual] = trozo
    return out


def claves_de(lineas):
    """Que claves declara un sub-bloque, en linea `{a: 1, b: 2}` o anidado.

    NO cuenta el nombre de la propia propiedad. La primera version si lo hacia
    -`  employeeId: { type: String }` daba {type, employeeId}- y con eso las 63
    propiedades del corpus salian TODAS "anotadas", que es justo la conclusion
    que esta medida tenia que poder desmentir.
    """
    if not lineas:
        return set()
    m = re.match(r"\s*([\w.-]+)\s*:", lineas[0])
    propio = m.group(1) if m else None
    sangria = len(lineas[0]) - len(lineas[0].lstrip())
    txt = "\n".join(lineas)
    ks = set(re.findall(r"[{,]\s*([\w]+)\s*:", txt))
    ks |= set(re.findall(r"^\s{%d,}([\w]+)\s*:" % (sangria + 1), txt, re.M))
    ks.discard(propio)
    return ks


def lista(txt, clave):
    """`clave: [a, b]` -> ['a','b'].  Tambien la forma con guiones."""
    m = re.search(r"%s:\s*\[([^\]]*)\]" % clave, txt)
    if m:
        return [x.strip().strip("\"'") for x in m.group(1).split(",") if x.strip()]
    m = re.search(r"%s:\s*\n((?:\s*-\s*\S+\n?)+)" % clave, txt)
    if m:
        return [x.strip().strip("\"'") for x in re.findall(r"-\s*(\S+)", m.group(1))]
    return []


def campos_vista(d):
    """`fields:` de una View -> {propiedad: columna}."""
    out = {}
    for k, ln in bloque(d, "fields").items():
        m = re.search(r":\s*([\w.\"']+)\s*$", ln[0])
        out[k] = m.group(1).strip("\"'") if m else k
    return out


# --------------------------------------------------------------------------
# indexar
# --------------------------------------------------------------------------
vistas, tablas, entidades = {}, {}, []

for d, f in documentos(RAIZ):
    c = caso_de(f)
    qn = espacio(d) + "." + nombre(d)
    if "kind: View" in d:
        desde_v = re.search(r"from:\s*\{?\s*view:\s*([\w.]+)", d)
        desde_t = re.search(r"from:\s*\{?\s*table:\s*([\w.]+)", d)
        vistas[(c, qn)] = {
            "campos": campos_vista(d),
            "espacio": espacio(d),
            "sobre_vista": desde_v.group(1) if desde_v else None,
            "sobre_tabla": desde_t.group(1) if desde_t else None,
        }
    elif "kind: Table" in d:
        tablas[(c, qn)] = {"key": lista(d, "key")}
    elif "kind: Entity" in d:
        entidades.append((c, qn, d, f))


def raiz_fisica(c, vqn, espacio_ent):
    """Baja por la cadena de vistas hasta la tabla, componiendo el renombrado.

    Devuelve (tabla, mapa propiedad->columna) o (None, {}).
    """
    if "." not in vqn:
        vqn = espacio_ent + "." + vqn
    mapa, saltos = None, 0
    while saltos < 12:
        v = vistas.get((c, vqn))
        if v is None:
            return None, {}
        mapa = v["campos"] if mapa is None else {
            p: v["campos"].get(col, col) for p, col in mapa.items()
        }
        if v["sobre_tabla"]:
            t = v["sobre_tabla"]
            if "." not in t:
                t = v["espacio"] + "." + t
            return tablas.get((c, t)), (mapa or {})
        if not v["sobre_vista"]:
            return None, (mapa or {})
        vqn = v["sobre_vista"]
        if "." not in vqn:
            vqn = v["espacio"] + "." + vqn
        saltos += 1
    return None, {}


# --------------------------------------------------------------------------
# A · las seis partes
# --------------------------------------------------------------------------
PARTES = [
    ("IDENTIDAD   primaryKey/timeKey/uniqueKeys", ["primaryKey", "timeKey", "uniqueKeys"]),
    ("SIGNIFICADO description/aiContext/is", None),
    ("SENSIBILIDAD labels en alguna propiedad", None),
    ("PROCEDENCIA derivedFrom", None),
    ("CONEXION    relations", ["relations"]),
    ("HISTORIA    temporal/moved/reserved", ["temporal", "moved", "reserved"]),
]

partes = collections.Counter()
usa_backedby, usa_binding_solo = 0, 0
bindings = set()
for d, f in documentos(RAIZ):
    if "kind: Binding" in d:
        m = re.search(r"targetEntity:\s*([\w.]+)", d)
        if m:
            bindings.add((caso_de(f), m.group(1)))

for c, qn, d, f in entidades:
    props = bloque(d, "properties")
    claves = set()
    for ln in props.values():
        claves |= claves_de(ln)
    if any(re.search(r"^\s*%s:" % k, d, re.M) for k in ("primaryKey", "timeKey", "uniqueKeys")):
        partes["IDENTIDAD"] += 1
    if ("description" in claves or "aiContext" in claves or "is" in claves
            or "description:" in d or "aiContext:" in d):
        partes["SIGNIFICADO"] += 1
    if "labels" in claves:
        partes["SENSIBILIDAD"] += 1
    if "derivedFrom" in claves:
        partes["PROCEDENCIA"] += 1
    if re.search(r"^\s*relations:", d, re.M):
        partes["CONEXION"] += 1
    if any(re.search(r"^\s*%s:" % k, d, re.M) for k in ("temporal", "moved", "reserved")) \
            or "temporal" in claves:
        partes["HISTORIA"] += 1

    tiene_bb = re.search(r"^\s*backedBy:", d, re.M) is not None
    if tiene_bb:
        usa_backedby += 1
    elif (c, qn) in bindings or (c, qn.split(".")[-1]) in bindings:
        usa_binding_solo += 1

print("== corpus:", RAIZ, "==")
print()
print("A - LAS SEIS PARTES de 02-entity S1.3, sobre %d entidades" % len(entidades))
orden = ["SIGNIFICADO", "IDENTIDAD", "SENSIBILIDAD", "CONEXION", "PROCEDENCIA", "HISTORIA"]
for k in orden:
    n = partes[k]
    barra = "#" * int(round(40.0 * n / max(1, len(entidades))))
    print("   %-13s %4d  %3d%%  %s" % (k, n, round(100.0 * n / len(entidades)), barra))

# --------------------------------------------------------------------------
# B · IDENTIDAD · ¿la sabe ya la copia?
# --------------------------------------------------------------------------
b = collections.Counter()
discrepan = []
for c, qn, d, f in entidades:
    pk = lista(d, "primaryKey")
    if not pk:
        continue
    b["entidades con primaryKey"] += 1
    bb = campo(d, "backedBy")
    if not bb:
        b["  sin backedBy - no se puede cotejar"] += 1
        continue
    tabla, mapa = raiz_fisica(c, bb, espacio(d))
    if tabla is None:
        b["  la cadena no llega a una tabla"] += 1
        continue
    if not tabla["key"]:
        b["  la tabla NO declara changes.key"] += 1
        continue
    fisica = [mapa.get(p, p) for p in pk]
    if set(fisica) == set(tabla["key"]):
        b["  la copia YA dice la misma clave"] += 1
    else:
        b["  la copia dice OTRA clave"] += 1
        discrepan.append((c, qn, pk, fisica, tabla["key"]))

print()
print("B - IDENTIDAD - `primaryKey` frente a `changes.key` de la tabla raiz")
for k, v in b.items():
    print("   %-38s %4d" % (k, v))
if discrepan:
    print("   discrepancias:")
    for c, qn, pk, fis, tk in discrepan[:12]:
        print("     %-26s %-18s pk=%s -> %s   tabla=%s"
              % ("/".join(c[1:])[:26], qn, pk, fis, tk))

# --------------------------------------------------------------------------
# C · CONEXION · ¿el `via` ya es campo de la vista?
# --------------------------------------------------------------------------
cc = collections.Counter()
fuera = []
for c, qn, d, f in entidades:
    rels = bloque(d, "relations")
    if not rels:
        continue
    cc["entidades con relaciones"] += 1
    bb = campo(d, "backedBy")
    for r, ln in rels.items():
        cc["  relaciones"] += 1
        vias = lista("\n".join(ln), "via")
        if not vias:
            cc["    sin via legible"] += 1
            continue
        if not bb:
            cc["    la entidad no tiene backedBy"] += 1
            continue
        _, mapa = raiz_fisica(c, bb, espacio(d))
        v = vistas.get((c, bb if "." in bb else espacio(d) + "." + bb))
        campos = set(v["campos"]) if v else set(mapa)
        if set(vias) <= campos:
            cc["    el via YA es campo de la vista"] += 1
        else:
            cc["    el via NO esta en la vista"] += 1
            fuera.append((c, qn, r, [x for x in vias if x not in campos]))

print()
print("C - CONEXION - el `via` de cada relacion frente a los campos de su vista")
for k, v in cc.items():
    print("   %-38s %4d" % (k, v))
if fuera:
    print("   las que se salen:")
    for c, qn, r, ausentes in fuera[:12]:
        print("     %-26s %-18s %-12s falta %s"
              % ("/".join(c[1:])[:26], qn, r, ausentes))

# --------------------------------------------------------------------------
# D · el texto de 02-entity.md
# --------------------------------------------------------------------------
print()
print("D - EL TEXTO de", SPEC.name)
if SPEC.exists():
    t = SPEC.read_text(encoding="utf-8")
    lineas = t.splitlines()
    norm = [l for l in lineas if re.search(r"\bDEBE\b|\bNO DEBE\b|\bDEBERIA\b|\bDEBERÍA\b", l)]
    print("   %-38s %4d" % ("lineas totales", len(lineas)))
    print("   %-38s %4d" % ("enunciados normativos (DEBE)", len(norm)))
    print("   %-38s %4d" % ("  de ellos, nombran el binding", len([l for l in norm if "inding" in l])))
    print("   %-38s %4d" % ("menciones de `binding` en total", t.lower().count("binding")))
    secciones, actual = [], None
    for l in lineas:
        if l.startswith("#"):
            actual = l.strip("# ").strip()
            secciones.append([actual, 0])
        elif secciones and "inding" in l.lower():
            secciones[-1][1] += 1
    tocadas = [(s, n) for s, n in secciones if n]
    print("   %-38s %4d de %d" % ("secciones que nombran el binding", len(tocadas), len(secciones)))
    for s, n in tocadas:
        print("     %2d  %s" % (n, s.encode("ascii", "replace").decode("ascii")))
else:
    print("   (no encontrado:", SPEC, ")")

# --------------------------------------------------------------------------
# E · la migracion
# --------------------------------------------------------------------------
print()
print("E - MIGRACION")
print("   %-38s %4d" % ("entidades en el corpus", len(entidades)))
print("   %-38s %4d  %3d%%" % ("  con backedBy (paradigma de vistas)", usa_backedby,
                               round(100.0 * usa_backedby / max(1, len(entidades)))))
print("   %-38s %4d" % ("  con Binding y sin backedBy", usa_binding_solo))
print("   %-38s %4d" % ("  sin lo uno ni lo otro", len(entidades) - usa_backedby - usa_binding_solo))


# --------------------------------------------------------------------------
# F - el sesgo del corpus, que hay que decir antes de interpretar A
# --------------------------------------------------------------------------
# Un caso de conformidad declara LO MINIMO que su regla necesita. Contar sobre
# el corpus entero mide "que reglas hay", no "que usa un paquete de verdad". El
# unico paquete realista es `examples/`, y va aparte.
print()
print("F - EL SESGO DEL CORPUS: donde vive cada entidad")
region = collections.Counter()
por_region = collections.defaultdict(collections.Counter)
for c, qn, d, f in entidades:
    r = "ejemplo" if c[0] == "ejemplo" else ("conformidad/" + c[1])
    region[r] += 1
    if re.search(r"^\s*backedBy:", d, re.M):
        por_region[r]["backedBy"] += 1
    if re.search(r"^\s*relations:", d, re.M):
        por_region[r]["relations"] += 1
    if "description" in d or "aiContext" in d:
        por_region[r]["significado"] += 1
for r, n in sorted(region.items(), key=lambda x: -x[1]):
    print("   %-22s %4d   backedBy %3d   relations %3d   significado %3d"
          % (r, n, por_region[r]["backedBy"], por_region[r]["relations"],
             por_region[r]["significado"]))

print()
print("G - LAS SEIS PARTES solo sobre `examples/`, el unico paquete realista")
ej = [e for e in entidades if e[0][0] == "ejemplo"]
p2 = collections.Counter()
for c, qn, d, f in ej:
    props = bloque(d, "properties")
    claves = set()
    for ln in props.values():
        claves |= claves_de(ln)
    if any(re.search(r"^\s*%s:" % k, d, re.M) for k in ("primaryKey", "timeKey", "uniqueKeys")):
        p2["IDENTIDAD"] += 1
    if "description" in claves or "aiContext" in claves or "is" in claves \
            or "description:" in d or "aiContext:" in d:
        p2["SIGNIFICADO"] += 1
    if "labels" in claves:
        p2["SENSIBILIDAD"] += 1
    if "derivedFrom" in claves:
        p2["PROCEDENCIA"] += 1
    if re.search(r"^\s*relations:", d, re.M):
        p2["CONEXION"] += 1
    if any(re.search(r"^\s*%s:" % k, d, re.M) for k in ("temporal", "moved", "reserved")):
        p2["HISTORIA"] += 1
for k in orden:
    n = p2[k]
    print("   %-13s %4d de %d  %3d%%  %s"
          % (k, n, len(ej), round(100.0 * n / max(1, len(ej))),
             "#" * int(round(40.0 * n / max(1, len(ej))))))


# --------------------------------------------------------------------------
# H - la premisa de M2: los nombres duplicados entre entidad y vista
# --------------------------------------------------------------------------
# `sustrato.md` M2 dice "los nueve nombres duplicados desaparecen". Esto lo
# cuenta sobre las entidades que YA tienen `backedBy`, que son las unicas donde
# la duplicacion puede existir.
print()
print("H - LA PREMISA DE M2: `properties` de la entidad vs `fields` de su vista")
h = collections.Counter()
detalle = []
for c, qn, d, f in entidades:
    bb = campo(d, "backedBy")
    if not bb:
        continue
    v = vistas.get((c, bb if "." in bb else espacio(d) + "." + bb))
    if v is None:
        h["backedBy que no resuelve"] += 1
        continue
    props = set(bloque(d, "properties"))
    if not props:
        continue
    h["entidades cotejables"] += 1
    campos = set(v["campos"])
    h["  nombres de propiedad"] += len(props)
    dup = props & campos
    h["    que YA son campo de la vista"] += len(dup)
    solo = props - campos
    h["    que NO estan en la vista"] += len(solo)
    if solo:
        detalle.append((c, qn, sorted(solo)))
    # De los duplicados: cuantos anotan algo que la vista NO puede decir.
    anotan = 0
    for p in dup:
        ks = claves_de(bloque(d, "properties")[p])
        if ks - {"type"}:
            anotan += 1
    h["      y ademas ANOTAN (labels/is/...)"] += anotan
    h["      y solo repiten el nombre y el tipo"] += len(dup) - anotan
for k, v in h.items():
    print("   %-42s %4d" % (k, v))
if detalle:
    print("   propiedades sin campo (derivedFrom o hueco):")
    for c, qn, solo in detalle[:10]:
        print("     %-26s %-18s %s" % ("/".join(c[1:])[:26], qn, solo))
