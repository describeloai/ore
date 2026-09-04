# -*- coding: utf-8 -*-
"""La medida de M4, contra el corpus entero.

Tres preguntas, y las tres se contestan del arbol sin ejecutar nada:

  1 · cuantas entidades declaran una relacion con `via`
  2 · de esas, cuantas se respaldan de una vista, y si esa vista se materializa
  3 · y la que decide todo: ¿esta la propiedad de `via` entre los campos de esa
      vista? Porque si no lo esta, «la copia ya contiene la arista» es falso.
"""
import pathlib, re, collections

RAIZ = pathlib.Path(r"C:\ORE\vendor\oos")

docs = []
for f in RAIZ.rglob("*.yaml"):
    txt = f.read_text(encoding="utf-8", errors="replace")
    for d in re.split(r"^---\s*$", txt, flags=re.M):
        if "kind:" in d:
            docs.append((d, f))


def qname(d):
    m = re.search(r"name:\s*([\w.-]+)", d)
    n = re.search(r"namespace:\s*([\w.-]+)", d)
    return (n.group(1) + "." if n else "") + (m.group(1) if m else "?")


# ── el indice de vistas ─────────────────────────────────────────────────────
vistas = {}
for d, f in docs:
    if "kind: View" not in d:
        continue
    campos = {}
    if "fields:" in d:
        cuerpo = re.split(r"^\s*(where|materialized|freshness):", d.split("fields:", 1)[1], maxsplit=1, flags=re.M)[0]
        for m in re.finditer(r"^\s+([\w]+)\s*:\s*(\S+)\s*$", cuerpo, re.M):
            campos[m.group(1)] = m.group(2)
    vistas[qname(d)] = {
        "campos": campos,
        "materializada": "materialized:" in d,
        "fichero": f,
    }

# ── las entidades con relacion ──────────────────────────────────────────────
tot = collections.Counter()
detalle = []
for d, f in docs:
    if "kind: Entity" not in d or "relations:" not in d:
        continue
    bloque = d.split("relations:", 1)[1]
    vias = re.findall(r"via:\s*\[([^\]]+)\]", bloque)
    if not vias:
        continue
    props = [v.strip() for via in vias for v in via.split(",")]
    simples = [p for via in vias for p in [ [x.strip() for x in via.split(",")] ] if len(p) == 1]
    eqn = qname(d)
    tot["entidades con via"] += 1
    tot["relaciones con via"] += len(vias)
    tot["  de ellas, via simple"] += len(simples)

    bb = re.search(r"backedBy:\s*([\w.]+)", d)
    if not bb:
        tot["sin backedBy · camino viejo"] += 1
        detalle.append((eqn, "sin backedBy", "", f.parent.parent.name))
        continue
    v = vistas.get(bb.group(1))
    if v is None:
        tot["backedBy que no resuelve"] += 1
        continue
    tot["con vista"] += 1
    if v["materializada"]:
        tot["  y la vista SE MATERIALIZA"] += 1
    else:
        tot["  y la vista es VIRTUAL"] += 1

    # la pregunta que decide
    fuera = [p for p in props if p not in v["campos"]]
    if fuera:
        tot["  la via NO es campo de la vista"] += 1
        detalle.append((eqn, bb.group(1), "no expone " + ",".join(fuera), f.parent.parent.name))
    else:
        tot["  la via SI es campo de la vista"] += 1

print("== corpus: vendor/oos ==")
for k, v in tot.items():
    print("  %-34s %3d" % (k, v))
if detalle:
    print("\n  casos que no encajan:")
    for e in detalle[:12]:
        print("    %-28s %-22s %s" % (e[0], e[1], e[2]))
