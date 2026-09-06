# -*- coding: utf-8 -*-
"""El protocolo para anadir una fuente, y hasta donde virtualiza la tabla.

Dos preguntas y una sola respuesta util: que hay que escribir para que un
origen entre, y que clases de origen caben de verdad en `kind: Table` sin que
haya que mentir en algun campo.

  A. EL PROTOCOLO   los tres verbos y el enchufe, derivados del codigo
  B. EL VOCABULARIO lo que la tabla sabe decir de un objeto
  C. HASTA DONDE    clase de origen por clase de origen: que entra entero, que
                    entra a medias y que no entra
  D. LOS TOPES      los limites duros, con donde esta escrito cada uno
  E. LO QUE FALTA   por orden de lo que desbloquea
"""
import json
import pathlib
import re
import textwrap

RAIZ = pathlib.Path(r"C:\ORE")
CRATES = RAIZ / "crates"
ESQUEMA = RAIZ / "vendor/oos/schemas/v1alpha8/table.schema.json"


def verbos_de(crate):
    f = CRATES / crate / "src/main.rs"
    if not f.is_file():
        return {}
    t = f.read_text(encoding="utf-8", errors="replace")
    out = {}
    for v in re.findall(r'(?:Some\()?"(\w+)"\)? =>', t):
        brazo = re.search(r'"%s" =>\s*(.{0,120})' % v, t, re.S)
        out[v] = not (brazo and "no sabe" in brazo.group(1))
    return out


print("== el protocolo de fuentes, y su alcance ==")

# -- A - EL PROTOCOLO --------------------------------------------------------
print()
print("A - EL PROTOCOLO: que hay que escribir para que un origen entre")
lector = (CRATES / "ore-cli/src/lector.rs").read_text(encoding="utf-8")
patron = re.search(r'let programa = format!\("([^"]+)"\)', lector)
print()
print("   1 · SE DECLARA LA FUENTE.  `ore source add --name <n> <url>`")
print("       El `type` sale del ESQUEMA DE LA URL —`postgres://` da")
print("       `postgres`— y el secreto se separa solo: el manifiesto guarda")
print("       `connectionEnv`, que dice DONDE buscarlo y no que es.")
print()
print("   2 · SE PONE UN BINARIO EN EL PATH.  `%s`" % (patron.group(1) if patron else "?"))
print("       Y ya esta: `ore` no cambia. No hay registro de tipos, no hay una")
print("       lista que actualizar, no hay una `match` que crezca.")
print()
print("   3 · ESE BINARIO CONTESTA TRES VERBOS, todos por stdin/stdout:")
print()
print("       %-10s %-46s %s" % ("verbo", "que recibe", "que devuelve"))
print("       " + "-" * 84)
print("       %-10s %-46s %s" % ("catalogo", "la URL", "el catalogo JSON del esquema"))
print("       %-10s %-46s %s" % ("testigo", "{objeto, url, cursor?}", "{modo, valor?} — un ORDINAL"))
print("       %-10s %-46s %s" % ("leer", "un FRAGMENTO DEL PLAN", "una fila JSON por linea"))
print()
print("   Y la pieza que lo hace barato es la tercera: la peticion NO ES SQL.")
print("   Es proyeccion, recorte por clave, filtros y rango. La misma sirve a")
print("   PostgreSQL y a un directorio de ficheros, y por eso `ore-read-jsonl`")
print("   existe — para demostrar que el corte estaba en el sitio correcto.")
print()
matriz = {c: verbos_de(c) for c in
          ("ore-read-postgres", "ore-read-bigquery", "ore-read-jsonl")}
print("   Lo que cuesta cada verbo, medido en lo que hay:")
for c, v in matriz.items():
    tiene = [k for k in ("catalogo", "leer", "testigo") if v.get(k)]
    print("     %-20s %s" % (c.replace("ore-read-", ""), ", ".join(tiene) or "-"))
print()
print("   Y si el origen habla SQL, la traduccion YA ESTA ESCRITA: `ore-sql`")
print("   pone la forma y el dialecto son tres ejes. Lo unico propio es el")
print("   TRANSPORTE.")

# -- B - EL VOCABULARIO ------------------------------------------------------
print()
print("B - EL VOCABULARIO: lo que la tabla sabe decir de un objeto")
d = json.loads(ESQUEMA.read_text(encoding="utf-8"))
spec = d["properties"]["spec"]["properties"]
reads = spec["reads"]["oneOf"][1]["properties"]
cambios = spec["changes"]["properties"]
print()
print("   spec        : %s" % ", ".join(spec))
print("   columns.<c> : %s"
      % ", ".join(spec["columns"]["additionalProperties"]["properties"]))
print()
print("   reads —la cara I, que se le puede PEDIR—:")
for k, v in reads.items():
    val = v.get("enum") or v.get("items", {}).get("enum") or v.get("type")
    print("     %-20s %s" % (k, val))
print("     `reads: none` es legal y dice NO SE LE PUEDE PEDIR NADA.")
print()
print("   changes —la cara D, que cambios EMITE—:")
for k in ("mode", "witness"):
    print("     %-20s %s" % (k, cambios[k].get("enum")))
print("     %-20s %s" % ("key, field, retention", "como se identifica y cuanto dura"))

# -- C - HASTA DONDE ---------------------------------------------------------
print()
print("C - HASTA DONDE VIRTUALIZA, clase por clase")
print()
CLASES = [
    ("base de datos OLTP", "ENTERA",
     "reads con empuje, changes por decodificacion logica, testigo por log"),
    ("almacen cloud (BigQuery…)", "ENTERA",
     "el sondeo sale de la metadata del objeto; el testigo, del historial"),
    ("stream (Kafka, tema)", "ENTERA, y sin `kind` nuevo",
     "es una tabla con `reads: none` y `changes: {append|retract}`. Y `OOS2020` "
     "obliga: lo que no se deja leer se DEBE materializar"),
    ("lakehouse (Iceberg, Delta)", "casi",
     "`witness: snapshot` existe y el rango por posicion NO lo sirve ningun "
     "driver: `rango_servible` se niega. Falta el transporte, no el vocabulario"),
    ("lago de ficheros (parquet, ndjson)", "a medias",
     "`leer` esta —`ore-read-jsonl`— y `catalogo` no: inferir columnas y tipos "
     "de los datos es otra decision. Y el objeto es una RUTA, no un "
     "identificador: ahi `ore-sql` no alcanza"),
    ("catalogo (Unity, Glue, Polaris)", "NO",
     "un catalogo no sirve filas: sirve OTROS catalogos. Una fuente apunta a UN "
     "esquema, asi que N esquemas son N fuentes declaradas a mano"),
    ("API SaaS (Salesforce, Stripe)", "a medias",
     "cabe como `reads` sin empuje, y entonces la proyeccion se respeta DESPUES "
     "de leer — que el vocabulario no sabe decir. Medido en "
     "`medida-el-dialecto-declarado` §E"),
]
print("   %-34s %-26s" % ("clase de origen", "¿entra?"))
print("   " + "-" * 64)
for nombre, veredicto, _ in CLASES:
    print("   %-34s %-26s" % (nombre, veredicto))
print()
for nombre, veredicto, por_que in CLASES:
    print("   %s — %s" % (nombre, veredicto))
    for linea in textwrap.wrap(por_que, 66):
        print("     %s" % linea)
    print()

# -- D - LOS TOPES -----------------------------------------------------------
print()
print("D - LOS TOPES DUROS, y donde esta escrito cada uno")
print()
TOPES = [
    ("una fuente = UN esquema",
     "`INFORMATION_SCHEMA` es por dataset y la URL lleva uno. Un proyecto con "
     "N datasets son N fuentes.", "lector.rs"),
    ("una entidad sale de UNA vista",
     "la federacion se retiro: «dos sistemas son dos entidades con una "
     "relacion, o una copia declarada».", "v1alpha8/00-scope §6"),
    ("la vista es pi y sigma",
     "sin junta, agregado, distinct ni limite. Es el fragmento INVERTIBLE, y "
     "ensancharlo deja el sustrato de solo lectura para siempre.",
     "v1alpha8/00-scope §6.1"),
    ("las columnas son planas",
     "un `STRUCT` anidado no se modela: se cita su `sourceType`. Traducirlo a "
     "`Opaque` tiraria un hecho; a entidades anidadas inventaria un modelo.",
     "lector.rs"),
    ("dos empujes declarados y sin lector",
     "`joinPushdown` y `aggregatePushdown` estan en el esquema y el "
     "planificador NO los lee: «leerlos sin usarlos daria un campo que promete "
     "algo».", "ore-view/capabilities.rs"),
    ("no hay verbo `check`",
     "Airbyte tiene `spec` y `check`; aqui no. Una fuente mal configurada se "
     "descubre al primer `catalogo`, no antes.", "ore-driver"),
]
for que, por_que, donde in TOPES:
    print("   · %-34s [%s]" % (que, donde))
    for linea in textwrap.wrap(por_que, 64):
        print("       %s" % linea)
print()
print("   Y uno que no es de diseno sino un descuido, medido ayer: el sondeo de")
print("   BigQuery pide `--max_rows=100000` y NO COMPRUEBA si toco el tope. Un")
print("   dataset ancho sale con tablas de menos y la ultima a medias, sin que")
print("   nada lo diga.")

# -- E - LO QUE FALTA --------------------------------------------------------
print()
print("E - LO QUE FALTA, por lo que desbloquea")
print()
print("   1 · UN CATALOGO PARA FICHEROS. Cierra el lago, y obliga a decidir si")
print("       se infieren columnas de los datos o se pide un esquema al lado.")
print("       Es la decision, no el codigo.")
print()
print("   2 · EL RANGO POR POSICION. `witness: snapshot` y `log` estan en el")
print("       vocabulario y ningun driver los sirve: `rango_servible` se niega")
print("       con los dos. Es lo que separa un lakehouse de «casi».")
print()
print("   3 · LA PALABRA QUE FALTA EN `reads`. Hoy sabe decir que un origen no")
print("       empuja nada; NO sabe decir que la proyeccion se respeta DESPUES")
print("       de leer. Sin ella, una fuente estilo Airbyte entra")
print("       indistinguible de una que si empuja — y la mascara deja de ser")
print("       estructural sin que el sello cambie.")
print()
print("   4 · UN VERBO `check`. Barato, y convierte «no se pudo leer el")
print("       catalogo» en «esta fuente no responde», que son dos tardes")
print("       distintas.")
