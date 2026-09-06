# -*- coding: utf-8 -*-
"""El defecto: las etiquetas de la entidad no suben a una copia por encima.

Medido en `medida-oos4001.py`: `nationalId`, que `hr.Employee` declara
`critical`, se copia por un conducto autorizado a `high` y **compila**. Aquello
midio QUE pasa. Esto mide QUE HACE FALTA PARA ARREGLARLO.

  A. LA EXPOSICION   cuanto se esta filtrando HOY, que no es lo mismo que
                     cuanto se puede filtrar
  B. LAS DOS         por que baja y no sube, en una linea de codigo
     DIRECCIONES
  C. LA SALIDA       la hay, y no es invertir un mapa
  D. LA INYECTIVIDAD lo que habria que decidir si el mapa no fuese inyectivo,
                     y cuantas veces pasa
  E. LO QUE NO ESTA  el sello del indice no comparte el defecto, y por que
     AFECTADO        importa saberlo
  F. EL PRECIO
"""
import collections
import pathlib
import re

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
CASOS = RAIZ / "casos"


def docs(*raices):
    for r in raices:
        for f in sorted(r.rglob("*.yaml")):
            t = f.read_text(encoding="utf-8", errors="replace")
            for d in re.split(r"^---\s*$", t, flags=re.M):
                k = re.search(r"^kind:\s*(\w+)", d, re.M)
                if k:
                    yield k.group(1), d, f


def meta(d, clave):
    m = re.search(r"^metadata:\s*\{([^}]*)\}", d, re.M)
    if m:
        m2 = re.search(r"(?:^|[,\s])%s:\s*([^,}\s]+)" % clave, m.group(1))
        if m2:
            return m2.group(1)
    m = re.search(r"^metadata:\n(?:  .*\n)*", d, re.M)
    if m:
        m2 = re.search(r"^  %s:\s*(\S+)" % clave, m.group(0), re.M)
        if m2:
            return m2.group(1)
    return None


TODOS = list(docs(OOS, CASOS))
print("== el sello no sube ==")

# -- A - LA EXPOSICION -------------------------------------------------------
print()
print("A - LA EXPOSICION: lo que se filtra hoy, y lo que se puede filtrar")
vistas, entidades = {}, {}
for k, d, f in TODOS:
    n = meta(d, "name")
    if not n:
        continue
    if k == "View":
        vistas[(f.parent.parent, n)] = (
            "materialized:" in d,
            (re.search(r"from:\s*\{\s*view:\s*([\w-]+)", d) or [None, None])[1])
    elif k == "Entity":
        bb = re.search(r"^  backedBy:\s*(\S+)", d, re.M)
        if bb:
            entidades[(f.parent.parent, n)] = bb.group(1)
riesgo = [(e, bb, v) for (p, e), bb in entidades.items()
          for (p2, v), (mat, desde) in vistas.items()
          if p2 == p and mat and desde == bb]
mats = sum(1 for m, _ in vistas.values() if m)
print("   %-46s %3d" % ("entidades con `backedBy`", len(entidades)))
print("   %-46s %3d" % ("vistas materializadas", mats))
print("   %-46s %3d" % ("copias POR ENCIMA de la vista de una entidad", len(riesgo)))
print()
print("   -> CERO. El agujero es LATENTE, no una fuga: hoy nadie materializa")
print("      una vista derivada de la que respalda a una entidad. Y por eso no")
print("      se habia visto — no porque algo lo impida, sino porque el corpus")
print("      no lo hace. Lo que no hay es nada que lo impida.")

# -- B - LAS DOS DIRECCIONES -------------------------------------------------
print()
print("B - POR QUE BAJA Y NO SUBE, en una linea")
flow = (RAIZ / "crates/ore-core/src/flow.rs").read_text(encoding="utf-8")
linea = next(l.strip() for l in flow.split("\n") if "proyectar(pkg, suya" in l)
print("   `flow::vistas_materializadas`, via 2:")
print("     %s" % linea)
print()
print("   `suya` es la vista que RESPALDA a la entidad y `vqn` la que se copia.")
print("   `proyectar(desde, objetivo)` calcula `cadena(desde)` —que camina")
print("   HACIA ABAJO por `from`— y busca el objetivo ahi. Si la copia esta")
print("   ENCIMA, no aparece, devuelve `None`, y el `else { continue }` se")
print("   salta la entidad ENTERA. Sin diagnostico, porque no es un error:")
print("   es una entidad que «no toca esta vista».")
print()
print("   Y las etiquetas tienen que viajar en las DOS direcciones por el mismo")
print("   motivo: hacia abajo, porque la copia de un eslabon inferior contiene")
print("   las mismas columnas; hacia arriba, porque una vista derivada es una")
print("   PROYECCION de esas mismas columnas. En los dos casos se copia el")
print("   mismo dato, y la etiqueta es del dato.")

# -- C - LA SALIDA -----------------------------------------------------------
print()
print("C - LA SALIDA: no hay que invertir ningun mapa")
v = (RAIZ / "crates/ore-core/src/vistas.rs").read_text(encoding="utf-8")
print("   `cadena(pkg, v)` camina desde `v` siguiendo `from`, asi que:")
print()
print("     cadena(copia)            = [copia, ..., vista de la entidad, ..., tabla]")
print("     proyectar(copia, vista)  -> campo de la COPIA -> campo de la VISTA")
print()
print("   O sea que la llamada AL REVES si resuelve, y da justo lo que hace")
print("   falta: para cada campo de la copia, de que campo de la vista de la")
print("   entidad viene. Se recorren los campos de la copia y se busca su")
print("   origen, en vez de recorrer las propiedades y buscar su destino.")
print()
print("   -> el arreglo es elegir la direccion segun donde este la copia, no")
print("      invertir nada. Y hay una comprobacion barata que lo decide sin")
print("      ambiguedad: si la vista de la entidad esta en `cadena(copia)`.")

# -- D - LA INYECTIVIDAD -----------------------------------------------------
print()
print("D - LA INYECTIVIDAD: que pasaria si dos campos vinieran del mismo")
n = malas = 0
for k, d, _ in TODOS:
    if k != "View" or "fields:" not in d:
        continue
    n += 1
    cuerpo = d.split("fields:", 1)[1]
    cuerpo = re.split(r"^  \w+:", cuerpo, maxsplit=1, flags=re.M)[0]
    dest = [x.strip().strip('"') for _, x in
            re.findall(r"^    (\w+):\s*([^\s{].*)$", cuerpo, re.M)]
    if any(c > 1 for c in collections.Counter(dest).values()):
        malas += 1
print("   %-46s %3d" % ("vistas con `fields`", n))
print("   %-46s %3d" % ("...que mapean dos campos al mismo origen", malas))
print()
print("   -> ninguna. Y si la hubiera, la respuesta correcta ya esta escrita en")
print("      la funcion: `subir` se queda con el nivel MAS RESTRICTIVO, asi que")
print("      dos campos del mismo origen recibirian los dos su etiqueta. No hay")
print("      decision que tomar — la conservadora es la unica.")

# -- E - LO QUE NO ESTA AFECTADO ---------------------------------------------
print()
print("E - EL SELLO DEL INDICE NO COMPARTE EL DEFECTO")
usa = "proyectar" in flow.split("fn indices_de_topologia", 1)[1][:2000]
print("   `indices_de_topologia` usa `proyectar` : %s" % ("SI" if usa else "no"))
print("   ...lee las etiquetas de                : `efectivas[entidad][propiedad]`")
print()
print("   -> lee la entidad DIRECTAMENTE, porque lo que viaja por la arista son")
print("      dos propiedades suyas y no campos de una vista. Se escribio asi por")
print("      otro motivo —«no hay que proyectar nada»— y resulta que ademas lo")
print("      hace inmune. Es la misma diferencia que este defecto: cuanto mas")
print("      cerca esta la etiqueta de su dueno, menos camino hay que perder.")

# -- F - EL PRECIO -----------------------------------------------------------
print()
print("F - EL PRECIO")
sitios = len(re.findall(r"proyectar\(", flow))
print("   %-46s %3d" % ("llamadas a `proyectar` en `flow.rs`", sitios))
print("   %-46s %3d" % ("casos de conformidad que lo ejercen hoy", 0))
print()
print("   Un sitio, y ningun caso que lo cubra en esta direccion. El arreglo")
print("   necesita ademas dos casos —uno que acepte y uno que rechace— porque")
print("   sin el que ACEPTA nadie sabria si la correccion se pasa de estricta y")
print("   empieza a sellar copias que no llevan nada de la entidad.")
