# -*- coding: utf-8 -*-
"""La forma del catalogo: la acepta un mando, no la emite ninguno, no la declara nadie.

`ore discover --from` acepta «un catalogo ya leido, VENGA DE DONDE VENGA», y
ningun mando de `ore` produce uno. La pregunta de esta medida no es si falta el
mando —eso ya se sabe— sino QUE FORMA tendria que emitir, y de donde sale esa
forma hoy.

  A. LA FORMA QUE SE ACEPTA    derivada del consumidor, no de memoria
  B. DONDE ESTA DECLARADA      esquema, tipo, prosa: se busca en los tres
  C. QUIEN LA PRODUCE          cuatro productores, y que emite cada uno
  D. LO QUE DIVERGE            y si divergir hoy cuesta algo
  E. EL HUECO, EXACTO          quien acepta, quien emite, quien se niega
  F. LA FORMA DEL MANDO        derivada de sus vecinos, no inventada
"""
import pathlib
import re
import subprocess
import textwrap

RAIZ = pathlib.Path(r"C:\ORE")
CONSUMIDOR = RAIZ / "crates/ore-cli/src/inductor.rs"
PRODUCTORES = [
    ("bigquery  (receta dentro de `ore`)", "crates/ore-cli/src/lector.rs"),
    ("postgres  (verbo `catalogo`)", "crates/ore-read-postgres/src/main.rs"),
    ("jsonl     (verbo `catalogo`)", "crates/ore-read-jsonl/src/catalogo.rs"),
    ("a mano    (`descubrimiento.rs`)", "crates/ore-cli/tests/descubrimiento.rs"),
]


def parrafo(t, sangria="     ", ancho=68):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def cuerpo(texto, firma):
    """El cuerpo de una funcion, contando llaves desde su firma."""
    i = texto.index(firma)
    j = texto.index("{", i)
    prof, p = 0, j
    while p < len(texto):
        if texto[p] == "{":
            prof += 1
        elif texto[p] == "}":
            prof -= 1
            if prof == 0:
                return texto[j:p]
        p += 1
    return texto[j:]


print("== la forma del catalogo, medida ==")

# -- A -----------------------------------------------------------------------
print()
print("A - LA FORMA QUE `--from` ACEPTA, derivada del consumidor")
print()
t = CONSUMIDOR.read_text(encoding="utf-8")
leer = cuerpo(t, "pub fn leer(texto: &str)")
# Dos formas de preguntar por una clave, y la segunda se me escapo la primera
# vez: `lista(t, "primaryKey")` no es `get("...")`, y contar solo la primera
# habria dicho que `primaryKey` no lo lee nadie. Lo leen tres productores.
claves = set(re.findall(r'get\("([A-Za-z]+)"\)', leer))
claves |= set(re.findall(r'lista(?:_de)?\([^,)]+,\s*"([A-Za-z]+)"\)', leer))
claves |= set(re.findall(r'cadena\("([A-Za-z]+)"\)', leer))

NIVEL = {
    "source": "raiz", "tables": "raiz",
    "name": "tabla/columna", "columns": "tabla", "kind": "tabla",
    "primaryKey": "tabla", "uniqueKeys": "tabla", "foreignKeys": "tabla",
    "rows": "tabla", "reads": "tabla", "changes": "tabla",
    "type": "columna", "sourceType": "columna", "required": "columna",
    "description": "columna",
    "references": "foranea", "toColumns": "foranea",
}
print("   %-14s %-14s" % ("clave", "nivel"))
print("   " + "-" * 40)
for k in sorted(claves):
    print("   %-14s %-14s" % (k, NIVEL.get(k, "?")))
print()
print("   %d claves, en %d niveles." % (len(claves), len(set(NIVEL.get(k, "?") for k in claves))))

# -- B -----------------------------------------------------------------------
print()
print("B - DONDE ESTA DECLARADA ESA FORMA")
print()
esquemas = list((RAIZ / "vendor/oos/schemas").rglob("*.json"))
con_catalogo = [f.name for f in esquemas
                if "catalog" in f.read_text(encoding="utf-8", errors="replace").lower()
                and "catalog" in f.name.lower()]
protocolo = (RAIZ / "crates/ore-driver/src/lib.rs").read_text(encoding="utf-8")
print("   esquema JSON en `vendor/oos/schemas`     %s" % (con_catalogo or "NINGUNO"))
print("   tipo en `ore-driver` (el protocolo)      %s"
      % ("si" if re.search(r"struct\s+Catalogo|fn\s+catalogo\(", protocolo) else "NINGUNO"))
print("   struct en el consumidor                  %s"
      % ("si" if "pub struct Catalogo" in t else "no"))
print()
parrafo("Asi que la forma existe en UN SITIO: el `struct Catalogo` del "
        "consumidor y su funcion `leer`. Los productores no la importan —son "
        "programas aparte, escriben JSON a pelo— asi que lo que los cuatro "
        "comparten no es un tipo: es haber leido el mismo fichero.")
print()
parrafo("Y esa es la asimetria de verdad, la que hay debajo de que falte el "
        "mando: `ore-driver` existe justamente para que el PROTOCOLO no viva "
        "dentro de un driver —`OPERADORES`, `leer_peticion`, `testigo`, "
        "`comprobacion` estan ahi—, y la salida de `catalogo`, que es la mayor "
        "de las cuatro, se quedo fuera.")

# -- C -----------------------------------------------------------------------
print()
print("C - QUIEN LA PRODUCE, clave a clave")
print()
# La primera version buscaba `insert("clave"` y `("clave", Json::`, y dijo que
# `foreignKeys` NO LO EMITE NADIE —cuando `references`, que vive dentro, si—.
# El motivo: los tres productores parten la llamada en varias lineas, y la clave
# queda sola: `insert(\n  "foreignKeys".to_string(),`. El arnes midiendo el
# ESTILO DE FORMATO y contandolo como ausencia, que es el fallo que estas
# medidas persiguen. Se busca la literal, que en estos ficheros no significa
# ninguna otra cosa.
#
# Y una segunda: buscando la literal en el fichero entero, `jsonl` salia
# emitiendo `primaryKey`. Su unica aparicion es una PRUEBA que afirma que no lo
# emite —`assert!(!c.contains("primaryKey"))`—. Contar una prueba que niega algo
# como si fuera el algo: se recortan los modulos de prueba.
def region(ruta, txt):
    if ruta.endswith("descubrimiento.rs"):
        # Aqui el productor ES un literal dentro de una prueba: el catalogo a
        # mano. Se acota a el y no al fichero.
        i = txt.index("const CATALOGO")
        return txt[i:txt.index('"#;', i)]
    return txt.split("#[cfg(test)]")[0]


emite = {}
for nombre, ruta in PRODUCTORES:
    txt = (RAIZ / ruta).read_text(encoding="utf-8", errors="replace")
    emite[nombre] = {k for k in claves if '"%s"' % k in region(ruta, txt)}

orden = sorted(claves, key=lambda k: (NIVEL.get(k, "z"), k))
print("   %-14s %s" % ("clave", "  ".join("%-9s" % n.split()[0] for n, _ in PRODUCTORES)))
print("   " + "-" * 62)
for k in orden:
    marcas = "  ".join("%-9s" % ("si" if k in emite[n] else "·") for n, _ in PRODUCTORES)
    print("   %-14s %s" % (k, marcas))

# -- D -----------------------------------------------------------------------
print()
print("D - LO QUE DIVERGE, Y SI CUESTA ALGO")
print()
for k in orden:
    quien = [n.split()[0] for n, _ in PRODUCTORES if k in emite[n]]
    if 0 < len(quien) < len(PRODUCTORES):
        print("   %-14s solo %s" % (k, ", ".join(quien)))
print()
parrafo("Divergir aqui NO es un fallo por si mismo: un fichero `.ndjson` no "
        "tiene claves foraneas y BigQuery no publica `uniqueKeys`. La ausencia "
        "es una respuesta, y el consumidor la trata como tal —P4—.")
print()
parrafo("Lo que cuesta es lo otro: que NO HAY NADA que diga cual es la lista, "
        "asi que un productor nuevo no puede saber que se esta dejando. Se "
        "entera el dia que una induccion salga mas pobre y nadie sepa por que, "
        "porque una tabla sin `primaryKey` y una tabla cuyo driver se olvido de "
        "emitirlo SE VEN EXACTAMENTE IGUAL.")

# -- E -----------------------------------------------------------------------
print()
print("E - EL HUECO, EXACTO")
print()
ayuda = subprocess.run(["cargo", "run", "-q", "-p", "ore-cli", "--", "source", "--help"],
                       cwd=RAIZ, capture_output=True, text=True).stdout
verbos = re.findall(r"^\s{2}(\w+)\s{2,}", ayuda, re.M)
print("   `ore source` tiene:      %s" % ", ".join(v for v in verbos if v != "help"))
print("   `ore discover --from`    ACEPTA un catalogo «venga de donde venga»")
print("   quien lo emite           NADIE")
print()
parrafo("Y por abajo tampoco hay salida para la familia que mas importa: "
        "`ore-read-bigquery catalogo` SE NIEGA a proposito —esa receta vive "
        "dentro de `ore` y es la que corre—, asi que el catalogo de BigQuery no "
        "se puede obtener con ninguna orden, ni de `ore` ni de su driver.")

# -- F -----------------------------------------------------------------------
print()
print("F - LA FORMA DEL MANDO, derivada de sus vecinos")
print()
print("   Los tres vecinos ya reparten el trabajo asi:")
print()
print("   %-20s %s" % ("`source check`", "¿responde? · no lee catalogo, no toca fichero"))
print("   %-20s %s" % ("`source explore`", "¿que contiene? · antes de declarar nada"))
print("   %-20s %s" % ("`discover --source`", "lee el catalogo Y induce, en un solo acto"))
print("   %-20s %s" % ("(el hueco)", "lee el catalogo Y PARA"))
print()
parrafo("El mando que falta es el primer acto de `discover` sin el segundo. Y "
        "hay una prueba de que la separacion es la correcta y no un capricho: "
        "`discover` YA la tiene por dentro —`--source` y `--from` existen "
        "porque «son dos actos, y se piden por separado porque fallan por "
        "separado»—. Lo unico que falta es poder QUEDARSE con lo de en medio.")
print()
parrafo("Y hay una segunda cosa que cae sola en cuanto exista: hoy «los tres "
        "drivers emiten la misma forma» esta escrito en tres cabeceras y no lo "
        "comprueba nadie. Con la forma en `ore-driver` y un catalogo capturado "
        "por familia, eso pasa de intencion a aserto.")
