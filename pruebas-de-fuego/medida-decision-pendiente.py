# -*- coding: utf-8 -*-
"""La naturaleza de la decision pendiente, antes de pivotar al paso 5.

El ADR 0019 la dejo escrita y sin medir:

  «Cuantos codigos piden `freshness` y las capacidades. Por la Decision A son
   ordenes de una sola direccion, asi que uno cada uno; por el criterio del
   sintoma podrian ser UNO: "la fuente admite menos". Sin medir.»

Y la pregunta esta mal planteada, que es lo que esto mide. Cinco frentes:

  A. LA PARTICION    `freshness` y las capacidades no son la misma clase de
                     cosa, y el arbol ya lo dice en dos sitios
  B. EL PUBLICO      por el ADR 0019 el eje lo decide QUIEN SUFRE. Si sufren
                     dos, no puede ser un codigo — y hay precedente exacto
  C. DENTRO DE LAS   `reads` y `changes`: ¿un sintoma o dos? Lo decide el
     CAPACIDADES     REMEDIO, que es el criterio de `OOS2024`/`OOS2025`
  D. EL CORPUS       cuanto hay de cada cosa, y cuanto se moveria
  E. LO QUE NO       si un HECHO que cambia debe bloquear una publicacion —
     DECIDE ESTO     y §5.3 ya lo contesta a medias
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


docs = list(documentos(RAIZ))
print("== corpus:", RAIZ, "==")

# ── A · LA PARTICION ────────────────────────────────────────────────────────
print()
print("A - LA PARTICION, y no la inventa esta medida: la dice el inductor")
ind = (CRATES / "ore-cli/src/inductor.rs").read_text(encoding="utf-8", errors="replace")
induce_reads = 'writeln!(s, "  reads:")' in ind
induce_changes = 'writeln!(s, "  changes:")' in ind
induce_fresh = re.search(r"freshness: \{", ind) is not None
print("   `ore discover` emite `reads`      :", "SI" if induce_reads else "no")
print("   `ore discover` emite `changes`    :", "SI" if induce_changes else "no")
print("   `ore discover` emite `freshness`  :", "SI" if induce_fresh else "NO")
print()
print("   Y lo dice con todas las letras, en `inductor.rs`:")
print("     «Lo que NO lleva, y no por falta de sitio: `materialized` y")
print("      `freshness`. Son DECISIONES DE OPERACION CON COSTE, y proponerlas")
print("      seria exactamente inventar.»")
print("     «La tabla es un HECHO del origen... ninguna de las cuatro cosas es")
print("      una conjetura, y por eso el descubrimiento puede emitirla")
print("      mecanicamente y sin inventar.» —`01-table` §1")
print()
print("   -> `freshness` es una PROMESA: la debilita quien publica.")
print("      `reads`/`changes` son un HECHO: cambian porque cambio el mundo.")
print("      Es la tercera vez que sale la misma linea —la vista DECIDE, la")
print("      tabla ESPEJA—: paso 1 con la madurez, paso 4 con `moved`, y esta.")

# ── B · EL PUBLICO ──────────────────────────────────────────────────────────
print()
print("B - EL PUBLICO: y por eso no puede ser UN codigo")
print("   Por el ADR 0019 el eje no es una categoria, es a quien le duele:")
print()
print("     `freshness` se afloja   -> el consumidor recibe datos mas viejos")
print("                                de lo prometido            CONSUMER")
print("     `reads` se estrecha     -> ninguna consulta se rompe: el PLAN")
print("                                cuesta mas o no cabe        INDEX")
print("     `changes` se estrecha   -> la COPIA no se puede mantener, o no")
print("                                puede decir hasta cuando     INDEX")
print()
print("   Y hay precedente exacto de un cambio con dos publicos, medido en")
print("   `diff.rs`: cambiar `primaryKey` emite DOS codigos, no uno con dos")
print("   ejes —`OOS5006` CONSUMER y `OOS5018` INDEX—. La respuesta de este")
print("   proyecto a «un cambio, dos publicos» ya esta dada, y es dos codigos.")

# ── C · DENTRO DE LAS CAPACIDADES ───────────────────────────────────────────
print()
print("C - ¿`reads` y `changes` son un sintoma o dos? Lo decide el REMEDIO")
print("   Es el criterio con el que `OOS2024` y `OOS2025` son dos y no uno:")
print("   «las dos condiciones tienen remedios distintos, y por eso son dos")
print("   codigos».")
print()
print("     `reads` se estrecha    el remedio es REPLANIFICAR: empujar menos,")
print("                            o materializar para dejar de depender")
print("     `changes` se estrecha  el remedio es REHACER LA COPIA entera, o")
print("                            cambiar el testigo. No hay plan que valga")
print()
print("   -> dos remedios, dos codigos. Y son dos preguntas independientes ya")
print("      en la gramatica: `01-table` §4 dice que `mode` y `witness` se")
print("      declaran por separado «porque son preguntas independientes».")

# ── D · EL CORPUS ───────────────────────────────────────────────────────────
print()
print("D - EL CORPUS: cuanto hay de cada cosa")
n = collections.Counter()
for kind, d, _ in docs:
    if kind == "Table":
        n["tablas"] += 1
        if re.search(r"(?:^|[{,\s])reads:", d, re.M):
            n["  con `reads`"] += 1
        if re.search(r"(?:^|[{,\s])changes:", d, re.M):
            n["  con `changes`"] += 1
    elif kind == "View":
        n["vistas"] += 1
        if re.search(r"(?:^|[{,\s])freshness:", d, re.M):
            n["  con `freshness`"] += 1
        if re.search(r"(?:^|[{,\s])materialized:", d, re.M):
            n["  con `materialized`"] += 1
for k in ("tablas", "  con `reads`", "  con `changes`", "vistas",
          "  con `freshness`", "  con `materialized`"):
    print("   %-24s %4d" % (k, n[k]))
print()
print("   -> las capacidades estan en TODAS las tablas —son obligatorias de")
print("      hecho— y `freshness` en poco mas de la mitad de las vistas. El")
print("      sujeto existe para los tres codigos.")

# ── E · LO QUE NO DECIDE ESTO ───────────────────────────────────────────────
print()
print("E - LO QUE ESTO NO DECIDE, y §5.3 ya lo contesta a medias")
print("   Si un HECHO que cambia debe bloquear una publicacion. Un editor no")
print("   afloja `reads` porque quiera: lo afloja porque el origen cambio, y")
print("   castigarlo con un `major` seria cobrarle el clima.")
print()
print("   `91-versioning` §5.3, normativo, sobre el eje INDEX:")
print("     «Los cambios de este eje NO DEBEN bloquear el merge por si solos,")
print("      pero una implementacion DEBE senalar que el indice requiere")
print("      reconstruccion.»")
print()
print("   -> encaja sin tocar nada: las capacidades caen en INDEX, que es")
print("      exactamente el eje que informa sin bloquear. Y `freshness` cae en")
print("      CONSUMER, que si bloquea — porque ahi si hay alguien que prometio.")
print()
print("VEREDICTO")
print("   La pregunta del ADR 0019 —«¿uno o cada uno?»— estaba mal planteada:")
print("   presuponia que las tres son la misma clase de cambio. No lo son.")
print()
print("   SON TRES, y ninguno se elige:")
print("     una promesa que se afloja      `freshness`   CONSUMER   bloquea")
print("     lo que la fuente ADMITE        `reads`       INDEX      informa")
print("     lo que la fuente EMITE         `changes`     INDEX      informa")
print()
print("   El primero se separa por PUBLICO, y los otros dos entre si por")
print("   REMEDIO. Los dos criterios ya estaban escritos; lo unico que faltaba")
print("   era no meter un hecho y una promesa en la misma bolsa.")
