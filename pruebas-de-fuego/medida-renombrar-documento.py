# -*- coding: utf-8 -*-
"""El paso 4b: renombrar un DOCUMENTO. Lo que `moved` no cubre.

`medida-moved-vista.py` destapo que `moved` renombra MIEMBROS —sus `from`/`to`
son `identifier`, no `qualifiedName`— asi que el nombre de un documento no lo
cubre nadie, ni para la entidad. Y es lo que la fusion de `entidad.md` §10.6
necesita: 25 de 25 parejas tienen nombres distintos y cada fusion mata uno.

Esto mide ese hueco. Seis frentes:

  A. QUE ES HOY          se renombra y se corre `diff`. Sin adivinar
  B. EL RADIO POR KIND   quien nombra cada `kind` por nombre cualificado, para
                         saber si esto es de la vista o del modelo entero
  C. LO QUE YA SE DECIDIO `01-package` §2.2 eligio `moved` frente al `id`
                         estable de ODCS, y dijo el precio. Falta la mitad
  D. EL COTEJO           Terraform, Avro, Protobuf, ODCS y Cognite: los cinco
                         resuelven esto y no de la misma manera
  E. DONDE VIVIRIA       dos candidatos, cada uno con su precedente, y la
                         pregunta que los separa
  F. EL PRECIO           que cuesta la fusion sin esto

CERRADO. `Package.spec.moved`/`reserved` existen desde `01-package` §3.4, y con
ellos un renombrado anunciado deja de contar como supresion —el caso
`v1alpha8/diff/a-renamed-view-is-not-a-deletion` lo afirma con el informe
VACIO—. Lo que este guion mide sigue valiendo como la fotografia del antes, y
vuelto a correr enseña el despues: la fila del renombrado ANUNCIADO ya no da
`OOS5007`.
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
    r"\b4ce4f86-cd8b-429f-9c14-8865e67fa2c6\scratchpad\renombrar"
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

# ── A · QUE ES HOY ──────────────────────────────────────────────────────────
print()
print("A - QUE ES HOY UN RENOMBRADO, corriendo `diff`")
CASO = RAIZ / "conformance/v1alpha8/valid/materialized-view-over-table-within-clearance/input"


def renombrar(etiqueta, *cambios):
    if TMP.exists():
        shutil.rmtree(TMP)
    shutil.copytree(CASO, TMP / "antes")
    shutil.copytree(CASO, TMP / "desp")
    for fi, vi, nu in cambios:
        p = TMP / "desp" / fi
        p.write_text(p.read_text(encoding="utf-8").replace(vi, nu), encoding="utf-8")
    m = TMP / "desp" / "package.yaml"
    m.write_text(m.read_text(encoding="utf-8").replace("version: 1.0.0", "version: 2.0.0"),
                 encoding="utf-8")
    cv, _ = correr("validate", TMP / "desp")
    _, sd = correr("diff", TMP / "antes", TMP / "desp")
    pares = sorted(set(re.findall(r'"code":"(OOS\d{4})","axis":"(\w+)"', sd.replace(" ", ""))))
    if not pares:
        pares = sorted(set(zip(re.findall(r'"code":\s*"(OOS\d{4})"', sd),
                               re.findall(r'"axis":\s*"(\w+)"', sd))))
    sujetos = re.findall(r'"subject":\s*"([^"]+)"', sd)
    print("   %-34s validate %-3s  diff: %s  %s" % (
        etiqueta, "ok" if cv == 0 else "NO",
        " ".join("%s/%s" % p for p in pares) or "NADA", sujetos))


renombrar("una VISTA (y su `backedBy`)",
          ("views/iberia.yaml", "name: iberia", "name: iberica"),
          ("entities/Employee.yaml", "backedBy: iberia", "backedBy: iberica"))
renombrar("una ENTIDAD",
          ("entities/Employee.yaml", "name: Employee", "name: Trabajador"))
renombrar("una TABLA (y el `from.table`)",
          ("tables/employees.yaml", "name: employees", "name: workers"),
          ("views/empleados.yaml", "table: erp.employees", "table: erp.workers"))
print()
print("   -> y hay un gradiente que no esperaba:")
print("      · ENTIDAD y VISTA se leen como un BORRADO —`OOS5007`— y lo que")
print("        aparece con el nombre nuevo NO se reporta, porque anadir es")
print("        compatible. Nadie relaciona los dos hechos;")
print("      · una TABLA no da NADA. `diff` la ve solo a traves de la raiz")
print("        RESUELTA de sus vistas —`datasource·objeto`—, y renombrar el")
print("        DOCUMENTO no cambia el objeto al que apunta. Es coherente con")
print("        el diseno del paso 3 y aun asi deja el renombrado invisible.")
print()
print("      Pasa en todos los kinds, no solo en la vista: esto no es un hueco")
print("      del sustrato, es del MODELO.")

# ── B · EL RADIO POR KIND ───────────────────────────────────────────────────
print()
print("B - EL RADIO POR KIND: quien nombra cada uno por nombre cualificado")
radio = collections.Counter()
for kind, d, f in docs:
    if kind == "Entity":
        if campo(d, "backedBy"):
            radio["View"] += 1
        radio["Concept"] += len(re.findall(r"(?:^|[{,\s])is:\s*[\w.]+", d, re.M))
        im = re.search(r"implements:\s*\[([^\]]*)\]", d)
        if im:
            radio["Interface"] += len([x for x in re.split(r"[,\s]+", im.group(1).strip()) if x])
        radio["Entity"] += len(re.findall(r"(?:^|[{,\s])target:\s*[\w.]+", d, re.M))
        radio["Entity"] += len(re.findall(r"derivedFrom:\s*\[", d))
    elif kind == "View":
        if re.search(r"from:\s*\{?\s*view:", d):
            radio["View"] += 1
        if re.search(r"from:\s*\{?\s*table:", d):
            radio["Table"] += 1
    elif kind == "Interface":
        rq = re.search(r"requires:\s*\[([^\]]*)\]", d)
        if rq:
            radio["Concept"] += len([x for x in re.split(r"[,\s]+", rq.group(1).strip()) if x])
    elif kind == "Binding":
        radio["Entity"] += 1
    elif kind == "Function":
        radio["Entity"] += len(re.findall(r"(?:^|[{,\s])writes:\s*[\w.]+", d, re.M))
    elif kind == "Resolution":
        radio["Entity"] += 1
    elif kind == "Package":
        ex = re.findall(r"exports:\s*\[([^\]]*)\]", d)
        if ex:
            radio["View"] += len([x for x in re.split(r"[,\s]+", ex[0].strip()) if x])
    # Toda etiqueta nombra un reticulo por su nombre cualificado.
    radio["Lattice"] += len(re.findall(r"(?:^|[{,\s])(\w+\.\w+):\s*\w+", meta_de(d), re.M))
for k, v in sorted(radio.items(), key=lambda x: -x[1]):
    print("   %-12s %4d referencias por nombre" % (k, v))
print("   -> ninguno esta a salvo. `Concept` y `Lattice` son ademas los que")
print("      CRUZAN de paquete —los dos unicos cruces del corpus son un")
print("      concepto— asi que renombrar uno rompe a quien no controlas.")

# ── C · LO QUE YA SE DECIDIO ────────────────────────────────────────────────
print()
print("C - LO QUE YA SE DECIDIO, y esta a medias")
print("   `01-package` §2.2, normativo, sobre el `id` estable de ODCS:")
print()
print("     «ODCS exige un identificador estable para que renombrar no rompa")
print("      referencias. En OOS no se escribe a mano... La respuesta de OOS al")
print("      mismo problema son `moved` y `reserved`, que ademas dicen en que se")
print("      convirtio cada nombre y por que. Es un enfoque distinto con sus")
print("      contrapartidas, no una mejora estricta: un consumidor que siguiera")
print("      por id sobreviviria a un renombrado sin hacer nada; uno que sigue")
print("      por nombre necesita leer el `moved`.»")
print()
print("   -> la decision esta TOMADA y razonada: seguimos por nombre y")
print("      anunciamos. Lo que falta no es decidir: es que `moved` solo llego")
print("      hasta los miembros. El consumidor «necesita leer el `moved`» y")
print("      para un documento NO HAY NINGUNO QUE LEER.")

# ── D · EL COTEJO ───────────────────────────────────────────────────────────
print()
print("D - EL COTEJO: los cinco que resuelven esto, y no igual")
COTEJO = [
    ("Terraform", "bloque `moved` { from, to } sobre la DIRECCION del recurso",
     "nivel de modulo · es la fuente que citamos, y su alcance es el ancho"),
    ("Avro", "`aliases` en el ESQUEMA, ademas de en cada campo",
     "vive en el SUPERVIVIENTE: el esquema nuevo dice como se llamaba"),
    ("Protobuf", "`reserved` para numeros y nombres de CAMPO",
     "renombrar el mensaje NO lo cubre — el caso ancho se queda fuera"),
    ("ODCS", "`id` estable, y renombrar no rompe nada",
     "lo rechazamos por escrito en §2.2: no dice EN QUE se convirtio"),
    ("Cognite", "la version va en la identidad de la view",
     "no renombra: publica otra y el consumidor sigue pinchado a la vieja"),
]
for q, que, nota in COTEJO:
    print("   %-10s %s" % (q, que))
    print("   %-10s   %s" % ("", nota))
print()
print("   -> dos lo resuelven con ALIAS —Terraform y Avro— y los dos al nivel")
print("      ancho. Protobuf tiene exactamente nuestro hueco. Y los otros dos")
print("      lo esquivan con una identidad que no es el nombre, que es")
print("      justamente el camino que este proyecto descarto.")

# ── E · DONDE VIVIRIA ───────────────────────────────────────────────────────
print()
print("E - DONDE VIVIRIA: dos candidatos con precedente")
print("   1 · en el SUPERVIVIENTE   `metadata.moved: hr.iberia` en el documento")
print("       nuevo. Es la forma de Avro. El nombre muerto no tiene documento,")
print("       asi que el unico sitio local posible es el que se queda;")
print("   2 · en el MANIFIESTO      `Package.spec.moved: [{from, to, since}]`.")
print("       Es la forma de Terraform —nivel de modulo— y la de `exports`,")
print("       que el paso 2 puso ahi porque el consumidor tiene que leerlo.")
print()
print("   La pregunta que los separa, y no la contesta un recuento:")
print("     ¿puede un nombre mudarse DE PAQUETE? Con (1) lo dice el paquete que")
print("     lo recibe; con (2), el que lo pierde. Y solo el que lo pierde sigue")
print("     estando en el `lock` de quien se rompio.")

# ── F · EL PRECIO ───────────────────────────────────────────────────────────
print()
print("F - EL PRECIO DE NO TENERLO, con la fusion delante")
pares = distintos = 0
for kind, d, _ in docs:
    if kind != "Entity":
        continue
    bb = campo(d, "backedBy")
    if not bb:
        continue
    pares += 1
    nombre = campo(meta_de(d), "name") or ""
    if nombre.lower() != bb.rsplit(".", 1)[-1].lower():
        distintos += 1
print("   %-46s %3d" % ("parejas entidad/vista que la fusion tocaria", pares))
print("   %-46s %3d" % ("  con nombres DISTINTOS, o sea que matan uno", distintos))
print("   -> cada una mata un nombre, y hoy eso se lee como `OOS5007`: un")
print("      borrado. La fusion entera se veria como «desaparecieron 36")
print("      documentos y aparecieron 36», sin una sola linea que los relacione.")
print("      Es tecnicamente correcto y practicamente inservible.")
