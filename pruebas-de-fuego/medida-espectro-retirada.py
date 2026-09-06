# -*- coding: utf-8 -*-
"""El espectro de la retirada de `Binding`, en orden de ejecucion.

`medida-retirar-binding` dijo QUE se cae. Esto dice EN QUE ORDEN, que no es el
mismo: hay dependencias, y hacerlo al reves deja el arbol rojo por motivos que
no son el cambio.

La decision de forma que gobierna todo lo demas se toma aqui y no en el codigo:

    `Kind::Binding` NO desaparece del enum. Se queda como NOMBRE que siempre se
    rechaza, porque el mensaje que hoy da —«en v1alpha8 esto son una `Table` y
    una `View`, y la entidad nombra a la vista con `backedBy`»— es la guia de
    migracion, y borrar el kind la convertiria en «kind desconocido».

    O sea que no se retira el nombre: se retira TODO LO QUE CUELGA DE EL.
"""
import pathlib
import re

RAIZ = pathlib.Path(r"C:\ORE")
CRATES = RAIZ / "crates"
CONF = RAIZ / "vendor/oos/conformance"


def sitios(patron):
    out = {}
    for f in sorted(CRATES.rglob("*.rs")):
        n = sum(1 for l in f.read_text(encoding="utf-8", errors="replace").split("\n")
                if not l.strip().startswith("//") and re.search(patron, l))
        if n:
            out[f.relative_to(RAIZ).as_posix()] = n
    return out


casos = [c.parent for c in sorted(CONF.rglob("case.yaml"))
         if any(re.search(r"^kind:\s*Binding", p.read_text(encoding="utf-8",
                                                           errors="replace"), re.M)
                for d in ("input", "before", "after") if (c.parent / d).is_dir()
                for p in (c.parent / d).rglob("*.yaml"))]

print("== el espectro de la retirada, en orden ==")
print()
print("LA DECISION DE FORMA, primero, porque gobierna el resto")
print("   `Kind::Binding` se queda en el enum como NOMBRE que siempre se")
print("   rechaza. Su mensaje es la guia de migracion; borrar el kind lo")
print("   degradaria a «kind desconocido» y perderiamos lo unico que le")
print("   dice a alguien que hacer con su fichero.")
print("   -> se retira lo que cuelga, no el nombre.")
print()

TRAMOS = [
    ("1 · EL CORPUS",
     "%d casos de conformidad con un `Binding` dentro" % len(casos),
     "va PRIMERO: en cuanto el motor rechace el kind, estos casos fallan por "
     "un motivo que no es el suyo y esconden los fallos de verdad"),
    ("2 · LAS RAMAS DEL MOTOR",
     "%d sitios que ramifican sobre `Kind::Binding`"
     % sum(sitios(r"Kind::Binding").values()),
     "cada uno es una rama de un `match` o un filtro de iteracion. Ninguno "
     "puede quedarse: un `pkg.of(Kind::Binding)` que nunca devuelve nada es "
     "codigo que no se puede probar"),
    ("3 · LOS DOS CODIGOS SIN SUJETO",
     "`OOS2014` y `OOS2015`",
     "el primero es «dos bindings del mismo objeto» y no tiene equivalente; el "
     "segundo YA lo tiene —`OOS2018`, un filtro exigido que no es columna— asi "
     "que retirarlo no deja hueco"),
    ("4 · EL RECHAZO",
     "`Kind::hasta()` pasa a v1alpha1",
     "hasta ahora rechazaba desde v1alpha8; ahora desde siempre. Y el texto de "
     "ayuda pierde su ultima frase, que decia que un documento v1alpha1 sigue "
     "compilando tal cual"),
    ("5 · LA ESPECIFICACION",
     "`03-binding.md`, `05-ejecutor.md`, y `91-versioning`",
     "los dos primeros de historicos a retirados. El tercero es el que duele: "
     "promete que un documento no caduca por haber sido escrito antes, y eso "
     "deja de ser cierto. Hay que reescribirlo, no borrarlo"),
    ("6 · LOS CENSOS",
     "registro de codigos, clasificacion de listas, invertibilidad",
     "este arbol tiene pruebas que fallan cuando el vocabulario cambia sin que "
     "alguien lo diga. Van al final porque son las que confirman que no queda "
     "nada suelto"),
]
for titulo, que, por_que in TRAMOS:
    print("%s" % titulo)
    print("   %s" % que)
    print("   %s" % por_que)
    print()

print("EL ORDEN IMPORTA en un sitio y solo en uno:")
print("   el corpus antes que el motor. Al reves, 35 casos fallan a la vez y")
print("   cada fallo del motor queda enterrado entre ellos.")
print()
print("LO QUE NO ENTRA EN ESTA ITERACION, y se dice para no confundirlo:")
print("   `Arista::derivada`, las acotaciones a v1alpha8+ y el censo de")
print("   paridad. Esos no SE CAEN: se quedan sin motivo, que es distinto y se")
print("   mira despues, con el arbol ya en un solo paradigma.")
