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

# -- C ------------------------------------------------------------------------
print()
print("C - LO QUE `docs/` AFIRMABA Y EL ARBOL YA NO SOSTIENE")
print()
# Cada fila: el fichero, lo que decia, y EL RASTRO DE SU CORRECCION. La primera
# version era una lista fija que no comprobaba nada — decia lo mismo antes y
# despues de arreglarlo, que es un arnes que no sirve de termometro.
CADUCOS = [
    ("functions.md",
     "el vocabulario de `View` es exactamente el fragmento invertible",
     r"la gram[aá]tica creci[oó]",
     "`groupBy`, el agregado y `having` SON vocabulario de v1alpha8"),
    ("functions.md",
     "`groupBy` como contraejemplo hipotetico",
     r"NO_INVERTIBLES",
     "esta clasificado en la tercera lista, que entonces no existia"),
    ("functions.md",
     "`Une`/`Agrupa`/`Limita` no salen de ningun documento",
     r"Quedan `Une` y `Limita`",
     "`Agrupa` si. Quedan dos"),
    ("ontologia-como-repositorio.md",
     "paquete   init · lock · pack",
     r"init · lock · pack · package",
     "`package` entra, y `drift-detect` sale de la lista corta"),
    ("ontologia-como-repositorio.md",
     "16 de los 22 verbos · seis sin implementar",
     r"17 de los 23 verbos",
     "23 verbos y 5 sin implementar, contados sobre `main.rs`"),
]
abiertos = 0
for f, decia, rastro, ahora in CADUCOS:
    t_doc = (DOCS / f).read_text(encoding="utf-8", errors="replace")
    ok = re.search(rastro, t_doc) is not None
    if not ok:
        abiertos += 1
    print("   %-3s %s" % ("ok" if ok else "NO", f))
    parrafo("decia: %s" % decia, "       ")
    parrafo("hoy:   %s" % ahora, "       ")
    print()
print("   %d claim(s) sin corregir." % abiertos)
print()
parrafo("Los tres primeros viven en un BLOQUE CITADO de `functions.md`, que es "
        "una cronica: «listo cuando», «hecho», «retirado tres dias despues». "
        "Una cronica no se reescribe —seria borrar el registro— asi que la "
        "correccion es UN ASIENTO NUEVO, y por eso lo que se busca aqui es el "
        "rastro del asiento y no la ausencia de la frase vieja.")

# -- D ------------------------------------------------------------------------
print()
print("D - LOS NUMEROS QUE LA DOCUMENTACION AFIRMA, RECONTADOS")
print()
parrafo("El recuento de verbos es el numero que mas se pudre: crece cada vez "
        "que se anade un mando y nadie se acuerda de la frase que lo cuenta. "
        "Aqui se recuenta sobre `main.rs` en vez de creerselo.")
print()
main = (RAIZ / "crates/ore-cli/src/main.rs").read_text(encoding="utf-8", errors="replace")
i0 = main.find("enum Command")
cuerpo = main[i0:main.find("\n}", i0)]
verbos = set()
for m in re.finditer(r'^\s*(?:#\[command\(name = "([a-z-]+)"[^\]]*\)\]\s*)?([A-Z][A-Za-z]*)\s*(\{|\(|,)',
                     cuerpo, re.M):
    verbos.add(m.group(1) or re.sub(r"(?<!^)(?=[A-Z])", "-", m.group(2)).lower())
sin = re.search(r'SIN_IMPLEMENTAR: \[&str; (\d+)\]', main)
n_sin = int(sin.group(1)) if sin else -1

doc = texto_doc = (DOCS / "ontologia-como-repositorio.md").read_text(encoding="utf-8", errors="replace")
m_doc = re.search(r"de los (\d+) verbos de la CLI", doc)
m_sin = re.search(r"(\w+) de los veinti\w+ est[aá]n declarados y no hacen nada", doc)
PALABRAS = {"cinco": 5, "seis": 6, "siete": 7, "cuatro": 4}

print("   %-34s %-8s %s" % ("", "arbol", "documentacion"))
print("   " + "-" * 62)
d_verbos = int(m_doc.group(1)) if m_doc else -1
d_sin = PALABRAS.get(m_sin.group(1), -1) if m_sin else -1
print("   %-34s %-8d %d %s" % ("verbos de la CLI", len(verbos), d_verbos,
                               "" if len(verbos) == d_verbos else "  <-- NO CUADRA"))
print("   %-34s %-8d %d %s" % ("declarados y sin implementar", n_sin, d_sin,
                               "" if n_sin == d_sin else "  <-- NO CUADRA"))
print()
print("   verbos: %s" % " · ".join(sorted(verbos)))
print()
parrafo("Y la pregunta que decide el alcance de todo lo anterior: la "
        "documentacion de este arbol NO es un manual — es el sitio donde vive "
        "el razonamiento que el codigo no puede llevar. Lo que hay que "
        "escribir no es «como se usa `ore package split`», que lo dice "
        "`--help`, sino POR QUE un corte cuesta un cruce y por que el mando "
        "dice el precio en vez de buscarlo.")
