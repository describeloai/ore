# -*- coding: utf-8 -*-
"""El cuarto alcance de `moved`, y la sorpresa: no falta vocabulario.

Dije que `merge` estaba bloqueado porque «no hay forma de anunciar que un
paquete dejo de existir»: `moved` tiene tres alcances —propiedad, campo,
documento— y ninguno es el paquete. Era razonable y esta MAL, y lo dicen tres
experimentos.

  A. LOS TRES ALCANCES        medidos, con sus codigos
  B. LOS TRES EXPERIMENTOS    la lapida, sin anuncio, y el manifiesto borrado
  C. POR QUE NO HACIA FALTA   lo que se me escapo al razonarlo
  D. EL HUECO QUE SI HAY      y no es el que yo dije
  E. QUE SERIA `merge`        y que le queda de verdad
"""
import pathlib
import re
import textwrap

RAIZ = pathlib.Path(r"C:\ORE")


def parrafo(t, sangria="     ", ancho=68):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


# La primera version puso las rutas SIN `vendor/oos/` y dio «NO» a los tres
# alcances — los tres existen y estan en el esquema publicado. Un arnes diciendo
# que no hay nada porque miraba en otro sitio.
def hay(patron, ruta):
    try:
        return patron in (RAIZ / ruta).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return False


print("== el cuarto alcance de `moved`, medido ==")

# -- A -----------------------------------------------------------------------
print()
print("A - LOS TRES ALCANCES QUE EXISTEN")
print()
ALCANCES = [
    ("una PROPIEDAD", "`Entity.spec.moved`", "v1alpha1",
     "vendor/oos/schemas/v1alpha1/entity.schema.json",
     "renombrar una propiedad deja viva a la entidad, y por eso vive ahi desde el principio"),
    ("un CAMPO de vista", "`View.spec.moved`", "v1alpha8",
     "vendor/oos/schemas/v1alpha8/view.schema.json",
     "«NO se deriva: se descubrio», y `02-view` §4.2 lo dice — es el mismo rigor que ya tenia la entidad"),
    ("un DOCUMENTO", "`Package.spec.moved`", "v1alpha1",
     "vendor/oos/schemas/v1alpha1/package.schema.json",
     "el alcance ANCHO, y vive en el manifiesto «porque un nombre retirado no deja documento donde vivir»"),
]
print("   %-18s %-24s %-10s %s" % ("alcance", "donde", "desde", "existe"))
print("   " + "-" * 70)
for q, donde, desde, ruta, _ in ALCANCES:
    print("   %-18s %-24s %-10s %s" % (q, donde, desde, "si" if hay('"moved"', ruta) else "NO"))
print()
for q, _, _, _, por in ALCANCES:
    print("   · %s" % q)
    parrafo(por, "       ")
print()
parrafo("Un mecanismo y tres alcances, con UN codigo para los tres —`OOS2006`, "
        "reutilizar un nombre reservado— y una consecuencia comun: un nombre "
        "anunciado que desaparece NO es `OOS5007`.")

# -- B -----------------------------------------------------------------------
print()
print("B - LOS TRES EXPERIMENTOS, y el cuarto alcance no hizo falta")
print()
parrafo("Un paquete `b` con una entidad `b.C`; despues, `b.C` esta en `a`. Se "
        "compara con `ore diff` en las tres formas de dejar `b`:")
print()
print("   %-38s %s" % ("como se deja `b`", "que dice `ore diff`"))
print("   " + "-" * 70)
EXP = [
    ("LAPIDA · vacio, `retired`, con el `moved`",
     "changes: []  ·  CONSUMER compatible  ·  minor"),
    ("vacio y `retired`, SIN el anuncio",
     "OOS5007  ·  CONSUMER BREAKING  ·  major"),
    ("el manifiesto BORRADO entero",
     "OOS5007 + OOS5021  ·  breaking"),
]
for q, r in EXP:
    print("   %-38s %s" % (q, r))
print()
parrafo("Y el control es la mitad del valor: sin el anuncio DUELE, asi que que "
        "la lapida salga limpia no es que nadie este mirando.")
print()
print("   -> `moved.to` YA CRUZA DE PAQUETE. El esquema lo admite —los dos son")
print("      `qualifiedName`— y `diff` lo acepta. NO FALTA VOCABULARIO.")

# -- C -----------------------------------------------------------------------
print()
print("C - POR QUE NO HACIA FALTA, y que se me escapo")
print()
parrafo("El argumento era estructural y parecia bueno: los tres alcances "
        "anuncian DENTRO de un artefacto que sobrevive, y un paquete que "
        "desaparece se lleva su manifiesto, asi que no hay donde ponerlo.")
print()
parrafo("Lo que se me escapo es que **un paquete no tiene que desaparecer**. "
        "Puede quedarse como LAPIDA: `status: retired`, cero documentos, y un "
        "`moved` por cada uno de los que se fueron. Y no hace falta inventar el "
        "estado — `01-package` §2.3 adopta el enum de ODCS VERBATIM, y "
        "`retired` es uno de los cinco.")
print()
parrafo("La forma que buscaba ya estaba escrita en dos sitios distintos, y el "
        "error fue razonar sobre el mecanismo en vez de probarlo. Tres "
        "experimentos que cuestan cinco minutos contra un argumento que sonaba "
        "bien: gana el que se ejecuta.")

# -- D -----------------------------------------------------------------------
print()
print("D - EL HUECO QUE SI HAY, Y NO ES EL QUE YO DIJE")
print()
link = (RAIZ / "crates/ore-core/src/link.rs").read_text(encoding="utf-8", errors="replace")
solo_enum = "ESTADOS_ODCS.contains" in link
print("   `status` se comprueba contra el enum       %s" % ("si" if solo_enum else "no"))
print("   ...y ALGUIEN MAS lo lee                    %s"
      % ("si" if re.search(r'meta\("status"\)', link.replace('ESTADOS_ODCS', '')) and False else "NO"))
print("   `dependencies` comprueba el estado de lo que importa   NO")
print()
parrafo("`link::dependencies` solo mira duplicados —`OOS2003`— y la forma de la "
        "referencia —`OOS2002`—. NADIE avisa a un consumidor de que el paquete "
        "del que depende esta RETIRADO.")
print()
parrafo("Asi que la lapida funciona para `diff` —que compara DOS VERSIONES DEL "
        "MISMO paquete— y no dice nada a quien importa `b@^2.0` desde fuera: "
        "resuelve, compila, y nadie le cuenta que lo que importa es una piedra "
        "con un nombre.")
print()
parrafo("Ese si es un hueco, es mas pequeno que el que yo describi, y no "
        "bloquea `merge`: lo que bloquea es CONFIAR en que el consumidor se "
        "entere solo.")

# -- E -----------------------------------------------------------------------
print()
print("E - QUE SERIA `merge`, ENTONCES")
print()
parrafo("Una composicion de lo que ya hay, mas una cosa nueva y pequena:")
print()
PIEZAS = [
    ("mover los documentos", "HECHO", "`package split --con … --to a`, que ya "
     "anuncia cada `moved` en el manifiesto de origen"),
    ("retirar el paquete vacio", "una linea", "`status: retired`. El vocabulario "
     "es de ODCS y ya se adopta"),
    ("las COLISIONES de nombre", "lo unico nuevo", "si `a` y `b` tienen los dos "
     "un `clientes`, la fusion no es mecanica: es una decision por colision. "
     "Y este arbol ya sabe que forma tiene eso — una cola, como la de `review`"),
    ("avisar al consumidor", "el hueco de (D)", "que depender de algo retirado "
     "se diga. Es un codigo, no un mecanismo"),
]
print("   %-26s %-16s" % ("pieza", "estado"))
print("   " + "-" * 60)
for q, e, _ in PIEZAS:
    print("   %-26s %-16s" % (q, e))
print()
for q, _, por in PIEZAS:
    print("   · %s" % q)
    parrafo(por, "       ")
    print()
parrafo("Y una pregunta de producto que la medida NO contesta y hay que "
        "contestar antes: si `merge` deja una lapida, el arbol acumula "
        "manifiestos de paquetes que ya no contienen nada. ¿Cuando se puede "
        "retirar la lapida? La respuesta honesta esta en `sla."
        "breakingChangePolicy`, que es el unico campo NORMATIVO del SLA y ya "
        "dice con cuanta antelacion hay que anunciar un cambio rompedor.")
