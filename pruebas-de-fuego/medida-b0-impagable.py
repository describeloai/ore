# -*- coding: utf-8 -*-
"""NOTA: `B0` no se escribio, y lo que salio de aqui —que copiar la carga y
copiar las aristas son dos decisiones— se convirtio en el sello del indice.

B0 no se puede escribir: el precio no es caro, es IMPAGABLE.

`handoff-topologia.md` §B0 pide un tercer gemelo:

    `OOS2026` — lo que se ATRAVIESA se debe materializar

y `sustrato.md` conto su precio en «una entidad —`supply.Shipment`— tendria
que declarar `materialized`». Esta medida implementa la regla y va a cobrar
ese precio. No se puede cobrar, y el motivo es estructural.

  A. QUIEN ENCIENDE   la regla implementada, contra el corpus
  B. EL PRECIO        pagarlo, de verdad, y mirar que sale
  C. POR QUE          dos conductos, dos decisiones — y `sustrato.md` conto
                      sobre un arbol que ya no es el de hoy
  D. EL AGUJERO       el conducto del indice existe y NADIE lo lee
  E. LO QUE SOBREVIVE la regla es cierta; lo que era falso es su consecuencia
"""
import pathlib
import re
import subprocess
import sys
import tempfile
import shutil

RAIZ = pathlib.Path(r"C:\ORE")
EJEMPLO = RAIZ / "vendor/oos/examples/acme-retail"
ORE = RAIZ / "target/debug/ore"


def validar(arbol):
    s = subprocess.run([str(ORE), "validate", str(arbol)],
                       capture_output=True, text=True, encoding="utf-8",
                       errors="replace")
    return s.stdout + s.stderr


print("== B0: lo que se atraviesa se debe materializar ==")

# ── A · QUIEN ENCIENDE ──────────────────────────────────────────────────────
print()
print("A - QUIEN ENCIENDE la regla, ya implementada")
salida = validar(EJEMPLO)
enciende = re.findall(r"error\[OOS2026\]: `([^`]+)`[^\n]*", salida)
for v in enciende:
    print("   ", v)
print("   -> %d vistas, y `sustrato.md` conto UNA. La cuenta era de otro arbol:" % len(enciende))
print("      su tabla dice «`hr.empleados` · materializada» y hoy el fichero")
print("      abre con «Sin materializar» y explica dos veces por que.")

# ── B · EL PRECIO ───────────────────────────────────────────────────────────
print()
print("B - PAGAR EL PRECIO: declarar `materialized` en las dos")
tmp = pathlib.Path(tempfile.mkdtemp()) / "r"
shutil.copytree(EJEMPLO, tmp)
for rel in ("packages/hr/views/empleados.yaml", "packages/supply/views/envios.yaml"):
    f = tmp / rel
    f.write_text(f.read_text(encoding="utf-8")
                 + '  materialized: { datasource: erp_snowflake, table: "cache.x" }\n',
                 encoding="utf-8")
salida = validar(tmp)
for l in sorted(set(re.findall(r"error\[OOS\d+\][^\n]*", salida))):
    print("   ", l)
print()
print("   -> el ejemplo insignia deja de compilar, y el autor NO puede")
print("      arreglarlo: si declara `materialization.payload` en la politica,")
print("      `OOS4002` cae sobre once campos `critical` —`nationalId`,")
print("      `baseSalary`...—. B0 obliga a elegir entre el sello y el grafo.")
shutil.rmtree(tmp.parent, ignore_errors=True)

# ── C · POR QUE ─────────────────────────────────────────────────────────────
print()
print("C - POR QUE: `materialized` es UN conducto, y el indice es OTRO")
pol = (EJEMPLO / "conduits.yaml").read_text(encoding="utf-8")
declarados = re.findall(r"^    (materialization\.\w+):", pol, re.M)
print("   conductos de materializacion que la politica declara:")
for c in declarados:
    m = re.search(re.escape(c) + r":\s*\n\s*gdpr\.sensitivity:\s*(\w+)", pol)
    print("     %-28s gdpr.sensitivity: %s" % (c, m.group(1) if m else "?"))
print("   `materialization.payload`         NO declarado -> BOTTOM (P4)")
print()
print("   Y las dos columnas que la travesia copia son la clave y la `via`:")
ent = (EJEMPLO / "packages/hr/entities/employee.yaml").read_text(encoding="utf-8")
for prop in ("managerId", "departmentId"):
    trozo = ent.split(prop + ":", 1)[1][:200] if prop + ":" in ent else ""
    tiene = "labels" in trozo.split("\n\n")[0]
    print("     %-14s labels propias: %s" % (prop, "si" if tiene else "no -> suelo del datasource"))
print("   -> son DOS DECISIONES y la politica ya las separa: por eso hay dos")
print("      conductos con dos autorizaciones distintas. Copiar la carga y")
print("      copiar las aristas se deciden por separado, y `B0` las juntaba.")
print()
print("      (Cual sea la respuesta de la SEGUNDA decision aqui, la mide")
print("       `medida-sello-del-indice`: el suelo de `hr_workday` es `high` y")
print("       el indice admite `medium`, asi que estas aristas tampoco caben")
print("       hoy. Eso no salva a `B0` —seguiria pidiendo el otro conducto—;")
print("       lo que hace es dar el motivo para encender el sello del indice.)")

# ── D · EL AGUJERO ──────────────────────────────────────────────────────────
print()
print("D - EL AGUJERO: el conducto del indice existe y nadie lo lee")
vista_rs = (RAIZ / "crates/ore-cli/src/vista.rs").read_text(encoding="utf-8")
flow_rs = (RAIZ / "crates/ore-core/src/flow.rs").read_text(encoding="utf-8")
usa_payload = 'CONDUCTO: &str = "materialization.payload"' in vista_rs
cuerpo = flow_rs.split("fn materializaciones", 1)[1][:1200]
eje = re.search(r"for eje in \[([^\]]+)\]", cuerpo)
kind = re.search(r"Kind::(\w+)", cuerpo)
print("   `spec.materialized` de la vista usa    : materialization.payload",
      "(si)" if usa_payload else "(?)")
print("   el unico sitio que compone otros ejes  : flow.rs, `for eje in [%s]`"
      % (eje.group(1).strip() if eje else "?"))
print("   ...y ese bucle itera sobre             : Kind::%s"
      % (kind.group(1) if kind else "?"))
print()
print("   -> `Binding` esta RETIRADO. El unico codigo que sellaba el eje del")
print("      indice cuelga de un `kind` que ya no se escribe. Hoy la travesia")
print("      copia aristas SIN SELLO: `hr.empleados` no puede copiar")
print("      `nationalId` y sus aristas se copian sin que nadie mire.")

# ── E · LO QUE SOBREVIVE ────────────────────────────────────────────────────
print()
print("E - LO QUE SOBREVIVE")
print("   La frase es cierta: lo que se atraviesa SE MATERIALIZA. Falsa era su")
print("   consecuencia —«luego el autor declara `materialized`»—, porque quien")
print("   se materializa no es la vista: son dos columnas suyas, y eso es")
print("   derivable (P2) y no se declara.")
print()
print("   Asi que B0 no es un `OOS2026` que PROHIBE. Es el sello que falta:")
print("     la clave y las `via` de una vista atravesada pasan por")
print("     `materialization.topology`, y ese conducto tiene que estar")
print("     autorizado (`OOS4011`) y admitir sus etiquetas (`OOS4002`).")
print()
print("   Mismo sujeto —la vista—, misma familia, y en vez de obligar a copiar")
print("   lo que el sello prohibe, sella lo que hoy se copia a escondidas.")
