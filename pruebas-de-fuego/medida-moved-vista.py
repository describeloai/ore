# -*- coding: utf-8 -*-
"""El paso 4: `moved` en la vista. Que renombra, quien lo nombra y que cuesta.

La escalera de `entidad.md` §10.6 lo pone aqui por una razon concreta: al
fusionar, 25 de 25 parejas entidad/vista tienen nombres DISTINTOS, y cada
fusion mata un nombre. Y el paso 3 dejo una mutacion muda que espera a esto:
un campo de vista que desaparece, porque `OOS5001` es «sin `moved` ni
`reserved`» y la vista no los tiene.

Antes de escribir nada hay que saber QUE renombra `moved`, y ahi hay una
sorpresa que corrige lo que veniamos diciendo. Seis frentes:

  A. QUE RENOMBRA        la forma de `moved` y `reserved`, leida del esquema
  B. CUANTO SE USA       en el corpus y en el paquete realista
  C. QUIEN NOMBRA UN     el radio de una renombrada: cuantas referencias hay a
     CAMPO DE VISTA      un campo, y de cuantas clases
  D. QUIEN NOMBRA UNA    lo mismo para el nombre del documento
     VISTA
  E. EL PRECIO           que pasa hoy al renombrar, corriendo el compilador
  F. LA TABLA            si le toca tambien, o es la asimetria de la madurez
"""
import collections
import pathlib
import re
import shutil
import subprocess
import sys

ORE = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\ORE\target\debug\ore.exe")
RAIZ = pathlib.Path(r"C:\ORE\vendor\oos")
TMP = pathlib.Path(
    r"C:\Users\PC\AppData\Local\Temp\claude\C--ORE"
    r"\b4ce4f86-cd8b-429f-9c14-8865e67fa2c6\scratchpad\moved"
)


def meta_de(d):
    if "metadata:" not in d:
        return ""
    return re.split(r"^\s*spec:", d.split("metadata:", 1)[1], maxsplit=1, flags=re.M)[0]


def campo(d, k):
    m = re.search(r"(?:^|[{,\s])%s:\s*([\w.\-/]+)" % k, d, re.M)
    return m.group(1) if m else None


def documentos(raiz):
    for f in sorted(raiz.rglob("*.yaml")):
        txt = f.read_text(encoding="utf-8", errors="replace")
        for d in re.split(r"^---\s*$", txt, flags=re.M):
            m = re.search(r"^kind:\s*(\w+)", d, re.M)
            if m:
                yield m.group(1), d, f


def correr(*a):
    r = subprocess.run([str(ORE), *[str(x) for x in a]], capture_output=True, text=True)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


docs = list(documentos(RAIZ))
print("== corpus:", RAIZ, "==")

# ── A · QUE RENOMBRA ────────────────────────────────────────────────────────
print()
print("A - QUE RENOMBRA `moved`, y no es lo que parecia")
print("   `moved`    { from, to, since }   from/to son `identifier`")
print("   `reserved` { name, reason, until }   name es `identifier`")
print()
print("   Un `identifier`, NO un `qualifiedName`. Asi que `moved` renombra")
print("   MIEMBROS —una propiedad— y no el documento. Ni una entidad puede")
print("   renombrarse con `moved`: lo que se renombra es lo que hay dentro.")
print()
print("   -> corrige lo que veniamos diciendo de la escalera. La fusion mata un")
print("      nombre de DOCUMENTO, y eso NO lo cubre `moved` para nadie. Son dos")
print("      problemas distintos, y hay que decir cual es cual.")
print()
print("   Y el cotejo con la fuente que la propia spec cita: el bloque `moved`")
print("   DE TERRAFORM renombra la DIRECCION de un recurso —`from =")
print("   aws_instance.a`, `to = aws_instance.b`—, no uno de sus atributos.")
print("   Nosotros tomamos el nombre y lo aplicamos un piso mas abajo.")
print()
print("   O sea que el caso ancho —renombrar el documento— no es una invencion")
print("   nueva: es la semantica ORIGINAL de lo que ya citamos. Lo que hay es")
print("   media importacion.")

# ── B · CUANTO SE USA ───────────────────────────────────────────────────────
print()
print("B - CUANTO SE USA HOY")
for k in ("moved", "reserved"):
    tot = sum(1 for kind, d, _ in docs
              if kind == "Entity" and re.search(r"^  %s:" % k, d, re.M))
    ej = sum(1 for kind, d, f in docs
             if kind == "Entity" and "examples" in f.as_posix()
             and re.search(r"^  %s:" % k, d, re.M))
    print("   %-12s %3d entidades del corpus · %d en `examples/`" % (k, tot, ej))
print("   -> poco, y no es un argumento en contra: son la unica disciplina de")
print("      renombrado que hay, y el corpus es de casos de conformidad. Lo que")
print("      si dice es que el precio de anadirlas a la vista es del mismo")
print("      orden: nadie tiene que escribir nada hasta que renombra.")

# ── C · QUIEN NOMBRA UN CAMPO DE VISTA ──────────────────────────────────────
print()
print("C - EL RADIO DE UN CAMPO: quien lo nombra, y de cuantas clases")
clases = collections.Counter()
for kind, d, f in docs:
    if kind == "Entity":
        if campo(d, "backedBy"):
            # `OOS2022`: cada propiedad DEBE ser campo de su vista.
            cuerpo = d.split("properties:", 1)[1] if "properties:" in d else ""
            m = re.search(r"^  (?:relations|moved|reserved|temporal):", cuerpo, re.M)
            cuerpo = cuerpo[: m.start()] if m else cuerpo
            clases["propiedad de una entidad"] += len(
                re.findall(r"^    ([A-Za-z_]\w*):", cuerpo, re.M))
        clases["`via` de una relacion"] += len(
            re.findall(r"(?:^|[{,\s])via:\s*\[", d, re.M))
        if re.search(r"validTime:", d):
            clases["`temporal.validTime.from/to`"] += 2
    elif kind == "View":
        if re.search(r"from:\s*\{?\s*view:", d):
            cuerpo = d.split("fields:", 1)[1] if "fields:" in d else ""
            m = re.search(r"^  (?:where|materialized|freshness|owner):", cuerpo, re.M)
            cuerpo = cuerpo[: m.start()] if m else cuerpo
            clases["valor de `fields` de la de arriba"] += len(
                re.findall(r"^    [A-Za-z_]\w*:\s*(\w+)", cuerpo, re.M))
            if "where:" in d:
                w = d.split("where:", 1)[1]
                m = re.search(r"^  \w+:", w, re.M)
                clases["clave de `where` de la de arriba"] += len(
                    re.findall(r"^    ([\w.]+):", w[: m.start()] if m else w, re.M))
for k, v in sorted(clases.items(), key=lambda x: -x[1]):
    print("   %-38s %4d" % (k, v))
print("   %-38s %4d" % ("TOTAL", sum(clases.values())))
print("   -> cinco clases, y una de ellas —`temporal.validTime`— NO LA COMPRUEBA")
print("      NADIE hoy. Un renombrado sin disciplina las rompe todas a la vez, y")
print("      esa se rompe en silencio.")

# ── D · QUIEN NOMBRA UNA VISTA ──────────────────────────────────────────────
print()
print("D - EL RADIO DE UNA VISTA: quien nombra el DOCUMENTO")
d_clases = collections.Counter()
for kind, d, f in docs:
    if kind == "Entity" and campo(d, "backedBy"):
        d_clases["`backedBy` de una entidad"] += 1
    elif kind == "View" and re.search(r"from:\s*\{?\s*view:", d):
        d_clases["`from.view` de otra vista"] += 1
    elif kind == "Package" and re.search(r"exports:", d):
        d_clases["`exports` del manifiesto"] += len(
            re.findall(r"exports:\s*\[([^\]]*)\]", d)[0].split(",")) if re.findall(
            r"exports:\s*\[([^\]]*)\]", d) else 0
for k, v in sorted(d_clases.items(), key=lambda x: -x[1]):
    print("   %-38s %4d" % (k, v))
print("   %-38s %4d" % ("TOTAL", sum(d_clases.values())))
print("   -> y desde el paso 2 hay una tercera clase: `exports`. Renombrar una")
print("      vista exportada rompe ademas el manifiesto de su propio paquete.")

# ── E · EL PRECIO ───────────────────────────────────────────────────────────
print()
print("E - EL PRECIO HOY, corriendo el compilador")
CASO = RAIZ / "conformance/v1alpha8/valid/materialized-view-over-table-within-clearance/input"


def probar(etiqueta, fichero, viejo, nuevo, *mas):
    if TMP.exists():
        shutil.rmtree(TMP)
    shutil.copytree(CASO, TMP)
    for fi, vi, nu in ((fichero, viejo, nuevo),) + tuple(mas):
        p = TMP / fi
        t = p.read_text(encoding="utf-8")
        assert vi in t, (etiqueta, vi)
        p.write_text(t.replace(vi, nu), encoding="utf-8")
    cod, out = correr("validate", TMP)
    codigos = sorted(set(re.findall(r"OOS\d{4}", out)))
    print("   %-44s %s" % (etiqueta, "ok" if cod == 0 else ",".join(codigos)))


probar("renombrar un CAMPO, solo en la vista",
       "views/empleados.yaml", "nationalId: national_id", "dni: national_id")
probar("renombrar un CAMPO y arreglar a los que lo nombran",
       "views/empleados.yaml", "nationalId: national_id", "dni: national_id",
       ("views/iberia.yaml", "dni: nationalId", "dni: dni"))
probar("renombrar la VISTA, solo el documento",
       "views/iberia.yaml", "name: iberia", "name: iberica")
probar("renombrar la VISTA y arreglar el `backedBy`",
       "views/iberia.yaml", "name: iberia", "name: iberica",
       ("entities/Employee.yaml", "backedBy: iberia", "backedBy: iberica"))
print("   -> renombrar es HOY un cambio rompedor sin ventana: o se arregla a")
print("      todos los que nombran en el mismo commit, o no compila. Que es")
print("      exactamente lo que `moved` existe para evitar un piso mas arriba.")

# ── F · LA TABLA ────────────────────────────────────────────────────────────
print()
print("F - ¿Y LA TABLA?")
cols = sum(len(re.findall(r"^    ([\w.\"]+):", d.split("columns:", 1)[1].split("reads:")[0], re.M))
           for kind, d, _ in docs if kind == "Table" and "columns:" in d)
print("   %-46s %4d" % ("columnas declaradas en tablas del corpus", cols))
print("   -> una columna de tabla NO la renombra nadie de aqui: la renombra el")
print("      ORIGEN, y lo que hay que hacer no es anunciarlo sino DETECTARLO —")
print("      `drift-detect`, que compara la declaracion con el esquema fisico y")
print("      esta declarado sin implementar. Es la misma asimetria que la")
print("      madurez: la vista DECIDE su nombre, la tabla lo ESPEJA.")

# ── G · QUE DESBLOQUEA ──────────────────────────────────────────────────────
print()
print("G - QUE DESBLOQUEA, y es concreto")
print("   [HECHO. `02-view` §4.2 y `01-package` §3.4 — y el paso 4 acabo siendo")
print("    UNO, no dos: el mismo mecanismo en tres alcances, con la regla `lo")
print("    dice el que sobrevive; si no sobrevive nadie, lo dice el paquete`.]")
print("   El paso 3 dejo tres mutaciones mudas, y UNA espera exactamente a")
print("   esto: «una vista pierde un campo». No se le puso codigo porque")
print("   `OOS5001` es «propiedad eliminada SIN `moved` NI `reserved`», y sin")
print("   los dos campos la regla no tendria valvula: todo renombrado seria")
print("   rompedor para siempre.")
print()
print("   Las otras dos —aflojar `freshness`, estrechar `reads`/`changes`— NO")
print("   dependen de esto. Son ordenes de una sola direccion y su codigo es")
print("   otra decision.")
