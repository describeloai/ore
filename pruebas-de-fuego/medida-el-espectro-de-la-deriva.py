# -*- coding: utf-8 -*-
"""`drift-detect`: que promete, que hay ya, y las dos preguntas que no son la misma.

Se declara en `--help` desde hace tiempo y no existe. Lo que cambio esta semana
es que su mitad cara —conseguir el esquema fisico del origen— dejo de serlo:
`ore source catalog` lo emite y hay un catalogo capturado en el arbol.

  A. QUE PROMETE               la frase del `--help`, que son TRES verbos
  B. QUE HAY YA CONSTRUIDO     medido, no recordado
  C. LAS DOS PREGUNTAS         `diff` contesta una y la deriva es la otra
  D. EL ESPECTRO               clase a clase, y que codigo la cubre
  E. DONDE PARA LO PURO        el tramo sin red, y el precedente de la casa
  F. EL COSTE POR TRAMO
"""
import pathlib
import re
import textwrap

RAIZ = pathlib.Path(r"C:\ORE")


def parrafo(t, sangria="     ", ancho=68):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def hay(patron, ruta):
    try:
        return patron in (RAIZ / ruta).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return False


print("== el espectro de `drift-detect`, medido ==")

# -- A -----------------------------------------------------------------------
print()
print("A - QUE PROMETE, y son tres verbos en una frase")
print()
print("   «Compara la declaracion con el esquema fisico real y abre un pull request»")
print()
for n, q, donde in [
    ("1 · CONSEGUIR", "el esquema fisico real del origen", "hecho: `ore source catalog`"),
    ("2 · COMPARAR", "la declaracion contra el", "a medias: ver (C)"),
    ("3 · PROPONER", "y abrir un pull request", "nada, y ver (E)"),
]:
    print("   %-14s %-38s %s" % (n, q, donde))
print()
parrafo("Que sean tres es la primera decision: hoy es UN comando que no existe, "
        "y los tres fallan por separado —el primero por credencial, el segundo "
        "por criterio, el tercero por red—. Es la misma figura que `discover`, "
        "que se partio en `--source` y `--from` por esto mismo.")

# -- B -----------------------------------------------------------------------
print()
print("B - QUE HAY YA CONSTRUIDO")
print()
PIEZAS = [
    ("emitir el catalogo de una fuente",
     hay("pub fn emitir_catalogo", "crates/ore-cli/src/lector.rs"),
     "`ore source catalog` · esta semana"),
    ("un catalogo real en el arbol",
     (RAIZ / "crates/ore-cli/tests/catalogos/bigquery-rubix-demo-ventas.json").exists(),
     "12 objetos, 36 columnas, capturado de BigQuery"),
    ("la forma del catalogo, declarada",
     hay("pub const FORMA", "crates/ore-driver/src/catalogo.rs"),
     "18 claves con censo: lo que puede derivar esta enumerado"),
    ("inducir es una funcion PURA de (catalogo, decisiones)",
     hay("pub fn inducir_con", "crates/ore-cli/src/inductor.rs"),
     "«contestar dos veces lo mismo produce el mismo paquete byte a byte»"),
    ("comparar dos paquetes y clasificar por eje",
     hay("pub fn diff(antes: &Package", "crates/ore-core/src/diff.rs"),
     "`ore diff`, 4 ejes y 30 codigos OOS5xxx"),
    ("`diff` acepta un ARBOL, no solo un `.oob`",
     True,
     "«cada entrada es un arbol o un `.oob`, y se pueden mezclar»"),
]
for que, esta, nota in PIEZAS:
    print("   [%s] %s" % ("x" if esta else " ", que))
    parrafo(nota, "       ")
print()
parrafo("O sea que la tuberia entera SE PUEDE ESCRIBIR HOY EN LA SHELL: emite "
        "el catalogo, induce con las decisiones que ya estan contestadas, y "
        "compara contra el paquete que hay. Lo que falta no es maquinaria.")

# -- C -----------------------------------------------------------------------
print()
print("C - LAS DOS PREGUNTAS, QUE NO SON LA MISMA")
print()
print("   `diff`          ¿QUIEN SE ROMPE?   una relacion entre dos versiones")
print("   la deriva       ¿QUE SE MOVIO?     una relacion entre el mundo y lo dicho")
print()
parrafo("Y no es una distincion de matiz: `diff` no guarda las tablas. Su "
        "`Shape` guarda entidades, conductos, politicas y vistas, y el sustrato "
        "«por su EFECTO sobre cada vista» —la raiz resuelta, el recorte "
        "acumulado, de donde salen las filas—.")
print()
parrafo("La consecuencia, exacta: UNA COLUMNA NUEVA EN EL ORIGEN QUE NINGUNA "
        "VISTA PROYECTA ES INVISIBLE PARA `diff`. Y esta bien que lo sea —no "
        "rompe a nadie, que es su pregunta— pero es justo la mitad que la "
        "deriva existe para contar: el operador quiere saber que el origen "
        "gano una columna, entre otras cosas para decidir si exponerla.")
print()
parrafo("Asi que reutilizar `diff` tal cual daria un `drift-detect` que se "
        "calla la mitad aditiva. La deriva necesita comparar EL PLANO FISICO "
        "—catalogo contra `kind: Table`—, y `diff` sigue contestando lo suyo "
        "encima: que de lo que se movio rompe a alguien.")

# -- D -----------------------------------------------------------------------
print()
print("D - EL ESPECTRO, clase a clase")
print()
codigos = (RAIZ / "crates/ore-core/src/code.rs").read_text(encoding="utf-8", errors="replace")


def existe(c):
    return re.search(r"Oos%s\b" % c[3:], codigos) is not None


CLASES = [
    ("un objeto nuevo en el origen", "ADITIVA", None,
     "no rompe a nadie y nadie lo mira. Es la deriva mas comun y hoy no se ve"),
    ("un objeto que desaparece", "ROMPE", "OOS5007",
     "si una vista lo nombraba, el codigo existe; si no lo nombraba nadie, no"),
    ("una columna nueva", "ADITIVA", None,
     "invisible para `diff` por construccion — ver (C)"),
    ("una columna que desaparece", "ROMPE", "OOS5007",
     "solo si alguna vista la proyecta. Si no, se pierde en silencio"),
    ("una columna que cambia de tipo", "ROMPE", None,
     "el inductor traduce el tipo del origen al de OOS; que `Integer` pase a "
     "`String` no tiene codigo de `diff` porque `Shape` no guarda columnas"),
    ("`reads` admite menos", "ROMPE", "OOS5031",
     "el codigo YA EXISTE y se llama «la fuente de una vista admite menos»"),
    ("`changes` deja de sostener la copia", "ROMPE", "OOS5032",
     "idem, y es el que decide si una materializacion se puede seguir mant."),
    ("`primaryKey` cambia", "ROMPE", "OOS5019",
     "«binding fisico de una propiedad indexada cambiado»"),
    ("`rows` cambia", "NO ES DERIVA", None,
     "es el dato moviendose, no el esquema. Meterlo daria ruido en cada pase"),
]
print("   %-34s %-13s %s" % ("clase de deriva", "que es", "codigo"))
print("   " + "-" * 70)
for q, tipo, cod, _ in CLASES:
    marca = "—" if cod is None else ("%s %s" % (cod, "si" if existe(cod) else "NO EXISTE"))
    print("   %-34s %-13s %s" % (q, tipo, marca))
print()
for q, _, cod, por in CLASES:
    if cod is None:
        print("   · %s" % q)
        parrafo(por, "       ")
print()
parrafo("Cuatro de nueve tienen codigo y son las que ROMPEN A ALGUIEN. Las que "
        "no lo tienen son las aditivas y la de tipo — y las aditivas no "
        "necesitan un codigo de compatibilidad: necesitan SALIR EN UN INFORME. "
        "Confundir las dos cosas seria inventar codigos para avisos.")

# -- E -----------------------------------------------------------------------
print()
print("E - DONDE PARA LO PURO, Y EL PRECEDENTE DE LA CASA")
print()
# Se recorre en Python y no llamando a `grep`: la primera version lo hizo por
# subproceso y se colgo esperando a un `grep` que en esta maquina no termina.
# Una medida que no vuelve no es una medida.
git = [
    str(f.relative_to(RAIZ))
    for f in (RAIZ / "crates").rglob("*.rs")
    if 'Command::new("git")' in f.read_text(encoding="utf-8", errors="replace")
]
print("   invocaciones a `git` en el arbol   %s" % (", ".join(git) or "NINGUNA"))
print("   cliente HTTP en `ore-cli`          %s"
      % ("si" if hay("reqwest", "crates/ore-cli/Cargo.toml") else "NINGUNO"))
print()
parrafo("Y la casa ya tiene resuelto este problema exacto, dos veces: `bq` y "
        "`psql`. «La credencial nunca entra en el espacio de direcciones de "
        "ORE: este programa no abre un socket, no lee un fichero de servicio y "
        "no sabe que es un token. Ejecuta un programa que el usuario ya "
        "autentico y lee su stdout.»")
print()
parrafo("El analogo para el tercer verbo es `gh`. No es comodidad: un cliente "
        "de la API de GitHub dentro de `ore` seria un token de escritura en el "
        "repositorio metido en el mismo binario que lee los datos del cliente, "
        "y eso es una superficie que este arbol lleva evitando desde el ADR "
        "0006.")
print()
parrafo("Y hay una salida mas barata todavia, que ademas es la que hace que se "
        "pueda probar: el tercer verbo NO ABRE NADA. Escribe los documentos "
        "corregidos y para. Quien abre el PR es el CI que lo llama, que ya "
        "tiene el permiso y ya sabe hacerlo. Un mando que abre un PR no se "
        "puede ejercer en la suite; uno que escribe ficheros, si.")

# -- F -----------------------------------------------------------------------
print()
print("F - EL COSTE POR TRAMO")
print()
TRAMOS = [
    ("1", "`ore source catalog`", "HECHO", "esta semana"),
    ("2", "comparar catalogo contra las `kind: Table` del paquete",
     "el grueso", "mapear nombre de objeto a documento —lo hace el inductor— y "
     "recorrer las 18 claves de `FORMA`. Sin red: las dos entradas son ficheros"),
    ("3", "clasificar: aditiva, rompe, o incomparable",
     "barato", "los cuatro codigos que existen se piden a `diff`; las aditivas "
     "son un informe y no un codigo"),
    ("4", "escribir la correccion en los documentos",
     "medio", "es el emisor del inductor otra vez, sobre una tabla que ya "
     "existe. El riesgo no es tecnico: es decidir QUE se corrige solo y que se "
     "pregunta"),
    ("5", "el PR", "fuera", "lo abre quien llama. Ver (E)"),
]
print("   %-3s %-52s %s" % ("", "tramo", "coste"))
print("   " + "-" * 70)
for n, q, coste, _ in TRAMOS:
    print("   %-3s %-52s %s" % (n, q, coste))
print()
for n, q, _, por in TRAMOS:
    if n in ("2", "4"):
        print("   tramo %s · %s" % (n, q))
        parrafo(por, "       ")
        print()
parrafo("Y el tramo 2 tiene una prueba que ya esta escrita sin quererlo: el "
        "catalogo capturado y el paquete que sale de el. Comparar los dos tiene "
        "que dar CERO deriva, y ese es el aserto que hace que el resto "
        "signifique algo — un detector que encuentra deriva donde no la hay no "
        "se puede usar dos veces.")
