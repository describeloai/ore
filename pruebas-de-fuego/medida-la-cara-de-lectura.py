# -*- coding: utf-8 -*-
"""La cara `reads`: quien lee cada clave, y que significa que una no la lea nadie.

Sale de una correccion. Dije que `joinPushdown` y `aggregatePushdown` estaban
declarados «y el planificador no los lee», enunciado como DEUDA. No lo es: un
planificador que leyera una capacidad que no sabe ejercer estaria prometiendo
algo. La pregunta correcta no es por que no los lee, sino si el reparto entre
«lo que se traduce» y «lo que no» esta ESCRITO en algun sitio o solo en prosa.

  A. EL VOCABULARIO PUBLICADO   las claves de `reads`, del esquema
  B. QUIEN LEE CADA UNA         medido sobre el arbol, no de memoria
  C. LOS OPERADORES             el enum de `predicatePushdown`, igual
  D. QUE SUJETA EL REPARTO      hay censo, o es una frase en un comentario
"""
import pathlib
import re
import textwrap

RAIZ = pathlib.Path(r"C:\ORE")
ESQUEMA = RAIZ / "vendor/oos/schemas/v1alpha8/table.schema.json"


def parrafo(t, sangria="     ", ancho=68):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def claves_de(texto, desde):
    """Las claves de primer nivel del objeto `properties` que sigue a `desde`."""
    # Anclar en `"reads": {` y no en `"reads"`: la primera version anclo en la
    # palabra, la encontro dentro de una DESCRIPCION varios kilobytes antes, y
    # midio las claves de `spec`. El arnes leyendo otra cosa y contandolo con
    # aplomo, que es lo que estas medidas persiguen.
    i = texto.index(desde)
    j = texto.index('"properties"', i)
    k = texto.index("{", j)
    prof, out, pos = 0, [], k
    while pos < len(texto):
        c = texto[pos]
        if c == "{":
            prof += 1
        elif c == "}":
            prof -= 1
            if prof == 0:
                break
        elif c == '"' and prof == 1:
            m = re.match(r'"([A-Za-z]+)"\s*:', texto[pos:])
            if m:
                out.append(m.group(1))
        pos += 1
    return out


texto = ESQUEMA.read_text(encoding="utf-8")

print("== la cara `reads`, medida ==")

# -- A -----------------------------------------------------------------------
print()
print("A - EL VOCABULARIO PUBLICADO")
print()
claves = claves_de(texto, '"reads": {')
sin_descripcion = []
for c in claves:
    m = re.search(r'"%s"\s*:\s*\{\s*"description"' % c, texto)
    if not m:
        sin_descripcion.append(c)
print("   %d claves: %s" % (len(claves), ", ".join(claves)))
print()
if sin_descripcion:
    print("   Y las que el esquema declara SIN UNA SOLA LINEA de descripcion:")
    print("     %s" % ", ".join(sin_descripcion))
    parrafo("No es cosmetico. Toda clave de esta cara lleva escrito que decide "
            "y que codigo dispara; estas dos entraron sin decir ninguna de las "
            "dos cosas, que es exactamente el sintoma de una clave que nadie "
            "ha tenido que justificar.")

# -- B -----------------------------------------------------------------------
print()
print("B - QUIEN LEE CADA UNA, medido sobre el arbol")
print()
fuentes = [p for p in (RAIZ / "crates").rglob("*.rs")]
print("   %-22s %s" % ("clave", "quien la lee"))
print("   " + "-" * 68)
lectores = {}
for c in claves:
    quien = []
    for f in fuentes:
        t = f.read_text(encoding="utf-8", errors="replace")
        # Un uso, no una mencion: la clave entre comillas, que es como se
        # pregunta por ella —`n.get("fullScan")`— y no como se comenta.
        if re.search(r'"%s"' % c, t):
            rel = str(f.relative_to(RAIZ / "crates")).replace("\\", "/")
            if "/tests/" not in rel:
                quien.append(rel)
    lectores[c] = quien
    print("   %-22s %s" % (c, ", ".join(quien) if quien else "NADIE"))

print()
print("   Y el reparto que sale de ahi, que son TRES clases y no dos:")
print()
CLASES = [
    ("se traduce", "el planificador la convierte en un campo de `Capacidades`",
     [c for c in claves if any("capabilities.rs" in q for q in lectores[c])]),
    ("la lee otro", "no es del planificador, y por eso no esta ahi",
     [c for c in claves
      if lectores[c] and not any("capabilities.rs" in q for q in lectores[c])]),
    ("sin lector", "un hecho del origen que hoy no consume nadie",
     [c for c in claves if not lectores[c]]),
]
for nombre, que, cuales in CLASES:
    print("   %-14s %s" % (nombre, ", ".join(cuales) if cuales else "—"))
    parrafo(que, "                  ")

# -- C -----------------------------------------------------------------------
print()
print("C - LOS OPERADORES de `predicatePushdown`")
print()
enum = re.search(r'"predicatePushdown".*?"enum"\s*:\s*\[(.*?)\]', texto, re.S)
ops = re.findall(r'"([a-zA-Z]+)"', enum.group(1)) if enum else []
cap = (RAIZ / "crates/ore-view/src/capabilities.rs").read_text(encoding="utf-8")
cuerpo = cap[cap.index("predicatePushdown"):cap.index("fullScan")]
print("   %-12s %s" % ("operador", "¿lo traduce `de_oos`?"))
print("   " + "-" * 68)
for o in ops:
    print("   %-12s %s" % (o, "si" if ('Some("%s")' % o) in cuerpo else "NO"))
print()
parrafo("Y este es el MISMO fallo que se arreglo esta semana en `ore-driver`: "
        "`leer_peticion` descartaba en silencio un operador que no conocia, "
        "mientras el comentario decia que se descartaba la peticion entera. "
        "Aqui la forma es identica —un `match` con `_ => {}`— y lo que la "
        "salva hoy es que el enum del esquema esta cerrado y no ha crecido.")

# -- D -----------------------------------------------------------------------
print()
print("D - QUE SUJETA EL REPARTO HOY")
print()
hay_censo = "CARA_DE_LECTURA" in cap
print("   censo de la cara `reads`      %s" % ("si" if hay_censo else "NO"))
vistas = (RAIZ / "crates/ore-core/src/vistas.rs").read_text(encoding="utf-8")
print("   censo del vocabulario de View %s" % ("si" if "mod censo" in vistas else "NO"))
print()
parrafo("El de `View` existe y tiene dientes: anadir una clave sin decir si se "
        "invierte NO COMPILA la suite. El de la cara `reads` no existe: lo que "
        "hay es un parrafo en el comentario de `de_oos` que enumera lo que no "
        "se traduce, y un parrafo no se rompe cuando el esquema crece.")
print()
parrafo("Y la asimetria que hay que dejar escrita, porque es la que hace que "
        "«sin lector» NO sea una deuda: una clave de `Table` registra un HECHO "
        "DEL ORIGEN —Workday no sabe juntar, y no sabra aunque nadie se lo "
        "pregunte— y por eso puede preceder a su lector. Un campo del "
        "planificador PROMETE UN COMPORTAMIENTO, y solo es cierto cuando "
        "alguien lo ejerce: ese no puede preceder a nadie.")
