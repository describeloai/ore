# -*- coding: utf-8 -*-
"""El paso 5: M2 y B0. Y una de las dos no es lo que `entidad.md` dice.

`entidad.md` §8 los pone juntos como «lo que se quita»:

    M2   los 38 nombres que solo repiten; `properties` pasa a ANOTAR
    B0   `OOS2026` · la arista, que ya esta en la copia        2 entidades

La segunda fila esta mal, y lo dice la fuente primaria. Cinco frentes:

  A. QUE ES B0        leido de `handoff-topologia.md`, que es quien lo define
  B. M2 · EL TIPO     los 38 «solo repiten nombre y tipo». El nombre si sobra.
                      ¿Y el tipo? De donde saldria si la propiedad desaparece
  C. M2 · LA REGLA    `OOS2022` se invierte, y hay que decir en que
  D. EL PRECIO        cuanto cuesta cada uno, contado
  E. ¿SON UNO O DOS?  si dependen entre si, y de que
"""
import collections
import pathlib
import re
import sys

RAIZ = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\ORE\vendor\oos")
CRATES = pathlib.Path(r"C:\ORE\crates")


def documentos(raiz):
    for f in sorted(raiz.rglob("*.yaml")):
        txt = f.read_text(encoding="utf-8", errors="replace")
        for d in re.split(r"^---\s*$", txt, flags=re.M):
            m = re.search(r"^kind:\s*(\w+)", d, re.M)
            if m:
                yield m.group(1), d, f


def seccion(d, clave, hermanas):
    """El cuerpo de una clave de `spec`, cortado en la siguiente hermana."""
    if clave + ":" not in d:
        return ""
    resto = d.split(clave + ":", 1)[1]
    m = re.search(r"^  (?:%s):" % "|".join(hermanas), resto, re.M)
    return resto[: m.start()] if m else resto


SPEC = ["backedBy", "implements", "nature", "principal", "primaryKey", "timeKey",
        "uniqueKeys", "temporal", "properties", "relations", "moved", "reserved"]


def propiedades(d):
    """nombre -> el texto de esa propiedad, y nada del vecino."""
    cuerpo = seccion(d, "properties", SPEC)
    out, actual, nombre = {}, [], None
    for linea in cuerpo.split("\n"):
        m = re.match(r"^    ([A-Za-z_]\w*):(.*)$", linea)
        if m:
            if nombre:
                out[nombre] = "\n".join(actual)
            nombre, actual = m.group(1), [m.group(2)]
        elif nombre is not None and (linea.startswith("      ") or not linea.strip()):
            actual.append(linea)
        elif nombre:
            out[nombre] = "\n".join(actual)
            nombre, actual = None, []
    if nombre:
        out[nombre] = "\n".join(actual)
    return out


docs = list(documentos(RAIZ))
print("== corpus:", RAIZ, "==")

# ── A · QUE ES B0 ───────────────────────────────────────────────────────────
print()
print("A - QUE ES B0, leido de quien lo define")
print("   `handoff-topologia.md` §B0:")
print("     «`OOS2026` — lo que se ATRAVIESA se debe materializar. El TERCER")
print("      GEMELO de una familia que ya tiene dos: `OOS2020` lo que no se")
print("      puede leer, `OOS2025` lo que se escribe.»")
print("     «Se atraviesa es DERIVABLE y no se declara: la entidad tiene una")
print("      `relations` con `via`.»")
print()
print("   -> B0 NO quita nada de la entidad: es una REGLA que obliga a")
print("      materializar. Y no puede quitar `via`, porque `via` es de donde")
print("      sale la arista —`sustrato.md` M4: el indice de topologia es «una")
print("      proyeccion de dos columnas... por cada relacion con `via`»—.")
print("      Quitarlo no cobraria una redundancia: borraria el dato.")
print()
print("   `entidad.md` §8 y §10 lo describen como «la arista, que ya esta en la")
print("   copia | 2 entidades», y esa lectura no se sostiene. Es la forma")
print("   arcaica que este peldano tiene que suprimir antes de escribir nada.")

# ── B · M2 · EL TIPO ────────────────────────────────────────────────────────
print()
print("B - M2: los 38 «solo repiten nombre y tipo». ¿De donde saldria el tipo?")
SOLO_TIPO = []
anotan = 0
for kind, d, f in docs:
    if kind != "Entity" or not re.search(r"backedBy:", d):
        continue
    for n, cuerpo in propiedades(d).items():
        claves = set(re.findall(r"(?:^|[{,\s])(\w+):", cuerpo))
        claves.discard("type")
        if claves:
            anotan += 1
        else:
            m = re.search(r"type:\s*([^,}\n]+)", cuerpo)
            SOLO_TIPO.append((n, (m.group(1).strip() if m else "?")))
print("   %-46s %3d" % ("propiedades que ANOTAN algo mas que el tipo", anotan))
print("   %-46s %3d" % ("propiedades que solo declaran `type`", len(SOLO_TIPO)))
print()
c = collections.Counter(t for _, t in SOLO_TIPO)
for k, v in c.most_common():
    print("     %-22s %3d" % (k, v))
print()
print("   -> el NOMBRE sobra —lo dice `fields`— pero el TIPO no esta en ninguna")
print("      vista: la vista no tipa. Asi que M2 no es borrar 38 lineas: es")
print("      DERIVAR el tipo de la columna, o quedarse con la propiedad.")

# ── B.2 · ¿hay de donde derivarlo? ──────────────────────────────────────────
print()
print("B.2 - ¿Y hay de donde derivarlo? Dos cosas que hay que mirar")
cols = collections.Counter()
for kind, d, _ in docs:
    if kind != "Table" or "columns:" not in d:
        continue
    cuerpo = seccion(d, "columns", ["reads", "changes", "profile", "datasource", "object"])
    for linea in re.findall(r"^    [\"\w.]+:\s*(.*)$", cuerpo, re.M):
        cols["con physicalType" if "physicalType" in linea else "SIN physicalType"] += 1
print("   %-46s %3d" % ("columnas de tabla con `physicalType`", cols["con physicalType"]))
print("   %-46s %3d" % ("  sin el —es opcional", cols["SIN physicalType"]))
lector = (CRATES / "ore-cli/src/lector.rs").read_text(encoding="utf-8", errors="replace")
nucleo = (CRATES / "ore-core/src/types.rs").read_text(encoding="utf-8", errors="replace")
print("   %-46s %s" % ("mapeo tipo fisico -> tipo OOS en el LECTOR",
                       "SI" if "fn tipo_oos" in lector else "no"))
print("   %-46s %s" % ("  ...y en el NUCLEO", "si" if "fn tipo_oos" in nucleo else "NO"))
print("   -> el mapeo existe, y existe UNA vez por driver y solo en el")
print("      descubrimiento. En `ore-core` no hay ninguno. Derivar el tipo al")
print("      compilar seria una pieza NUEVA — y con un 13 % de columnas sin")
print("      `physicalType`, no siempre habria de donde.")

# ── C · LA REGLA QUE SE INVIERTE ────────────────────────────────────────────
print()
print("C - LA REGLA QUE SE INVIERTE: `OOS2022`")
print("   hoy   cada PROPIEDAD debe ser campo de su vista, o declarar")
print("         `derivedFrom`. La entidad manda y la vista tiene que cubrirla;")
print("   con M2  cada ANOTACION debe nombrar un campo. La vista manda y la")
print("         entidad solo puede hablar de lo que hay.")
print()
print("   -> es la misma comprobacion con los papeles cambiados, asi que no")
print("      hace falta codigo nuevo. Lo que cambia es de quien es el error.")

# ── D · EL PRECIO ───────────────────────────────────────────────────────────
print()
print("D - EL PRECIO, contado")
con_via = sum(1 for k, d, _ in docs
              if k == "Entity" and re.search(r"(?:^|[{,\s])via:", d, re.M))
con_via_y_bb = sum(1 for k, d, _ in docs
                   if k == "Entity" and re.search(r"(?:^|[{,\s])via:", d, re.M)
                   and re.search(r"backedBy:", d))
print("   B0 · entidades con `via`                        %3d" % con_via)
print("        de esas, ya migradas a `backedBy`          %3d" % con_via_y_bb)
print("        el handoff lo conto: UNA tendria que anadir `materialized`")
print("   M2 · propiedades que se irian                   %3d" % len(SOLO_TIPO))
print("        propiedades que se quedan como anotacion   %3d" % anotan)

# ── E · ¿UNO O DOS? ─────────────────────────────────────────────────────────
print()
print("E - ¿SON UN PELDANO O DOS?")
print("   DOS, y ni siquiera del mismo plano:")
print("     B0 es una regla sobre la VISTA —vive en `02-view` §5 y en")
print("        `vistas::comprobar`, como sus dos gemelos— y no toca la entidad;")
print("     M2 es una reforma de la ENTIDAD, y arrastra una pieza que no")
print("        existe: el mapeo tipo fisico -> tipo OOS en el nucleo.")
print()
print("   -> B0 es del tamano de un gemelo: gramatica cero, un codigo, un caso.")
print("      M2 no es «borrar 38 lineas» y hasta hoy se contaba asi.")
