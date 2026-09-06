# -*- coding: utf-8 -*-
"""Los niveles de conformidad: cuales valen proyectados, y cuales no.

`L2` esta huerfano, y al mirarlo salio que el problema es mas grande que L2:
son CUATRO niveles, dos de ellos definidos sobre vocabulario retirado, y el
ejecutor de la suite no lee el campo que los declara.

La pregunta no es «como arreglamos L2». Es cuales de estos conceptos aportan
valor genuino cuando se proyectan, y cuales son friccion. Y se contesta con
dos preguntas por concepto, las dos medibles:

    ¿QUE COMPRA?   que puede verificar alguien gracias a esto, hoy
    ¿QUE CUESTA?   que hay que escribir, mantener o creerse para tenerlo

Un concepto que compra algo y no cuesta nada se queda. Uno que cuesta y no
compra es friccion con nombre propio, que es la peor clase.
"""
import collections
import pathlib
import re
import subprocess

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
SPEC = OOS / "spec"
CONF = OOS / "conformance"
RUNNER = RAIZ / "crates/ore-cli/tests/conformance.rs"


def grep(patron, donde, ext="*.md"):
    """Se lee en Python en vez de invocar `grep`.

    La version con `subprocess` devolvia vacio para el arbol de `crates` y
    tardaba minutos en el de la spec, y las dos cosas son el mismo fallo:
    un arnes que sale del proceso para leer ficheros hereda el sistema de
    ficheros de otro. Aqui no hay rutas que citar ni salida que partir —y de
    paso desaparece el problema de que `C:` lleve dos puntos.
    """
    out = []
    for f in sorted(pathlib.Path(donde).rglob(ext)):
        for i, l in enumerate(f.read_text(encoding="utf-8", errors="replace")
                              .split("\n"), 1):
            if re.search(patron, l):
                out.append((f.name, i, l.strip()))
    return out


print("== los niveles de conformidad, medidos ==")

# -- A - EL SISTEMA COMPLETO -------------------------------------------------
print()
print("A - SON CUATRO, no tres, y dos nombran vocabulario retirado")
NIVELES = [
    ("L0", "Validador", "analiza, valida, ejecuta el flujo `OOS4xxx`, emite el digest",
     "no", ["esquema", "referencial", "flujo"], True),
    ("L1", "Servidor de contexto", "sirve entidades, relaciones, tipos, politicas, linaje",
     "no", ["artefacto compilado"], True),
    ("L2", "Ejecutor", "resuelve BINDINGS contra fuentes reales, federa consultas",
     "si", ["Binding", "05-ejecutor"], False),
    ("L3", "Actor", "ejecuta funciones con capacidades, verifica el acto de un endoso",
     "si, con escritura", ["Function", "endoso"], None),
]
for n, papel, que, dato, vocab, vivo in NIVELES:
    print("   %-4s %-22s toca dato: %-18s" % (n, papel, dato))
    print("        %s" % que)
print()
hist = []
for f in ("v1alpha1/03-binding.md", "v1alpha1/05-ejecutor.md"):
    t = (SPEC / f).read_text(encoding="utf-8", errors="replace")
    if re.search(r"Estado:.{0,40}hist[oó]rico", t):
        hist.append(pathlib.Path(f).name)
print("   documentos historicos que L2 nombra: %s" % ", ".join(hist))
print("   -> `L2` se define como «resuelve bindings» y su documento de")
print("      referencia acaba de pasar a historico. `L3` se apoya en `Function`,")
print("      que si esta vivo, pero su certificacion nunca existio.")

# -- B - QUIEN LOS CONSUME ---------------------------------------------------
print()
print("B - QUIEN CONSUME UN NIVEL. Es la pregunta que lo decide todo")
campos = collections.Counter()
for c in sorted(CONF.rglob("case.yaml")):
    for k in re.findall(r"^(\w+):", c.read_text(encoding="utf-8", errors="replace"), re.M):
        campos[k] += 1
runner = RUNNER.read_text(encoding="utf-8", errors="replace")
print("   campos que los %d casos declaran, y si el ejecutor los lee:"
      % max(campos.values()))
for k, v in campos.most_common():
    lee = ('"%s"' % k) in runner
    print("     %-10s %3d casos   %s" % (k, v, "LEIDO" if lee else "nadie lo lee"))
print()
motor = grep(r'"level"', RAIZ / "crates", "*.rs")
print("   y en el motor, quien AFIRMA un nivel:")
for f, _, txt in motor:
    if "levels" not in txt:
        print("     %-14s %s" % (f, txt[:66]))
print()
print("   -> el ejecutor de la suite lee UN campo: `expects`. `level`, `rule` y")
print("      `summary` los declaran los %d casos y no los consume nada."
      % max(campos.values()))
print("      El unico sitio del motor que afirma un nivel es el servidor MCP,")
print("      que se declara `L1` a si mismo — y nadie lo comprueba.")

# -- C - QUE COMPRA CADA UNO -------------------------------------------------
print()
print("C - QUE COMPRA CADA NIVEL: que puede verificar alguien, hoy")
por_nivel = collections.Counter()
for c in sorted(CONF.rglob("case.yaml")):
    m = re.search(r"^level:\s*(\S+)", c.read_text(encoding="utf-8", errors="replace"), re.M)
    por_nivel[m.group(1) if m else "(ninguno)"] += 1
for n, v in por_nivel.most_common():
    print("     %-10s %3d casos" % (n, v))
print()
print("   Y lo que la spec dice de por que L2/L3 no tienen ninguno —esto NO es")
print("   un hueco, esta escrito y decidido en `v1alpha2/00-scope` §5:")
print("     «Es una comprobacion sobre datos, y por tanto L2/L3: NO ES")
print("      CERTIFICABLE POR UNA SUITE DE FICHEROS. Va despues.»")
print("     «Invocar una funcion es L3, y L2/L3 probablemente no sean")
print("      certificables por una suite de ficheros.»")
print()
print("   -> asi que L2 y L3 no estan incompletos: NO SON NIVELES DE")
print("      CONFORMIDAD. La spec ya lo sabia y los dejo en la misma tabla que")
print("      L0, que si lo es. Ese es el arcaismo — no que L2 nombre `Binding`.")

# -- D - EL VALOR DE L0, Y QUE ACABA DE CRECER -------------------------------
print()
print("D - EL VALOR DE L0, que es lo que el estandar vende")
codigos = re.findall(r"Oos(\d{4})", (RAIZ / "crates/ore-core/src/code.rs")
                     .read_text(encoding="utf-8"))
fam = collections.Counter(c[0] for c in codigos)
NOMBRE = {"1": "forma", "2": "referencia", "3": "canonica", "4": "FLUJO",
          "5": "diff", "6": "firma", "7": "emision", "8": "gobierno", "9": "runtime"}
for k in sorted(fam):
    print("     OOS%sxxx  %-12s %2d codigos" % (k, NOMBRE.get(k, "?"), fam[k]))
print()
print("   %d codigos, y **todos se comprueban sin abrir una conexion**. Eso es"
      % len(set(codigos)))
print("   la afirmacion de `00-overview`, y es la unica del documento que")
print("   ninguna plataforma del mercado puede copiar:")
print("     «Un auditor puede comprobar que un paquete no filtra informacion")
print("      clasificada EJECUTANDO UN VALIDADOR SOBRE EL REPOSITORIO, sin que")
print("      nadie le conceda acceso a un solo dato de la empresa.»")
print()
print("   Y esta semana L0 CRECIO, que es el dato que faltaba: el sello del")
print("   indice movio al compilador algo que antes no miraba nadie —las")
print("   aristas se copiaban sin conducto— y `owner` en el suelo movio otra.")
print("   Las dos eran preocupaciones de tiempo de ejecucion en cualquier otro")
print("   sistema. Aqui fallan al compilar.")

# -- E - LA FRICCION, CONTADA ------------------------------------------------
print()
print("E - LA FRICCION: que cuesta hoy tener cuatro niveles")
n_casos = max(campos.values())
print("   %-46s %3d" % ("ficheros que declaran `level` sin lector", n_casos))
print("   %-46s %3d" % ("menciones a L2/L3 en la spec",
                        len(grep(r"\bL[23]\b", SPEC))))
print("   %-46s %3d" % ("...y casos que las ejercen", por_nivel.get("L2", 0)
                        + por_nivel.get("L3", 0)))
print()
print("   -> el coste no es escribir `level: L0` 256 veces: es que la tabla de")
print("      niveles PROMETE una escalera de conformidad y dos de sus cuatro")
print("      peldanos no se pueden pisar. Quien lee `00-overview` §3.2 entiende")
print("      que puede certificarse L2. No puede, y no por falta de trabajo:")
print("      por definicion, y esta escrita tres documentos mas alla.")

# -- F - PROYECTADO ----------------------------------------------------------
print()
print("F - PROYECTADO: ¿aguanta la particion cuando lleguen los drivers?")
CRATES = [("leer de una fuente", "ore-read-postgres"),
          ("el protocolo", "ore-driver"),
          ("materializar una copia", "ore-store-r2"),
          ("mantenerla incremental", "ore-maintain"),
          ("servir contexto", "ore-cli/src/mcp.rs")]
for que, c in CRATES:
    existe = (RAIZ / "crates" / c).exists()
    print("     %-26s %-22s %s" % (que, c, "existe" if existe else "NO"))
print()
print("   -> `L2` junta TRES cosas que ya viven en crates distintos y con")
print("      estados distintos: leer, materializar y mantener. Una")
print("      implementacion puede leer sin materializar, y hoy la nuestra")
print("      materializa sin poder leer casi ninguna fuente. Un solo nivel para")
print("      las tres no describe a nadie.")

# -- G - VEREDICTO -----------------------------------------------------------
print()
print("G - VEREDICTO: que concepto vale proyectado y cual no")
print()
print("   VALE, y es lo unico que el estandar puede vender")
print("     L0 · hermetico. %d codigos, %d casos, cero conexiones. Es la"
      % (len(set(codigos)), por_nivel.get("L0", 0)))
print("        propiedad que ninguna plataforma del mercado tiene, y crece cada")
print("        vez que algo pasa de tiempo de ejecucion a tiempo de compilacion.")
print()
print("   VALE, con poco que ensenar todavia")
print("     L1 · servir el artefacto. %d casos y el MCP se declara L1. Es"
      % por_nivel.get("L1", 0))
print("        comprobable —servir un bundle es determinista— y esta sin")
print("        ejercer, que es distinto de no ser certificable.")
print()
print("   NO VALE COMO NIVEL DE CONFORMIDAD")
print("     L2 · L3. La spec ya dice que no son certificables por una suite de")
print("        ficheros, asi que ponerlos en la misma escalera que L0 promete")
print("        algo que no se puede cobrar. No son niveles: son CAPACIDADES.")
print("        Un producto las anuncia; un estandar no las certifica.")
print()
print("   FRICCION PURA")
print("     `level`, `rule` y `summary` en %d casos, leidos por nadie. `expects`" % n_casos)
print("        es el unico que se gana el sitio, y por eso es el unico que")
print("        rompe la suite cuando esta mal.")
