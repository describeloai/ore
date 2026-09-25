"""MEDIDA · el terreno de la View con cuerpo SQL (antes de redactar OOS).

Decidido (2026-09-25): `View.spec` lleva el SQL como cuerpo, más el contrato
derivado (columnas y tipos), y promocionar una vista sigue siendo SQL con la
capa de gobierno aplicada. Antes de escribir la spec, lo que hay:

  T1  las Views de hoy como SQL: `ore ask --sql` (el camino de `a_sql` que ya
      sirve `/v1` y el puesto) sobre cada View del repositorio. ¿Cuántas salen,
      y el SQL expone los mismos campos que la View declara? Las que no, ¿por qué?
  T2  quién lee la forma estructurada, por función de `ore-core/vistas.rs`
      (el embudo): cuántas llamadas, y qué la sustituiría con un cuerpo SQL
  T3  lo que depende de los CAMPOS de una View: las Entity con `backedBy`
      (y su `primaryKey`): con el contrato derivado les basta
  T4  la regla del `where` (el canal lateral): cuántas Views filtran, por qué
      columnas, y cuántas de esas columnas llevan etiqueta (datasource o
      entidad): lo que la regla sobre el linaje tendría que mirar
  T6  el dialecto: el cuerpo en DuckDB, y `/v1` sirve también Spark (hoy desde
      el plan): cuántas Views de T1 salen en los dos dialectos

Mide y dice; no falla. Necesita `ore` (target/release) y python con pyyaml.
"""
import collections
import glob
import json
import os
import re
import subprocess
import sys

import yaml

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ORE = next((p for p in (os.path.join(RAIZ, "target", "release", "ore.exe"), os.path.join(RAIZ, "target", "release", "ore")) if os.path.exists(p)), None)


def arboles():
    """Cada árbol (un `ontology.config.yaml`) con sus Views."""
    out = collections.defaultdict(list)
    for cfg in glob.glob(os.path.join(RAIZ, "**", "ontology.config.yaml"), recursive=True):
        if os.sep + "target" + os.sep in cfg or "node_modules" in cfg:
            continue
        raiz = os.path.dirname(cfg)
        for f in glob.glob(os.path.join(raiz, "packages", "**", "*.yaml"), recursive=True):
            try:
                d = yaml.safe_load(open(f, encoding="utf-8"))
            except Exception:  # noqa: BLE001
                continue
            if isinstance(d, dict) and d.get("kind") in ("View", "Entity", "Table", "Dataset"):
                out[raiz].append((f, d))
    return out


def qn(d):
    m = d.get("metadata") or {}
    s = m.get("schema")
    return "%s.%s.%s" % (m.get("namespace"), s, m.get("name")) if s and s != "default" else "%s.%s" % (m.get("namespace"), m.get("name"))


def t1_t6(arb):
    print("T1 · las Views de hoy como SQL (`ore ask --sql`, el camino de `a_sql`)")
    motivos = collections.Counter()
    salen, mismas, total, spark = 0, 0, 0, 0
    ejemplos = {}
    for raiz, docs in arb.items():
        for f, d in docs:
            if d.get("kind") != "View":
                continue
            total += 1
            n = qn(d)
            r = subprocess.run([ORE, "ask", raiz, "--vista", n, "--sql"], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60)
            if r.returncode != 0:
                m = " ".join((r.stderr or r.stdout).split())
                clave = ("lee de una Table (no de un dataset)" if "y no de un dataset" in m
                         else "el plan no se expande" if "no se expande" in m
                         else "el árbol no compila / la View no está" if ("no hay" in m or "OOS" in m)
                         else m[:70])
                motivos[clave] += 1
                ejemplos.setdefault(clave, "%s · %s" % (n, m[:160]))
                continue
            salen += 1
            try:
                j = json.loads(r.stdout.strip().splitlines()[-1])
            except Exception:  # noqa: BLE001
                motivos["salida no es JSON"] += 1
                continue
            cols = [c if isinstance(c, str) else c.get("nombre") or c.get("name") for c in j.get("columnas", [])]
            campos = list(((d.get("spec") or {}).get("fields") or {}).keys())
            if sorted(cols) == sorted(campos):
                mismas += 1
            else:
                ejemplos.setdefault("otros campos", "%s · sql %s · view %s" % (n, cols, campos))
            if "consulta" in j and len(ejemplos) < 12:
                ejemplos.setdefault("un SQL", "%s → %s" % (n, " ".join(j["consulta"].split())[:200]))
            # T6: el dialecto de Spark, como lo sirve `/v1`
            r2 = subprocess.run([ORE, "ask", raiz, "--vista", n, "--sql", "--catalogo"], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60)
            if r2.returncode == 0 and "spark" in r2.stdout.lower():
                spark += 1
    print("  · %d Views · salen como SQL %d · con los mismos campos que declaran %d · no salen: %s" % (total, salen, mismas, dict(motivos)))
    for k, v in ejemplos.items():
        print("    - %s: %s" % (k, v))
    print("T6 · el dialecto: de las %d que salen, %d salen también en el de Spark (el que `/v1` sirve)" % (salen, spark))


def t2():
    print("T2 · quién lee la forma, por función de `vistas.rs` (el embudo)")
    fuentes = [f for f in glob.glob(os.path.join(RAIZ, "crates", "**", "*.rs"), recursive=True) if not f.endswith(os.path.join("core", "src", "vistas.rs"))]
    cuenta = collections.Counter()
    ficheros = collections.defaultdict(set)
    for f in fuentes:
        t = open(f, encoding="utf-8", errors="replace").read()
        for m in re.finditer(r"vistas::(\w+)\(", t):
            cuenta[m.group(1)] += 1
            ficheros[m.group(1)].add(os.path.relpath(f, RAIZ).replace("\\", "/").split("/src/")[0].replace("crates/", ""))
    sustituto = {
        "campos": "el contrato (columnas derivadas)", "expone": "el contrato", "expone_en": "el contrato",
        "filtros": "el linaje (aristas INDIRECT de where/join/group)", "agrupacion": "el binder (lo comprueba el motor)",
        "teniendo": "el binder", "agregados": "el linaje (derivadas)", "fuente": "lo que lee (el analizador del paso 1)",
        "raiz": "el linaje por columna hasta la raíz", "raiz_de_lectura": "lo que lee (si todo son datasets, se lee)",
        "cadena": "lo que lee, recursivo", "respaldo": "igual (la Entity nombra la View)", "comprobar": "compilar: fuentes existen, contrato cuadra",
        "invertible": "no invertible (una View SQL es opaca)",
    }
    for k, n in cuenta.most_common():
        print("  · vistas::%-18s %3d llamadas en %-40s → %s" % (k, n, ", ".join(sorted(ficheros[k])), sustituto.get(k, "?")))


def t3_t4(arb):
    print("T3 · lo que depende de los CAMPOS: las Entity con `backedBy`")
    ent, con_pk, vistas_respaldo = 0, 0, set()
    for raiz, docs in arb.items():
        for f, d in docs:
            if d.get("kind") != "Entity":
                continue
            b = (d.get("spec") or {}).get("backedBy")
            if b:
                ent += 1
                vistas_respaldo.add((raiz, str(b.get("view") if isinstance(b, dict) else b)))
                if (d.get("spec") or {}).get("primaryKey"):
                    con_pk += 1
    print("  · %d Entities con backedBy (%d con primaryKey) sobre %d Views: les basta que el contrato exponga sus campos" % (ent, con_pk, len(vistas_respaldo)))

    print("T4 · la regla del where: cuántas filtran, por qué, y con etiqueta")
    con_where, columnas, valores = 0, collections.Counter(), collections.Counter()
    ds_con_labels = 0
    for raiz, docs in arb.items():
        cfg = yaml.safe_load(open(os.path.join(raiz, "ontology.config.yaml"), encoding="utf-8")) or {}
        etiquetados = {ds.get("name") for ds in (cfg.get("datasources") or []) if isinstance(ds, dict) and ds.get("labels")}
        ds_con_labels += len(etiquetados)
        tablas = {qn(d): (d.get("spec") or {}).get("datasource") for _, d in docs if d.get("kind") == "Table"}
        for f, d in docs:
            if d.get("kind") != "View":
                continue
            w = (d.get("spec") or {}).get("where") or {}
            if not w:
                continue
            con_where += 1
            fr = (d.get("spec") or {}).get("from") or {}
            t = fr.get("table")
            ns = (d.get("metadata") or {}).get("namespace")
            ds = tablas.get(t) or tablas.get("%s.%s" % (ns, t))
            for c, v in w.items():
                columnas["con etiqueta (su datasource la lleva)" if ds in etiquetados else "sin etiqueta del datasource"] += 1
                valores["lista (in)" if isinstance(v, list) else "null (ausencia)" if v is None else "igualdad"] += 1
    print("  · %d Views con where · %d predicados: %s · por forma: %s · datasources con labels: %d" % (con_where, sum(columnas.values()), dict(columnas), dict(valores), ds_con_labels))
    print("    (la etiqueta de ENTIDAD llega a la columna raíz por la cadena `backedBy` → `raiz`; con el cuerpo SQL, por el linaje por columna)")


def inducidos():
    """Árboles como los de un inquilino: `ore discover` del catálogo de
    BigQuery de los tests, una base foreign y una standard (con sus Datasets)."""
    import shutil
    import tempfile
    out = {}
    cat = os.path.join(RAIZ, "crates", "ore-cli", "tests", "catalogos", "bigquery-rubix-demo-ventas.json")
    for tipo in ("foreign", "standard"):
        d = tempfile.mkdtemp(prefix="ore-terreno-%s-" % tipo)
        subprocess.run([ORE, "init", ".", "--name", "demo"], cwd=d, capture_output=True)
        shutil.copy(cat, os.path.join(d, "cat.json"))
        # lo que el inquilino tiene: el datasource del origen y el conducto de la copia
        with open(os.path.join(d, "ontology.config.yaml"), "a", encoding="utf-8") as c:
            c.write("datasources:" + chr(10) + "  - { name: bq_ventas, type: bigquery, connectionEnv: BQ_URL }" + chr(10))
        open(os.path.join(d, "conduits.yaml"), "w", encoding="utf-8").write(chr(10).join([
            "apiVersion: oos.dev/v1alpha1", "kind: ConduitPolicy", "metadata: { name: demo }", "spec:",
            "  owner: team:security", "  conduits:", "    materialization.payload: { oos.maturity: DRAFT }", ""]))
        objetos = [t["name"] for t in json.load(open(cat, encoding="utf-8"))["tables"]]
        solo = [x for o in objetos for x in ("--only", o)]
        r = subprocess.run([ORE, "discover", "--from", "cat.json", "--out", "packages/ventas", "--owner", "team:ventas",
                            "--no-model", "--type", tipo] + solo, cwd=d, capture_output=True, text=True, encoding="utf-8", errors="replace")
        if r.returncode != 0:
            print("  ⛔ discover %s: %s" % (tipo, " ".join((r.stderr or r.stdout).split())[:200]))
            continue
        docs = []
        for f in glob.glob(os.path.join(d, "packages", "**", "*.yaml"), recursive=True):
            y = yaml.safe_load(open(f, encoding="utf-8"))
            if isinstance(y, dict) and y.get("kind") in ("View", "Entity", "Table", "Dataset"):
                docs.append((f, y))
        v = subprocess.run([ORE, "validate", "."], cwd=d, capture_output=True, text=True, encoding="utf-8", errors="replace")
        print("  · inducido %s: %s · ore validate: %s" % (tipo, dict(collections.Counter(y["kind"] for _, y in docs)), (v.stdout.strip().splitlines() or ["?"])[-1]))
        out[d] = docs
    return out


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    if not ORE:
        print("⛔ no hay `ore` en target/release")
        return
    print("T0 · los árboles")
    ind = inducidos()
    arb = arboles()
    print("  · %d árboles del repositorio con documentos (muchos, casos de prueba rotos a propósito)" % len(arb))
    print("── sobre los INDUCIDOS (como los de un inquilino) ──")
    t1_t6(ind)
    print("── sobre los del repositorio ──")
    t1_t6(arb)
    t2()
    t3_t4(arb)


if __name__ == "__main__":
    main()
