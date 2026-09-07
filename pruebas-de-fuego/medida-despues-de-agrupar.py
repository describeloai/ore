# -*- coding: utf-8 -*-
"""Lo que `groupBy` deja abierto: la entidad, el `having` y el agregado global.

Todo lo de abajo se CORRIO — paquetes escritos en un directorio temporal y
pasados por `ore view` y `ore validate`, mas cuatro planes contra el IR. Las
tablas son la salida, no un recuerdo de ella.

  A. LA ENTIDAD          y las dos mitades del problema, una tapando la otra
  B. EL `having`         la maquina, medida
  C. PARA QUE            y el argumento que no es de informes
  D. EL AGREGADO GLOBAL  por que se nego, y que costaria devolverlo
  E. Y UNA COSA QUE SALIO SIN BUSCARLA
"""
import textwrap


def parrafo(t, sangria="     ", ancho=70):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


print("== despues de agrupar, medido ==")

# -- A -------------------------------------------------------------------------
print()
print("A - LA ENTIDAD SOBRE UNA VISTA AGRUPADA")
print()
parrafo("Una `Entity` con `backedBy` a una vista que agrupa, `primaryKey: "
        "[pais]` y dos propiedades — `pais`, que sale de una columna, y `n`, "
        "que sale de `count()`. No compila:")
print()
print("   error[OOS2022]: `hr.por_pais` no expone `n`, que `hr.ConteoPais` declara")
print()
parrafo("Y el diagnostico esta EQUIVOCADO: la vista si expone `n`. Lo que pasa "
        "es que `vistas::campos` dejo de devolver los agregados —bien, porque "
        "sus cinco lectores preguntan «de que COLUMNA sale este campo», y de un "
        "agregado la respuesta es que de ninguna— y TRES de esos lectores "
        "preguntan en realidad otra cosa: «que campos EXPONE esta vista».")
print()
print("   %-46s %s" % ("quien llama a `campos`", "que pregunta de verdad"))
print("   " + "-" * 74)
LECTORES = [
    ("vistas.rs · OOS2018, la columna existe", "de que columna sale"),
    ("vistas.rs · invertible", "de que columna sale"),
    ("vistas.rs · raiz(), la cadena hasta el suelo", "de que columna sale"),
    ("ore-cli/vista.rs · el plan", "de que columna sale"),
    ("ore-cli/vista.rs · etiquetas de raiz", "de que columna sale"),
    ("ore-cli/vista.rs · requiredFilters", "de que columna sale"),
    ("vistas.rs · OOS2018, `from.view` expone", "QUE EXPONE"),
    ("vistas.rs · OOS2018, `version.field`", "QUE EXPONE"),
    ("vistas.rs · OOS2011 y OOS2022, la entidad", "QUE EXPONE"),
]
for q, p in LECTORES:
    print("   %-46s %s" % (q, p))
print()
parrafo("Son dos funciones y hoy hay una. Eso es lo que la medida de `groupBy` "
        "anuncio como «el precio de tocar `fields`» y yo salde LEYENDO los "
        "lectores en vez de corriendo uno.")
print()
print("   Y LA SEGUNDA MITAD, que la primera estaba tapando:")
print()
print("   %-42s %s" % ("la propiedad `masa`, etiquetada `high`", "que dice"))
print("   " + "-" * 74)
print("   %-42s %s" % ("sale de `salary`, columna", "OOS4002 · no cabe en el conducto"))
print("   %-42s %s" % ("sale de `sum(salary)`, agregado", "OOS2022 · ni llega a preguntarse"))
print()
parrafo("El control es lo que da valor a la fila de arriba: la misma etiqueta, "
        "la misma copia y el mismo conducto, y por una columna SI se niega. Asi "
        "que `OOS2022` esta funcionando hoy de escudo por accidente, y arreglar "
        "«que expone» sin arreglar «de donde sale» abriria la fuga: "
        "`raiz.columnas` tampoco lleva la columna de un agregado, asi que la "
        "etiqueta de `salary` no subiria hasta `masa`.")
print()
parrafo("LAS DOS COSAS VAN JUNTAS O NINGUNA. Es la misma forma que `ore package "
        "move`: tres cosas a la vez o el arbol queda peor que antes.")

# -- B -------------------------------------------------------------------------
print()
print("B - EL `having`: LA MAQUINA, MEDIDA")
print()
parrafo("Un `Filtra` encima de un `Agrupa`, construido a mano y pasado por las "
        "tres piezas:")
print()
print("   %-14s %s" % ("esquema", "n: Integer · pais: String"))
print("   %-14s %s" % ("linaje de `n`", "Indirecto(Agrupacion) + Indirecto(Filtro)"))
print("   %-14s %s" % ("linaje de `pais`", "Directo(Identidad) + Indirecto(Filtro)"))
print("   %-14s %s" % ("motivos", "[] — se mantiene incrementalmente"))
print()
parrafo("Funciona entero, y el linaje hace lo correcto sin que nadie se lo "
        "pida: filtrar por el conteo deja una arista INDIRECTA sobre `pais`, "
        "porque QUE PAISES SALEN depende de cuantos empleados tienen. Es el "
        "mismo flujo implicito que el `where`, un piso mas arriba.")
print()
parrafo("Asi que `having` es trabajo de VOCABULARIO, exactamente como lo era "
        "`groupBy`: una clave, su forma, su clasificacion en `invertible`, y "
        "compilarlo por encima del `Agrupa` en vez de por debajo.")

# -- C -------------------------------------------------------------------------
print()
print("C - PARA QUE HACE FALTA, Y NO ES PARA LOS INFORMES")
print()
parrafo("«Paises con mas de 100 empleados» es el ejemplo obvio y es el "
        "argumento debil: eso lo filtra el consumidor. El argumento fuerte "
        "estaba escrito en este arbol desde antes de que hubiera agregados:")
print()
print("   `OOS4007` · «aggregate sin minGroupSize o por debajo del umbral»")
print("   `OOS5016` · «minGroupSize de aggregate reducido» — cambio rompedor")
print("   Cedar     · @obligation(\"aggregate:minGroupSize=8\")")
print()
parrafo("El desclasificador `aggregate` no dice «agregar quita la etiqueta». "
        "Dice «agregar quita la etiqueta SI el grupo es bastante grande», y ese "
        "umbral es, palabra por palabra, un `having count() >= N`.")
print()
parrafo("Hoy ese umbral vive en una POLITICA y lo comprueba alguien en "
        "ejecucion. Con `having`, la vista lo DECLARA y entra en el plan: la "
        "diferencia entre una promesa y una consulta. Y mientras no este, "
        "`groupBy` tiene un agujero con nombre propio:")
print()
parrafo("· `groupBy: [pais, enfermedad]` con `count()`. Un grupo de UNO no es "
        "una estadistica, es una reidentificacion, y nada en el documento lo "
        "impide. La k-anonimidad la inventaron para esto y el vocabulario del "
        "desclasificador ya la nombra — lo que falta es donde escribirla.",
        "     ")
print()
parrafo("Y por eso `having` sube de prioridad al construir `groupBy`: no es la "
        "siguiente comodidad, es la mitad que le falta a la que se acaba de "
        "construir.")

# -- D -------------------------------------------------------------------------
print()
print("D - EL AGREGADO GLOBAL")
print()
parrafo("`OOS2033` lo niega, y el motivo medido es que su linaje sale VACIO. "
        "Conviene ser exacto sobre por que: no es que agregar sin agrupar sea "
        "raro —SQL lo admite y todo cuadro de mando lo usa—, es que este "
        "producto gobierna POR LINAJE, y una columna de la que no sale nada no "
        "tiene de donde heredar una etiqueta.")
print()
print("   Y la razon esta en una estructura, no en una regla:")
print()
print("     lineage::Raiz { datasource, objeto, campo }")
print()
parrafo("`campo` es obligatorio. NO EXISTE la nocion de «la tabla entera» como "
        "raiz, asi que un `count()` global no tiene ninguna raiz que nombrar. "
        "Devolverlo no es levantar `OOS2033`: es dar a la tabla el estatuto de "
        "raiz, y entonces decidir que etiqueta lleva la CARDINALIDAD de un "
        "objeto — que es una pregunta que este proyecto no ha contestado.")
print()
print("   %-24s %-16s %s" % ("", "cuesta", "para que"))
print("   " + "-" * 72)
print("   %-24s %-16s %s" % ("`having`", "vocabulario", "cerrar el umbral de k-anonimidad"))
print("   %-24s %-16s %s" % ("agregado global", "modelo de linaje", "un numero en un panel"))
print()
parrafo("Con eso a la vista el orden se elige solo, y el agregado global puede "
        "esperar: quien quiera el total puede agrupar por algo y sumar, y quien "
        "no tenga por que agrupar probablemente esta pidiendo justo la cifra "
        "que nadie clasifico.")

# -- E -------------------------------------------------------------------------
print()
print("E - Y UNA COSA QUE SALIO SIN BUSCARLA")
print()
parrafo("Comprobando por donde se etiqueta una columna aparecio esto:")
print()
print("     columns:")
print("       salary: { chorizo: 3 }")
print()
print("     $ ore validate  ->  ok · sin errores")
print()
parrafo("El esquema publicado de `Table` dice `additionalProperties: false` "
        "para una columna —solo `physicalType` y `description`— y el nucleo no "
        "lo comprueba. Es `OOS1005` un piso mas abajo, y la deriva contra el "
        "esquema la deberia atrapar la suite de conformidad: no la atrapa "
        "porque ningun caso lo intenta.")
print()
parrafo("No es de esta iteracion y no se arregla aqui. Se deja escrito porque "
        "callarlo seria dejarlo en la cabeza de nadie.")
