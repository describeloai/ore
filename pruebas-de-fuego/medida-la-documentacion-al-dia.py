# -*- coding: utf-8 -*-
"""¿Esta la documentacion al dia desde la iteracion de paquetes?

No es la medida de `medida-la-documentacion.py` —aquella pregunta si el modelo
se dice UNA vez y si los numeros se sostienen—. Esta pregunta otra cosa, mas
estrecha y mas urgente: **lo que se ha construido desde entonces, ¿esta dicho
en alguna parte?**

  A. LAS ITERACIONES     cuales tocaron `docs/` y cuales no
  B. LOS CONCEPTOS       lo construido, y si aparece
  C. LO QUE YA ES FALSO  claims que el arbol dejo de sostener
  D. DONDE VA CADA COSA  y cuanto es de verdad
"""
import pathlib
import re
import subprocess
import textwrap

RAIZ = pathlib.Path(r"C:\ORE")
DOCS = RAIZ / "docs"


def parrafo(t, sangria="     ", ancho=70):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def git(*args):
    return subprocess.run(["git", *args], cwd=str(RAIZ),
                          capture_output=True, text=True, errors="replace").stdout


def en_docs(patron):
    """Que ficheros de `docs/` mencionan esto. Se busca en el TEXTO y no con
    `git grep` porque interesa lo que hay hoy, no lo que se commiteo."""
    out = []
    for f in sorted(DOCS.rglob("*.md")):
        if re.search(patron, f.read_text(encoding="utf-8", errors="replace"), re.I):
            out.append(f.relative_to(DOCS).as_posix())
    return out


print("== la documentacion al dia, medido ==")

# -- A -------------------------------------------------------------------------
print()
print("A - LAS ITERACIONES, Y CUAL TOCO `docs/`")
print()
# El primer commit de la iteracion de paquetes. De ahi hacia arriba.
DESDE = "d33e6d5"
lineas = git("log", "--oneline", "--reverse", "%s^..HEAD" % DESDE).splitlines()
tocan = set(git("log", "--format=%h", "%s^..HEAD" % DESDE, "--", "docs/").split())
print("   %-9s %-6s %s" % ("commit", "docs/", "asunto"))
print("   " + "-" * 74)
for l in lineas:
    sha, _, asunto = l.partition(" ")
    marca = "si" if sha in tocan else "--"
    print("   %-9s %-6s %s" % (sha, marca, asunto[:56]))
print()
n_si = sum(1 for l in lineas if l.split(" ")[0] in tocan)
parrafo("%d commits desde que empezo la iteracion de paquetes, y %d tocaron "
        "`docs/`. Los paquetes ENTEROS —la regla, los cuatro verbos, los dos "
        "codigos— no tocaron ninguno."
        % (len(lineas), n_si))

# -- B -------------------------------------------------------------------------
print()
print("B - LOS CONCEPTOS CONSTRUIDOS, Y SI ESTAN DICHOS")
print()
CONCEPTOS = [
    ("paquetes", "la pertenencia por kind", r"OOS2030|pertenencia por"),
    ("paquetes", "`ore package new`", r"package new"),
    ("paquetes", "`move` / `split` / `merge`", r"package (move|split|merge)"),
    ("paquetes", "la lapida y `OOS2031`", r"OOS2031|l[aá]pida"),
    ("deriva", "`ore drift-detect`", r"drift-detect"),
    ("deriva", "el ensanche de tipos", r"OOS5002|ensancha"),
    ("catalogo", "la forma en `ore-driver`", r"catalogo::|Catalogo \{|forma del cat"),
    ("catalogo", "`ore source catalog`", r"source catalog"),
    ("vista", "`groupBy` y el agregado", r"groupBy"),
    ("vista", "`having` y el umbral", r"having"),
    ("vista", "`OOS2032` / `OOS2033` / `OOS2034`", r"OOS203[234]"),
    ("vista", "`OOS5033`", r"OOS5033"),
    ("vista", "`expone` vs `campos`", r"`expone`|Raiz\.agrega|`agrega`"),
]
print("   %-12s %-34s %s" % ("iteracion", "concepto", "donde"))
print("   " + "-" * 76)
huerfanos = 0
for it, q, pat in CONCEPTOS:
    d = en_docs(pat)
    if not d:
        huerfanos += 1
    print("   %-12s %-34s %s" % (it, q, ", ".join(d) if d else "EN NINGUN SITIO"))
print()
parrafo("%d de %d conceptos no aparecen en `docs/`. Y no todos tienen que "
        "aparecer —un codigo vive en `99-errors.md` de OOS, que es su sitio— "
        "pero un verbo del mando y una clave del vocabulario si: son lo que "
        "alguien busca cuando quiere usar esto."
        % (huerfanos, len(CONCEPTOS)))

# -- C -------------------------------------------------------------------------
print()
print("C - LO QUE `docs/` AFIRMA Y EL ARBOL YA NO SOSTIENE")
print()
CADUCOS = [
    ("functions.md",
     "no se puede declarar una vista que agregue: el vocabulario de `View` en "
     "v1alpha8 es exactamente el fragmento invertible",
     "`groupBy` y `having` SON vocabulario de v1alpha8. El fragmento invertible "
     "y el vocabulario dejaron de coincidir, que es exactamente lo que la "
     "guarda esperaba"),
    ("functions.md",
     "un censo ata la clasificacion al vocabulario ... falsificado anadiendo "
     "`groupBy` y viendolo saltar",
     "el censo sigue, y `groupBy` ya no es el contraejemplo hipotetico: esta "
     "clasificado en `NO_INVERTIBLES`, la tercera lista que entonces no existia"),
    ("functions.md",
     "la costura solo construye cuatro nodos del IR y `Une`/`Agrupa`/`Limita` "
     "no aparecen en ningun camino que salga de un documento",
     "`Agrupa` si aparece. Quedan `Une` y `Limita`"),
    ("ontologia-como-repositorio.md",
     "paquete   init · lock · pack",
     "faltan `new`, `move`, `split` y `merge`, que son la iteracion entera"),
]
for f, dice, ya_no in CADUCOS:
    existe = "" if (DOCS / f).exists() else "  (el fichero no esta)"
    print("   · %s%s" % (f, existe))
    parrafo("dice: %s" % dice, "       ")
    parrafo("hoy:  %s" % ya_no, "       ")
    print()
parrafo("Los tres primeros viven en un BLOQUE CITADO de `functions.md`, que es "
        "una cronica: «listo cuando», «hecho», «retirado tres dias despues». "
        "Una cronica no se reescribe —seria borrar el registro— pero una frase "
        "en presente sobre la version VIGENTE si enga�a, y ahi la forma de la "
        "casa es anadir el siguiente asiento, no tachar el anterior.")

# -- D -------------------------------------------------------------------------
print()
print("D - DONDE VA CADA COSA, Y CUANTO ES")
print()
DESTINOS = [
    ("`docs/view-engine.md`", "groupBy, having, la entidad y la etiqueta",
     "ya tiene §5 al dia con `Agrupa`; le falta `having` y le falta el hueco "
     "del agregado global. Es el sitio canonico del motor"),
    ("`docs/ontologia-como-repositorio.md`", "los cuatro verbos de paquete",
     "es donde vive la tabla de mandos, y esta desactualizada por cuatro "
     "verbos y por `drift-detect`"),
    ("`docs/functions.md`", "el asiento nuevo de la cronica",
     "tres frases en presente que la version vigente ya no sostiene. No se "
     "reescribe: se anade lo que paso despues"),
    ("`docs/sustrato.md`", "nada, probablemente",
     "habla de la tabla y sus caras, y la iteracion no las toco. Conviene "
     "COMPROBARLO antes de darlo por bueno"),
]
for q, que, por in DESTINOS:
    print("   · %-38s %s" % (q, que))
    parrafo(por, "       ")
    print()
parrafo("Y la pregunta que decide el alcance: la documentacion de este arbol "
        "NO es un manual — es el sitio donde vive el razonamiento que el codigo "
        "no puede llevar. Asi que lo que hay que escribir no es «como se usa "
        "`ore package split`», que lo dice `--help`, sino POR QUE un corte "
        "cuesta un cruce y por que el mando dice el precio en vez de buscarlo.")
