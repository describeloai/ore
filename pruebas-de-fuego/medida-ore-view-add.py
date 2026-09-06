# -*- coding: utf-8 -*-
"""`ore view add`: que tendria que preguntar, que puede derivar y donde se niega.

Es el hueco que deja la interfaz de cliente sin poder cerrarse: `discover`
espeja, `review` decide, y **autorar una pregunta nueva sobre un hecho no lo
hace nadie**. Hoy se escribe el YAML a mano, y eso convierte a quien lo escriba
—una UI, un backend— en un SEGUNDO EMISOR.

  A. LO QUE UNA VISTA EXIGE   del esquema publicado, no de memoria
  B. DERIVAR O PREGUNTAR      cada campo, y de donde saldria
  C. FUERA DE pi-sigma        que pide un usuario que el vocabulario no admite,
                              y que tiene que contestar la interfaz
  D. LAS REGLAS QUE DISPARA   lo que una vista nueva puede romper el mismo dia
  E. EL SECTOR                Cognite y Foundry, y que de eso se traslada
"""
import json
import pathlib
import re
import textwrap

RAIZ = pathlib.Path(r"C:\ORE")
CRATES = RAIZ / "crates"
ESQUEMA = RAIZ / "vendor/oos/schemas/v1alpha8/view.schema.json"


def parrafo(texto, sangria="     ", ancho=68):
    for l in textwrap.wrap(texto, ancho):
        print("%s%s" % (sangria, l))


print("== `ore view add`, medido ==")

# -- A - LO QUE UNA VISTA EXIGE ----------------------------------------------
print()
print("A - LO QUE UNA VISTA EXIGE, del esquema publicado")
d = json.loads(ESQUEMA.read_text(encoding="utf-8"))
spec = d["properties"]["spec"]
obligatorios = spec.get("required", [])
print()
print("   %-16s %-12s %s" % ("campo", "¿obligatorio?", "qué es"))
print("   " + "-" * 72)
QUE_ES = {
    "owner": "quién responde. Sin dueño no hay política que herede",
    "from": "de dónde sale: una tabla o **otra vista**",
    "fields": "el renombre. propiedad -> columna",
    "freshness": "cuánto puede tardar en reflejar el origen",
    "where": "el recorte de filas. Una igualdad o una lista",
    "materialized": "dónde vive la copia, si la hay",
    "moved": "de qué nombre viene un campo, para no romper mudo",
    "reserved": "nombres que no se pueden reutilizar",
}
for k in spec["properties"]:
    print("   %-16s %-12s %s" % (k, "SÍ" if k in obligatorios else "no", QUE_ES.get(k, "")))

# -- B - DERIVAR O PREGUNTAR -------------------------------------------------
print()
print("B - DERIVAR O PREGUNTAR: de dónde sale cada uno")
print()
print("   Y la regla de la casa lo decide sola —P2, lo derivable no se declara,")
print("   y su corolario en `source add`: «un campo que se puede computar y aun")
print("   así se pide es una oportunidad de escribirlo mal».")
print()
FILAS = [
    ("from", "SE PASA", "es el argumento del mando: `ore view add --from <tabla>`"),
    ("fields", "SE DERIVA, y se resta",
     "por defecto TODAS las columnas de la tabla, renombradas al identificador "
     "de OOS. Es lo que hace Foundry al elegir un datasource —«mapea cada "
     "columna a una propiedad, y puedes descartar las que no quieras»— y es el "
     "defecto correcto: quitar es una decisión visible y añadir una columna "
     "olvidada no lo es"),
    ("name", "SE DERIVA", "del objeto, con la misma función que ya usa el inductor"),
    ("namespace", "SE DERIVA", "del paquete donde se escribe"),
    ("owner", "SE PREGUNTA",
     "no se puede derivar de nada: es quién responde. El inductor escribe "
     "`cambiame`, que NO valida, en vez de inventar un handle"),
    ("oos.maturity", "SE FIJA", "`DRAFT`. Nada recién escrito es verdad todavía"),
    ("where", "opcional, se pasa", "y solo admite igualdad o lista — ver (C)"),
    ("freshness", "NO se propone",
     "es una decisión de operación con coste. El inductor tampoco la propone, "
     "y por lo mismo"),
    ("materialized", "NO se propone",
     "ídem, y además dispara la regla de flujo: una copia se sella"),
]
for campo, como, por_que in FILAS:
    print("   %-14s %s" % (campo, como))
    parrafo(por_que)
    print()

# -- C - FUERA DE PI-SIGMA ---------------------------------------------------
print()
print("C - FUERA DE pi-sigma: lo que un usuario va a pedir y no cabe")
print()
print("   Una `View` es `from` + `fields` + `where`. **No es SQL libre**, y ese")
print("   es el punto donde una interfaz promete lo que el modelo rechaza.")
print()
FUERA = [
    ("una junta", "no es invertible: no se sabe a cuál de las bases escribir"),
    ("un agregado", "una fila del resultado no corresponde a una fila de la base"),
    ("deduplicar", "la inversa no es una función"),
    ("limitar", "qué filas están es un hecho del orden, no del dato"),
    ("una entidad de N objetos", "la federación se retiró: son dos entidades con una relación"),
]
print("   %-26s %s" % ("lo que se pide", "por qué no"))
print("   " + "-" * 72)
for q, p in FUERA:
    print("   %-26s %s" % (q, p))
print()
print("   Y el motivo se dice UNA VEZ y vale para las cinco: admitir cualquiera")
print("   de ellas dejaría el sustrato de SOLO LECTURA PARA SIEMPRE, porque")
print("   escribir a través de una vista es `Q^-1`.")
print()
print("   -> lo que `view add` tiene que hacer con eso no es fallar con un error")
print("      de esquema: es CONTESTAR. Un `--join` no es una opción no")
print("      implementada, es una que tiene respuesta escrita, y darla es la")
print("      diferencia entre una herramienta y un formulario.")
print()
print("   Y las dos salidas que sí existen, que es lo que hay que ofrecer:")
print("     · dos entidades con una RELACIÓN, que es lo que el mundo tiene")
print("     · una COPIA declarada, si de verdad hay que juntar bytes")

# -- D - LAS REGLAS QUE DISPARA ----------------------------------------------
print()
print("D - LO QUE UNA VISTA NUEVA PUEDE ROMPER EL MISMO DÍA")
print()
codigos = {}
for f in (CRATES / "ore-core/src").rglob("*.rs"):
    t = f.read_text(encoding="utf-8", errors="replace")
    for m in re.finditer(r"Code::(Oos\d{4})", t):
        # El enum se llama `Oos2018` y la tabla de abajo `OOS2018`: la primera
        # version comparo los dos tal cual y dio NO a los seis codigos, que es
        # el arnes contando la MAYUSCULA en vez del codigo.
        codigos.setdefault(m.group(1).upper(), set()).add(f.stem)
REGLAS = [
    ("OOS2018", "la tabla no existe, o `fields` nombra una columna que no tiene"),
    ("OOS2020", "la raíz declara `reads: none` y la vista es virtual: no hay dónde preguntar"),
    ("OOS2029", "la raíz no empuja la proyección y la vista se materializa"),
    ("OOS2004", "el `datasource` de la copia no está declarado en el manifiesto"),
    ("OOS4011", "se materializa y `materialization.payload` no está autorizado"),
    ("OOS4002", "la copia lleva una etiqueta por encima de lo que el conducto admite"),
]
print("   %-10s %-8s %s" % ("código", "existe", "cuándo"))
print("   " + "-" * 72)
for c, cuando in REGLAS:
    lineas = textwrap.wrap(cuando, 52)
    print("   %-10s %-8s %s" % (c, "sí" if c in codigos else "NO", lineas[0]))
    for l in lineas[1:]:
        print("   %-10s %-8s %s" % ("", "", l))
print()
print("   -> ninguna de estas la puede evitar el mando: son del PAQUETE, no del")
print("      documento. Lo que sí puede es correr `validate` al terminar y")
print("      enseñarlas, en vez de dejar un fichero que rompe el árbol y callar.")

# -- E - EL SECTOR -----------------------------------------------------------
print()
print("E - EL SECTOR, y qué de eso se traslada")
print()
print("   COGNITE · contenedor y vista, que es el mismo corte que aquí:")
parrafo("un contenedor guarda el dato y define propiedades; una vista MAPEA "
        "propiedades de contenedor y las expone. Y tres cosas suyas valen la "
        "pena mirar: TODA vista lleva un `filter` —el recorte no es opcional, "
        "es parte de lo que una vista es—; `implements` deja que una vista "
        "herede de otra; y su versionado distingue el cambio que obliga a "
        "subir version del que no.")
print()
print("   FOUNDRY · el defecto que hay que copiar tal cual:")
parrafo("al elegir un datasource, «mapea CADA COLUMNA a una propiedad, y "
        "puedes descartar las que no quieras». Empezar con todo y restar, no "
        "empezar vacio y sumar. Quitar una columna es una decision visible; "
        "olvidarse de anadir una no lo es.")
print()
print("   Y una suya que aqui NO cabe, y conviene saber por que:")
parrafo("Foundry tiene «edit-only properties», propiedades que no mapean a "
        "ninguna columna del dataset. Aqui eso seria un campo sin columna, y "
        "`OOS2022` lo rechaza: una entidad sale de UNA vista, asi que una "
        "propiedad que la vista no da no tiene de donde salir. La unica "
        "excepcion es `derivedFrom`, que declara de que OTRAS sale — y eso no "
        "es una propiedad sin origen, es una con el origen dicho.")
print()
print("   LO QUE NINGUNO DE LOS DOS TIENE, y es lo que este mando anade:")
parrafo("el fragmento invertible. Cognite filtra y mapea; Foundry mapea "
        "columnas. Ninguno de los dos se plantea si la vista se puede correr "
        "AL REVES, porque ninguno de los dos escribe a traves de ella. Aqui "
        "esa pregunta decide el vocabulario entero, y por eso `view add` puede "
        "contestar «no» con un motivo y no con un «no soportado».")

# -- F - LA FORMA ------------------------------------------------------------
print()
print("F - LA FORMA QUE PROPONE ESTA MEDIDA")
print()
print("   ore view add <nombre> --from <tabla|vista> [--out <paquete>]")
print("                         [--field <prop>=<col>]...   restar o renombrar")
print("                         [--where <col>=<valor>]...  el recorte")
print("                         [--owner <handle>]")
print()
print("   · sin `--field`, TODAS las columnas. Con ellos, solo esos.")
print("   · el emisor es EL DEL INDUCTOR y no uno nuevo: una vista escrita a")
print("     mano y una inducida tienen que ser el mismo texto, o hay dos")
print("     emisores y divergen en el caso que ninguna prueba ejerce.")
print("   · al terminar, `validate` del paquete. Escribir un documento que")
print("     rompe el arbol y no decirlo es la mitad del trabajo.")
