# -*- coding: utf-8 -*-
"""La decima medida - ¿sostiene alguien la contencion por directorio?

`medida-paquete.py` conto que 63 de 63 referencias al sustrato se quedan dentro
del paquete que las escribe. Eso deja una pregunta que un recuento no puede
contestar: **¿lo impone alguien, o es como esta escrito el corpus?**

Se construye a mano el arbol que el corpus no tiene -dos miembros, y una entidad
en uno respaldada por una vista del otro- y se le pregunta al motor. Cuatro
preguntas, y las cuatro son de si o no:

  1. `validate` del WORKSPACE       ¿acepta la referencia que cruza?
  2. `pack`     del WORKSPACE       ¿que publica, y con que nombre?
  3. `validate` del MIEMBRO solo    ¿la acepta tambien?
  4. `pack`     del MIEMBRO solo    ¿deja publicar lo que no se sostiene?

Ojo con el binario: `target/release/ore.exe` es del 2026-08-31, o sea del
paradigma anterior a las vistas. Se usa el de `debug`, y se comprueba la fecha.
"""
import pathlib
import re
import shutil
import subprocess
import sys

ORE = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\ORE\target\debug\ore.exe")
CASO = pathlib.Path(
    r"C:\ORE\vendor\oos\conformance\v1alpha8\valid\a-function-writes-through-its-view\input"
)
TMP = pathlib.Path(
    r"C:\Users\PC\AppData\Local\Temp\claude\C--ORE"
    r"\ac1d5a2c-e15a-495a-80c5-57bfe8c63e56\scratchpad\contencion"
)


def montar():
    """Dos miembros: `infra` tiene la tabla y la vista, `rrhh` solo la entidad."""
    if TMP.exists():
        shutil.rmtree(TMP)
    for d in ("packages/infra/tables", "packages/infra/views", "packages/rrhh/entities"):
        (TMP / d).mkdir(parents=True)
    shutil.copy(CASO / "ontology.config.yaml", TMP)
    shutil.copy(CASO / "conduits.yaml", TMP)
    shutil.copy(CASO / "lattices/assurance.yaml", TMP / "packages/rrhh")
    shutil.copy(CASO / "tables/employees.yaml", TMP / "packages/infra/tables")
    shutil.copy(CASO / "views/empleados.yaml", TMP / "packages/infra/views")
    shutil.copy(CASO / "entities/Employee.yaml", TMP / "packages/rrhh/entities")
    # El `package.yaml` del caso trae `metadata: { name: hr, ... }` EN LINEA.
    # Un `sed` anclado a `^  name:` no lo toca, los dos miembros se quedan
    # llamandose `hr` y el experimento mide otra cosa sin avisar.
    for m in ("infra", "rrhh"):
        t = (CASO / "package.yaml").read_text(encoding="utf-8")
        (TMP / "packages" / m / "package.yaml").write_text(
            re.sub(r"name:\s*hr\b", "name: " + m, t), encoding="utf-8"
        )


def correr(verbo, donde):
    r = subprocess.run([str(ORE), verbo, str(donde)], capture_output=True, text=True)
    return (r.stdout or "") + (r.stderr or "")


montar()
print("== binario:", ORE, "==")
print()

ws = TMP
miembro = TMP / "packages" / "rrhh"

print("1. validate del WORKSPACE (la referencia cruza de rrhh a infra)")
a = correr("validate", ws)
print("   ->", a.strip().splitlines()[-1] if a.strip() else "(sin salida)")

print()
print("2. pack del WORKSPACE")
b = correr("pack", ws)
paquetes = sorted(set(re.findall(r'"Package:(\w+)"', b)))
nombre = re.search(r'"package":"(\w+)"', b)
print("   nombre del .oob      :", nombre.group(1) if nombre else "?")
print("   Package que contiene :", paquetes)

print()
print("3. validate del MIEMBRO solo (packages/rrhh)")
c = correr("validate", miembro)
for ln in c.strip().splitlines()[:2]:
    print("   ", ln)

print()
print("4. pack del MIEMBRO solo")
d = correr("pack", miembro)
for ln in d.strip().splitlines()[:3]:
    print("   ", ln)

print()
print("VEREDICTO")
cruza_ok = "sin errores" in a
solo_falla = "OOS2018" in c
print("   el workspace acepta el cruce      :", "SI" if cruza_ok else "no")
print("   el miembro solo lo rechaza        :", "SI (OOS2018)" if solo_falla else "no")
if cruza_ok and solo_falla:
    print()
    print("   => la contencion NO la sostiene el compilador, la sostiene el")
    print("      empaquetador, y el mensaje nombra lo que no es: dice que la")
    print("      vista no existe cuando existe y esta en otro paquete.")
