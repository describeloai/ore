# -*- coding: utf-8 -*-
"""Agrupar vistas en paquetes: la base que ya hay, y la pregunta que nadie contesto.

El concepto tiene base, y mas de la que parece: `moved` y `reserved` sobre
NOMBRES DE DOCUMENTO, `exports` como superficie publica, y cuatro codigos que ya
cobran cada forma de equivocarse. Lo que no hay es el verbo — y hay algo mas
gordo que eso, medido con dos experimentos.

  A. QUE ES UN PAQUETE          y la pregunta que nadie ha tenido que contestar
  B. LA BASE QUE YA ESTA        pieza a pieza, medida
  C. LO QUE NO SE ROMPE         la mitad buena, y es grande
  D. LO QUE SI, y donde         los cuatro sitios que hay que tocar a la vez
  E. LO QUE NO EXISTE
  F. LA FORMA QUE PROPONE
"""
import pathlib
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


print("== agrupar vistas en paquetes, medido ==")

# -- A -----------------------------------------------------------------------
print()
print("A - QUE ES UN PAQUETE, Y LA PREGUNTA QUE NADIE HA CONTESTADO")
print()
print("   %-22s %s" % ("la membresia", "la dice EL DIRECTORIO"))
parrafo("`01-package` §3.3 lo midio y lo escribio: la primera formulacion pedia "
        "que el manifiesto dijera de que se compone el paquete —como el data "
        "model de Cognite lista sus `views`— y «medido sobre el corpus, eso ya "
        "lo dice el directorio, y redeclararlo seria declarar lo derivable, P2».")
print()
print("   %-22s %s" % ("el nombre", "lo dice `metadata.namespace`"))
parrafo("`qname()` es `<namespace>.<name>`, y es con lo que TODO el arbol se "
        "refiere a un documento: `backedBy`, `from.view`, `exports`.")
print()
print("   %-22s %s" % ("la superficie", "la dice `exports` en `package.yaml`"))
parrafo("y es VISIBILIDAD, no membresia — por eso el nombre es el de Java y el "
        "de Node, y no el de Cognite. «Ausente significa NADA, no todo».")
print()
print("   -> Y AQUI ESTA EL HUECO: NADIE ATA LAS DOS PRIMERAS.")
print()
parrafo("Medido con un experimento, no supuesto. Un documento que vive en "
        "`packages/ventas` y se llama a si mismo "
        "`otro_paquete_distinto.E` valida LIMPIO:")
print()
print("       $ ore validate <arbol>")
print("       ok · sin errores")
print()
parrafo("Y el segundo experimento cierra la pinza: si ese mismo paquete pone "
        "`exports: [ventas.E]`, salta `OOS2027` —«nombra algo que este paquete "
        "no contiene»—, porque `exports` habla en NOMBRE CUALIFICADO y el "
        "documento se llama otra cosa.")
print()
print("   %-22s %s" % ("la pertenencia", "el directorio"))
print("   %-22s %s" % ("la identidad", "el `namespace`"))
print("   %-22s %s" % ("quien los ata", "NADIE"))
print()
parrafo("Asi que «mover una vista a otro paquete» NO TIENE HOY UN SIGNIFICADO "
        "UNICO: son dos cosas —el fichero y el nombre— y se pueden mover por "
        "separado sin que nada proteste. Esto se decide ANTES de escribir el "
        "verbo, o el verbo movera las dos y nada comprobara que sigan de "
        "acuerdo la proxima vez que alguien las toque a mano.")

# -- B -----------------------------------------------------------------------
print()
print("B - LA BASE QUE YA ESTA, pieza a pieza")
print()
CODE = "crates/ore-core/src/code.rs"
PIEZAS = [
    ("`moved` sobre NOMBRES DE DOCUMENTO",
     hay("moved", "vendor/oos/spec/v1alpha1/01-package.md"),
     "`01-package` §3.4: el manifiesto PUEDE declarar `moved` y `reserved` "
     "sobre nombres cualificados de documento. `{ from: hr.iberia, to: "
     "hr.iberica, since: 2.0.0 }`"),
    ("un documento anunciado NO es `OOS5007`",
     hay("Oos5007", CODE),
     "es la puerta entera: sin ella, mover algo es siempre un rompedor de "
     "`CONSUMER` y un salto mayor"),
    ("reutilizar un nombre reservado es `OOS2006`",
     hay("Oos2006", CODE),
     "y reservar lo que nunca existio es legal — reservar mira hacia delante"),
    ("`exports`, la superficie publica",
     hay("Oos2027", CODE) and hay("Oos2028", CODE),
     "`OOS2027` si nombra lo que no contiene; `OOS2028` si otro paquete "
     "referencia lo que este no exporta — y NO es `OOS2018`, porque el nombre "
     "existe y decir «no existe» manda a mirar el fichero equivocado"),
    ("`diff` ya lee lo anunciado",
     hay("anunciados_doc", "crates/ore-core/src/diff.rs"),
     "«lo que el MANIFIESTO anuncia: nombres de documento que se movieron o se "
     "retiraron. Es el alcance ancho de la misma disciplina»"),
    ("pruebas de los dos alcances nuevos",
     (RAIZ / "crates/ore-cli/tests/renombrar.rs").exists(),
     "el renombrado anunciado de un documento, el campo que desaparece sin "
     "anuncio y el nombre que vuelve"),
]
for que, esta, nota in PIEZAS:
    print("   [%s] %s" % ("x" if esta else " ", que))
    parrafo(nota, "       ")
    print()
parrafo("O sea que LA DISCIPLINA ESTA COMPLETA. No falta ni una regla: falta "
        "quien la ejecute.")

# -- C -----------------------------------------------------------------------
print()
print("C - LO QUE NO SE ROMPE AL MOVER, y es la mitad buena")
print()
NO_ROMPE = [
    ("el `datasource` de una tabla",
     "`datasources` vive en `ontology.config.yaml`, que es del WORKSPACE y no "
     "del paquete — ademas «no viaja dentro del `.oob`»"),
    ("el reticulo y los conductos",
     "`lattices/` y `conduits.yaml` estan en la raiz del workspace, no dentro "
     "de un paquete. Una etiqueta sigue resolviendo despues de mudarse"),
    ("el digest",
     "«nombra documentos por su nombre cualificado, no por su ruta», asi que "
     "la equivalencia de disposicion de `90-canonical-form` §5.2 se conserva: "
     "mover un fichero DE SITIO dentro del mismo paquete no cambia nada"),
]
for q, por in NO_ROMPE:
    print("   · %s" % q)
    parrafo(por, "       ")
    print()
parrafo("Es mas de lo que parecia. Lo unico que de verdad cruza el limite de un "
        "paquete es EL NOMBRE, y por eso todo lo que sigue es sobre nombres.")

# -- D -----------------------------------------------------------------------
print()
print("D - LO QUE SI SE ROMPE, Y LOS CUATRO SITIOS QUE HAY QUE TOCAR A LA VEZ")
print()
print("   %-28s %-16s %s" % ("si se olvida", "quien lo caza", "cuando"))
print("   " + "-" * 70)
SITIOS = [
    ("mover el fichero", "nadie", "es la fuente: sin esto no hay movimiento"),
    ("cambiar el `namespace`", "NADIE", "el experimento de (A). Silencio total"),
    ("`moved` en el manifiesto viejo", "OOS5007", "al siguiente `ore diff`"),
    ("`exports` en el paquete nuevo", "OOS2028", "al compilar, si alguien lo referencia"),
    ("reapuntar `backedBy` / `from`", "OOS2018", "al validar"),
]
for q, quien, cuando in SITIOS:
    print("   %-28s %-16s %s" % (q, quien, cuando))
print()
parrafo("Cuatro sitios, cuatro ficheros distintos, y uno de los cinco fallos "
        "NO LO CAZA NADIE. Ese es el argumento entero del verbo: no es "
        "comodidad, es que hacerlo a mano tiene un modo de fallo silencioso.")
print()
parrafo("Y el orden importa, que es lo que un mando puede garantizar y una "
        "persona no: si se anuncia el `moved` antes de que el destino exporte, "
        "hay una ventana en la que el arbol no compila; si se hace al reves, "
        "hay una en la que el nombre viejo sigue vivo en dos sitios.")

# -- E -----------------------------------------------------------------------
print()
print("E - LO QUE NO EXISTE")
print()
FALTA = [
    ("ningun mando mueve nada", "no hay `ore package`; `ore pack` escribe un "
     "`.oob` y es otra cosa"),
    ("nadie escribe `exports`", "el inductor no lo emite y `ore view add` "
     "tampoco: la superficie publica de todo paquete inducido es VACIA, que "
     "por P4 significa que no exporta nada"),
    ("`ore view add` escribe en un sitio fijo", "el `views/` hermano del "
     "directorio del origen. No sabe elegir paquete, y esta bien que no lo "
     "sepa: elegirlo es otra decision"),
]
for q, por in FALTA:
    print("   · %s" % q)
    parrafo(por, "       ")
    print()

# -- F -----------------------------------------------------------------------
print()
print("F - LA FORMA QUE PROPONE ESTA MEDIDA")
print()
print("   ore package move <qname> --to <paquete> [--path <raiz>]")
print()
for q in [
    "mueve el fichero, reescribe el `namespace` y ANUNCIA el `moved` en el "
    "manifiesto de origen, en un solo acto: son las tres cosas que hoy se "
    "hacen por separado y una de ellas no la caza nadie",
    "anade el nombre a `exports` del destino SOLO si alguien de fuera lo "
    "referencia — porque «ausente significa nada» y exportar lo que nadie usa "
    "es ampliar la superficie publica sin motivo",
    "reapunta lo que lo nombraba, o se niega diciendo quien: es la misma "
    "eleccion que `ore view add` ya hace al no sobrescribir",
    "y NO decide `since`: la version del `moved` es de quien publica",
]:
    parrafo("· " + q, "     ")
    print()
parrafo("Pero lo primero no es el verbo. Es contestar (A): si la pertenencia la "
        "dice el directorio y la identidad el `namespace`, ATARLOS ES UNA REGLA "
        "QUE FALTA — y es mas barata que el mando, se comprueba al validar, y "
        "sin ella el mando puede dejar el arbol en un estado que nada detecta.")
