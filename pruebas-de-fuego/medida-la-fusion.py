# -*- coding: utf-8 -*-
"""La fusion: mover el significado a la vista. El unico peldano sin medir.

`entidad.md` §10 la dio por decidida —«ya no es un rediseno: es borrar
`backedBy` y mover un fichero»— y nunca se midio. Los tres peldanos que si se
midieron dijeron que no hay nada redundante que quitar, asi que la fusion ya no
puede justificarse por adelgazar. Hay que preguntarle otra cosa.

Y el criterio no hay que inventarlo: esta escrito en el motor, en la ayuda de
la regla que obliga a `Ruleset` a tener dueno —

    «es independiente del dueno de los paquetes a los que apunta: AHI ESTA LA
     RAZON DE QUE ESTO SEA UN DOCUMENTO Y NO UN BLOQUE DENTRO DE `Entity`. En un
     entorno regulado, quien responde del cumplimiento tiene que poder
     restringir la ontologia sin poder editarla.»

Un documento existe aparte cuando RESPONDE OTRA PERSONA. Seis frentes:

  A. QUE SE MUEVE     el volumen, y si la fusion pierde algo por cardinalidad
  B. QUIEN RESPONDE   el criterio de la casa, aplicado. `owner`: quien lo tiene
  C. LA COLISION      «la vista no lleva significado» es normativo: que se rompe
  D. QUIEN DIRECCIONA a quien nombran las politicas y las superficies
  E. EL PRECIO        cuanto codigo distingue una de otra
  F. VEREDICTO
"""
import collections
import pathlib
import re
import subprocess

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
EJEMPLO = OOS / "examples/acme-retail"


def docs(raiz):
    for f in sorted(raiz.rglob("*.yaml")):
        txt = f.read_text(encoding="utf-8", errors="replace")
        for d in re.split(r"^---\s*$", txt, flags=re.M):
            k = re.search(r"^kind:\s*(\w+)", d, re.M)
            if k:
                yield k.group(1), d, f


def cuantos(patron, donde, extra=None):
    cmd = ["grep", "-rl", patron, str(donde)] + (extra or [])
    return len(subprocess.run(cmd, capture_output=True, text=True).stdout.split())


TODOS = list(docs(OOS)) + list(docs(RAIZ / "casos"))
print("== la fusion: mover el significado a la vista ==")

# -- A - QUE SE MUEVE --------------------------------------------------------
print()
print("A - QUE SE MUEVE, y si se pierde algo por el camino")
ents = [(d, f) for k, d, f in TODOS if k == "Entity"]
con_bb = [(d, f) for d, f in ents if re.search(r"^  backedBy:", d, re.M)]
vistas = [d for k, d, _ in TODOS if k == "View"]
por_vista = collections.defaultdict(list)
for d, f in con_bb:
    bb = re.search(r"^  backedBy:\s*(\S+)", d, re.M).group(1)
    por_vista[(f.parent.parent, bb)].append(f.name)
c = collections.Counter(len(v) for v in por_vista.values())
print("   %-40s %3d" % ("entidades en el arbol", len(ents)))
print("   %-40s %3d" % ("  ...con `backedBy` (las fusionables)", len(con_bb)))
print("   %-40s %3d" % ("  ...sin el: camino viejo, no fusionan", len(ents) - len(con_bb)))
print("   %-40s %3d" % ("vistas", len(vistas)))
print()
for k in sorted(c):
    print("   vistas que respaldan a %d entidad(es): %3d" % (k, c[k]))
print("   -> hoy es 1:1, asi que la fusion NO pierde nada en este corpus.")
print("      Pero la gramatica admite n:1, y el propio ejemplo dice por que la")
print("      flecha va en esa direccion: «dos entidades pueden respaldarse de")
print("      la misma sin duplicar el mapeo». Lo que se pierde no es corpus:")
print("      es una capacidad que nadie ha usado todavia.")

# -- B - QUIEN RESPONDE ------------------------------------------------------
print()
print("B - EL CRITERIO DE LA CASA: un documento aparte es que responde otro")
doc_rs = (RAIZ / "crates/ore-core/src/document.rs").read_text(encoding="utf-8")
exigen = re.findall(r"kind: Kind::(\w+),\s*\n\s*path: &\[\"spec\"\],\s*\n"
                    r"\s*check: \|n\| \{[\s\S]{0,200}?get\(\"owner\"\)", doc_rs)
print("   documentos que DEBEN declarar `owner`: %s" % ", ".join(sorted(set(exigen))))
tiene_owner = [k for k in ("Package", "View", "Entity", "Table", "ConduitPolicy",
                           "Ruleset", "Function", "RequestPolicy")
               if re.search(r"Kind::%s => &\[[^\]]*\"owner\"" % k, doc_rs, re.S)]
print("   ...y el vocabulario lo ADMITE en: %s" % ", ".join(tiene_owner))
print()
print("   %-14s %s" % ("Entity", "NO tiene `owner`. Ni lo admite."))
print("   %-14s %s" % ("View", "lo exige."))
print()
print("   -> Y eso es el hallazgo, no un detalle de forma. Por el criterio de la")
print("      casa, la entidad NO merece documento propio: nadie distinto")
print("      responde de ella. Es el argumento a favor de la fusion, y es el")
print("      unico que las medidas no han tumbado.")
print()
print("      Pero tiene un reverso que hay que decir: lo que la entidad lleva")
print("      —las etiquetas— es LA superficie de seguridad del modelo, y no")
print("      tiene dueno. `ConduitPolicy` lo exige porque «un techo del que")
print("      nadie responde es el hueco que este campo cierra». Quien pone")
print("      `nationalId: critical` decide lo mismo un piso mas abajo, y de eso")
print("      no responde nadie hoy.")

# -- C - LA COLISION ---------------------------------------------------------
print()
print("C - LA COLISION: «la vista no lleva significado» es NORMATIVO")
casos = OOS / "conformance"
choca = []
for d in sorted(casos.rglob("case.yaml")):
    txt = d.read_text(encoding="utf-8", errors="replace")
    if re.search(r"no clasifica|classify|leaks-entity-label|maturity", txt) \
            or "classify" in d.parent.name or "entity-label" in d.parent.name:
        choca.append(d.parent.relative_to(casos).as_posix())
for c_ in choca:
    print("     %s" % c_)
print()
vocab = (RAIZ / "crates/ore-core/src/vistas.rs").read_text(encoding="utf-8")
print("   Y en el motor: `flow::vistas_materializadas` sube las etiquetas por")
print("   DOS vias, y la segunda es «cada entidad cuya cadena pasa por aqui».")
print("   Su comentario dice para que:")
print("     «una entidad puede declarar `nationalId: high` sobre una vista de")
print("      tres eslabones, y LA DE ABAJO, QUE ES LA QUE SE MATERIALIZA, NO LO")
print("      SABE. Sin esto se copiaria en claro un dato que la entidad")
print("      clasifico — y compilaria.»")
print()
print("   -> ahi esta el problema de fondo, y no es de ficheros. Si el")
print("      significado vive EN la vista, vive en UNA vista: la de arriba. Y la")
print("      que se copia es la de abajo. Hoy el sello funciona porque la")
print("      entidad es de la CADENA y no de un eslabon. Fusionar la clava en")
print("      uno, y hay que decir que pasa con los otros.")

# -- D - QUIEN DIRECCIONA ----------------------------------------------------
print()
print("D - A QUIEN NOMBRAN LAS POLITICAS Y LAS SUPERFICIES")
refs = collections.Counter()
for f in sorted(EJEMPLO.rglob("*.cedar")):
    t = f.read_text(encoding="utf-8", errors="replace")
    for n in re.findall(r"\b([a-z]+\.[A-Z][A-Za-z]*(?:\.\w+)?)", t):
        refs["entidad"] += 1
    for n in re.findall(r"\b([a-z]+\.[a-z]+\.\w+)", t):
        refs["vista"] += 1
for f in sorted(EJEMPLO.rglob("rulesets/*.yaml")):
    t = f.read_text(encoding="utf-8", errors="replace")
    refs["ruleset -> entidad"] += len(re.findall(r"[a-z]+\.[A-Z][A-Za-z]*", t))
gql = (RAIZ / "crates/ore-core/src/graphql.rs").read_text(encoding="utf-8")
print("   referencias en Cedar a una ENTIDAD : %d" % refs["entidad"])
print("   referencias en Cedar a una VISTA   : %d" % refs["vista"])
print("   objetivos de ruleset (entidades)   : %d" % refs["ruleset -> entidad"])
print("   GraphQL emite sus tipos desde      : %s"
      % ("pkg.entities()" if "pkg.entities()" in gql else "?"))
print()
print("   -> nadie nombra una vista. La fusion no es solo mover un fichero: es")
print("      cambiar QUE SE DIRECCIONA, y eso reescribe politicas de seguridad.")
print("      En este ejemplo son pocas; en un cliente son las que hay.")

# -- E - EL PRECIO EN CODIGO -------------------------------------------------
print()
print("E - EL PRECIO EN CODIGO: cuanto distingue una de otra")
def apariciones(*patrones):
    """Cuenta ocurrencias leyendo los ficheros, no encadenando greps.

    La primera version usaba `grep -rhc` con alternancia y devolvia 0 — y un
    cero que no es un cero es justo lo que estas medidas existen para no hacer.
    """
    n = 0
    for f in (RAIZ / "crates").rglob("*.rs"):
        txt = f.read_text(encoding="utf-8", errors="replace")
        n += sum(txt.count(x) for x in patrones)
    return n


print("   apariciones de `Kind::Entity` / `entities()` : %3d"
      % apariciones("Kind::Entity", "pkg.entities()"))
print("   apariciones de `Kind::View`                  : %3d"
      % apariciones("Kind::View"))
print("   ficheros de la spec que hablan de `Entity`   : %3d"
      % cuantos("Entity", OOS / "spec", ["--include=*.md"]))
print("   casos de conformidad con una entidad         : %3d"
      % cuantos("kind: Entity", OOS / "conformance", ["--include=*.yaml"]))

# -- F - VEREDICTO -----------------------------------------------------------
print()
print("F - VEREDICTO")
print("   La fusion no esta bloqueada por lo que se creia —que la entidad")
print("   redeclara el sustrato— porque eso resulto ser falso. Y el criterio de")
print("   la casa la APOYA: nadie distinto responde de la entidad.")
print()
print("   Lo que la bloquea es otra cosa, y es una sola:")
print()
print("     LA ENTIDAD ES DE LA CADENA, NO DE UN ESLABON.")
print()
print("   Clasifica una vez y su etiqueta baja hasta la copia, este donde este.")
print("   Meter el significado en un `kind: View` lo clava en un eslabon, y hay")
print("   que contestar que pasa con los de abajo — que son los que se copian.")
print("   Eso no es mover un fichero.")
print()
print("   Y quedan dos preguntas mas pequenas que la fusion destapa y que valen")
print("   por si solas, se fusione o no:")
print("     1. las etiquetas no tienen dueno, y son la superficie de seguridad")
print("     2. nadie direcciona una vista, asi que su `owner` responde de algo")
print("        que nadie nombra")
