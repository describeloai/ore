"""DISCOVER CON SCHEMA (0038 P5) · lo que `ore discover` hace hoy con los schemas
del origen, antes de tocar el inductor.

Decidido (0038 §5): `discover` lleva el schema del origen al del catálogo —
`public.ai_insights` de la fuente es `foreign_test.public.ai_insights`—. Hoy lo
aplana: la tabla se llama `public_ai_insights` en el schema `default` (medido
en el árbol de victor, 38 de 38).

Un catálogo sintético, como el que da `ore-read-postgres`, con lo que hay que
ver: dos schemas (`public`, `sales`), una tabla con el MISMO nombre en los dos
(`pedidos`), una foránea dentro de un schema y otra que cruza, una vista, y
una tabla sin schema (`suelta`, como un fichero).

  §1  por clase (foreign, standard): los ficheros, el `name`, el `object`, la
      versión, las referencias (`from`, `backedBy`, `relations`) y el
      `metadata.schema` si lo hay
  §2  lo que el inductor pregunta (colisiones: `pedidos` en dos schemas)
  §3  `ore validate` de lo que sale

    python pruebas-de-fuego/medida-discover-con-schema.py
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.dirname(AQUI)


def ore_bin():
    if os.environ.get("ORE"):
        return os.environ["ORE"]
    for c in ("target/release/ore.exe", "target/release/ore", "target/debug/ore.exe", "target/debug/ore"):
        if os.path.exists(os.path.join(RAIZ, c)):
            return os.path.join(RAIZ, c)
    sys.exit("no hay binario de `ore`")


def col(n, t="Integer", req=False):
    return {"name": n, "type": t, "required": req}


CATALOGO = {
    "source": "pg",
    "tables": [
        {"name": "public.clientes", "kind": "table", "columns": [col("id", req=True), col("nombre", "String")], "primaryKey": ["id"], "rows": 10},
        {"name": "public.pedidos", "kind": "table", "columns": [col("id", req=True), col("cliente_id"), col("total", "Decimal")],
         "primaryKey": ["id"], "rows": 10,
         "foreignKeys": [{"columns": ["cliente_id"], "references": "public.clientes", "toColumns": ["id"]}]},
        {"name": "sales.pedidos", "kind": "table", "columns": [col("id", req=True), col("cliente_id")], "primaryKey": ["id"], "rows": 10,
         "foreignKeys": [{"columns": ["cliente_id"], "references": "public.clientes", "toColumns": ["id"]}]},
        {"name": "sales.resumen", "kind": "view", "columns": [col("pais", "String"), col("n")], "rows": 3},
        {"name": "suelta", "kind": "table", "columns": [col("id", req=True)], "primaryKey": ["id"], "rows": 1},
    ],
}


def arbol(t):
    os.makedirs(os.path.join(t, "packages"), exist_ok=True)
    open(os.path.join(t, "ontology.config.yaml"), "w").write(
        "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: m, version: 0.1.0 }\n"
        "datasources:\n  - { name: pg, type: postgres, connectionEnv: PG_URL }\n")


def que_dice(texto):
    campos = []
    for k in ("apiVersion", "kind"):
        m = re.search(r"^%s: (.*)$" % k, texto, re.M)
        campos.append(m.group(1) if m else "?")
    m = re.search(r"^metadata:(.*)$", texto, re.M)
    campos.append((m.group(1).strip() if m else "?")[:80])
    for k in ("object", "from", "backedBy", "table", "dataset", "view", "target", "targetEntity"):
        for m in re.finditer(r"^\s+%s: (.*)$" % k, texto, re.M):
            campos.append("%s=%s" % (k, m.group(1).strip()[:60]))
    return " · ".join(campos)


ORE = ore_bin()
for clase in ("foreign", "standard"):
    t = tempfile.mkdtemp(prefix="ore-discover-")
    try:
        arbol(t)
        cat = os.path.join(t, "cat.json")
        json.dump(CATALOGO, open(cat, "w"))
        r = subprocess.run([ORE, "discover", "--from", cat, "--out", os.path.join(t, "packages", "foreign_test"),
                            "--name", "foreign_test", "--type", clase, "--owner", "team:data"]
                           + [x for tb in CATALOGO["tables"] for x in ("--only", tb["name"])],
                           capture_output=True, text=True, encoding="utf-8", errors="replace", cwd=t)
        print("\n§1 · --type %s  (rc=%d)" % (clase, r.returncode))
        if r.returncode not in (0, 1):
            print("   " + (r.stderr or r.stdout)[:600])
        base = os.path.join(t, "packages", "foreign_test")
        for raiz, _, fs in sorted(os.walk(base)):
            for f in sorted(fs):
                if not f.endswith(".yaml"):
                    continue
                rel = os.path.relpath(os.path.join(raiz, f), base).replace("\\", "/")
                print("   %-48s %s" % (rel, que_dice(open(os.path.join(raiz, f), encoding="utf-8").read())))
        print("§2 · lo que pregunta:")
        for l in (r.stdout + r.stderr).splitlines():
            if re.search(r"colisi|mismo|pedidos|schema|pendiente|decisi", l, re.I):
                print("   " + l.strip()[:200])
        v = subprocess.run([ORE, "validate", t], capture_output=True, text=True, encoding="utf-8", errors="replace")
        codigos = sorted(set(re.findall(r"OOS\d{4}", v.stdout + v.stderr)))
        print("§3 · validate rc=%d · códigos: %s" % (v.returncode, ", ".join(codigos) or "ninguno"))
        for l in (v.stdout + v.stderr).splitlines():
            if re.search(r"OOS\d{4}", l) and "OOS2010" not in l:
                print("   " + l.strip()[:220])
    finally:
        shutil.rmtree(t, ignore_errors=True)
