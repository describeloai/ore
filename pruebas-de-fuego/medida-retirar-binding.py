# -*- coding: utf-8 -*-
"""NOTA: EJECUTADO. `Binding` se retiro de la lectura, y con el todo lo que
colgaba: 23 sitios del motor a 5 —el nombre, que se conserva para el
diagnostico—, 35 casos, dos codigos y dos documentos de spec. Lo de abajo
es el terreno tal como estaba antes, que es lo que decidio hacerlo.

Si nada de esto es de un cliente, ¿que deja de haber que mantener?

`Binding` se conserva legible porque v1alpha1 no caduca, y eso cuesta: 23
sitios en el motor, dos codigos con sujeto propio, un documento historico y un
paradigma entero que convive con el nuevo. La justificacion era la promesa
—un paquete firmado hace dos anos sigue compilando— y esa promesa solo vale si
alguien tiene uno.

    CLAIM: todos los repositorios y ontologias del arbol son mocks. No hay
    clientes. Nada es ajeno, todo es material experimental y desechable.

Esto comprueba la premisa y lista las superficies que dejarian de pedir
trabajo. Seis frentes:

  A. LA PREMISA      comprobada contra el arbol, no aceptada
  B. LO QUE SE CAE   sitio por sitio
  C. LO QUE SE       lo que existe SOLO porque hay dos paradigmas, y por
     SIMPLIFICA      tanto deja de tener sentido
  D. LO QUE NO       v1alpha1 no es `Binding`. Conviene no confundirlos
     SE CAE
  E. EL COSTE DE     lo que se pierde al retirarlo, dicho de frente
     RETIRARLO
  F. LA LISTA
"""
import collections
import pathlib
import re

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
CONF = OOS / "conformance"
CRATES = RAIZ / "crates"


def vivo(patron, f):
    """Ocurrencias fuera de comentario."""
    return sum(1 for l in f.read_text(encoding="utf-8", errors="replace").split("\n")
               if not l.strip().startswith("//") and re.search(patron, l))


print("== retirar `Binding`: que deja de pedir trabajo ==")

# -- A - LA PREMISA ----------------------------------------------------------
print()
print("A - LA PREMISA, comprobada")
oob = list(OOS.rglob("*.oob"))
en_casos = [p for p in oob if "conformance" in p.parts]
locks = list(OOS.rglob("ontology.lock"))
registro = any("registry.oos.dev" in p.read_text(encoding="utf-8", errors="replace")
               for p in locks)
print("   %-46s %3d" % ("bundles `.oob` en el arbol", len(oob)))
print("   %-46s %3d" % ("  ...de ellos, fixtures dentro de un caso", len(en_casos)))
print("   %-46s %3d" % ("  ...fuera de un caso: de alguien de verdad",
                        len(oob) - len(en_casos)))
print("   %-46s %s" % ("los `ontology.lock` apuntan a", "registry.oos.dev"
                       if registro else "(nada)"))
print()
print("   `registry.oos.dev` no existe. Asi que no hay un solo artefacto")
print("   firmado ahi fuera cuya verificacion dependa de que el motor siga")
print("   leyendo v1alpha1. La premisa se sostiene: la promesa que justificaba")
print("   el coste no tiene beneficiario todavia.")

# -- B - LO QUE SE CAE -------------------------------------------------------
print()
print("B - LO QUE SE CAE, sitio por sitio")
sitios = []
for f in sorted(CRATES.rglob("*.rs")):
    n = vivo(r"Kind::Binding", f)
    if n:
        sitios.append((f.relative_to(RAIZ).as_posix(), n, "tests" in f.parts))
total = sum(n for _, n, _ in sitios)
print("   MOTOR — `Kind::Binding` fuera de comentario: %d en %d ficheros"
      % (total, len(sitios)))
for r, n, es_test in sorted(sitios, key=lambda x: -x[1]):
    print("     %-46s %2d %s" % (r, n, "(prueba)" if es_test else ""))
print()
FUNCIONES = [
    ("flow::materializaciones", "el sello del eje del binding — entera"),
    ("aristas::fisicas", "la rama de bindings; queda la de vistas"),
    ("aristas::columnas_de_binding", "entera"),
    ("selector.rs", "`OOS2014`: dos bindings del mismo objeto"),
]
print("   FUNCIONES que se quedan sin sujeto:")
for n, q in FUNCIONES:
    print("     %-34s %s" % (n, q))
print()
casos = [c.parent.relative_to(CONF).as_posix() for c in sorted(CONF.rglob("case.yaml"))
         if any(re.search(r"^kind:\s*Binding", p.read_text(encoding="utf-8",
                                                           errors="replace"), re.M)
                for d in ("input", "before", "after") if (c.parent / d).is_dir()
                for p in (c.parent / d).rglob("*.yaml"))]
print("   CONFORMIDAD — casos con un `Binding` dentro: %d" % len(casos))
por_borrador = collections.Counter(c.split("/")[0] if c.startswith("v1alpha") else "v1alpha1"
                                   for c in casos)
for b, n in sorted(por_borrador.items()):
    print("     %-12s %2d" % (b, n))
print()
print("   ESPECIFICACION:")
print("     03-binding.md        historico -> se puede retirar del todo")
print("     05-ejecutor.md       historico, y su sujeto era el ejecutor de")
print("                          bindings: se va con el")

# -- C - LO QUE SE SIMPLIFICA ------------------------------------------------
print()
print("C - LO QUE SE SIMPLIFICA: existe SOLO porque hay dos paradigmas")
aristas = (CRATES / "ore-core/src/aristas.rs").read_text(encoding="utf-8")
print("   `Arista::derivada`                 : %s"
      % ("existe" if "pub derivada" in aristas else "no"))
print("     Lo anadi para que el sello del indice no contara dos veces el")
print("     camino viejo. Con un solo paradigma el campo sobra, y con el la")
print("     mitad del comentario que lo justifica.")
print()
print("   El CENSO DE PARIDAD                : deja de tener objeto")
print("     Todo el patron que faltaba —«que regla vale en los dos y solo se")
print("     prueba en uno»— es una consecuencia de tener dos. Con uno, la")
print("     pregunta no existe.")
print()
gating = len(re.findall(r"V1Alpha8", (CRATES / "ore-core/src/link.rs")
                        .read_text(encoding="utf-8")))
print("   ACOTACIONES POR VERSION            : %d en `link.rs`" % gating)
print("     Cada regla nueva se acota a v1alpha8+ para no cambiar un resultado")
print("     viejo. Sin resultados viejos que preservar, la acotacion es ruido")
print("     — y es la que mas cuesta razonar cada vez.")

# -- D - LO QUE NO SE CAE ----------------------------------------------------
print()
print("D - LO QUE NO SE CAE, y conviene no confundirlo")
kinds_v1 = ["Entity", "Lattice", "ConduitPolicy", "Package", "OntologyConfig"]
print("   v1alpha1 NO es `Binding`. Siguen siendo suyos y siguen vivos:")
print("     %s" % ", ".join(kinds_v1))
print("     y la forma canonica, el digest, la regla de flujo y el versionado.")
print()
sin_sustrato = 0
for c in sorted(CONF.rglob("case.yaml")):
    toca = any(re.search(r"^kind:\s*(Binding|Table|View)",
                         p.read_text(encoding="utf-8", errors="replace"), re.M)
               for d in ("input", "before", "after") if (c.parent / d).is_dir()
               for p in (c.parent / d).rglob("*.yaml"))
    sin_sustrato += not toca
print("   %-46s %3d" % ("casos que no tocan sustrato: se quedan", sin_sustrato))
print("   -> retirar `Binding` no es retirar v1alpha1 ni adelgazar la suite a")
print("      la mitad. Es quitar UN kind y lo que cuelga solo de el.")

# -- E - EL COSTE DE RETIRARLO -----------------------------------------------
print()
print("E - EL COSTE, dicho de frente")
print("   1 · Se pierde la DEMOSTRACION de que el estandar sabe cargar con un")
print("       vocabulario retirado. Hoy es la unica prueba viva de que la")
print("       promesa de versionado no es prosa. Se puede volver a demostrar el")
print("       dia que haya un cliente, pero ese dia sera con datos de alguien.")
print()
print("   2 · Los %d casos con binding se van, y con ellos la unica cobertura de"
      % len(casos))
print("       los codigos cuyo sujeto era el binding. Eso es consistente —sin")
print("       sujeto no hace falta caso— pero el marcador BAJA, y este proyecto")
print("       tiene escrito que el marcador solo sube. Habria que decir por que")
print("       esta vez no.")
print()
print("   3 · `91-versioning` promete que un documento no caduca por haber sido")
print("       escrito antes. Retirar `Binding` de la LECTURA rompe esa frase, no")
print("       solo la practica. Hay que reescribirla o acotarla a «desde que")
print("       haya un consumidor».")

# -- F - LA LISTA ------------------------------------------------------------
print()
print("F - LA LISTA: superficies que dejan de pedir trabajo")
LISTA = [
    ("`Kind::Binding` y sus %d sitios" % total, "motor"),
    ("`flow::materializaciones`", "el sello del eje, entero"),
    ("`aristas::fisicas` rama vieja + `columnas_de_binding`", "motor"),
    ("`selector.rs` · `OOS2014`", "sin sujeto"),
    ("`Arista::derivada`", "solo existia para separar los dos caminos"),
    ("el censo de paridad", "sin objeto"),
    ("las acotaciones a v1alpha8+", "nada viejo que preservar"),
    ("`03-binding.md` y `05-ejecutor.md`", "de historicos a retirados"),
    ("%d casos de conformidad" % len(casos), "y su mantenimiento"),
    ("`IMPLEMENTADAS`: las entradas de codigos solo-viejo", "censo"),
]
for que, donde in LISTA:
    print("   · %-52s %s" % (que, donde))
print()
print("   Y la que no esta en la lista y es la mas grande: **la pregunta**.")
print("   Con un solo paradigma dejan de existir las tres medidas que hemos")
print("   hecho esta semana para averiguar si una regla vale en los dos.")
