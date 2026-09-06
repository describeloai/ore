# -*- coding: utf-8 -*-
"""El corpus: cual es la deuda de verdad, y que compra pagarla.

«268 entidades sin `backedBy`» es un numero, no un problema. El problema es lo
que ese numero causa, y hasta ahora se ha dicho de oido: «las medidas se apoyan
en el unico ejemplo completo». Esto lo mide.

La pregunta util no es cuantos documentos son viejos. Es:

    ¿QUE REGLA VIVA SOLO SE PRUEBA CON VOCABULARIO QUE YA NO SE ESCRIBE?

Porque esa regla esta verde y ciega a la vez: pasa su caso, y nadie sabe si
sigue valiendo sobre tablas y vistas. Cinco frentes:

  A. EL INVENTARIO   que hay, por borrador y por paradigma
  B. LA COBERTURA    cada codigo, y en que paradigma se prueba
  C. LOS CIEGOS      los que SOLO se prueban con bindings. La matter
  D. LO QUE COSTO    donde se noto ya, con nombre
  E. EL CAMINO       que es mecanico y que no
"""
import collections
import pathlib
import re

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
CONF = OOS / "conformance"
CODE = RAIZ / "crates/ore-core/src/code.rs"


def paradigma(entrada):
    """Con que vocabulario esta escrito el arbol de entrada de un caso."""
    viejo = nuevo = False
    for f in entrada.rglob("*.yaml"):
        t = f.read_text(encoding="utf-8", errors="replace")
        if re.search(r"^kind:\s*Binding", t, re.M):
            viejo = True
        if re.search(r"^kind:\s*(Table|View)", t, re.M):
            nuevo = True
    if viejo and nuevo:
        return "los dos"
    if viejo:
        return "binding"
    if nuevo:
        return "tabla+vista"
    return "ninguno"


CASOS = []
for c in sorted(CONF.rglob("case.yaml")):
    t = c.read_text(encoding="utf-8", errors="replace")
    m = re.search(r"^expects:\s*(.+)$", t, re.M)
    entradas = [d for d in (c.parent / "input", c.parent / "before",
                            c.parent / "after") if d.is_dir()]
    CASOS.append({
        "nombre": c.parent.relative_to(CONF).as_posix(),
        "borrador": c.parent.relative_to(CONF).parts[0],
        "expects": (m.group(1).strip() if m else ""),
        "paradigma": (paradigma(entradas[0]) if entradas else "sin entrada"),
    })

print("== el corpus ==")

# -- A - EL INVENTARIO -------------------------------------------------------
print()
print("A - EL INVENTARIO: por borrador y por paradigma")
tabla = collections.defaultdict(collections.Counter)
for c in CASOS:
    b = c["borrador"] if c["borrador"].startswith("v1alpha") else "v1alpha1"
    tabla[b][c["paradigma"]] += 1
cols = ["binding", "tabla+vista", "los dos", "ninguno", "sin entrada"]
print("   %-10s %s" % ("borrador", " ".join("%12s" % x for x in cols)))
for b in sorted(tabla):
    print("   %-10s %s" % (b, " ".join("%12d" % tabla[b][x] for x in cols)))
tot = collections.Counter()
for b in tabla:
    tot.update(tabla[b])
print("   %-10s %s" % ("TOTAL", " ".join("%12d" % tot[x] for x in cols)))

# -- B - LA COBERTURA --------------------------------------------------------
print()
print("B - LA COBERTURA: cada codigo, y con que vocabulario se prueba")
codigos = re.findall(r'Oos(\d{4})\s*=\s*"(OOS\d{4})"', CODE.read_text(encoding="utf-8"))
todos = sorted({c[1] for c in codigos})
donde = collections.defaultdict(collections.Counter)
for c in CASOS:
    for cod in re.findall(r"OOS\d{4}", c["expects"]):
        donde[cod][c["paradigma"]] += 1
con_caso = [c for c in todos if c in donde]
print("   %-40s %3d" % ("codigos declarados en el motor", len(todos)))
print("   %-40s %3d" % ("...con al menos un caso", len(con_caso)))
print("   %-40s %3d" % ("...sin ninguno", len(todos) - len(con_caso)))

# -- C - LOS CIEGOS ----------------------------------------------------------
print()
print("C - LA MATTER: los que SOLO se prueban con `Binding`")
ciegos = [c for c in con_caso
          if donde[c]["binding"] and not donde[c]["tabla+vista"]
          and not donde[c]["los dos"]]
solo_nuevo = [c for c in con_caso if donde[c]["tabla+vista"] and not donde[c]["binding"]]
ambos = [c for c in con_caso if donde[c]["binding"] and
         (donde[c]["tabla+vista"] or donde[c]["los dos"])]
print("   %-40s %3d" % ("probados SOLO con binding", len(ciegos)))
print("   %-40s %3d" % ("probados en los dos paradigmas", len(ambos)))
print("   %-40s %3d" % ("probados solo con tabla+vista", len(solo_nuevo)))
print()
for c in ciegos:
    print("     %s" % c)
print()
print("   -> esos son los que estan VERDES Y CIEGOS: pasan su caso, y nadie")
print("      sabe si siguen valiendo sobre el vocabulario que hoy se escribe.")
print("      No es que fallen — es que su prueba no dice nada del camino nuevo.")
print()
print("   Y no es una preocupacion teorica. Se intentaron provocar dos de los")
print("   diez sobre el ejemplo v1alpha8, sin un binding a la vista:")
print()
print("     `OOS2005`  SALTA. Se quita una propiedad que una `via` nombra y el")
print("                motor la caza. Ciego, pero vivo: le falta el caso, no")
print("                la regla.")
print()
print("     `OOS4001`  NO SALTA donde deberia. `hr.Employee.totalCompensation`")
print("                deriva de `baseSalary` y `bonus`, las dos `critical`, y")
print("                al materializarla el sello dice:")
print("                  «lleva `gdpr.sensitivity:high` (HEREDADA)»")
print("                —el suelo del datasource— y no `critical` (computada).")
print("                El codigo que emite `OOS4001` no llega.")
print()
print("   No se afirma aqui que sea un defecto: no esta medido, y puede que la")
print("   propagacion sea correcta por un motivo que no se ha mirado. Lo que SI")
print("   esta medido es que la unica prueba de `OOS4001` usa un `Binding`, asi")
print("   que si esto fuera un defecto la suite estaria en verde igualmente.")
print("   Esa es la matter, en un ejemplo concreto y no en una cifra.")

# -- D - LO QUE COSTO --------------------------------------------------------
print()
print("D - DONDE SE NOTO YA, con nombre")
print("   Las medidas de los ultimos peldanos tuvieron que apoyarse en el unico")
print("   ejemplo completo, y eso costo dos numeros equivocados:")
print("     `sustrato.md` M4 conto el precio de `B0` en UNA entidad, sobre un")
print("       arbol en el que `hr.empleados` estaba materializada. Eran dos.")
print("     `medida-b0-impagable` leyo el suelo de `hr_workday` como `medium`")
print("       de un comentario envuelto. Es `high`.")
print()
print("   Los dos son el mismo fallo: una sola fuente de verdad para el")
print("   paradigma nuevo, y sin nada que la contradiga.")
n_aristas = len([c for c in CASOS if c["paradigma"] in ("tabla+vista", "los dos")])
print()
print("   %-40s %3d" % ("casos escritos en el paradigma nuevo", n_aristas))
print("   %-40s %3d" % ("...de los cuales son de v1alpha8",
                        sum(1 for c in CASOS if c["borrador"] == "v1alpha8"
                            and c["paradigma"] in ("tabla+vista", "los dos"))))

# -- E - EL CAMINO -----------------------------------------------------------
print()
print("E - EL CAMINO: que es mecanico y que no")
print("   Un `Binding` decia TRES cosas: donde vive el objeto, que columnas")
print("   tiene, y que sale de el con que nombre. Las dos primeras son la")
print("   `Table` y la tercera es la `View` — y esa particion es exactamente lo")
print("   que `03-binding` dice al cerrarse.")
print()
print("   Asi que la traduccion es mecanica salvo en dos sitios:")
print("     - `capabilities` del binding -> la cara `reads` de la tabla, que es")
print("       un cambio de sujeto: lo que el ORIGEN admite, no lo que la vista")
print("       pide. Un binding sin `capabilities` no dice lo mismo que una")
print("       tabla sin `reads`")
print("     - `materialization.<eje>` -> `materialized` mas el sello del")
print("       indice, que es lo que se acaba de encender")
print()
print("   -> y por eso la migracion NO es un `sed`: cada caso migrado hay que")
print("      volver a mirarlo, porque el paradigma nuevo dice cosas que el")
print("      viejo no podia decir. Migrar %d casos a ciegas convertiria una"
      % tot["binding"])
print("      suite que mide en una que repite.")
