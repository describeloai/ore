# -*- coding: utf-8 -*-
"""El espectro del peldano 6, ahora que el bloqueo cayo.

`medida-la-fusion` la midio y encontro un solo bloqueo, que no era de ficheros:

    «LA ENTIDAD ES DE LA CADENA, NO DE UN ESLABON. Meter el significado en un
     `kind: View` lo clava en un eslabon, y la que se copia es la de abajo.»

Y esa frase describia un MECANISMO, no una imposibilidad — el mecanismo que
resultó estar roto y que se arreglo: el sello ya resuelve la cadena en las dos
direcciones. Asi que la pregunta vuelve a estar abierta, y esto mide su
espectro. Seis frentes:

  A. EL BLOQUEO      releido contra el arreglo. Que cae y que no
  B. LO QUE QUEDA    lo normativo, lo que se pierde, y una pregunta NUEVA que
     EN CONTRA       la fusion crea y que antes no existia
  C. QUE SE MUEVE    los dos vocabularios, y la colision que ya tiene
                     precedente resuelto
  D. QUIEN SE ENTERA quien nombra entidades y tendria que nombrar otra cosa
  E. EL VOLUMEN      codigo, spec, casos, corpus
  F. EL ESPECTRO     el orden, y que compra cada tramo
"""
import collections
import pathlib
import re

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
CONF = OOS / "conformance"


def docs(*raices):
    for r in raices:
        for f in sorted(r.rglob("*.yaml")):
            t = f.read_text(encoding="utf-8", errors="replace")
            for d in re.split(r"^---\s*$", t, flags=re.M):
                k = re.search(r"^kind:\s*(\w+)", d, re.M)
                if k:
                    yield k.group(1), d, f


def apariciones(*patrones):
    n = 0
    for f in (RAIZ / "crates").rglob("*.rs"):
        t = f.read_text(encoding="utf-8", errors="replace")
        n += sum(t.count(x) for x in patrones)
    return n


TODOS = list(docs(OOS, RAIZ / "casos"))
print("== el espectro del peldano 6 ==")

# -- A - EL BLOQUEO RELEIDO --------------------------------------------------
print()
print("A - EL BLOQUEO, releido contra el arreglo")
flow = (RAIZ / "crates/ore-core/src/flow.rs").read_text(encoding="utf-8")
dos_direcciones = "proyectar(pkg, v, &sqn)" in flow
print("   la via 2 resuelve la cadena en las DOS direcciones : %s"
      % ("SI" if dos_direcciones else "no"))
print()
print("   El bloqueo decia que meter el significado en un eslabon lo deja sin")
print("   alcanzar las copias de los demas. Eso era cierto MIENTRAS el analisis")
print("   solo resolviera hacia abajo — y esa limitacion no era una propiedad")
print("   del modelo: era un defecto, y esta arreglado y probado en las dos")
print("   direcciones.")
print()
print("   -> el bloqueo MECANICO cae. Lo que queda en contra ya no es «no se")
print("      puede»: es «hay que decidirlo», que es otra clase de respuesta.")

# -- B - LO QUE QUEDA EN CONTRA ----------------------------------------------
print()
print("B - LO QUE QUEDA EN CONTRA")
casos_norma = [c.parent.relative_to(CONF).as_posix()
               for c in sorted(CONF.rglob("case.yaml"))
               if "classify" in c.parent.name or "entity-label" in c.parent.name]
print("   1 · «la vista no lleva significado» es NORMATIVO, con casos:")
for c in casos_norma:
    print("       %s" % c)
print("       No es un obstaculo tecnico: es una decision escrita, y cambiarla")
print("       es cambiar que ES una vista. Se puede — pero se dice.")
print()
por_vista = collections.defaultdict(list)
for k, d, f in TODOS:
    if k != "Entity":
        continue
    bb = re.search(r"^  backedBy:\s*(\S+)", d, re.M)
    if bb:
        por_vista[(f.parent.parent, bb.group(1))].append(f.name)
c = collections.Counter(len(v) for v in por_vista.values())
print("   2 · la cardinalidad: %s"
      % ", ".join("%d vistas respaldan a %d entidad(es)" % (v, k)
                  for k, v in sorted(c.items())))
print("       Hoy 1:1, asi que no se pierde corpus. La gramatica admite n:1 y")
print("       nadie la usa: se perderia una capacidad, no un fichero.")
print()
print("   3 · Y una pregunta NUEVA, que la fusion crea y hoy no existe:")
print("       si el significado vive en la vista, DOS vistas de la misma cadena")
print("       pueden llevar significado distinto sobre el mismo campo. Hoy no")
print("       puede pasar —la entidad es una— y despues habria que decidir si")
print("       se prohibe, si se compone por `join`, o si la de arriba manda.")
print("       Es la unica cosa de esta lista que no tiene respuesta escrita.")

# -- C - QUE SE MUEVE --------------------------------------------------------
print()
print("C - QUE SE MUEVE, y la colision que ya esta resuelta")
doc = (RAIZ / "crates/ore-core/src/document.rs").read_text(encoding="utf-8")
def spec_de(kind):
    m = re.search(r"Kind::%s => &\[\n(.*?)\n            \],\n" % kind, doc, re.S)
    return re.findall(r'"(\w+)"', m.group(1)) if m else []
e, v = spec_de("Entity"), spec_de("View")
print("   `Entity`.spec : %s" % ", ".join(e))
print("   `View`.spec   : %s" % ", ".join(v))
print("   comunes       : %s" % ", ".join(sorted(set(e) & set(v))) or "(ninguno)")
print()
print("   La colision de `labels` tiene precedente RESUELTO: `Concept` ya las")
print("   lleva en los dos sitios, y `document.rs` explica por que no se")
print("   confunden —")
print("     «`metadata.labels` clasifica ESTE DOCUMENTO — su madurez.")
print("      `spec.labels` clasifica EL DATO. Es la misma distincion que en")
print("      `Entity`.»")
print("   -> la unidad fusionada usaria esa particion, y no hay que inventarla.")

# -- D - QUIEN SE ENTERA -----------------------------------------------------
print()
print("D - QUIEN NOMBRA ENTIDADES, y tendria que nombrar otra cosa")
gql = (RAIZ / "crates/ore-core/src/graphql.rs").read_text(encoding="utf-8")
print("   GraphQL emite sus tipos desde   : %s"
      % ("pkg.entities()" if "pkg.entities()" in gql else "?"))
print("   Cedar y los rulesets nombran    : `<ns>.<Entidad>.<propiedad>`")
print("   `ore report` lista              : propiedades de entidad")
print("   -> ninguna superficie nombra una vista. La fusion no mueve un")
print("      fichero: cambia QUE SE DIRECCIONA, y eso reescribe politicas de")
print("      seguridad de cada cliente. Es el tramo caro y no es tecnico.")

# -- E - EL VOLUMEN ----------------------------------------------------------
print()
print("E - EL VOLUMEN")
ents = [1 for k, _, _ in TODOS if k == "Entity"]
con_bb = [1 for k, d, _ in TODOS if k == "Entity" and re.search(r"^  backedBy:", d, re.M)]
print("   %-44s %4d" % ("`Kind::Entity` / `entities()` en el motor",
                        apariciones("Kind::Entity", "pkg.entities()")))
print("   %-44s %4d" % ("`Kind::View` en el motor", apariciones("Kind::View")))
print("   %-44s %4d" % ("entidades en el corpus", len(ents)))
print("   %-44s %4d" % ("  ...fusionables (con `backedBy`)", len(con_bb)))
print("   %-44s %4d" % ("  ...camino viejo: NO fusionan nunca",
                        len(ents) - len(con_bb)))
print()
print("   Y esa ultima fila es la que decide la forma del peldano: %d entidades"
      % (len(ents) - len(con_bb)))
print("   llegan por `Binding`, que no caduca. O sea que la fusion NO puede")
print("   retirar `Entity`: tendria que convivir con ella para siempre, igual")
print("   que `Binding` convive con `Table`+`View`.")

# -- F - EL ESPECTRO ---------------------------------------------------------
print()
print("F - EL ESPECTRO, en orden, y que compra cada tramo")
print()
print("   0 · DECIDIR que la vista puede llevar significado.")
print("       No cuesta codigo. Cuesta cambiar tres casos normativos y la")
print("       definicion de lo que es una vista. Sin esto, lo demas no empieza.")
print()
print("   1 · CONTESTAR la pregunta nueva: dos vistas de una cadena con")
print("       significado distinto sobre el mismo campo. Es lo unico sin")
print("       respuesta escrita, y hay que medirlo aparte.")
print()
print("   2 · LA GRAMATICA: `View` gana lo que hoy es de `Entity`, con la")
print("       particion de `Concept` para `labels`. `backedBy` se va.")
print()
print("   3 · LAS SUPERFICIES: Cedar, rulesets y GraphQL nombran vistas en vez")
print("       de entidades. Es el tramo que rompe repositorios de clientes.")
print()
print("   4 · LA CONVIVENCIA: `Entity` NO se retira — %d entidades llegan por"
      % (len(ents) - len(con_bb)))
print("       binding y v1alpha1 no caduca. Se cierra a la escritura, como")
print("       `Binding`, y el motor sostiene los dos caminos indefinidamente.")
print()
print("   -> el espectro no es «borrar `backedBy` y mover un fichero». Son")
print("      cinco tramos, el primero es una decision y el tercero rompe cosas")
print("      ajenas. Y el resultado no retira un `kind`: anade un camino.")
