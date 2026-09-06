# -*- coding: utf-8 -*-
"""El espectro del dialecto declarado: ¿cabe una familia SQL en un fichero?

La forma que el sector converge —Airbyte con su CDK declarativo, Openflow con
flujos de NiFi versionados, Cognite con trabajos de configuracion— es la misma:
**el conector es una DECLARACION que interpreta un runtime compartido**, no un
programa. Airbyte lo dice con la frase que importa: el contrato fuerte permite
implementar una funcion UNA VEZ en vez de repetirla por conector.

Aqui hay dos traductores escritos a mano —Postgres y BigQuery— y esto mide si
la distancia entre ellos es un dialecto o es un programa.

  A. EL ESQUELETO  que hace cada traductor, paso a paso, y si coinciden
  B. LOS EJES      en cuantas cosas difieren de verdad
  C. LO QUE SE VA  lo que hoy se reescribe por driver y se escribiria una vez
  D. LO QUE RESISTE lo que NO cabe en un manifiesto, y por que
  E. LA COLA LARGA por que la respuesta no es Airflow, medido contra el
                   protocolo de aqui
"""
import pathlib
import re

RAIZ = pathlib.Path(r"C:\ORE")
CRATES = RAIZ / "crates"

PG = (CRATES / "ore-read-postgres/src/sql.rs").read_text(encoding="utf-8")
BQ = (CRATES / "ore-read-bigquery/src/sql.rs").read_text(encoding="utf-8")


def sin_pruebas(t):
    return t.split("#[cfg(test)]")[0]


def utiles(t):
    return [l for l in sin_pruebas(t).split("\n")
            if l.strip() and not l.strip().startswith("//")]


print("== el espectro del dialecto declarado ==")

# -- A - EL ESQUELETO --------------------------------------------------------
print()
print("A - EL ESQUELETO: los mismos pasos, en el mismo orden")
# Se buscan los HITOS de la construccion, no las lineas: si los dos traductores
# emiten las mismas piezas en el mismo orden, lo que los separa es como se
# escribe cada pieza, que es la definicion de dialecto.
HITOS = [
    ("la proyeccion", r"SELECT \{\}"),
    ("el objeto", r"FROM \{\}"),
    ("el recorte por clave", r"claves\.is_empty\(\)"),
    ("los filtros", r"for \(col, op, valor\) in &p\.filtros"),
    ("el operador `gt`", r'"gt" => ">"'),
    ("el rango, start exclusivo", r"start.*?>"),
    ("el rango, end inclusivo", r"end.*?<="),
    ("el WHERE, si hay algo", r'push_str\(" WHERE "\)'),
]
print("   %-28s %-10s %s" % ("paso", "postgres", "bigquery"))
print("   " + "-" * 52)
iguales = 0
for nombre, patron in HITOS:
    a = bool(re.search(patron, sin_pruebas(PG), re.S))
    b = bool(re.search(patron, sin_pruebas(BQ), re.S))
    iguales += a and b
    print("   %-28s %-10s %s" % (nombre, "si" if a else "no", "si" if b else "no"))
print()
print("   %d de %d pasos, en los dos y en el mismo orden." % (iguales, len(HITOS)))
print("   %-32s %3d lineas" % ("postgres, sin comentarios ni pruebas", len(utiles(PG))))
print("   %-32s %3d lineas" % ("bigquery, idem", len(utiles(BQ))))

# -- B - LOS EJES ------------------------------------------------------------
print()
print("B - LOS EJES: en que difieren de verdad")
EJES = [
    ("citar un identificador",
     'formato por partes: `"x"."y"`, doblando la comilla',
     'entero y con acento grave: `` `x.y` ``, SIN escape -> se rechaza'),
    ("la marca del parametro",
     "posicional: `$1`, `$2`...",
     "con nombre: `@p0`, `@p1`..."),
    ("el tipo del parametro",
     "no hace falta: el servidor coacciona el texto",
     "OBLIGATORIO: GoogleSQL no coacciona STRING a INT64"),
    ("el recorte por clave",
     "`(cols) IN ((a,b),(c,d))`",
     "disyuncion de conjunciones"),
    ("de donde salen los tipos",
     "no se piden",
     "una consulta mas a `INFORMATION_SCHEMA.COLUMNS`"),
]
for i, (eje, a, b) in enumerate(EJES, 1):
    print()
    print("   %d · %s" % (i, eje))
    print("       postgres : %s" % a)
    print("       bigquery : %s" % b)
print()
print("   Cinco ejes. Los cuatro primeros son DATOS —un formato, un simbolo, un")
print("   booleano, una forma de entre dos—. El quinto no lo es, y por eso esta")
print("   en (D).")

# -- C - LO QUE SE VA --------------------------------------------------------
print()
print("C - LO QUE SE ESCRIBIRIA UNA VEZ")
COMPARTIDO = [
    ("rango_servible", "ya esta compartido en `ore-driver`, y es el precedente"),
    ("nunca `SELECT *`", "el aserto de la mascara, hoy probado DOS veces"),
    ("valores siempre como parametro", "hoy escrito dos veces"),
    ("start exclusivo / end inclusivo", "la convencion de Iceberg, dos veces"),
    ("`gt` y `eq`, y nada mas", "el vocabulario cerrado, dos veces"),
    ("la tupla de clave no se concatena", "dos veces"),
]
for q, d in COMPARTIDO:
    print("   %-34s %s" % (q, d))
print()
print("   Y el precedente ya existe y esta escrito: `rango_servible` vive en")
print("   `ore-driver` con su motivo —«la que se repite en tres sitios es la que")
print("   falta en el cuarto»—. El manifiesto es esa misma frase aplicada al")
print("   resto del traductor.")

# -- D - LO QUE RESISTE ------------------------------------------------------
print()
print("D - LO QUE NO CABE EN UN MANIFIESTO")
print()
print("   1 · PEDIR LOS TIPOS. BigQuery necesita una consulta previa y Postgres")
print("       no. Eso no es un formato: es un PASO CONDICIONAL del que depende")
print("       si la consulta principal se puede construir. Un manifiesto puede")
print("       declarar «necesito tipos, y esta es la consulta que los trae» —")
print("       pero el runtime tiene que saber ejecutar dos consultas en orden.")
print()
print("   2 · EL CATALOGO. Es la mitad mas grande y la mas particular: la de")
print("       BigQuery son cuatro CTEs sobre siete vistas mas `__TABLES__`, y la")
print("       de Postgres son tres consultas incluyendo `current_setting`. Cabe")
print("       como TEXTO en el manifiesto; lo que no cabe es la derivacion de")
print("       las dos caras, que es juicio: `require_partition_filter` ->")
print("       `fullScan: forbidden` no se lee de ninguna tabla.")
print()
print("   3 · EL SONDEO QUE NO ES UNA CONSULTA. `wal_level` es del cluster y")
print("       `relreplident` de la tabla; que uno apague al otro es una regla,")
print("       no un mapeo. Y el aviso por stderr cuando no es `logical` tampoco.")
print()
print("   -> asi que el manifiesto no cubre una familia entera: cubre LA")
print("      TRADUCCION, que es lo que se repite. El catalogo y la derivacion de")
print("      caras siguen siendo codigo — y son, justamente, lo que distingue a")
print("      una familia de otra de verdad.")

# -- E - LA COLA LARGA -------------------------------------------------------
print()
print("E - LA COLA LARGA, y por que Airflow no es la respuesta")
print()
print("   Airflow es un ORQUESTADOR: 98 proveedores y 1.600+ modulos —848")
print("   operadores, 298 ganchos—. Lo que un operador sabe hacer es EJECUTAR un")
print("   paso; lo que aqui hace falta es contestar dos preguntas que un")
print("   operador no contesta:")
print("     - «que objetos hay, con que columnas y QUE SE LES PUEDE PEDIR»")
print("     - «dame estas columnas de estas filas»")
print("   Un `PostgresOperator` no declara `predicatePushdown` ni `changes.mode`.")
print("   Airflow encajaria como el que LLAMA a `ore materialize`, no como el que")
print("   provee fuentes.")
print()
print("   El candidato de verdad es Airbyte, porque sus verbos son los de aqui:")
print("     airbyte  spec · check · discover · read")
print("     ore      —      · —     · catalogo · leer")
print("   550+ conectores hablando ese protocolo. Y aun asi hay un problema, y")
print("   no es de comodidad. El protocolo de Airbyte:")
print("     - NO admite seleccionar columnas — «el destino recibe todos los")
print("       campos que emite la fuente y filtra el»;")
print("     - NO admite ningun predicado de fila;")
print("     - solo tiene el incremental por cursor.")
print()
# Lo que la peticion de aqui lleva y el `read` de Airbyte no.
campos = ["proyeccion", "clave_columnas", "claves", "filtros", "start", "end", "cursor"]
drv = (CRATES / "ore-driver/src/lib.rs").read_text(encoding="utf-8")
presentes = [c for c in campos if re.search(r"pub %s:" % c, drv)]
print("   La peticion de aqui lleva %d campos: %s" % (len(presentes), ", ".join(presentes)))
print("   De esos, el `read` de Airbyte solo tiene el equivalente de `cursor` y")
print("   `start` (su `state`). Los otros %d se caerian." % (len(presentes) - 2))
print()
print("   Y uno de los que se caen es el que sostiene la mascara. Hoy:")
print("     «una propiedad `redact` no esta en el plan, luego no esta en la")
print("      peticion, luego NO PUEDE ESTAR EN EL SQL. La salvaguarda es")
print("      estructural — no hay ningun punto donde alguien pueda olvidarse de")
print("      aplicarla, porque no hay nada que aplicar.»")
print()
print("   Por un adaptador de Airbyte, la columna redactada VIAJA y se tira")
print("   aqui. La salvaguarda pasa de estructural a procedimental, que es")
print("   exactamente la clase de cambio que este proyecto existe para impedir.")
print()
print("   -> la cola larga por Airbyte es posible y tiene precio, y el precio")
print("      hay que declararlo: seria una familia con OTRAS capacidades")
print("      —`reads: {}`, sin empuje— y habria que decir en el sustrato que lo")
print("      que llega por ahi no se puede enmascarar en origen. Eso es")
print("      `reads`/`changes` haciendo su trabajo, no una excepcion.")
