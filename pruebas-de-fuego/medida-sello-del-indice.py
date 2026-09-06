# -*- coding: utf-8 -*-
"""El sello del indice: no hay que inventarlo, hay que devolverle el sujeto.

`B0` murio porque confundia dos conductos. Lo que quedo abierto es su reverso:
la travesia copia dos columnas, y esa copia no la mira nadie. Seis frentes:

  A. EL SUJETO      cuantas copias de aristas hay, contadas por el motor
  B. LA CONTRADICCION  el mismo mando dice «nada que copiar» y «4 copias»
  C. NO HAY QUE      el sello EXISTE, es normativo desde v1alpha1 y tiene
     INVENTARLO      caso de conformidad. Lo que perdio es el sujeto
  D. EL NOMBRE       tres nombres en juego, y el del ejemplo no es ninguno
  E. QUE PASA SI SE  simulado con la maquina VIVA: el motor calcula el sello
     MIRA            sobre las dos columnas exactas de cada arista
  F. EL PRECIO       y si hacen falta codigos nuevos
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


def correr(*args):
    s = subprocess.run([str(ORE), *map(str, args)], capture_output=True,
                       text=True, encoding="utf-8", errors="replace")
    return s.stdout + s.stderr


def cuantos(patron, donde, extra=None):
    cmd = ["grep", "-rl", patron, str(donde)] + (extra or [])
    return len(subprocess.run(cmd, capture_output=True, text=True).stdout.split())


print("== el sello del indice de topologia ==")

# -- A - EL SUJETO -----------------------------------------------------------
print()
print("A - EL SUJETO: cuantas copias de aristas hay, contadas por el motor")
vista = correr("view", EJEMPLO)
m = re.search(r"registro . (\d+) copias . nadie (\d+) . .ndice de topolog.a (\d+)",
              vista)
if m:
    print("   `ore view` dice: %s copias · nadie %s · indice de topologia %s"
          % m.groups())
for n, d in re.findall(r"^  ([\w.]+)\n    plan.*\n    destino   (\S+)", vista, re.M):
    print("     %-28s -> %s" % (n, d))
print("   -> no es una hipotesis: `registro::topologia` construye un plan real")
print("      —`Proyecta(Lee(objeto), {desde, hasta})`— con destino y refresco")
print("      propios. Es una copia, y esta escrita.")

# -- B - LA CONTRADICCION ----------------------------------------------------
print()
print("B - LA CONTRADICCION, en la MISMA salida del MISMO mando")
for l in re.findall(r"^  flujo     (.*)$", vista, re.M):
    print("   flujo:    ", l)
print("   registro:  4 copias, y dos de ellas leen de `hr_workday`·`Worker`,")
print("              que es justo la vista que acaba de decir «nada que copiar»")
print()
print("   -> el analisis de flujo y el registro de copias no cuentan lo mismo.")
print("      Uno mira `spec.materialized` y el otro deriva de `relations`.")

# -- C - NO HAY QUE INVENTARLO -----------------------------------------------
print()
print("C - EL SELLO YA EXISTE, y es normativo desde v1alpha1")
caso = OOS / "conformance/valid/index-below-clearance/README.md"
print("   caso de conformidad:", caso.relative_to(OOS).as_posix())
print("     «`materialization.topology` autoriza hasta `medium`. Lo que se")
print("      materializa es la topologia —la clave primaria y la propiedad")
print("      `via` de la relacion—...»")
print("     «se copian LAS ARISTAS, no la carga util, y por eso la operacion es")
print("      legitima aunque la entidad contenga campos criticos que no se")
print("      copian.»")
print()
print("   -> esa frase es EXACTAMENTE el argumento que mato a `B0`, y llevaba")
print("      en la spec desde v1alpha1 con caso que la ejerce. `B0` contradecia")
print("      una regla normativa mas vieja que el problema que queria resolver.")
print()
flow = (RAIZ / "crates/ore-core/src/flow.rs").read_text(encoding="utf-8")
cuerpo = flow.split("fn materializaciones", 1)[1][:1200]
print("   Y lo que perdio no es la regla: es EL SUJETO.")
print("     el sello del eje itera sobre        : Kind::%s"
      % re.search(r"Kind::(\w+)", cuerpo).group(1))
print("     ficheros con `kind: Binding` en oos : %d"
      % cuantos("kind: Binding", OOS, ["--include=*.yaml"]))
print("     ...de ellos, en el ejemplo v1alpha8 : %d"
      % cuantos("kind: Binding", EJEMPLO))
print("     aristas que el ejemplo v1alpha8 crea: 4")
print("   -> vivo en el paradigma viejo, CIEGO en el nuevo. La misma forma")
print("      exacta que tenian `OOS5019`/`OOS5020` antes del paso 3.")

# -- D - EL NOMBRE -----------------------------------------------------------
print()
print("D - EL NOMBRE DEL CONDUCTO: tres en juego, y el del ejemplo no es ninguno")
spec = (OOS / "spec/v1alpha1/03-binding.md").read_text(encoding="utf-8")
pol = (EJEMPLO / "conduits.yaml").read_text(encoding="utf-8")
declarados = re.findall(r"^    (materialization\.\w+):", pol, re.M)
print("   la spec (`03-binding`) nombra       : materialization.topology  (%s)"
      % ("si" if "materialization.topology" in spec else "no"))
print("   el codigo (flow.rs) compone         : materialization.topology  (%s)"
      % ("si" if '"topology", "payload"' in flow else "no"))
print("   el ejemplo declara                  : %s" % ", ".join(declarados))
print("   el destino del plan se llama        : oretopo")
print()
print("   -> `materialization.index` no lo conoce NI la spec NI el motor. El")
print("      ejemplo declara un conducto decorativo, y el conducto de verdad")
print("      esta sin declarar: BOTTOM por P4. Al encender el sello, `OOS4011`.")

# -- E - QUE PASA SI SE MIRA -------------------------------------------------
print()
print("E - QUE PASA SI SE MIRA: que lo calcule el motor, no yo")
print("   Se simula el indice con la maquina VIVA: una vista que proyecta solo")
print("   las dos columnas de la arista, materializada, y el conducto de la")
print("   carga con EXACTAMENTE la autorizacion que la politica da al indice.")
tmp = pathlib.Path(tempfile.mkdtemp()) / "r"
shutil.copytree(EJEMPLO, tmp)
p = tmp / "conduits.yaml"
p.write_text(p.read_text(encoding="utf-8").replace(
    "    materialization.index:",
    "    materialization.payload:\n      gdpr.sensitivity: medium\n"
    "      acme.residency:   eu_only\n      oos.maturity:     REVIEWED\n\n"
    "    materialization.index:", 1), encoding="utf-8")
ARISTAS = [("hr", "empleados", "manager", "employeeId", "managerId", "hr_workday"),
           ("hr", "empleados", "department", "employeeId", "departmentId", "hr_workday"),
           ("supply", "envios", "supplier", "shipmentId", "supplierId", "erp_snowflake"),
           ("supply", "envios", "sku", "shipmentId", "skuCode", "erp_snowflake")]
for ns, base, rel, clave, via, ds in ARISTAS:
    (tmp / ("packages/%s/views/arista-%s.yaml" % (ns, rel))).write_text(
        "apiVersion: oos.dev/v1alpha8\nkind: View\n"
        "metadata: { name: arista-%s, namespace: %s }\nspec:\n"
        "  owner: team:x\n  from: { view: %s }\n  fields:\n"
        "    %s: %s\n    %s: %s\n"
        "  materialized: { datasource: %s, table: \"oretopo.%s\" }\n"
        % (rel, ns, base, clave, clave, via, via, ds, rel), encoding="utf-8")
salida = correr("validate", tmp)
fallan = set()
for qn, campo, niv, adm in re.findall(
        r"error\[OOS4002\]: `([\w.-]+)\.(\w+)` lleva `([\w.:]+)`[\s\S]*?admite `([\w.:]+)`",
        salida):
    fallan.add(qn)
    print("     %-24s %-14s %-26s > admite %s" % (qn, campo, niv, adm))
pasan = [r for _, _, r, _, _, _ in ARISTAS
         if not any(("arista-" + r) in q for q in fallan)]
print()
print("   -> de las CUATRO aristas, %d no pasan y %d si (%s)."
      % (len(fallan), len(pasan), ", ".join(pasan)))
print("      Y no lo digo yo: lo dice el motor, con la etiqueta, su origen")
print("      —heredada del suelo `high` de `hr_workday`— y quien la rechaza.")
shutil.rmtree(tmp.parent, ignore_errors=True)
print()
print("   OJO - esto corrige un error de `medida-b0-impagable`, que dijo «el")
print("      suelo de `hr_workday` es `medium`... las aristas SI se pueden")
print("      copiar». El suelo es `high`, y esta escrito en la config. Las")
print("      aristas de RRHH NO caben hoy en lo que el ejemplo autoriza — que")
print("      es justo el motivo por el que el sello merece encenderse.")

# -- F - EL PRECIO Y LOS CODIGOS ---------------------------------------------
print()
print("F - EL PRECIO, y si hacen falta codigos nuevos")
print("   codigos: NO. `OOS4011` (conducto sin autorizacion) y `OOS4002` (la")
print("     etiqueta no cabe) con OTRO SUJETO. El precedente esta en el mismo")
print("     fichero: `materializaciones` y `vistas_materializadas` son dos")
print("     funciones que emiten los MISMOS codigos sobre sujetos distintos.")
print()
print("   el ejemplo: renombrar `materialization.index` -> `.topology`, y")
print("     decidir si se eleva a `high` o si las aristas de RRHH no se copian.")
print("     Es una decision de @acme-security, que es de quien la politica dice")
print("     que es: «elevar una autorizacion es una decision de seguridad».")
print()
print("   cualquier otro repo con `via`: tiene que declarar el conducto. Omitirlo")
print("     no lo deja abierto, lo cierra (P4) — y eso es lo que se pretende.")
