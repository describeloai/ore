# -*- coding: utf-8 -*-
"""Las superficies retiradas: ¿estan cerradas, o quedan trazas vivas?

Antes de construir drivers hay que saber que se cerro limpio. Y «cerrado» aqui
tiene una definicion que no es opinable:

    Una superficie esta CERRADA cuando no queda ni una traza VIVA. Una traza
    viva es codigo que se ejecuta, gramatica que se admite, o un caso de
    conformidad que la ejerce. Una mencion en prosa que dice «esto se retiro»
    NO es una traza viva: es el registro, y el registro se queda.

    El fallo tipico no es el codigo huerfano —ese se ve—: es un DOCUMENTO QUE
    SIGUE DICIENDO «normativo» sobre un sujeto que ya no existe.

Se miden las once superficies de la tabla de estado, mas una que no esta en
ella y sostiene el producto: el paquete como conjunto de vistas.

Y este arnes necesito TRES pasadas, todas por el mismo fallo: contar
apariciones sin decir cuantas deberia haber. Etiqueto «TRAZA VIVA» la
topologia y `Binding` —que son piezas vivas—, luego `Lectura` —que es el nodo
del plan y no el camino del ejecutor— y luego el sexto gemelo, porque
`Oos202[6-9]` casaba con `exports`. Por eso ahora cada fila DECLARA lo que
espera, y el veredicto se lee contra esa expectativa y no contra cero.
"""
import pathlib
import re
import subprocess

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
SPEC = OOS / "spec"
CONF = OOS / "conformance"


def rs_vivo(patron):
    """Apariciones en codigo Rust FUERA de comentarios y de pruebas.

    Un `//` delante lo convierte en registro, y el registro no es una traza.
    """
    n = 0
    for f in (RAIZ / "crates").rglob("*.rs"):
        if "tests" in f.parts:
            continue
        for linea in f.read_text(encoding="utf-8", errors="replace").split("\n"):
            s = linea.strip()
            if s.startswith("//"):
                continue
            n += len(re.findall(patron, linea))
    return n


def ficheros(patron, donde, ext="*.md"):
    cmd = ["grep", "-rl", patron, str(donde), "--include=" + ext]
    return len(subprocess.run(cmd, capture_output=True, text=True).stdout.split())


def casos(patron):
    """Casos de conformidad cuyo INPUT ejerce esto: es la traza mas viva."""
    out = subprocess.run(["grep", "-rl", patron, str(CONF), "--include=*.yaml"],
                         capture_output=True, text=True).stdout.split()
    return len({p for p in out if "input" in p or "before" in p or "after" in p})


def estado(rel):
    f = SPEC / rel
    if not f.exists():
        return "(no existe)"
    for l in f.read_text(encoding="utf-8", errors="replace").split("\n")[:8]:
        if "Estado:" in l:
            return re.sub(r"[*`\[\]]", "", l).replace("Estado:", "").strip()[:52]
    return "(sin cabecera de estado)"


print("== las superficies, y si queda algo vivo ==")
print()
print("   %-34s %-10s %-6s %-6s %s" % ("superficie", "que espera", "codigo", "casos", "veredicto"))
print("   " + "-" * 88)

# Cada fila declara QUE ESPERA, y sin eso la tabla miente. La primera version
# no lo hacia y etiqueto «TRAZA VIVA» cuatro filas: la topologia y `Binding`
# tienen codigo porque SON PIEZAS VIVAS, no residuos. Contar apariciones sin
# decir cuantas deberia haber es exactamente el arnes que confunde «igual» con
# «roto».
#
#   retirada   se suprimio: cero trazas vivas o hay residuo
#   soportada  sigue en v1alpha1 a proposito: las trazas son correctas
#   viva       es una pieza de hoy: la pregunta no es si queda, es si esta bien
#   nombre     se retiro, y solo sobrevive el NOMBRE para el diagnostico
#   descartada se midio y NO se escribio: comprobar que sigue sin existir
#   documento  no es codigo: lo que se mide es su CABECERA DE ESTADO
FILAS = [
    ("1 · la escritura · la cara W", "retirada", r"cara_w\b|\bF0a\b", None, "cara: W"),
    ("2 · el ejecutor · ore-exec", "retirada", r"ore_exec|ore-exec", None, None),
    ("3 · 05-ejecutor.md", "documento", None, r"05-ejecutor", None),
    ("4 · la topologia · el indice", "viva", r"aristas::|oretopo", None, None),
    # `Lectura` es el nodo de lectura del PLAN, y esta vivo. Lo que se retiro
    # era el camino del ejecutor, no la lectura: los dos caminos se separaron y
    # el viejo se fue con `ore-exec`.
    ("5 · la lectura desde la abstraccion", "viva", r"Lectura\b", None, None),
    ("6 · functions.md §7.1", "retirada", r"\bFuncion7\b", None, None),
    # Paso de «soportada» a «retirada» el dia que dejo de leerse. Lo que queda
    # NO es residuo: es el NOMBRE, que se conserva a proposito para que un
    # fichero viejo reciba la guia de migracion en vez de «kind desconocido».
    # Por eso se cuenta aparte y con su propio patron.
    ("7 · 03-binding · el kind", "nombre", r"Kind::Binding", None, "kind: Binding"),
    ("8 · L2 · nivel de conformidad", "documento", None, r"\bL2\b", None),
    ("9 · la regla del residuo", "retirada", r"residuo_regla", None, None),
    # Nunca se escribio, asi que lo que se comprueba es que siga sin existir.
    # `Oos202[6-9]` casaba con `Oos2027`/`Oos2028`, que son `exports`.
    ("10 · servir · el sexto gemelo", "descartada", r"Oos2026\b", None, None),
    ("11 · 02-entity", "documento", None, r"02-entity", None),
]
for nombre, espera, prs, pspec, pcaso in FILAS:
    c = rs_vivo(prs) if prs else None
    s = ficheros(pspec, SPEC) if pspec else None
    k = casos(pcaso) if pcaso else None
    hay = (c or 0) + (k or 0)
    if espera == "retirada":
        v = "cerrado" if hay == 0 else "RESIDUO — %d traza(s)" % hay
    elif espera == "soportada":
        v = "trazas correctas: v1alpha1 sigue vivo" if hay else "sin sujeto"
    elif espera == "nombre":
        # El enum, `as_str`, `hasta` y las dos ramas que un `match` exhaustivo
        # exige. Ni una mas: cualquier otra seria una rama que nunca corre.
        v = ("solo el nombre — %d sitios" % c) if c <= 6 else "RESIDUO: %d sitios" % c
    elif espera == "descartada":
        v = "nunca se escribio · `sustrato.md` §8.5" if hay == 0 else "SE ESCRIBIO"
    elif espera == "descartada":
        v = "nunca se escribio · `sustrato.md` §8.5" if hay == 0 else "SE ESCRIBIO"
    elif espera == "viva":
        v = "viva — la traza es la pieza" if hay else "SIN SUJETO"
    else:
        v = "-> ver cabecera, abajo"
    def n(x):
        return "-" if x is None else str(x)
    print("   %-34s %-10s %-6s %-6s %s"
          % (nombre, espera, n(c), n(k), v))



print()
print("   Y las cabeceras de estado de la spec, que es donde se esconde el fallo:")
for rel in ("v1alpha1/02-entity.md", "v1alpha1/03-binding.md", "v1alpha1/05-ejecutor.md",
            "v1alpha1/04-flow.md", "v1alpha8/01-table.md", "v1alpha8/02-view.md"):
    print("     %-26s %s" % (rel, estado(rel)))

# -- Lo que cada fila necesita, dicho una vez ---------------------------------
print()
print("LAS QUE NO ESTAN CERRADAS, y que le falta a cada una")
print()
ej = SPEC / "v1alpha1/05-ejecutor.md"
sec = len(re.findall(r"^## ", ej.read_text(encoding="utf-8", errors="replace"), re.M)) \
    if ej.exists() else 0
hist = len(re.findall(r"hist[oó]rico", ej.read_text(encoding="utf-8", errors="replace"), re.I)) \
    if ej.exists() else 0
print("   3 · `05-ejecutor.md`")
print("       secciones: %d · menciones a «historico»: %d" % (sec, hist))
print("       cabecera: %s" % estado("v1alpha1/05-ejecutor.md"))
print("       -> el sujeto se borro —4.529 lineas de `ore-exec`— y el documento")
print("          sigue diciendo NORMATIVO. Es la traza mas viva que queda, y no")
print("          es codigo: es una norma que nadie puede cumplir ni incumplir.")
print()
print("   8 · `L2`")
usos = subprocess.run(["grep", "-rn", r"\bL2\b", str(SPEC), "--include=*.md"],
                      capture_output=True, text=True).stdout.strip().split("\n")
print("       menciones en la spec: %d" % len([u for u in usos if u]))
for u in [u for u in usos if u][:3]:
    print("         %s" % u.split(":", 2)[-1].strip()[:76])
print("       -> definido sobre `Binding`, que es historico. Un nivel de")
print("          conformidad que nombra un `kind` retirado no se puede")
print("          reclamar: la implementacion no sabe contra que se mide.")
print()
print("   9 · la regla del residuo — sin medir, y sigue sin medir")
print("   11 · `02-entity` — ya NO esta sin medir: `docs/entidad.md` lo midio")
print("        entero. Lo que queda es reescribir su encuadre, no medirlo.")

# -- El pilar que no estaba en la tabla --------------------------------------
print()
print("EL PILAR QUE FALTA EN LA TABLA: el paquete como conjunto de vistas")
manifiestos = subprocess.run(["grep", "-rl", "kind: Package", str(OOS),
                              "--include=*.yaml"], capture_output=True,
                             text=True).stdout.split()
con_vista = 0
for m in manifiestos:
    d = pathlib.Path(m).parent
    if any(d.rglob("*.yaml")) and subprocess.run(
            ["grep", "-rl", "kind: View", str(d), "--include=*.yaml"],
            capture_output=True, text=True).stdout.strip():
        con_vista += 1
print("   manifiestos de paquete            : %3d" % len(manifiestos))
print("   ...con alguna vista dentro        : %3d" % con_vista)
print("   `exports` en la gramatica         : %s"
      % ("si" if rs_vivo(r'"exports"') else "no"))
print("   una regla que ate paquete y vistas: %s"
      % ("si" if rs_vivo(r"Oos2027|Oos2028") else "no"))
print()
print("   -> el paquete YA es un conjunto de vistas por CONTENCION —es un")
print("      directorio, no una seleccion— y ya dice que deja usar a otro")
print("      (`exports`, peldano 2). Lo que le falta no es gramatica: es que el")
print("      corpus lo ejerza. Con %d de %d manifiestos sin una sola vista, la"
      % (len(manifiestos) - con_vista, len(manifiestos)))
print("      frase describe una intencion mas que el arbol.")
