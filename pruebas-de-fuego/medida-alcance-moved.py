# -*- coding: utf-8 -*-
"""El alcance de la iteracion de `moved`, antes de escribir nada.

La forma esta cotejada: UN mecanismo —`moved`/`reserved`— y tres casas
elegidas por una sola regla, **lo dice el que sobrevive; si no sobrevive nadie,
lo dice el paquete**:

  Entity.spec    un nombre de PROPIEDAD    ya existe
  View.spec      un nombre de CAMPO        falta
  Package.spec   un nombre de DOCUMENTO    falta

Esto mide lo que cuesta, en seis frentes:

  A. LA PUERTA        donde se admite el campo, y quien lo tiene ya
  B. EL RIGOR QUE HAY que comprueba el motor HOY sobre `moved` y `reserved`.
                      La extension debe igualarlo, ni mas ni menos
  C. QUIEN LEE        `anunciados` en `diff`, y de donde se llena
  D. EL SUJETO        cuantos documentos habria que tocar
  E. LAS REGLAS       las dos que aparecen, y una de ellas la dejo pendiente
                      el paso 3
  F. EL TAMANO        que ficheros se tocan
"""
import collections
import pathlib
import re
import shutil
import subprocess
import sys

ORE = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\ORE\target\debug\ore.exe")
RAIZ = pathlib.Path(r"C:\ORE\vendor\oos")
CRATES = pathlib.Path(r"C:\ORE\crates")
TMP = pathlib.Path(
    r"C:\Users\PC\AppData\Local\Temp\claude\C--ORE"
    r"\b4ce4f86-cd8b-429f-9c14-8865e67fa2c6\scratchpad\alcance-moved"
)


def documentos(raiz):
    for f in sorted(raiz.rglob("*.yaml")):
        txt = f.read_text(encoding="utf-8", errors="replace")
        for d in re.split(r"^---\s*$", txt, flags=re.M):
            m = re.search(r"^kind:\s*(\w+)", d, re.M)
            if m:
                yield m.group(1), d, f


def correr(*a):
    r = subprocess.run([str(ORE), *[str(x) for x in a]], capture_output=True, text=True)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


docs = list(documentos(RAIZ))
print("== corpus:", RAIZ, "==")

# ── A · LA PUERTA ───────────────────────────────────────────────────────────
print()
print("A - LA PUERTA: quien admite `moved` hoy, en `document.rs::spec_keys`")
doc = (CRATES / "ore-core/src/document.rs").read_text(encoding="utf-8", errors="replace")
tiene = re.findall(r"Kind::(\w+) => &\[([^\]]*)\]", doc, re.S)
for k, cuerpo in tiene:
    if '"moved"' in cuerpo:
        print("   %-14s ya lo tiene" % k)
print("   %-14s lo necesita para un CAMPO" % "View")
print("   %-14s lo necesita para un DOCUMENTO" % "Package")
print("   -> dos brazos del `match` y dos esquemas. No hay tipo nuevo:")
print("      `qualifiedName` es `^id(\\.id)*$`, asi que `hr.iberia` y")
print("      `hr.empleados.nationalId` son el mismo tipo.")

# ── B · EL RIGOR QUE HAY ────────────────────────────────────────────────────
print()
print("B - EL RIGOR QUE HAY HOY, medido corriendo el compilador")
CASO = RAIZ / "conformance/v1alpha8/valid/materialized-view-over-table-within-clearance/input"


def con(bloque, etiqueta):
    if TMP.exists():
        shutil.rmtree(TMP)
    shutil.copytree(CASO, TMP)
    p = TMP / "entities/Employee.yaml"
    p.write_text(p.read_text(encoding="utf-8").replace(
        "  properties:", bloque + "\n  properties:", 1), encoding="utf-8")
    cod, out = correr("validate", TMP)
    codigos = sorted(set(re.findall(r"OOS\d{4}", out)))
    print("   %-48s %s" % (etiqueta, ",".join(codigos) if cod else "ok"))


con("  moved:\n    - { from: dni, to: nationalId, since: 2.0.0 }",
    "`moved.from` sigue existiendo como propiedad")
con("  moved:\n    - { from: viejo, to: noExiste, since: 2.0.0 }",
    "`moved.to` no existe")
con("  reserved:\n    - { name: fantasma, reason: retirada }",
    "`reserved` de un nombre que nunca existio")
con("  reserved:\n    - { name: dni, reason: retirada }",
    "`reserved` de una propiedad VIVA")
print("   -> el motor comprueba UNA cosa: que no se reutilice un nombre")
print("      reservado —`OOS2006`—. `moved` no se comprueba en el enlazado:")
print("      solo alimenta a `diff`. La extension IGUALA ese rigor y no lo")
print("      sube: subirlo aqui seria cambiar la regla de la entidad de paso.")

# ── C · QUIEN LEE ───────────────────────────────────────────────────────────
print()
print("C - QUIEN LEE `anunciados`, y de donde se llena")
diff = (CRATES / "ore-core/src/diff.rs").read_text(encoding="utf-8", errors="replace")
print("   %-46s %3d" % ("veces que `anunciados` aparece en `diff.rs`",
                        len(re.findall(r"anunciados", diff))))
print("   %-46s %s" % ("se llena desde", "`moved.from` y `reserved` de la ENTIDAD"))
print("   %-46s %s" % ("lo consume", "`OOS5001`, para no gritar un borrado anunciado"))
print("   -> un solo sitio que leer y un solo sitio que llenar. Anadir dos")
print("      origenes mas no cambia a quien lo usa.")

# ── D · EL SUJETO ───────────────────────────────────────────────────────────
print()
print("D - EL SUJETO: cuantos documentos habria que tocar")
n = collections.Counter(k for k, _, _ in docs)
print("   %-46s %3d" % ("vistas en el corpus", n["View"]))
print("   %-46s %3d" % ("manifiestos", n["Package"]))
print("   %-46s %3d" % ("de esos, que tendrian que escribir algo", 0))
print("   -> cero, como en la madurez y como en `exports`: los tres campos son")
print("      opcionales y solo se escriben el dia que se renombra.")

# ── E · LAS REGLAS ──────────────────────────────────────────────────────────
print()
print("E - LAS DOS REGLAS QUE APARECEN, y ningun codigo nuevo")
print("   `OOS2006` gana el caso ancho: crear una vista, una tabla o una")
print("             entidad con el nombre de una RETIRADA deja de compilar.")
print("             Hoy solo mira miembros;")
print("   `OOS5001` gana sujeto: un CAMPO de vista que desaparece sin anuncio.")
print("             Es una de las tres mutaciones que el paso 3 dejo mudas, y")
print("             la unica que esperaba a esto.")
print()
print("   Y el renombrado deja de leerse como un borrado — que es todo el")
print("   punto: hoy `hr.iberia -> hr.iberica` sale como `OOS5007`, «desaparecio»,")
print("   sin una linea que lo relacione con el que aparece.")

# ── F · EL TAMANO ───────────────────────────────────────────────────────────
print()
print("F - EL TAMANO")
for f, que in (("ore-core/src/document.rs", "dos brazos de `spec_keys`"),
               ("ore-core/src/link.rs", "`OOS2006`, hoy solo sobre miembros"),
               ("ore-core/src/diff.rs", "`anunciados`, dos origenes mas"),
               ("ore-core/src/normalize.rs", "`moved`/`reserved` ya estan en CONJUNTOS")):
    t = (CRATES / f).read_text(encoding="utf-8", errors="replace")
    print("   %-32s %5d lineas   %s" % (f.split("/")[-1], t.count("\n") + 1, que))
print("   + los dos esquemas de la spec, y los casos de conformidad.")
print("   -> ningun codigo nuevo, ningun tipo nuevo, ningun eje nuevo. Es la")
print("      iteracion mas barata de las cuatro, y la que menos decide: la")
print("      forma la decidio el cotejo.")
