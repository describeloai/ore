#!/usr/bin/env python3
"""0038 P6 · Medida: ¿qué hay que tocar para RENOMBRAR un schema?

Un árbol descubierto (el catálogo de bigquery de las pruebas deja el schema
`rubix_demo_ventas` en `packages/ventas/`), con lo que lo nombra desde fuera:

- una View en `default` del mismo paquete que lee `ventas.rubix_demo_ventas.clientes`;
- un paquete `eu` cuya View lee la tabla y cuya Entity apunta a la Entity;
- un `.sql` que la lee en tres partes;
- el puntero de su copia en `datasets/ventas/rubix_demo_ventas/clientes.json`.

Y se renombra por pasos, compilando en cada uno:

  0. tal cual
  1. sólo la carpeta
  2. + `metadata.name` del Schema y `metadata.schema` de lo de dentro
  3. + las referencias de tres partes de fuera (texto `ventas.rubix_demo_ventas.`)
  4. + el `.sql`

Y `ore diff` entre 0 y 3/4: ¿el cambio de nombre es un OOS5007?

Uso: ORE=target/release/ore python pruebas-de-fuego/medida-renombrar-schema.py
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

RAIZ = Path(__file__).resolve().parent.parent
ORE = os.environ.get("ORE", str(RAIZ / "target/release/ore"))
CAT = RAIZ / "crates/ore-cli/tests/catalogos/bigquery-rubix-demo-ventas.json"
VIEJO, NUEVO = "rubix_demo_ventas", "ventas_es"


def ore(d, *a):
    r = subprocess.run([ORE, *a], cwd=d, capture_output=True, text=True, encoding="utf-8")
    return r.returncode, r.stdout + r.stderr


def codigos(d):
    rc, out = ore(d, "validate", ".")
    return rc, Counter(re.findall(r"OOS\d{4}", out)), out


def escribir(p, t):
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(t, encoding="utf-8", newline="\n")


def main():
    tmp = Path(tempfile.mkdtemp(prefix="medida-renombrar-"))
    d = tmp / "a"
    d.mkdir()
    ore(d, "init", ".", "--name", "demo")
    shutil.copy(CAT, d / "cat.json")
    rc, out = ore(d, "discover", "--from", "cat.json", "--out", "packages/ventas")
    print("discover rc", rc)
    pk = d / "packages/ventas"
    print("carpetas:", sorted(p.name for p in pk.iterdir()))
    sch = pk / VIEJO
    print("en el schema:", sorted(str(p.relative_to(sch)).replace("\\", "/") for p in sch.rglob("*.yaml"))[:12], "…")
    # Las referencias de DENTRO: ¿en una parte o en tres?
    dentro = Counter()
    for p in sch.rglob("*.yaml"):
        t = p.read_text(encoding="utf-8")
        dentro["tres"] += t.count(f"ventas.{VIEJO}.")
        for m in re.finditer(r"^\s*(backedBy|target|table|view|dataset):\s*([\w.]+)", t, re.M):
            dentro[f"{m.group(1)}:{m.group(2).count('.') + 1}p"] += 1
    print("refs dentro del schema:", dict(dentro))

    # Lo que lo nombra desde fuera.
    vista_tabla = next((sch / "views").glob("*clientes.yaml"))
    print("--- una vista del schema ---\n" + vista_tabla.read_text(encoding="utf-8"))
    escribir(pk / "views/resumen.yaml", f"""apiVersion: oos.dev/v1alpha13
kind: View
metadata: {{ name: resumen, namespace: ventas }}
spec:
  owner: "team:ventas"
  from: {{ table: ventas.{VIEJO}.clientes }}
  fields: {{ id: id }}
""")
    ore(d, "package", "new", "eu", "--owner", "team:eu")
    escribir(d / "packages/eu/views/copia.yaml", f"""apiVersion: oos.dev/v1alpha13
kind: View
metadata: {{ name: copia, namespace: eu }}
spec:
  owner: "team:eu"
  from: {{ table: ventas.{VIEJO}.clientes }}
  fields: {{ id: id }}
""")
    escribir(d / "packages/ventas/transforms/cuenta.sql",
             f"create or replace table ventas.cuenta as select count(*) as n from ventas.{VIEJO}.clientes\n")
    escribir(d / f"datasets/ventas/{VIEJO}/clientes.json",
             '{"metadata_location":"s3://b/ore/v2/catalogo/ventas/%s/clientes/metadata/00001-x.metadata.json"}\n' % VIEJO)
    subprocess.run(["git", "init", "-q"], cwd=d); subprocess.run(["git", "config", "core.autocrlf", "false"], cwd=d)
    subprocess.run(["git", "add", "-A"], cwd=d)
    subprocess.run(["git", "-c", "user.email=a@b", "-c", "user.name=a", "commit", "-qm", "0"], cwd=d)

    rc, c0, out0 = codigos(d)
    print(f"\n[0] tal cual: rc={rc} {dict(c0)}")
    base = tmp / "base"
    shutil.copytree(d, base)

    # 1 · la carpeta
    (pk / VIEJO).rename(pk / NUEVO)
    rc, c1, out1 = codigos(d)
    print(f"[1] + carpeta: rc={rc} {dict(c1 - c0)} (nuevos)")
    for l in out1.splitlines():
        if re.search(r"OOS\d{4}", l) and "error" in l.lower():
            print("     ", l.strip()[:160])
            break

    # 2 · los nombres
    for p in (pk / NUEVO).rglob("*.yaml"):
        t = p.read_text(encoding="utf-8")
        t2 = re.sub(rf"(\bschema:\s*){VIEJO}\b", rf"\g<1>{NUEVO}", t)
        if p.name == "schema.yaml":
            t2 = re.sub(rf"(\bname:\s*){VIEJO}\b", rf"\g<1>{NUEVO}", t2)
        if t2 != t:
            escribir(p, t2)
    rc, c2, out2 = codigos(d)
    print(f"[2] + metadata: rc={rc} {dict(c2 - c0)} (nuevos)")
    for l in out2.splitlines():
        if re.search(r"OOS\d{4}", l):
            print("     ", l.strip()[:170])

    # 3 · las de tres partes (sólo yaml)
    tocados = []
    for p in (d / "packages").rglob("*.yaml"):
        t = p.read_text(encoding="utf-8")
        if f"ventas.{VIEJO}." in t:
            escribir(p, t.replace(f"ventas.{VIEJO}.", f"ventas.{NUEVO}."))
            tocados.append(str(p.relative_to(d)).replace("\\", "/"))
    rc, c3, out3 = codigos(d)
    print(f"[3] + refs yaml ({len(tocados)} ficheros: {tocados}): rc={rc} {dict(c3 - c0)} (nuevos)")
    for l in out3.splitlines():
        if re.search(r"OOS\d{4}", l):
            print("     ", l.strip()[:170])

    # 4 · el .sql
    s = d / "packages/ventas/transforms/cuenta.sql"
    s.write_text(s.read_text().replace(f"ventas.{VIEJO}.", f"ventas.{NUEVO}."), newline="\n")
    rc, c4, out4 = codigos(d)
    print(f"[4] + .sql: rc={rc} {dict(c4 - c0)} (nuevos)")
    rc, o = ore(d, "sql", "packages/ventas/transforms/cuenta.sql")
    print("    ore sql:", o.strip().splitlines()[:3])

    # diff
    rc, o = ore(d, "diff", str(base), ".")
    print(f"\n[diff 0→4] rc={rc}")
    print("   ", Counter(re.findall(r"OOS\d{4}", o)))
    print("\n".join("    " + l for l in o.splitlines()[:25]))

    # el índice de assets: ¿la carpeta nueva?
    rc, o = ore(d, "assets", "--json")
    print("\n[assets] carpetas de ventas:", re.findall(r'"carpetas":\s*\[[^\]]*\]', o)[:3])
    print("tmp:", tmp)


if __name__ == "__main__":
    sys.exit(main())
