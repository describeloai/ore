# -*- coding: utf-8 -*-
"""`OOS4001` no salta donde deberia. Y el motivo no es `OOS4001`.

La medida del corpus dejo una sospecha: `totalCompensation` deriva de dos
propiedades `critical` y, al materializarla, el sello dice que lleva `high`
HEREDADA —el suelo del datasource— en vez de `critical` computada. Se dijo que
no se afirmaba nada sin medirlo. Esto lo mide, y lo que sale es mas grande.

  A. LA PROPAGACION   ¿esta mal el calculo de la entidad? Se comprueba sin
                      tocar el sello, por un camino independiente
  B. EL REPARTO       cuatro propiedades, cuatro origenes distintos, y todas
                      llegan igual. Eso descarta que sea cosa de las derivadas
  C. LA PRUEBA        se abre el conducto justo por encima del suelo y se mira
                      si un dato `critical` se copia sin que nadie lo diga
  D. EL ESPEJO        el mismo arbol materializando la vista de la entidad en
                      vez de una de encima. Aqui es donde se fija el mecanismo
  E. EL MECANISMO     leido del codigo, no inferido
  F. VEREDICTO
"""
import pathlib
import re
import shutil
import subprocess
import tempfile

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
EJEMPLO = OOS / "examples/acme-retail"
ORE = RAIZ / "target/debug/ore"

CONDUCTO = """    materialization.payload:
      gdpr.sensitivity: %s
      acme.residency:   eu_only
      oos.maturity:     REVIEWED

    materialization.topology:"""


def correr(*args):
    s = subprocess.run([str(ORE), *map(str, args)], capture_output=True,
                       text=True, encoding="utf-8", errors="replace")
    return s.stdout + s.stderr


def arbol(campo=None, nivel="low", materializar_la_de_la_entidad=False):
    """Copia del ejemplo con una vista materializada y el conducto a `nivel`."""
    d = pathlib.Path(tempfile.mkdtemp()) / "r"
    shutil.copytree(EJEMPLO, d)
    if materializar_la_de_la_entidad:
        v = d / "packages/hr/views/empleados.yaml"
        v.write_text(v.read_text(encoding="utf-8")
                     + '  materialized: { datasource: hr_workday, '
                       'table: "cache.e", key: [employeeId] }\n', encoding="utf-8")
    else:
        (d / "packages/hr/views/p.yaml").write_text(
            "apiVersion: oos.dev/v1alpha8\nkind: View\n"
            "metadata: { name: p, namespace: hr }\nspec:\n"
            "  owner: team:hr\n  from: { view: empleados }\n  fields:\n"
            "    employeeId: employeeId\n    %s: %s\n"
            '  materialized: { datasource: hr_workday, table: "cache.p", '
            "key: [employeeId] }\n" % (campo, campo), encoding="utf-8")
    p = d / "conduits.yaml"
    p.write_text(p.read_text(encoding="utf-8").replace(
        "    materialization.topology:", CONDUCTO % nivel, 1), encoding="utf-8")
    return d


print("== OOS4001: por que no salta ==")

# -- A - LA PROPAGACION ------------------------------------------------------
print()
print("A - ¿ESTA MAL EL CALCULO DE LA ENTIDAD? No, y se comprueba sin el sello")
rep = correr("report", EJEMPLO)
exige = [l for l in rep.split("\n") if "totalCompensation" in l]
for l in exige:
    print("   ", " ".join(l.split())[:88])
ret = (EJEMPLO / "lattices/gdpr.yaml").read_text(encoding="utf-8")
m = re.search(r"requiresGovernance:\s*\n((?:\s+\w+:.*\n)+)", ret)
print()
print("   y el reticulo dice desde que nivel se exige cada clase:")
for l in (m.group(1).rstrip().split("\n") if m else []):
    print("     %s" % l.strip())
print()
print("   -> `authorization` SOLO se exige en `critical`, y `ore report` dice")
print("      que `totalCompensation` la exige. Asi que la entidad la tiene en")
print("      `critical`: LA PROPAGACION FUNCIONA. El nivel se pierde despues.")

# -- B - EL REPARTO ----------------------------------------------------------
print()
print("B - CUATRO PROPIEDADES, CUATRO ORIGENES, Y TODAS LLEGAN IGUAL")
CAMPOS = [("nationalId", "declarada `critical` en la entidad"),
          ("baseSalary", "declarada `critical`"),
          ("totalCompensation", "DERIVADA de dos `critical`"),
          ("grade", "sin etiqueta propia — solo el suelo `high`")]
for campo, que in CAMPOS:
    d = arbol(campo, "low")
    salida = correr("validate", d)
    shutil.rmtree(d.parent, ignore_errors=True)
    mm = re.search(r"hr\.p\.%s` lleva `(gdpr[^`]*)` \((\w+)\)" % campo, salida)
    print("   %-20s %-38s -> %s" % (campo, que,
                                    "%s (%s)" % mm.groups() if mm else "(no dice nada)"))
print()
print("   -> las cuatro llegan como `high` HEREDADA, que es el suelo del")
print("      datasource. Ni una etiqueta de la entidad alcanza la copia, asi")
print("      que esto NO es cosa de las derivadas ni de `OOS4001`.")

# -- C - LA PRUEBA -----------------------------------------------------------
print()
print("C - LA PRUEBA: se abre el conducto justo por encima del suelo")
print("   Con el conducto en `low` compila mal por el suelo, y eso TAPA el")
print("   fallo. Se pone en `high` —lo que el suelo ya trae— para que lo unico")
print("   que pueda quejarse sea la etiqueta de la entidad.")
d = arbol("nationalId", "high")
salida = correr("validate", d)
shutil.rmtree(d.parent, ignore_errors=True)
limpio = "error[" not in salida
print()
print("   se copia `nationalId`, que `hr.Employee` declara `critical`,")
print("   por un conducto autorizado hasta `high`:")
print("     `ore validate` -> %s" % ("ok · SIN ERRORES" if limpio else "falla"))
print()
if limpio:
    print("   -> COMPILA. Y es exactamente lo que el comentario de")
    print("      `flow::vistas_materializadas` dice que no puede pasar:")
    print("        «Sin esto se copiaria en claro un dato que la entidad")
    print("         clasifico — y compilaria.»")

# -- D - EL ESPEJO -----------------------------------------------------------
print()
print("D - EL ESPEJO: el mismo arbol, materializando la vista DE LA ENTIDAD")
d = arbol(nivel="high", materializar_la_de_la_entidad=True)
salida = correr("validate", d)
shutil.rmtree(d.parent, ignore_errors=True)
for l in sorted(set(re.findall(r"hr\.empleados\.(\w+)` lleva `(gdpr[^`]*)` \((\w+)\)",
                               salida)))[:4]:
    print("     %-20s %s (%s)" % l)
print()
print("   -> aqui SI llegan, y `declarada`. La diferencia entre los dos arboles")
print("      es una sola: si la vista materializada esta POR ENCIMA de la vista")
print("      de la entidad, o ES la vista de la entidad.")

# -- E - EL MECANISMO --------------------------------------------------------
print()
print("E - EL MECANISMO, leido del codigo y no inferido")
v = (RAIZ / "crates/ore-core/src/vistas.rs").read_text(encoding="utf-8")
cuerpo = v.split("pub fn proyectar", 1)[1][:700]
print("   `vistas::proyectar(pkg, desde, objetivo)`:")
for l in cuerpo.split("\n")[4:12]:
    print("     %s" % l.strip()[:72])
print()
print("   `cadena(pkg, desde)` es la cadena de `desde` HACIA ABAJO, y busca la")
print("   posicion del objetivo en ella. Si el objetivo esta por ENCIMA, no")
print("   esta en esa cadena, `position(...)?` devuelve `None`, y la via 2 de")
print("   `vistas_materializadas` hace `continue`: la entidad entera se salta.")
print()
print("   La asimetria esta al reves de donde importa. Una vista POR ENCIMA de")
print("   la de la entidad es mas estrecha y mas cercana al consumidor: es el")
print("   sitio natural para materializar una copia de servicio.")

# -- F - VEREDICTO -----------------------------------------------------------
print()
print("F - VEREDICTO")
print("   `OOS4001` no salta porque NINGUNA etiqueta de la entidad llega a una")
print("   copia que este por encima de su vista. No es un fallo del codigo:")
print("   es un fallo del SELLO, y `OOS4002` esta igual de ciego.")
print()
print("   Lo que lo hace serio y no academico:")
print("     - el ejemplo insignia compila con un dato `critical` copiado por un")
print("       conducto de `high`, y nadie lo dice")
print("     - la unica prueba de `OOS4001` usa un `Binding`, asi que la suite")
print("       esta en verde y seguiria estandolo")
print("     - y la garantia de L0 —«un auditor comprueba que un paquete no")
print("       filtra sin acceso a un solo dato»— es exactamente lo que aqui")
print("       falla en silencio")
print()
print("   La correccion no es de una linea: `proyectar` resuelve hacia abajo")
print("   porque los renombres se componen en esa direccion. Subir exige")
print("   invertir el mapa de campos, y un renombre no siempre es invertible.")
print("   Eso es una medida propia antes de escribir nada.")
