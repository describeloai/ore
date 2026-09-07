# -*- coding: utf-8 -*-
"""`ore drift`, tramos 2 y 3: la comparacion y la clasificacion, con el sector delante.

La deriva de esquema es un problema resuelto en publico cinco veces, en cinco
disciplinas distintas, y las cinco publican su criterio. Mirarlo antes de
escribir acota mas que discutirlo.

  A. EL SECTOR                 seis referencias, y que transfiere cada una
  B. QUE DE ESO YA TIENE ORE   medido sobre el arbol, no supuesto
  C. TRAMO 2 · LA COMPARACION  contra que, exactamente — y una correccion mia
  D. LA FRONTERA               origen contra gobierno: lo que decide si sirve
  E. TRAMO 3 · LA REJILLA      cada clase, con las tres preguntas del sector
  F. LO QUE ACOTA Y LO QUE NO
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


print("== `ore drift`, tramos 2 y 3, con lo publicado delante ==")

# -- A -----------------------------------------------------------------------
print()
print("A - EL SECTOR: seis referencias, y la regla que transfiere cada una")
print()
SECTOR = [
    ("TERRAFORM", "plan -refresh-only",
     "«propone actualizar el estado REGISTRADO para que case con los objetos "
     "remotos; NO propone cambiar los objetos remotos». La correccion va "
     "siempre hacia la declaracion, jamas hacia el origen."),
    ("TERRAFORM", "plan -detailed-exitcode",
     "0 sin deriva · 1 error · 2 HAY DERIVA. Es la convencion publicada para "
     "que un detector se pueda poner en un pipeline sin parsear su salida."),
    ("ATLAS", "pre-apply drift check",
     "la comprobacion corre AL EMPEZAR `migrate apply`, antes de que se "
     "ejecute un solo fichero, y con `on_error = FAIL` aborta. La deriva no es "
     "un informe: es una PRECONDICION DE LA ESCRITURA."),
    ("AWS GLUE", "SchemaChangePolicy",
     "dos ejes independientes —`UpdateBehavior` y `DeleteBehavior`— y el "
     "defecto recomendado es ASIMETRICO: actualizar en los cambios, solo "
     "REGISTRAR en los borrados. Nunca se borra solo."),
    ("CONFLUENT", "BACKWARD / FORWARD / FULL",
     "la compatibilidad tiene DIRECCION, y cual importa depende de quien se "
     "mueva primero. Quitar un campo rompe hacia atras; anadir uno con "
     "defecto, no. Un solo eje «rompe/no rompe» es demasiado grueso."),
    ("ICEBERG", "promocion de tipos",
     "solo en la direccion que ENSANCHA y preserva informacion: int->long, "
     "float->double. La contraria arriesga perdida. Un cambio de tipo no es "
     "una clase de deriva: son dos, y opuestas."),
    ("OBSERVABILIDAD", "Monte Carlo · Datafold · Metaplane",
     "todos detectan el cambio de esquema, y el modo de fallo que todos "
     "reportan es LA FATIGA DE ALERTAS. La respuesta del sector es puntuar por "
     "impacto aguas abajo —«blast radius»— y aprender que tablas importan por "
     "su uso real."),
]
for quien, que, regla in SECTOR:
    print("   %-16s %s" % (quien, que))
    parrafo(regla, "       ")
    print()

# -- B -----------------------------------------------------------------------
print()
print("B - QUE DE ESO YA TIENE ORE, medido")
print()
diff = (RAIZ / "crates/ore-core/src/diff.rs").read_text(encoding="utf-8", errors="replace")
tipos = (RAIZ / "crates/ore-core/src/types.rs").read_text(encoding="utf-8", errors="replace")
TIENE = [
    ("la DIRECCION de la compatibilidad", "cada dirección le duele" in diff,
     "los pares espejo `OOS5009`/`OOS5011`, `OOS5012`/`OOS5026`, "
     "`OOS5028`/`OOS5029`: «cada direccion le duele a otro». Es lo de "
     "Confluent, y estaba antes de mirarlo."),
    ("el BLAST RADIUS, exacto y no aprendido",
     hay("pub fn linaje", "crates/ore-view/src/lineage.rs"),
     "el linaje por columna dice de que columna raiz sale cada salida, "
     "INCLUIDA la arista `INDIRECT` del `where`. El sector lo aproxima con "
     "«que tablas se usan mas»; aqui se sabe."),
    ("la correccion en UNA sola direccion", True,
     "estructural, no un modo: `ore` no puede escribir en BigQuery. Lo que "
     "Terraform tiene que ofrecer como bandera aqui es un hecho."),
    ("un orden de ENSANCHE de tipos",
     bool(re.search(r"fn (ensancha|widen|promociona)", tipos)),
     "`Type` tiene `Scalar`, `Parametric`, `List`, `Imported` y NINGUNA "
     "relacion de orden. Lo de Iceberg habria que escribirlo."),
    ("un codigo de salida para la deriva", False,
     "no existe el mando. El arbol usa sysexits —64, 65, 73—; Terraform usa "
     "2 para «hay deriva», que NO es un sysexit y es la convencion del oficio."),
    ("la deriva como precondicion de escribir", False,
     "`ore materialize` puebla una copia sin preguntarle al origen si sigue "
     "siendo el que la tabla declara. Es justo lo que Atlas aborta."),
]
for que, esta, nota in TIENE:
    print("   [%s] %s" % ("x" if esta else " ", que))
    parrafo(nota, "       ")
    print()

# -- C -----------------------------------------------------------------------
print()
print("C - TRAMO 2: CONTRA QUE SE COMPARA, y una correccion mia")
print()
parrafo("La medida anterior decia «comparar el catalogo contra las `kind: "
        "Table` del paquete». ES FALSO, y lo dice el propio inductor: las 18 "
        "claves del catalogo NO aterrizan en un solo documento.")
print()
EMITE = [
    ("Table", "columns · reads · changes", "+ `datasource` y `object`"),
    ("Entity", "primaryKey · uniqueKeys · properties · relations",
     "los TIPOS viven aqui, y las foraneas salen como relaciones"),
    ("View", "fields · where", "la proyeccion"),
]
print("   %-8s %-46s" % ("kind", "que recibe del catalogo"))
print("   " + "-" * 70)
for k, q, _ in EMITE:
    print("   %-8s %-46s" % (k, q))
print()
for k, _, n in EMITE:
    print("   · %-8s %s" % (k, n))
print()
parrafo("Asi que comparar solo contra `Table` se dejaria TRES clases enteras "
        "del espectro —clave primaria, claves alternativas y foraneas— que son "
        "justo donde vive `OOS5019`. Y el tipo de una columna, que es la clase "
        "que Iceberg parte en dos, esta en la ENTIDAD.")
print()
parrafo("La consecuencia es que la comparacion honesta es CATALOGO CONTRA "
        "PAQUETE, y la unica forma de hacerla sin escribir un segundo "
        "repartidor es usar el que ya hay: inducir el catalogo nuevo CON LAS "
        "DECISIONES YA CONTESTADAS —`inducir_con` es una funcion pura de "
        "(catalogo, decisiones)— y comparar los dos arboles.")

# -- D -----------------------------------------------------------------------
print()
print("D - LA FRONTERA QUE DECIDE SI ESTO SIRVE: origen contra gobierno")
print()
parrafo("Y ahi esta el problema real del tramo 2, que no es tecnico. Un "
        "paquete gobernado tiene cosas que el catalogo NO PUEDE SABER: quien "
        "responde, la madurez, la frescura, si se materializa, las etiquetas, "
        "las descripciones que escribio una persona. Un arbol reinducido las "
        "trae en blanco.")
print()
print("   %-26s %s" % ("del ORIGEN · se compara", "de GOBIERNO · se ignora"))
print("   " + "-" * 70)
PARES = [
    ("columns y sus tipos", "owner"),
    ("reads · changes", "oos.maturity"),
    ("primaryKey · uniqueKeys", "freshness"),
    ("relations (de foreignKeys)", "materialized"),
    ("el objeto fisico", "labels y descripciones humanas"),
]
for a, b in PARES:
    print("   %-26s %s" % (a, b))
print()
parrafo("Comparar sin esa frontera daria deriva EN TODOS LOS PAQUETES "
        "GOBERNADOS, siempre, y el detector seria inservible el primer dia. Es "
        "la fatiga de alertas del apartado (A) pero por construccion, no por "
        "umbral mal puesto.")
print()
parrafo("Y hay un aserto que la fija y que ya se puede escribir: EL CATALOGO "
        "CAPTURADO CONTRA EL PAQUETE QUE SALE DE EL TIENE QUE DAR CERO. Si da "
        "cualquier otra cosa, lo que sobra es exactamente lo que hay que "
        "meter en la columna de la derecha.")

# -- E -----------------------------------------------------------------------
print()
print("E - TRAMO 3: LA REJILLA, con las tres preguntas del sector")
print()
print("   Cada deriva se responde tres veces, y las tres son de sitios distintos:")
print()
print("     ¿ENSANCHA o ESTRECHA?   Iceberg · es del cambio en si")
print("     ¿A QUIEN LE DUELE?      Confluent · tiene direccion")
print("     ¿A CUANTOS?             observabilidad · y aqui se sabe, por linaje")
print()
REJILLA = [
    ("objeto nuevo", "ensancha", "a nadie", "0", "informe"),
    ("objeto que desaparece", "estrecha", "a quien lo nombre", "linaje", "OOS5007"),
    ("columna nueva", "ensancha", "a nadie", "0", "informe"),
    ("columna que desaparece", "estrecha", "a quien la proyecte", "linaje", "OOS5007"),
    ("tipo que ENSANCHA", "ensancha", "a nadie", "0", "informe"),
    ("tipo que ESTRECHA", "estrecha", "a quien la lea", "linaje", "sin codigo"),
    ("`reads` admite menos", "estrecha", "al planificador", "linaje", "OOS5031"),
    ("`reads` admite mas", "ensancha", "a nadie", "0", "informe"),
    ("`changes` degrada", "estrecha", "a la copia", "materializadas", "OOS5032"),
    ("`primaryKey` cambia", "incomparable", "al indice", "linaje", "OOS5019"),
]
print("   %-24s %-12s %-20s %s" % ("clase", "direccion", "a quien", "salida"))
print("   " + "-" * 74)
for q, d, a, _, s in REJILLA:
    print("   %-24s %-12s %-20s %s" % (q, d, a, s))
print()
parrafo("Seis de diez son informe y no codigo. Eso NO es que falten seis "
        "codigos: es que una columna nueva no rompe un contrato, y darle un "
        "codigo de compatibilidad seria inventar errores para avisos — que es "
        "literalmente como se llega a la fatiga de alertas que el sector "
        "reporta.")
print()
parrafo("Y el «a cuantos» es lo que aqui sale gratis y a los demas no. Monte "
        "Carlo puntua «incident likelihood y blast radius» APRENDIENDO de que "
        "tablas se usan; el linaje por columna lo sabe sin aprender nada, "
        "porque la vista que proyecta esa columna esta escrita.")

# -- F -----------------------------------------------------------------------
print()
print("F - LO QUE LA INVESTIGACION ACOTA, Y LO QUE DEJA ABIERTO")
print()
print("   CERRADO por lo publicado:")
for q in [
    "el codigo de salida: 0 sin deriva, 2 con deriva, 1 error (Terraform)",
    "no se borra nada solo, nunca: `DeleteBehavior: LOG` es el defecto que "
    "recomienda quien lleva anos con esto (Glue)",
    "la correccion va a la declaracion y no al origen — aqui ni siquiera es "
    "una eleccion (Terraform)",
    "el cambio de tipo se parte en dos por la direccion del ensanche (Iceberg)",
    "la salida se ordena por a quien le duele, no por que cambio "
    "(observabilidad)",
]:
    print()
    parrafo("· " + q, "     ")
print()
print("   ABIERTO, y hay que decidirlo:")
for q in [
    "¿es `ore drift` una precondicion de `ore materialize`? Atlas aborta el "
    "apply si el esquema derivo, y aqui poblar una copia contra un origen que "
    "ya no es el declarado es el mismo fallo sin sintoma que la truncacion",
    "¿el orden de ensanche de tipos se escribe en OOS o se queda en el "
    "detector? Es vocabulario, y el vocabulario es de la especificacion",
    "¿el detector lee el catalogo del origen, o de un fichero? Las dos: es la "
    "misma pareja `--source` / `--from` que `discover` ya tiene, y por el "
    "mismo motivo",
]:
    print()
    parrafo("· " + q, "     ")
