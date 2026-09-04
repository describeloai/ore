# -*- coding: utf-8 -*-
"""¿Es `Entity` un lastre? El residuo, contado campo a campo.

La tesis a comprobar: **despues de M2 y B0, todo lo que queda en una entidad se
puede escribir como anotacion sobre una vista o sobre uno de sus campos.** Si es
cierta, `Entity` no es una abstraccion: es un fichero aparte, y `backedBy` es el
precio de tenerlo aparte.

Esto NO decide si se fusiona. Contesta UNA pregunta, que es la que convierte la
intuicion en algo decidible:

  ¿que hay en las 290 entidades del corpus que NO cabria en un documento de
  vista, ni como anotacion de la unidad ni como anotacion de un campo?

Cinco frentes, y los cinco son residuos posibles:

  A. EL REPARTO       cada campo de `Entity`, y donde aterrizaria
  B. SIN ANCLA        entidades sin `backedBy`: no hay vista que anotar
  C. SIN CAMPO        propiedades que no son campo de su vista: no hay nada
                      que anotar aunque haya vista
  D. CRUZADO          `derivedFrom` que nombra la propiedad de OTRA entidad:
                      se convierte en una referencia entre unidades que no es
                      composicion, y el vocabulario de la vista no la tiene
  E. EL NOMBRE        la unidad fusionada tiene UN nombre, y hoy cada pareja
                      tiene dos. Es coste de migracion, y se cuenta

Lo que salga de aqui no dice «hazlo» ni «no lo hagas». Dice QUE CUESTA.
"""
import collections
import pathlib
import re
import sys

RAIZ = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\ORE\vendor\oos")

# El vocabulario real, de `document.rs`. No se escribe de memoria.
META = ["name", "namespace", "labels", "description", "aiContext"]
SPEC = ["backedBy", "implements", "nature", "principal", "primaryKey", "timeKey",
        "uniqueKeys", "temporal", "properties", "relations", "moved", "reserved"]
PROP = ["type", "labels", "description", "required", "unique", "temporal", "enum",
        "derivedFrom", "expression", "examples", "aiContext", "is", "confidence"]

# Donde aterriza cada uno en un documento fusionado.
DESTINO = {
    "name": "se funde con el de la vista  -> E",
    "namespace": "se funde con el de la vista",
    "backedBy": "DESAPARECE — es la direccion entre los dos documentos",
    "properties": "se funde con `fields`: cada campo, anotado",
    "relations": "anota la unidad (o se retira: B0/OOS2026)",
}
for k in ("labels", "description", "aiContext"):
    DESTINO[k] = "anota la unidad — CHOCA con la clave homonima de la vista"
for k in ("implements", "nature", "principal", "primaryKey", "timeKey",
          "uniqueKeys", "temporal", "moved", "reserved"):
    DESTINO[k] = "anota la unidad"


def meta_de(d):
    if "metadata:" not in d:
        return ""
    return re.split(r"^\s*spec:", d.split("metadata:", 1)[1], maxsplit=1, flags=re.M)[0]


def campo(d, k):
    m = re.search(r"(?:^|[{,\s])%s:\s*([\w.\-/]+)" % k, d, re.M)
    return m.group(1) if m else None


def documentos(raiz):
    for f in sorted(raiz.rglob("*.yaml")):
        txt = f.read_text(encoding="utf-8", errors="replace")
        for d in re.split(r"^---\s*$", txt, flags=re.M):
            m = re.search(r"^kind:\s*(\w+)", d, re.M)
            if m:
                yield m.group(1), d, f


def arbol_de(f):
    d = f.parent
    for _ in range(8):
        if (d / "ontology.config.yaml").exists():
            return d
        d = d.parent
    return None


def corto(x):
    return x.rsplit(".", 1)[-1]


def cuerpo_props(d):
    """El bloque `properties:` de un `spec`, sin lo que venga despues."""
    if "properties:" not in d:
        return ""
    resto = d.split("properties:", 1)[1]
    # Corta en la siguiente clave de `spec` al mismo nivel (dos espacios).
    m = re.search(r"^  (?:%s):" % "|".join(SPEC), resto, re.M)
    return resto[: m.start()] if m else resto


def nombres_de_props(d):
    """Los nombres de propiedad: claves a cuatro espacios dentro del bloque."""
    return re.findall(r"^    ([A-Za-z_][\w]*):", cuerpo_props(d), re.M)


# ── recolecta ───────────────────────────────────────────────────────────────
entidades, vistas = [], {}
for kind, d, f in documentos(RAIZ):
    a = arbol_de(f)
    qn = (campo(meta_de(d), "namespace") or "") + "." + (campo(meta_de(d), "name") or "?")
    if kind == "Entity":
        entidades.append((qn, d, f, a))
    elif kind == "View":
        vistas[(a, corto(qn))] = d
        vistas.setdefault((a, qn), d)

print("== corpus:", RAIZ, "==")
print("   entidades:", len(entidades), " vistas:", len(set(id(v) for v in vistas.values())))


def vista_de(a, ref):
    return vistas.get((a, ref)) or vistas.get((a, corto(ref)))


def campos_de_vista(dv):
    if dv is None or "fields:" not in dv:
        return set()
    resto = dv.split("fields:", 1)[1]
    m = re.search(r"^  (?:where|materialized|freshness|owner|from):", resto, re.M)
    resto = resto[: m.start()] if m else resto
    return set(re.findall(r"^    ([A-Za-z_][\w]*):", resto, re.M)) | set(
        re.findall(r"[{,]\s*([A-Za-z_][\w]*):", resto)
    )


# ── A · EL REPARTO ──────────────────────────────────────────────────────────
print()
print("A - EL REPARTO: cada campo de `Entity`, cuanto se usa, y donde aterriza")
uso = collections.Counter()
for qn, d, f, a in entidades:
    m = meta_de(d)
    for k in META:
        if re.search(r"(?:^|[{,\s])%s:" % k, m, re.M):
            uso[k] += 1
    for k in SPEC:
        if re.search(r"(?:^|[{,\s])%s:" % re.escape(k), d, re.M):
            uso[k] += 1
for k in META + SPEC:
    print("   %-12s %4d   %s" % (k, uso[k], DESTINO.get(k, "?")))
print()
print("   Y dentro de cada propiedad — todo esto anota UN CAMPO de la vista:")
pu = collections.Counter()
for qn, d, f, a in entidades:
    c = cuerpo_props(d)
    for k in PROP:
        if re.search(r"(?:^|[{,\s])%s:" % k, c, re.M):
            pu[k] += 1
print("   " + " · ".join("%s %d" % (k, pu[k]) for k in PROP if pu[k]))

# ── B · SIN ANCLA ───────────────────────────────────────────────────────────
print()
print("B - SIN ANCLA: entidades sin `backedBy`")
sin_bb = [(qn, f) for qn, d, f, a in entidades if not campo(d, "backedBy")]
con_binding = sum(1 for qn, f in sin_bb
                  if any((f.parent.parent / "bindings").glob("*.yaml")))
print("   %-46s %3d de %d" % ("sin `backedBy`", len(sin_bb), len(entidades)))
print("   %-46s %3d" % ("  ...y su paquete tiene bindings v1alpha1", con_binding))
print("   %-46s %3d" % ("  ...y no tiene NADA", len(sin_bb) - con_binding))
print("   -> `sustrato.md` §3.4 dice que una entidad SE SIENTA sobre una vista")
print("      -«promete filas, y una promesa de filas necesita quien las")
print("      conteste»-, asi que esto es deuda de migracion, no una capacidad.")
print("      El caso «significado sin datos» ya tiene `Concept` e `Interface`.")

# ── C · SIN CAMPO ───────────────────────────────────────────────────────────
print()
print("C - SIN CAMPO: propiedades que su vista no expone")
tot_p = huerf = 0
detalle = []
for qn, d, f, a in entidades:
    bb = campo(d, "backedBy")
    if not bb:
        continue
    dv = vista_de(a, bb)
    if dv is None:
        continue
    campos = campos_de_vista(dv)
    for p in nombres_de_props(d):
        tot_p += 1
        if p not in campos:
            huerf += 1
            derivada = re.search(r"^    %s:.*?derivedFrom" % re.escape(p),
                                 cuerpo_props(d), re.S | re.M) is not None
            detalle.append((qn, p, "derivedFrom" if derivada else "SIN ORIGEN"))
print("   %-46s %3d" % ("propiedades de entidades con vista resuelta", tot_p))
print("   %-46s %3d" % ("  que NO son campo de esa vista", huerf))
for qn, p, por in detalle[:10]:
    print("       %-26s %-18s %s" % (qn, p, por))
print("   -> las `derivedFrom` caben: en un documento fusionado son un campo")
print("      que declara de que otros sale. Las «SIN ORIGEN» no caben, y")
print("      tampoco compilan hoy si la entidad declara v1alpha8 (OOS2022).")

# ── D · CRUZADO ─────────────────────────────────────────────────────────────
print()
print("D - CRUZADO: `derivedFrom` que nombra la propiedad de OTRA entidad")
propio = ajeno = 0
ejemplos = []
for qn, d, f, a in entidades:
    for m in re.finditer(r"derivedFrom:\s*\[([^\]]*)\]", d):
        for x in re.split(r"[,\s]+", m.group(1).strip()):
            if not x or x.count(".") < 2:
                continue
            duena = ".".join(x.split(".")[:-1])
            if duena == qn or corto(duena) == corto(qn):
                propio += 1
            else:
                ajeno += 1
                ejemplos.append((qn, x))
print("   %-46s %3d" % ("derivedFrom a una propiedad de la MISMA entidad", propio))
print("   %-46s %3d" % ("derivedFrom a la de OTRA", ajeno))
for qn, x in ejemplos[:6]:
    print("       %-26s -> %s" % (qn, x))
print("   -> el propio se funde sin ruido. El ajeno se convierte en una")
print("      referencia entre unidades que NO es `from`, y el vocabulario de la")
print("      vista no tiene ninguna. Es el unico residuo de forma, no de deuda.")

# ── E · EL NOMBRE ───────────────────────────────────────────────────────────
print()
print("E - EL NOMBRE: la unidad fusionada tiene UNO, y hoy hay dos")
pares = distinto = 0
muestra = []
for qn, d, f, a in entidades:
    bb = campo(d, "backedBy")
    if not bb or vista_de(a, bb) is None:
        continue
    pares += 1
    if corto(qn).lower() != corto(bb).lower():
        distinto += 1
        if len(muestra) < 6:
            muestra.append((qn, bb))
print("   %-46s %3d" % ("parejas entidad/vista resueltas", pares))
print("   %-46s %3d" % ("  con nombres DISTINTOS", distinto))
for qn, bb in muestra:
    print("       %-26s <- %s" % (qn, bb))
print("   -> uno de los dos nombres muere en cada pareja, y todo lo que lo")
print("      nombre hay que reescribirlo. Es exactamente para lo que existe")
print("      `moved` — que la vista todavia no tiene.")
