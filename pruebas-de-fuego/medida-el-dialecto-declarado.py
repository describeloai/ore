# -*- coding: utf-8 -*-
"""El dialecto declarado: cuanto de un driver SQL cabe en un fichero.

La pregunta es la de Airbyte aplicada a la figura de ORE. Su CDK de bajo codigo
dice por que vale la pena: **el contrato fuerte permite implementar una funcion
UNA VEZ —paginacion, incremental, resumable full refresh— en vez de repetirla
por conector**. Un fallo se arregla una vez.

Aqui hay dos traductores escritos por separado —Postgres y BigQuery— y esto
mide cuanto de ellos es LA MISMA FORMA y cuanto es dialecto. Si el dialecto son
unos pocos ejes, una familia SQL nueva deja de ser una crate de Rust y pasa a
ser un fichero.

  A. LA FORMA COMPARTIDA  el esqueleto que los dos construyen
  B. LOS EJES             lo que de verdad difiere, extraido de los dos
  C. LO QUE SE RESISTE    lo que no cabe en un manifiesto, y por que
  D. EL PRESUPUESTO       cuanto se movería y cuanto se queda
  E. AIRBYTE              hasta donde encaja, y donde rompe algo que aqui
                          no es una optimizacion sino la salvaguarda
"""
import pathlib
import re

RAIZ = pathlib.Path(r"C:\ORE")
PG = RAIZ / "crates/ore-read-postgres/src/sql.rs"
BQ = RAIZ / "crates/ore-read-bigquery/src/sql.rs"


def codigo(f):
    """El fichero sin pruebas, sin comentarios y sin lineas en blanco."""
    t = f.read_text(encoding="utf-8", errors="replace")
    t = t.split("#[cfg(test)]")[0]
    return [l for l in t.split("\n")
            if l.strip() and not l.strip().startswith(("//", "///", "//!"))]


def fn(f, nombre):
    t = f.read_text(encoding="utf-8", errors="replace").split("#[cfg(test)]")[0]
    m = re.search(r"^(?:pub )?fn %s\b.*?\{" % nombre, t, re.M | re.S)
    if not m:
        return ""
    i, prof = m.end(), 1
    while i < len(t) and prof:
        prof += (t[i] == "{") - (t[i] == "}")
        i += 1
    return t[m.start():i]


print("== el dialecto declarado ==")

# -- A - LA FORMA COMPARTIDA -------------------------------------------------
print()
print("A - LA FORMA COMPARTIDA: el esqueleto que los dos construyen")
print()
print("   SELECT <proyeccion> FROM <objeto>")
print("   WHERE  <recorte por clave>  AND  <filtros>  AND  <rango del cursor>")
print()
# Se comprueba que los dos lo construyen igual, en vez de afirmarlo.
HITOS = [
    # `format!(` y su cadena pueden ir en lineas distintas: BigQuery lo parte
    # porque la linea no cabia. Exigirlos pegados dio «no» a un fichero que lo
    # tiene, que es el arnes contando la ENVOLTURA en vez del hito.
    ("SELECT ... FROM", r'format!\(\s*"SELECT \{\} FROM \{\}"'),
    ("condiciones unidas por AND", r'condiciones\.join\(" AND "\)'),
    ("WHERE solo si hay condiciones", r'q\.push_str\(" WHERE "\)'),
    ("`gt` -> `>`, resto `=`", r'"gt" => ">"'),
    ("start exclusivo, end inclusivo", r'" > "|\{\} > \{\}|> \$\{\}|> \{\}'),
]
print("   %-34s %-10s %s" % ("hito de la forma", "postgres", "bigquery"))
print("   " + "-" * 60)
for nombre, patron in HITOS:
    hay = []
    for f in (PG, BQ):
        t = f.read_text(encoding="utf-8", errors="replace").split("#[cfg(test)]")[0]
        hay.append("si" if re.search(patron, t) else "no")
    print("   %-34s %-10s %s" % (nombre, hay[0], hay[1]))

# -- B - LOS EJES ------------------------------------------------------------
print()
print("B - LOS EJES QUE VARIAN, extraidos de los dos ficheros")
print()


def cita(f):
    """Como cita un identificador cada dialecto."""
    c = fn(f, "ident")
    # La cadena de formato lleva comillas escapadas en Postgres —`"\"{}\""`—
    # asi que el literal hay que leerlo respetando el escape, no cortando en la
    # primera comilla: la primera version devolvio una barra suelta.
    m = re.search(r'format!\(\s*"((?:[^"\\]|\\.)*)"', c)
    escapa = re.search(r'\.replace\(([^)]*)\)', c)
    rechaza = "Err(" in c
    return (m.group(1) if m else "?",
            "escapa: %s" % escapa.group(1) if escapa else
            ("RECHAZA si aparece" if rechaza else "-"))


def marca(f):
    c = fn(f, "sql") or fn(f, "consulta")
    m = re.findall(r'format!\("(@?\$?\{?p?\}?\{?\}?)"', c)
    posicional = "${}" in c or "${" in c
    con_tipo = "{n}:{t}:" in c or ":{t}:" in c
    return ("$N posicional" if posicional else "@pN con nombre",
            "lleva TIPO" if con_tipo else "sin tipo")


def clave(f):
    c = fn(f, "sql") or fn(f, "consulta")
    if "IN (" in c:
        return "(cols) IN (tuplas)"
    if " OR " in c:
        return "disyuncion de conjunciones"
    return "?"


def catalogo_de(f):
    """Quien hace la consulta del catalogo de esta familia, y donde."""
    main = (f.parent / "main.rs").read_text(encoding="utf-8", errors="replace")
    # No vale buscar `INFORMATION_SCHEMA` en el fichero: el driver de BigQuery
    # lo nombra para pedir TIPOS, no el catalogo, y con eso el arnes dijo que
    # los dos lo tienen. Lo que decide es si el verbo `catalogo` HACE algo.
    arm = re.search(r'"catalogo" =>\s*(.{0,80})', main, re.S)
    if arm and "Err(" not in arm.group(1):
        return "en el driver (`main.rs`)"
    if re.search(r'Some\("catalogo"\)', main):
        return "en el driver (`main.rs`)"
    return "en `ore` (receta); el driver solo tipa"


EJES = [
    ("1 · cita del identificador", lambda f: " · ".join(cita(f))),
    ("2 · marca del parametro", lambda f: " · ".join(marca(f))),
    ("3 · recorte por clave", clave),
    # Se mira el CRATE entero, no solo `sql.rs`: la consulta del catalogo de
    # Postgres vive en su `main.rs`, asi que preguntarselo a `sql.rs` decia que
    # no la tiene.
    ("4 · consulta del catalogo", lambda f: catalogo_de(f)),
]
print("   %-30s %-32s %s" % ("eje", "postgres", "bigquery"))
print("   " + "-" * 88)
for nombre, g in EJES:
    print("   %-30s %-32s %s" % (nombre, g(PG)[:32], g(BQ)[:40]))
print()
print("   Cuatro ejes, y los cuatro son DATOS: una cadena de formato, un patron")
print("   de marca, una plantilla de recorte y una consulta. Ninguno es una")
print("   decision que haya que tomar mirando el plan.")

# -- C - LO QUE SE RESISTE ---------------------------------------------------
print()
print("C - LO QUE SE RESISTE, y por que")
print()
print("   1 · LOS TIPOS DE COLUMNA · BigQuery obliga a una consulta MAS antes de")
print("       traducir, porque no coacciona `STRING` a `INT64`; Postgres no la")
print("       necesita. Eso no es una plantilla: es un PASO del procedimiento,")
print("       y un manifiesto que lo declarara tendria que declarar tambien")
print("       cuando se ejecuta. Cabe, pero como bandera —«este dialecto exige")
print("       tipar los parametros»— no como texto.")
print()
print("   2 · EL JUICIO SOBRE LAS CARAS · `fullScan: expensive` en BigQuery no")
print("       sale del catalogo: sale de saber que BigQuery FACTURA POR BYTES.")
print("       Postgres dice `cheap`. Es un juicio sobre el modelo de precios de")
print("       un producto, y ningun `INFORMATION_SCHEMA` lo contesta.")
print()
print("   3 · EL SONDEO DE LA CARA D · en Postgres es `wal_level` del CLUSTER;")
print("       en BigQuery, `enable_change_history` de la TABLA. No es la misma")
print("       consulta con otro nombre: el hecho no vive en el mismo sitio.")
print()
print("   -> los tres se resisten a ser TEXTO y ninguno se resiste a ser DATO:")
print("      una bandera, un valor por defecto y una consulta con su nivel.")

# -- D - EL PRESUPUESTO ------------------------------------------------------
print()
print("D - EL PRESUPUESTO")
lpg, lbq = len(codigo(PG)), len(codigo(BQ))
print("   %-40s %4d lineas" % ("`ore-read-postgres/sql.rs`", lpg))
print("   %-40s %4d lineas" % ("`ore-read-bigquery/sql.rs`", lbq))
print("   %-40s %4d" % ("juntos", lpg + lbq))
print()
print("   De esas, lo que es FORMA —recorrer la peticion, montar condiciones,")
print("   unir con AND, decidir si hay WHERE— esta escrito DOS VECES. Lo que es")
print("   dialecto son los cuatro ejes de (B), y en texto no llegan a diez")
print("   lineas por familia.")
print()
print("   El presupuesto de la migracion, entonces:")
print("     se queda   un `ore-read-sql` con la forma, el rango, la negativa")
print("                ante un rango no servible y el `INFORMATION_SCHEMA`")
print("     se mueve   cuatro ejes por familia, a un fichero")
print("     desaparece la segunda copia de la forma — que es donde divergen dos")
print("                derivaciones de lo mismo, que es el fallo que este arbol")
print("                lleva toda la semana persiguiendo")
print()
print("   Y una familia SQL nueva pasa de ser una crate a ser un fichero. Los")
print("   binarios a mano se reservan para donde el protocolo no llega: la")
print("   decodificacion logica de un CDC, OPC-UA, una API nativa.")

# -- E - AIRBYTE -------------------------------------------------------------
print()
print("E - AIRBYTE: hasta donde encaja")
print()
print("   Su protocolo son cuatro verbos:")
print("     spec()                                  -> ConnectorSpecification")
print("     check(config)                           -> estado de la conexion")
print("     discover(config)                        -> AirbyteCatalog")
print("     read(config, configuredCatalog, state)  -> Stream<AirbyteMessage>")
print()
print("   Y el de ORE son tres. La correspondencia NO es uno a uno:")
print()
print("   %-26s %-30s %s" % ("ORE pide", "Airbyte da", "veredicto"))
print("   " + "-" * 88)
FILAS = [
    ("catalogo · columnas/tipos", "discover -> streams + JSON Schema", "encaja"),
    ("catalogo · primaryKey", "source_defined_primary_key", "encaja"),
    ("catalogo · `reads`", "(nada)", "HAY QUE INVENTARLO"),
    ("catalogo · `changes`", "sync modes: full_refresh/incremental", "parcial"),
    ("leer · proyeccion", "el stream ENTERO", "NO ENCAJA"),
    ("leer · recorte por clave", "(nada)", "NO ENCAJA"),
    ("leer · filtros", "(nada)", "NO ENCAJA"),
    ("leer · rango", "incremental por cursor_field", "encaja"),
    ("testigo ANTES de leer", "el `state` sale CON las filas", "NO ENCAJA"),
]
for a, b, c in FILAS:
    print("   %-26s %-30s %s" % (a, b, c))
print()
print("   LA FILA QUE DECIDE es «leer · proyeccion», y no por rendimiento.")
print()
print("   La seleccion de columnas de Airbyte no baja al conector: «la")
print("   infraestructura de Airbyte ELIMINA los campos no seleccionados")
print("   durante la sincronizacion». Lo eligieron a proposito —una API REST no")
print("   se beneficiaria, y habria que tocar todos los conectores—, y para")
print("   ellos es correcto.")
print()
print("   Para esto no, y esta escrito en `ore-driver`:")
print()
print("     «una propiedad `redact` no esta en el plan, luego no esta en la")
print("      peticion, luego NO PUEDE ESTAR EN EL SQL. La salvaguarda es")
print("      estructural — no hay ningun punto donde alguien pueda olvidarse de")
print("      aplicarla, porque no hay nada que aplicar.»")
print()
print("   Con Airbyte debajo, la columna enmascarada SALE del origen y alguien")
print("   la tira despues. Eso no es la misma garantia mas lenta: es OTRA")
print("   garantia —una que se aplica en vez de no existir— y el arbol no tiene")
print("   hoy vocabulario para decir cual de las dos tiene una fuente.")
print()
print("   Y el testigo es el segundo desencuentro, con su motivo ya escrito:")
print("     «Meterlo en `leer` seria peor que en `catalogo`: llegaria CON las")
print("      filas, y quien pregunta lo hace para decidir SI hace falta")
print("      leerlas.»")
print("   El `state` de Airbyte es exactamente eso que el protocolo rechaza.")
print()
print("   -> DONDE SI: la cola larga de APIs SaaS —Salesforce, Stripe, Jira—,")
print("      que ORE no va a escribir nunca y donde leer el stream entero es la")
print("      norma de todos modos, porque esas APIs no aceptan un predicado")
print("      arbitrario. Ahi Airbyte no quita nada: no habia nada que empujar.")
print()
print("   -> DONDE NO: bases de datos y almacenes. Son justo las familias que")
print("      este arbol ya tiene, y meterlas por Airbyte seria cambiar")
print("      proyeccion, recorte por clave y filtros por un stream entero.")
print()
print("   -> LO QUE FALTA ANTES DE PODER DECIDIRLO: `reads` sabe decir que un")
print("      origen no empuja nada —`predicatePushdown: []`— y NO sabe decir")
print("      que la proyeccion se respeta despues de leer. Sin esa palabra, una")
print("      fuente de Airbyte entra al arbol indistinguible de una que si")
print("      empuja, y el sello no cambia. Es la misma forma de todos los")
print("      hallazgos de esta semana: lo que falta se parece a lo que esta bien.")
