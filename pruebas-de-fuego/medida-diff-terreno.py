# -*- coding: utf-8 -*-
"""El paso 3: que costaria que `ore diff` viera el sustrato.

`medida-diff-sustrato.py` midio el agujero: 13 de 19 mutaciones de una variable
validan en verde y `diff` no dice nada, incluidas invertir el `where` de una
vista raiz —cambia QUE FILAS son la respuesta— y dejar de materializar.

Esto mide el TERRENO DE LA REPARACION, no el agujero. Seis frentes, y el
primero es el que decide el diseno antes de escribir una linea:

  A. LA FRONTERA DE CRATES  donde vive `diff` y que puede usar desde ahi
  B. EL VOCABULARIO         cada clave de `View` y `Table`, cuanto se usa, y
                            en que eje caeria su cambio
  C. LOS CODIGOS QUE HAY    `OOS5019` y `OOS5020` existen y se calculan sobre
                            `Kind::Binding`. ¿Cuanto corpus pueden tocar aun?
  D. LA COBERTURA           cuantos casos `diff` hay y cuantos ven el sustrato
  E. EL MAPA                cada mutacion muda del agujero -> que la cazaria
  F. EL TAMANO              que hay que tocar en `diff.rs`, contado
"""
import collections
import pathlib
import re
import sys

RAIZ = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\ORE\vendor\oos")
CRATES = pathlib.Path(r"C:\ORE\crates")


def documentos(raiz):
    for f in sorted(raiz.rglob("*.yaml")):
        txt = f.read_text(encoding="utf-8", errors="replace")
        for d in re.split(r"^---\s*$", txt, flags=re.M):
            m = re.search(r"^kind:\s*(\w+)", d, re.M)
            if m:
                yield m.group(1), d, f


docs = list(documentos(RAIZ))
print("== corpus:", RAIZ, "==")

# ── A · LA FRONTERA DE CRATES ───────────────────────────────────────────────
print()
print("A - LA FRONTERA DE CRATES, y decide el diseno")
def deps(c):
    t = (CRATES / c / "Cargo.toml").read_text(encoding="utf-8", errors="replace")
    return sorted(set(re.findall(r"^(ore-[\w-]+)\s*=", t, re.M)))


for c in ("ore-core", "ore-view", "ore-cli"):
    print("   %-10s depende de: %s" % (c, ", ".join(deps(c)) or "(nada del arbol)"))
print()
print("   `diff` vive en `ore-core`. `ore-view` depende de `ore-core`, asi que")
print("   `diff` NO PUEDE usar el digest del plan: seria un ciclo. La opcion que")
print("   parecia mas barata —comparar digests de plan, que ya separan la")
print("   pregunta de la promesa— no esta disponible donde `diff` vive.")
print()
print("   Y hay precedente de la alternativa: `diff` ya compara el `Binding`")
print("   ESTRUCTURALMENTE —`Bind { source, ejes }`— sin plan ninguno. Comparar")
print("   `View` y `Table` por lo declarado es lo mismo que ya hace.")

# ── B · EL VOCABULARIO ──────────────────────────────────────────────────────
print()
print("B - EL VOCABULARIO A COMPARAR, y en que eje cae cada cambio")
VIEW = [
    ("from", "INDEX", "de donde salen las filas"),
    ("fields", "CONSUMER", "que sale y como se llama"),
    ("where", "CONSUMER", "QUE FILAS son la respuesta"),
    ("materialized", "INDEX", "si la copia existe, y donde"),
    ("freshness", "CONSUMER", "es una promesa al consumidor"),
    ("owner", "PACKAGE", "custodia, no semantica"),
    ("labels", "CONSUMER", "el estado del documento — `oos.maturity`"),
]
TABLE = [
    ("object", "INDEX", "a que objeto fisico apunta"),
    ("datasource", "INDEX", "de que fuente"),
    ("columns", "INDEX", "que columnas hay"),
    ("reads", "INDEX", "que se le puede pedir"),
    ("changes", "INDEX", "que cambios emite y que los fecha"),
    ("profile", "INDEX", "quien traduce el objeto"),
]
for etiqueta, claves in (("View", VIEW), ("Table", TABLE)):
    print("   %s" % etiqueta)
    for k, eje, por in claves:
        n = sum(1 for kind, d, _ in docs
                if kind == etiqueta and re.search(r"(?:^|[{,\s])%s:" % k, d, re.M))
        print("     %-14s %3d docs  %-9s %s" % (k, n, eje, por))
print()
print("   -> los cuatro ejes existentes bastan. No hace falta uno nuevo: lo que")
print("      cambia QUE SE RESPONDE es CONSUMER y lo que cambia QUIEN RESPONDE")
print("      es INDEX, que es la misma particion que `OOS5019`/`OOS5020` ya")
print("      usaban sobre el binding.")

# ── C · LOS CODIGOS HUERFANOS ───────────────────────────────────────────────
print()
print("C - LOS CODIGOS QUE YA EXISTEN, Y SOBRE QUE SUJETO")
bindings = sum(1 for k, _, _ in docs if k == "Binding")
vistas = sum(1 for k, _, _ in docs if k == "View")
print("   %-46s %3d" % ("documentos `Binding` en el corpus", bindings))
print("   %-46s %3d" % ("documentos `View`", vistas))
d_bind = sum(1 for k, _, f in docs if k == "Binding" and "conformance/diff" in f.as_posix())
d_view = sum(1 for k, _, f in docs if k == "View" and "conformance/diff" in f.as_posix())
print("   %-46s %3d" % ("  de esos, en casos `diff/`: Binding", d_bind))
print("   %-46s %3d" % ("  de esos, en casos `diff/`: View", d_view))
b_v8 = sum(1 for k, _, f in docs if k == "Binding" and "v1alpha8" in f.as_posix())
print("   %-46s %3d" % ("  Binding dentro de un arbol v1alpha8", b_v8))
print("   -> y aqui hay que ser exacto: `OOS5019` -binding fisico cambiado- y")
print("      `OOS5020` -modo de materializacion cambiado- NO estan muertos: se")
print("      calculan sobre `Kind::Binding` y quedan 53 fuera de v1alpha8. Lo")
print("      que no pueden es dispararse por una VISTA, que es la unidad del")
print("      paradigma actual. Estan vivos en el anterior y ciegos en este.")
print("      Su texto normativo ya describe la regla correcta —`91-versioning`")
print("      §6 y `99-errors`—: lo que cambia es el SUJETO, no el codigo.")

# ── D · LA COBERTURA ────────────────────────────────────────────────────────
print()
print("D - LA COBERTURA QUE FALTA")
casos = sorted({f.parent.parent for k, d, f in docs if "conformance/diff" in f.as_posix()})
raices = sorted({c.parent if c.name in ("before", "after") else c for c in casos})
con_sustrato = set()
for k, d, f in docs:
    if "conformance/diff" in f.as_posix() and k in ("View", "Table"):
        con_sustrato.add(f.as_posix().split("conformance/diff/")[1].split("/")[0])
todos = {f.as_posix().split("conformance/diff/")[1].split("/")[0]
         for k, d, f in docs if "conformance/diff" in f.as_posix()}
print("   %-46s %3d" % ("casos `conformance/diff`", len(todos)))
print("   %-46s %3d" % ("  que contienen una vista o una tabla", len(con_sustrato)))
print("   -> la superficie que el paso 3 crea empieza con CERO cobertura, y")
print("      cada codigo necesita su caso: un `before/` y un `after/` que")
print("      difieran en una sola clave, como los tres de `exports`.")

# ── E · EL MAPA ─────────────────────────────────────────────────────────────
print()
print("E - EL MAPA: cada mutacion muda del agujero, y que la cazaria")
MAPA = [
    ("vista · invierte su `where`", "CONSUMER", "NUEVO", "cambia que filas son la respuesta"),
    ("vista · estrecha el `where`", "CONSUMER", "NUEVO", "idem, en la de arriba"),
    ("vista · pierde un campo", "CONSUMER", "OOS5001", "es «propiedad eliminada» un piso abajo"),
    ("vista · afloja la frescura", "CONSUMER", "NUEVO", "una promesa que se relaja"),
    ("vista · la copia se muda", "INDEX", "OOS5020", "cambia el modo de materializacion"),
    ("vista · deja de materializarse", "INDEX", "OOS5020", "el mismo, y es su caso puro"),
    ("vista · desaparece entera", "CONSUMER", "OOS5007", "«entidad o relacion eliminada»"),
    ("tabla · apunta a otro objeto", "INDEX", "OOS5019", "es el binding fisico, con otro sujeto"),
    ("tabla · el escaneo se encarece", "INDEX", "NUEVO", "cambia que se le puede pedir"),
    ("tabla · deja de retractar", "INDEX", "NUEVO", "cambia que se puede mantener"),
]
reuso = collections.Counter()
for m, eje, cod, por in MAPA:
    print("   %-32s %-9s %-8s %s" % (m, eje, cod, por))
    reuso["reutiliza" if cod != "NUEVO" else "nuevo"] += 1
print()
print("   %-46s %3d" % ("mutaciones que reutilizan un codigo", reuso["reutiliza"]))
print("   %-46s %3d" % ("que necesitan uno nuevo", reuso["nuevo"]))
print("   -> la mitad se cubre con codigos que YA EXISTEN y solo cambian de")
print("      sujeto. Los nuevos son de una sola familia: «lo que la fuente")
print("      admite» -reads, changes- y «lo que se promete» -where, freshness-.")

# ── F · EL TAMANO ───────────────────────────────────────────────────────────
print()
print("F - EL TAMANO, contado sobre `diff.rs`")
t = (CRATES / "ore-core/src/diff.rs").read_text(encoding="utf-8", errors="replace")
print("   %-46s %4d" % ("lineas de `diff.rs`", t.count("\n") + 1))
print("   %-46s %4d" % ("campos de `Shape`", len(re.findall(
    r"^    (?:/// .*\n)*    ?\w+: ", t, re.M)) or len(re.findall(r"^    \w+: ", t, re.M))))
print("   %-46s %4d" % ("brazos `Kind::` en el despacho",
                        len(re.findall(r"crate::document::Kind::\w+ =>", t))))
print("   %-46s %4d" % ("funciones de comparacion (`fn .*Shape`)",
                        len(re.findall(r"fn \w+\(a: &Shape, b: &Shape", t))))
print("   %-46s %4d" % ("menciones de `View` o `Table`", len(re.findall(r"View|Table", t))))
print()
print("   -> la forma esta: un campo mas en `Shape`, un brazo mas en el")
print("      despacho y una funcion de comparacion mas, que es exactamente como")
print("      entro cada plano anterior. Lo que NO esta es el sujeto de dos")
print("      codigos y los casos que lo prueben.")
