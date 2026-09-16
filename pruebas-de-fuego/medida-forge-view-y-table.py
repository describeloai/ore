# -*- coding: utf-8 -*-
"""MEDIDA · View y Table antes de escribir `/documentos/View` y `/documentos/Table` (Forge I2).

Lo que hay que saber antes de escribir el verbo, y no después:

    1  la FORMA        qué admite cada esquema (v1alpha8), qué es obligatorio, qué extensiones
    2  el COMPILADOR   qué dice `ore validate` en cada rotura que un PUT o un DELETE pueden
                       provocar — es lo que decide si hace falta un 409 explícito o sobra
    3  el INDUCTOR     qué escribe `ore discover` como Table y View, con qué nombre de fichero,
                       y si ese árbol compila TAL CUAL (no: y eso cambia la puerta del PUT)
    4  el EMISOR       `ore view add` es EL emisor de vistas; un PUT que emita YAML propio
                       sería un segundo emisor
    5  lo DERIVADO     lo que `ore view .` cuenta de cada vista: plan, linaje, refresco,
                       empuje, cotejo — el material de `/derivados` (I3), no de `/documentos`

    uso:  PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-forge-view-y-table.py
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ORE = os.path.join(RAIZ, "target", "release", "ore.exe" if os.name == "nt" else "ore")
ACME = os.path.join(RAIZ, "vendor", "oos", "examples", "acme-retail")
if not os.path.exists(ORE):
    print("sin binario `ore` en target/release: cargo build --release -p ore-cli")
    sys.exit(1)


def ore(*args, cwd):
    r = subprocess.run([ORE, *args], cwd=cwd, capture_output=True, text=True, encoding="utf-8", errors="replace")
    return r.returncode, r.stdout + r.stderr


def codigos(salida):
    return sorted(set(re.findall(r"OOS\d{4}", salida)))


def leer(p):
    return open(p, encoding="utf-8").read()


def escribir(p, s):
    with open(p, "w", encoding="utf-8", newline="\n") as f:
        f.write(s)


print("\n  ═══ VIEW Y TABLE, ANTES DE ESCRIBIR EL VERBO ═══")

# ── 1 · la forma ─────────────────────────────────────────────────────────────
print("\n  ① la forma (schemas/v1alpha8):")
for k in ("view", "table"):
    s = json.load(open(os.path.join(RAIZ, "vendor", "oos", "schemas", "v1alpha8", f"{k}.schema.json"), encoding="utf-8"))
    m, sp = s["properties"]["metadata"], s["properties"]["spec"]
    print("     %-6s metadata %s (obligatorio %s) · spec %s (obligatorio %s) · extensiones %s" % (
        k.capitalize(), list(m["properties"]), m.get("required"), list(sp["properties"]), sp.get("required"),
        "x-<proveedor>-*" if s.get("patternProperties") else "no"))

# ── 2 · el compilador ───────────────────────────────────────────────────────
T = tempfile.mkdtemp(prefix="forge-vt-")
shutil.copytree(ACME, T, dirs_exist_ok=True)
V = os.path.join(T, "packages", "hr", "views", "empleados.yaml")
TB = os.path.join(T, "packages", "hr", "tables", "workday.yaml")
E = os.path.join(T, "packages", "hr", "entities", "Employee.yaml")
ORIG = {p: leer(p) for p in (V, TB, E)}
AGG = os.path.join(T, "packages", "hr", "views", "agg.yaml")


def caso(nombre, mutar):
    mutar()
    rc, out = ore("validate", ".", cwd=T)
    primera = next((l for l in out.splitlines() if l.startswith("error[")), "ok")
    print("     %-58s %s  %s" % (nombre, "pasa " if rc == 0 else "falla", primera[:110]))
    for p, s in ORIG.items():
        escribir(p, s)
    if os.path.exists(AGG):
        os.remove(AGG)
    return codigos(out)


def sed(p, a, b):
    escribir(p, leer(p).replace(a, b, 1))


VISTA_SOBRE_VISTA = "apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: activos, namespace: hr }\nspec:\n  owner: team:people-data\n  from: { view: empleados }\n  fields: { employeeId: employeeId, %s }\n"

print("\n  ② lo que dice `ore validate` (acme-retail, hr.empleados sobre hr.workday_worker):")
print("     %-58s %s" % ("caso", "resultado"))
R = {}
R["A"] = caso("A · borrar la vista que Employee.backedBy nombra", lambda: os.remove(V))
R["B"] = caso("B · borrar la tabla que empleados.from.table nombra", lambda: os.remove(TB))
R["P"] = caso("P · renombrar la vista (= borrarla para quien la nombra)", lambda: sed(V, "  name: empleados\n", "  name: empleados2\n"))
R["C"] = caso("C · un field lee una columna que la tabla no tiene", lambda: sed(V, '"Compensation_Data.Grade_Reference"', '"No_Existe"'))
R["K"] = caso("K · from.table a una tabla que no existe", lambda: sed(V, "from: { table: workday_worker }", "from: { table: no_existe }"))
R["D"] = caso("D · una vista sobre otra vista (from.view)", lambda: escribir(AGG, VISTA_SOBRE_VISTA % "fullName: fullName"))
R["D2"] = caso("D2 · from.view con un campo que la de abajo no expone", lambda: escribir(AGG, VISTA_SOBRE_VISTA % "nombre: noExiste"))
R["Q"] = caso("Q · where por una clave que no es columna", lambda: sed(V, "  fields:\n", "  where:\n    CD_STATUS: activo\n  fields:\n"))
R["E1"] = caso("E1 · x-rubix-displayName en View.metadata", lambda: sed(V, "  name: empleados\n", "  name: empleados\n  x-rubix-displayName: Empleados\n"))
R["E2"] = caso("E2 · x-rubix-displayName en Table.metadata", lambda: sed(TB, "  name: workday_worker\n", "  name: workday_worker\n  x-rubix-displayName: Worker\n"))
R["F"] = caso("F · View.metadata.labels con gdpr.sensitivity", lambda: sed(V, "  name: empleados\n", "  name: empleados\n  labels: { gdpr.sensitivity: high }\n"))
R["F2"] = caso("F2 · View.metadata.labels con oos.maturity: DRAFT", lambda: sed(V, "  name: empleados\n", "  name: empleados\n  labels: { oos.maturity: DRAFT }\n"))
R["H"] = caso("H · View sin owner", lambda: sed(V, "  owner: team:people-data\n", ""))
R["G"] = caso("G · Table sin changes", lambda: escribir(TB, leer(TB)[: leer(TB).index("  changes:")]))
R["J"] = caso("J · Table.datasource no declarada en el manifiesto", lambda: sed(TB, "datasource: hr_workday", "datasource: no_declarada"))
R["O"] = caso("O · Table reads: none con una vista virtual encima", lambda: escribir(TB, re.sub(r"  reads:\n(    .*\n)+", "  reads: none\n", leer(TB))))
R["N"] = caso("N · materializar sin conducto que lo autorice", lambda: sed(V, "from: { table: workday_worker }\n", 'from: { table: workday_worker }\n  materialized: { datasource: hr_workday, table: "cache.empleados" }\n'))
R["M"] = caso("M · agrupar con count(col) en vez de count()", lambda: escribir(AGG, "apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: porDepartamento, namespace: hr }\nspec:\n  owner: team:people-data\n  from: { view: empleados }\n  groupBy: [departmentId]\n  fields: { departmentId: departmentId, personas: \"count(employeeId)\" }\n  having: { personas: \">= 8\" }\n"))
R["M2"] = caso("M2 · agrupar sobre una TABLA (count(), groupBy, having)", lambda: escribir(AGG, "apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: porDepartamento, namespace: hr }\nspec:\n  owner: team:people-data\n  from: { table: workday_worker }\n  groupBy: [\"Organization_Data.Cost_Center_Reference\"]\n  fields:\n    departmentId: \"Organization_Data.Cost_Center_Reference\"\n    personas: \"count()\"\n  having:\n    personas: \">= 8\"\n"))
R["M3"] = caso("M3 · agrupar sobre una VISTA (sum(baseSalary) de empleados)", lambda: escribir(AGG, "apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: porDepartamento, namespace: hr }\nspec:\n  owner: team:people-data\n  from: { view: empleados }\n  groupBy: [departmentId]\n  fields:\n    departmentId: departmentId\n    total: \"sum(baseSalary)\"\n"))
if R["M3"]:
    print("       ⚠ M3: un agregado sobre una vista falla con OOS2018 y un nombre VACÍO (`lee ``…`); sobre una tabla (M2) pasa.")
    print("         La spec §5.8 lo admite y la conformidad no lo cubre (9 casos agrupan sobre tabla, 0 sobre vista):")
    print("         fallo del compilador al resolver el argumento del agregado contra la vista de abajo. Se anota; no se arregla aquí.")

# ── 3 · el inductor ─────────────────────────────────────────────────────────
I = tempfile.mkdtemp(prefix="forge-ind-")
ore("init", ".", cwd=I)
escribir(os.path.join(I, "catalogo.json"), json.dumps({"source": "demo", "tables": [
    {"name": "public.clientes", "columns": [{"name": "id", "type": "Integer"}, {"name": "email", "type": "String"}]},
    {"name": "public.pedidos", "columns": [{"name": "id", "type": "Integer"}, {"name": "cliente_id", "type": "Integer"}]}]}))
ore("discover", "--from", "catalogo.json", "--out", "packages/demo", "--name", "demo", cwd=I)
ficheros = sorted(os.path.relpath(os.path.join(d, f), I).replace("\\", "/") for d, _, fs in os.walk(os.path.join(I, "packages")) for f in fs if f.endswith(".yaml"))
print("\n  ③ lo que `ore discover` escribe:")
for f in ficheros:
    txt = leer(os.path.join(I, f))
    kind = re.search(r"^kind: (\w+)", txt, re.M).group(1)
    name = re.search(r"name: (\w+)", txt).group(1)
    nota = ""
    if kind == "Table":
        nota = "reads: {} · changes: { mode: none, witness: none } (no se sondeó, no se inventa)"
    if kind == "View":
        nota = "labels: { oos.maturity: DRAFT } · owner: cambiame (NO valida)"
    print("     %-8s %-48s name=%-16s %s" % (kind, f, name, nota))
rc, out = ore("validate", ".", cwd=I)
print("     → el árbol inducido compila TAL CUAL: %s  %s" % ("sí" if rc == 0 else "NO", " ".join(codigos(out))))
print("       (OOS2009 owner cambiame · OOS2010 sin primaryKey hasta `review` · OOS2004 fuente sin `source add`)")
print("     → el nombre del FICHERO no es metadata.name: `Clientes__public_clientes.yaml` guarda `name: clientes`")

# ── 4 · el emisor ───────────────────────────────────────────────────────────
rc, out = ore("view", "add", "--from", "workday_worker", "--owner", "team:people-data", "--field", "id=Worker_Reference.ID", "--path", "packages/hr", "solo_ids", cwd=T)
nuevo = os.path.join(T, "packages", "hr", "views", "solo_ids.yaml")
print("\n  ④ `ore view add` (el único emisor de View, el mismo que el inductor):")
print("     rc=%d · fichero %s" % (rc, "escrito: packages/hr/views/solo_ids.yaml" if os.path.exists(nuevo) else "NO escrito · " + out.strip().splitlines()[0][:100] if out.strip() else "NO escrito"))
if os.path.exists(nuevo):
    for l in leer(nuevo).splitlines():
        print("       │ " + l)
    rc2, out2 = ore("validate", ".", cwd=T)
    print("     → compila: %s %s" % ("sí" if rc2 == 0 else "no", " ".join(codigos(out2))))
    os.remove(nuevo)
print("     → no hay `ore table add`: la tabla la escribe el inductor (es un hecho) o una persona a mano")

# ── 5 · lo derivado ─────────────────────────────────────────────────────────
rc, out = ore("view", ".", cwd=T)
print("\n  ⑤ `ore view .`: lo que se DERIVA de una vista (material de /derivados, no de /documentos):")
for l in out.splitlines()[:8]:
    print("       │ " + l[:120])
claves = sorted(set(re.findall(r"^  ([a-záéíóú]+) ", out, re.M)))
print("     → renglones por vista: %s" % ", ".join(claves))

shutil.rmtree(T, ignore_errors=True)
shutil.rmtree(I, ignore_errors=True)

# ── lo que sale de aquí ─────────────────────────────────────────────────────
print("\n  ⇒ lo que decide el verbo:")
print("     · borrar o renombrar una View/Table que alguien nombra es OOS2018 en QUIEN la nombra (A, B, K, P):")
print("       el 409 explícito con los nombres hace falta por lo mismo que en Entity — la verdad contada desde el sitio correcto")
print("     · el compilador cubre lo que un PUT puede romper: field/where sin columna (C, D2, Q), fuente sin declarar (J),")
print("       sin changes (G), reads: none sin copia (O), copia sin conducto (N), agregado mal escrito (M). El verbo no duplica nada")
print("     · agrupar sobre una tabla (M2) y sobre una vista (M3) compilan; M3 fallaba el 2026-09-16 y se arregló en vistas.rs"
      if not R["M3"] else
      "     · agrupar sobre una tabla compila (M2); sobre una vista NO (M3): un PUT de esa vista daría 422 por un fallo que no es suyo")
print("     · View admite x-rubix-displayName (E1) y sólo oos.maturity en labels (F/F2); Table admite x-rubix-displayName (E2) y no labels")
print("     · View sin owner PASA el esquema (H): `owner` lo exige el emisor (cambiame no valida: OOS2009), no el compilador")
print("     · un árbol inducido NO compila hasta review (③): la puerta del PUT no puede ser «el árbol compila» sino")
print("       «el árbol no empeora» — diagnósticos después ⊆ diagnósticos antes. Vale también para Entity (I1)")
print("     · el fichero no se llama como el documento (③): buscar por metadata.name, nunca por nombre de fichero")
print("     · PUT desde JSON reescribe el YAML y PIERDE los comentarios; acme-retail está lleno. Decidir: emitir (ore view add)")
print("       o aceptar el YAML tal cual y validar")
print()
