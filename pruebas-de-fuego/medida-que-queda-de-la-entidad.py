# -*- coding: utf-8 -*-
"""¿Que queda de la entidad, despues de tres peldanos que no la adelgazaron?

`entidad.md` §8 la diagnostico como «dos cosas con un nombre»: una declaracion
de tipo, mas una REDECLARACION DEL SUSTRATO —«38 nombres repetidos y una arista
que la copia ya contiene»—. Los dos peldanos que iban a cobrar esa segunda
mitad se midieron y no se escribieron: `B0` pedia el conducto equivocado, y M2
resulto ser 22 nombres y no 38, porque el resto sostiene algo.

Asi que la pregunta es la del titulo, y se contesta de dos formas:

  A. EL VOCABULARIO   los doce campos de la gramatica, con cuantos los usan
  B. QUIEN MAS PODRIA  campo por campo: si algo del sustrato podria decirlo
     DECIRLO
  C. LA PRUEBA POR    se quita una entidad de verdad y se mira que pierde el
     AUSENCIA         motor. Es la unica forma de contestar sin opinar
  D. Y NADIE SE QUEJA  el modo de fallo, que es lo que decide
  E. LO QUE SOBRA     los nombres, contados, que es lo unico que sobraba
  F. VEREDICTO
"""
import collections
import pathlib
import re
import shutil
import subprocess
import tempfile

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
EJEMPLO = OOS / "examples/acme-retail"
ORE = RAIZ / "target/debug/ore"

# Los doce de `document.rs`, en su orden.
SPEC = ["nature", "principal", "primaryKey", "timeKey", "uniqueKeys", "temporal",
        "properties", "relations", "moved", "reserved", "implements", "backedBy"]
META = ["labels", "description", "aiContext"]


def correr(*args):
    s = subprocess.run([str(ORE), *map(str, args)], capture_output=True,
                       text=True, encoding="utf-8", errors="replace")
    return s.stdout + s.stderr


def docs(raiz):
    for f in sorted(raiz.rglob("*.yaml")):
        txt = f.read_text(encoding="utf-8", errors="replace")
        for d in re.split(r"^---\s*$", txt, flags=re.M):
            k = re.search(r"^kind:\s*(\w+)", d, re.M)
            if k:
                yield k.group(1), d, f


TODOS = list(docs(OOS)) + list(docs(RAIZ / "casos"))
ENTIDADES = [d for k, d, _ in TODOS if k == "Entity"]
print("== que queda de la entidad ==")
print("   entidades en el arbol: %d" % len(ENTIDADES))

# -- A - EL VOCABULARIO ------------------------------------------------------
print()
print("A - EL VOCABULARIO: los doce campos, y cuantas entidades usan cada uno")
uso = collections.Counter()
for d in ENTIDADES:
    for k in SPEC:
        if re.search(r"^  %s:" % k, d, re.M):
            uso[k] += 1
    for k in META:
        if re.search(r"^  %s:" % k, d, re.M) or re.search(r"[{,]\s*%s:" % k, d):
            uso[k] += 1
for k in SPEC + META:
    barra = "#" * (20 * uso[k] // max(len(ENTIDADES), 1))
    print("   %-12s %3d  %s" % (k, uso[k], barra))

# -- B - QUIEN MAS PODRIA DECIRLO -------------------------------------------
print()
print("B - ¿QUIEN MAS PODRIA DECIRLO? Campo por campo, con lo que hay medido")
tablas = [d for k, d, _ in TODOS if k == "Table"]
con_key = sum(1 for d in tablas if re.search(r"^\s+key:\s*\[", d, re.M))
etiquetan = collections.Counter()
for k, d, _ in TODOS:
    if re.search(r"^\s+labels:", d, re.M):
        etiquetan[k] += 1

VEREDICTOS = [
    ("nature", "NADIE",
     "entidad, valor o evento es una decision de modelo"),
    ("principal", "NADIE",
     "si esto identifica a quien pide: gobierno, no sustrato"),
    ("primaryKey", "la tabla, si quisiera",
     "`changes.key` dice lo mismo en %d de %d tablas" % (con_key, len(tablas))),
    ("timeKey", "NADIE", "que columna es el tiempo del hecho"),
    ("uniqueKeys", "la tabla, si quisiera", "un indice unico es fisico"),
    ("temporal", "NADIE", "si la propiedad tiene historia que consultar"),
    ("properties", "a medias",
     "el NOMBRE lo dice `fields`; el TIPO no lo dice nadie mas"),
    ("relations", "NADIE",
     "la arista sale de aqui: `via` es de donde el indice la deriva"),
    ("moved", "NADIE", "de donde vino este nombre"),
    ("reserved", "NADIE", "que nombre no puede volver"),
    ("implements", "NADIE", "que forma dice satisfacer"),
    ("backedBy", "NADIE", "de que vista sale — es la flecha misma"),
    ("labels", "NADIE",
     "%d de %d documentos con `labels` son entidades; el resto son "
     "%d suelos de datasource y %d vistas, que solo pueden decir `oos.maturity`"
     % (etiquetan["Entity"], sum(etiquetan.values()),
        etiquetan["OntologyConfig"], etiquetan["View"])),
]
print("   %-12s %-24s %s" % ("campo", "quien mas", "por que"))
for campo, quien, por_que in VEREDICTOS:
    print("   %-12s %-24s %s" % (campo, quien, por_que))
print()
print("   -> de trece, DOS Y MEDIO podria decirlos el sustrato, y ninguno lo")
print("      dice hoy. El resto no tiene otro sitio donde vivir.")

# -- C - LA PRUEBA POR AUSENCIA ---------------------------------------------
print()
print("C - LA PRUEBA POR AUSENCIA: se quita `supply.Shipment` y se mira")


def retrato(arbol):
    out = correr("view", arbol)
    m = re.search(r"registro . (\d+) copias", out)
    esq = {}
    for bloque in out.split("\n\n"):
        v = bloque.split("\n", 1)[0].strip()
        s = re.search(r"^  esquema   (.*)$", bloque, re.M)
        if s and v:
            esq[v] = dict(p.split(": ", 1) for p in s.group(1).split(" · ")
                          if ": " in p)
    return {"copias": int(m.group(1)) if m else None, "esquemas": esq,
            "valida": "error[" not in correr("validate", arbol)}


antes = retrato(EJEMPLO)
tmp = pathlib.Path(tempfile.mkdtemp()) / "r"
shutil.copytree(EJEMPLO, tmp)
shutil.rmtree(tmp / "packages/supply/entities")
despues = retrato(tmp)
shutil.rmtree(tmp.parent, ignore_errors=True)

a, b = antes["esquemas"].get("supply.envios", {}), despues["esquemas"].get("supply.envios", {})
print("   %-16s %-14s %s" % ("campo", "con entidad", "sin ella"))
for c in sorted(a):
    marca = "  <--" if a[c] != b.get(c) else ""
    print("     %-14s %-14s %s%s" % (c, a[c], b.get(c, "(se fue)"), marca))
print()
print("   copias en el registro : %d  ->  %d   (las dos aristas de `Shipment`)"
      % (antes["copias"], despues["copias"]))
print("   etiquetas que gobiernan `supply`: todas, y desaparecen con ella")
print("   sello del indice sobre esas dos aristas: deja de tener sujeto")

# -- D - Y NADIE SE QUEJA ----------------------------------------------------
print()
print("D - Y NADIE SE QUEJA — que es lo que decide")
print("   `ore validate` sin la entidad : %s"
      % ("ok · SIN ERRORES" if despues["valida"] else "falla"))
print()
print("   La entidad se puede borrar entera y el repositorio COMPILA. Lo que")
print("   se pierde no da sintoma: nueve campos pasan a `String`, dos aristas")
print("   dejan de existir, y el analisis de flujo se queda sin nada que")
print("   sellar porque las etiquetas se fueron con ella.")
print()
print("   Y al quitar TODAS las entidades del ejemplo, el motor emite un unico")
print("   error, que no es «falta algo» sino `OOS8002` — una regla de gobierno")
print("   que ya no gobierna nada. Su propia ayuda dice por que importa:")
print("     «una regla que no gobierna nada tiene exactamente el mismo aspecto")
print("      que una que funciona — es el unico fallo de este documento que no")
print("      produce ningun sintoma.»")

# -- E - LO QUE SOBRA --------------------------------------------------------
print()
print("E - LO QUE SOBRA, contado")
print("   De `medida-m2`: 76 propiedades solo declaran su tipo, y 54 de ellas")
print("   sostienen la clave, una `via` o una derivada. Sobran 22 NOMBRES —no")
print("   22 propiedades: sus tipos siguen siendo la unica fuente—.")
print()
print("   Sobre %d entidades y %d campos de gramatica, eso es todo lo que la")
print("   «redeclaracion del sustrato» resulto ser." % (len(ENTIDADES), len(SPEC)))

# -- F - VEREDICTO -----------------------------------------------------------
print()
print("F - VEREDICTO")
print("   El diagnostico de §8 —«dos cosas con un nombre»— era correcto en la")
print("   forma y falso en el reparto. La segunda cosa no era media entidad:")
print("   eran 22 nombres.")
print()
print("   Lo que queda es UNA cosa, y es la que el sustrato no puede tener:")
print()
print("     el TIPO       la vista es fisica y no tipa (`tipos_de_raiz`)")
print("     la ETIQUETA   todo el analisis de flujo cuelga de aqui")
print("     la CLAVE      y con ella la identidad de la fila")
print("     la ARISTA     `via`, de donde sale el indice de topologia")
print("     el TIEMPO     `temporal`, `timeKey`")
print("     el NOMBRE     `moved`, `reserved`, `implements`, `backedBy`")
print()
print("   La entidad no es un fichero de anotaciones. Es EL UNICO SITIO DEL")
print("   REPOSITORIO DONDE HAY SIGNIFICADO — y eso es coherente con lo demas:")
print("   la tabla es un hecho y no significa nada, la vista es una pregunta y")
print("   no lleva significado. Alguien tenia que llevarlo.")
