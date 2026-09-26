"""MEDIDA · ADR 0040 paso 4 · ¿servir la consulta de una vista da las filas de hoy?

Hoy una View se sirve compilada: `ore ask --vista X --sql` escribe el plan del
motor (`ore-view::a_sql`) sobre sus datasets, cada uno registrado en DuckDB como
`"__ore_dataset"."<p>.<n>"`. Desde v1alpha14 una View es su consulta, escrita
con nombres del árbol (1, 2 o 3 partes), y el paso 4 la sirve tal cual. Antes de
cambiar cómo se sirve, con datos sintéticos en DuckDB:

  S1  cada View del repositorio que hoy se sirve (`ore ask --sql` sale):
      A = su consulta de hoy sobre `__ore_dataset`;
      B = la vista traducida a SQL (`linaje::como_sql`, el ejemplo `como_sql`),
          creada como `CREATE VIEW` con sus nombres del árbol —cada base un
          catálogo (`ATTACH`), cada schema el suyo y `default` = `main`— junto
          con todo lo que lee, y leída entera.
      ¿Mismas filas? Las que no, por qué.
  S2  lo que el nombrado del árbol en DuckDB no admite: un dataset y una vista
      con el mismo nombre (v1alpha12 lo permitía), nombres que DuckDB no crea.
  S3  las vistas SQL de la conformance de v1alpha14 que leen datasets: su
      `spec.sql` tal cual, con los nombres del árbol, ¿corre?
  S4  hecho el paso 4a: la consulta que se servía (`a_sql`, el binario de
      `ORE_ANTES`) contra la que se sirve ahora (`ore_core::servir`), con los
      mismos datos sintéticos en `__ore_dataset`. ¿Mismas filas?

Mide y dice; no falla. Necesita `ore` y `examples/como_sql` en target/release, y
python con duckdb y pyyaml.
"""
import collections
import glob
import json
import os
import random
import re
import subprocess

import duckdb
import yaml

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = ".exe" if os.name == "nt" else ""
ORE = os.path.join(RAIZ, "target", "release", "ore" + EXE)
COMO = os.path.join(RAIZ, "target", "release", "examples", "como_sql" + EXE)

TIPOS = {
    "Integer": "BIGINT",
    "String": "VARCHAR",
    "Boolean": "BOOLEAN",
    "Decimal": "DECIMAL(18,2)",
    "Double": "DOUBLE",
    "Float": "DOUBLE",
    "Date": "DATE",
    "Timestamp": "TIMESTAMP",
}


def correr(*args):
    r = subprocess.run(list(args), capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=120)
    return r.returncode, r.stdout, r.stderr


def q(ident):
    return '"%s"' % ident.replace('"', '""')


def arboles():
    out = []
    for cfg in glob.glob(os.path.join(RAIZ, "**", "ontology.config.yaml"), recursive=True):
        if os.sep + "target" + os.sep in cfg or "node_modules" in cfg:
            continue
        out.append(os.path.dirname(cfg))
    return sorted(out)


def datasets_del_arbol(arbol):
    """qname → (schema, [(columna, tipo)]) de cada Dataset, por el índice de assets."""
    rc, out, _ = correr(ORE, "assets", arbol, "--json")
    if rc != 0:
        return {}
    try:
        items = json.loads(out)["items"]
    except Exception:  # noqa: BLE001
        return {}
    ds = {}
    for k, v in items.items():
        if not k.startswith("dataset:"):
            continue
        qn = k[len("dataset:"):]
        ds[qn] = (v.get("schema") or "default", [(c["name"], c.get("type")) for c in v.get("expone", [])])
    return ds


def vistas_como_sql(arbol):
    rc, out, _ = correr(COMO, arbol)
    vs = {}
    for linea in out.splitlines():
        partes = linea.split("\t", 2)
        if len(partes) == 3:
            vs[partes[0]] = (partes[1], partes[2].replace("\\n", "\n"))
    return vs


def literales(textos):
    out = set()
    for t in textos:
        out.update(re.findall(r"'([^']*)'", t))
    return sorted(out)


def valor(tipo, pool, rnd):
    if tipo in ("Integer",):
        nums = [int(x) for x in pool if re.fullmatch(r"-?\d+", x)]
        return rnd.choice(nums + list(range(0, 12)))
    if tipo == "Boolean":
        return rnd.choice([True, False])
    if tipo in ("Decimal", "Double", "Float"):
        return round(rnd.uniform(0, 1000), 2)
    if tipo == "Date":
        return "2026-0%d-1%d" % (rnd.randint(1, 9), rnd.randint(0, 9))
    if tipo == "Timestamp":
        return "2026-0%d-1%d 10:00:00" % (rnd.randint(1, 9), rnd.randint(0, 9))
    return rnd.choice(pool + ["x", "y", "z"]) if pool else rnd.choice(["x", "y", "z"])


def partes(qn, schema):
    """El nombre del árbol en DuckDB: (base, schema, nombre); `default` es `main`."""
    ps = qn.split(".")
    base, nombre = ps[0], ps[-1]
    sc = ps[1] if len(ps) == 3 else schema
    return base, ("main" if sc in (None, "", "default") else sc), nombre


def cargar(con, ds, pool, seed, filas=80):
    """Cada dataset con filas sintéticas: en `__ore_dataset` (A) y con su nombre (B)."""
    con.execute("CREATE SCHEMA IF NOT EXISTS __ore_dataset")
    bases = set()
    for qn, (schema, cols) in sorted(ds.items()):
        if not cols:
            continue
        rnd = random.Random(seed + qn)
        defs = ", ".join("%s %s" % (q(c), TIPOS.get(t or "", "VARCHAR")) for c, t in cols)
        con.execute('CREATE TABLE "__ore_dataset".%s (%s)' % (q(qn), defs))
        marcas = ", ".join("?" for _ in cols)
        con.executemany(
            'INSERT INTO "__ore_dataset".%s VALUES (%s)' % (q(qn), marcas),
            [[valor(t, pool, rnd) for _, t in cols] for _ in range(filas)],
        )
        base, sc, n = partes(qn, schema)
        if base not in bases:
            con.execute("ATTACH ':memory:' AS %s" % q(base))
            bases.add(base)
        con.execute("CREATE SCHEMA IF NOT EXISTS %s.%s" % (q(base), q(sc)))
        con.execute('CREATE TABLE %s.%s.%s AS SELECT * FROM "__ore_dataset".%s' % (q(base), q(sc), q(n), q(qn)))
    return bases


def crear_vistas(con, vistas, bases):
    """Las vistas del árbol con sus nombres, en el orden en que se dejan crear."""
    pendientes = dict(vistas)
    fallos = {}
    choques = []
    while pendientes:
        avanzo = False
        for qn, (schema, sql) in list(pendientes.items()):
            base, sc, n = partes(qn, schema)
            try:
                if base not in bases:
                    con.execute("ATTACH ':memory:' AS %s" % q(base))
                    bases.add(base)
                con.execute("CREATE SCHEMA IF NOT EXISTS %s.%s" % (q(base), q(sc)))
                existe = con.execute(
                    "SELECT count(*) FROM duckdb_tables() WHERE database_name = ? AND schema_name = ? AND table_name = ?",
                    [base, sc, n],
                ).fetchone()[0]
                if existe:
                    choques.append(qn)
                    del pendientes[qn]
                    continue
                pass  # sin USE: los nombres de dos partes resuelven al `main` de su base
                con.execute("CREATE VIEW %s.%s.%s AS %s" % (q(base), q(sc), q(n), sql))
                con.execute("USE memory.main")
                del pendientes[qn]
                avanzo = True
            except Exception as e:  # noqa: BLE001
                con.execute("USE memory.main")
                fallos[qn] = str(e).splitlines()[0][:140]
        if not avanzo:
            break
    return {k: fallos.get(k, "?") for k in pendientes}, choques


def filas_de(con, sql):
    return sorted(tuple("NULL" if x is None else str(x) for x in r) for r in con.execute(sql).fetchall())


def s1_s2():
    print("S1 · servir la consulta: la de hoy (A) contra la vista como SQL con sus nombres (B)")
    total = servidas = iguales = 0
    motivos = collections.Counter()
    distintas, ejemplos = [], {}
    choques_total = []
    for arbol in arboles():
        vistas = vistas_como_sql(arbol)
        if not vistas:
            continue
        ds = datasets_del_arbol(arbol)
        rel = os.path.relpath(arbol, RAIZ)
        for qn in sorted(vistas):
            total += 1
            rc, out, err = correr(ORE, "ask", arbol, "--vista", qn, "--sql")
            if rc != 0:
                m = " ".join((err or out).split())
                clave = ("lee de una Table" if "no de un dataset" in m else
                         "vista SQL (paso 4)" if "vista SQL" in m else
                         "el árbol no compila" if "OOS" in m or "error" in m else m[:60])
                motivos["hoy no se sirve: " + clave] += 1
                continue
            servidas += 1
            j = json.loads(out.strip().splitlines()[-1])
            pool = literales([j["consulta"]] + [s for _, s in vistas.values()])
            con = duckdb.connect()
            try:
                bases = cargar(con, ds, pool, seed=qn)
                cols = list(j["columnas"].keys())
                try:
                    a = filas_de(con, "SELECT %s FROM (%s)" % (", ".join(q(c) for c in cols), j["consulta"]))
                except Exception as e:  # noqa: BLE001
                    motivos["A no corre (datos sintéticos)"] += 1
                    ejemplos.setdefault("A no corre", "%s · %s" % (qn, str(e).splitlines()[0][:140]))
                    continue
                fallos, choques = crear_vistas(con, vistas, bases)
                choques_total.extend((rel, c) for c in choques)
                if qn in fallos or qn in choques:
                    clave = "B: choque de nombre con un dataset" if qn in choques else "B no se crea"
                    motivos[clave] += 1
                    ejemplos.setdefault(clave, "%s · %s · %s" % (rel, qn, fallos.get(qn, "")))
                    continue
                base, sc, n = partes(qn, vistas[qn][0])
                try:
                    b = filas_de(con, "SELECT %s FROM %s.%s.%s" % (", ".join(q(c) for c in cols), q(base), q(sc), q(n)))
                except Exception as e:  # noqa: BLE001
                    motivos["B no corre"] += 1
                    ejemplos.setdefault("B no corre", "%s · %s · %s" % (rel, qn, str(e).splitlines()[0][:140]))
                    continue
                if a == b:
                    iguales += 1
                else:
                    distintas.append((rel, qn, len(a), len(b)))
            finally:
                con.close()
    print("  · %d Views · hoy se sirven %d · A == B %d · distintas %d" % (total, servidas, iguales, len(distintas)))
    for k, v in motivos.most_common():
        print("    - %d × %s" % (v, k))
    for k, v in ejemplos.items():
        print("      %s: %s" % (k, v))
    for d in distintas[:8]:
        print("    ≠ %s %s · A %d filas · B %d filas" % d)
    print("S2 · choques de nombre al crear con los nombres del árbol: %d" % len(choques_total))
    for c in choques_total[:6]:
        print("    - %s · %s" % c)


def s3():
    print("S3 · las vistas SQL de v1alpha14 sobre datasets: su `spec.sql` tal cual")
    for arbol in sorted(glob.glob(os.path.join(RAIZ, "vendor", "oos", "conformance", "v1alpha14", "valid", "*", "input"))):
        vistas = {}
        for f in glob.glob(os.path.join(arbol, "**", "*.yaml"), recursive=True):
            d = yaml.safe_load(open(f, encoding="utf-8"))
            if isinstance(d, dict) and d.get("kind") == "View" and "sql" in (d.get("spec") or {}):
                m = d["metadata"]
                vistas["%s.%s" % (m["namespace"], m["name"])] = (m.get("schema") or "default", d["spec"]["sql"])
        ds = datasets_del_arbol(arbol)
        if not vistas or not ds:
            continue
        con = duckdb.connect()
        bases = cargar(con, ds, literales([s for _, s in vistas.values()]), seed="s3")
        fallos, choques = crear_vistas(con, vistas, bases)
        rel = os.path.basename(os.path.dirname(arbol))
        for qn in sorted(vistas):
            if qn in fallos:
                print("  · %s · %s: no se crea · %s" % (rel, qn, fallos[qn]))
                continue
            base, sc, n = partes(qn, vistas[qn][0])
            try:
                r = con.execute("SELECT count(*) FROM %s.%s.%s" % (q(base), q(sc), q(n))).fetchone()[0]
                print("  · %s · %s: corre · %d filas" % (rel, qn, r))
            except Exception as e:  # noqa: BLE001
                print("  · %s · %s: no corre · %s" % (rel, qn, str(e).splitlines()[0][:140]))
        con.close()


def s4():
    antes = os.environ.get("ORE_ANTES")
    if not antes or not os.path.exists(antes):
        print("S4 · sin `ORE_ANTES`: no hay binario de antes con que comparar")
        return
    print("S4 · lo que se servía (a_sql) contra lo que se sirve (servir)")
    total = iguales = 0
    distintas, fallos = [], collections.Counter()
    for arbol in arboles():
        vistas = vistas_como_sql(arbol)
        if not vistas:
            continue
        ds = None
        for qn in sorted(vistas):
            ra = correr(antes, "ask", arbol, "--vista", qn, "--sql")
            if ra[0] != 0:
                continue
            total += 1
            rb = correr(ORE, "ask", arbol, "--vista", qn, "--sql")
            if rb[0] != 0:
                fallos["ahora no se sirve: " + " ".join(rb[2].split())[:80]] += 1
                continue
            ja = json.loads(ra[1].strip().splitlines()[-1])
            jb = json.loads(rb[1].strip().splitlines()[-1])
            if ds is None:
                ds = datasets_del_arbol(arbol)
            con = duckdb.connect()
            try:
                cargar(con, ds, literales([ja["consulta"], jb["consulta"]]), seed=qn)
                cols = ", ".join(q(c) for c in ja["columnas"])
                a = filas_de(con, "SELECT %s FROM (%s)" % (cols, ja["consulta"]))
                b = filas_de(con, "SELECT %s FROM (%s)" % (cols, jb["consulta"]))
                if a == b and sorted(ja["datasets"]) == sorted(jb["datasets"]) and ja["columnas"] == jb["columnas"]:
                    iguales += 1
                else:
                    distintas.append((os.path.relpath(arbol, RAIZ), qn, len(a), len(b)))
            except Exception as e:  # noqa: BLE001
                fallos["no corre: " + str(e).splitlines()[0][:80]] += 1
            finally:
                con.close()
    print("  · %d vistas que se servían · iguales (filas, datasets, columnas) %d · distintas %d" % (total, iguales, len(distintas)))
    for k, v in fallos.most_common():
        print("    - %d × %s" % (v, k))
    for d in distintas:
        print("    ≠ %s %s · antes %d filas · ahora %d filas" % d)


def main():
    if not (os.path.exists(ORE) and os.path.exists(COMO)):
        print("⛔ faltan `ore` o `examples/como_sql` en target/release")
        return
    if os.environ.get("ORE_SOLO_S4"):
        s4()
        return
    s1_s2()
    s3()
    s4()


if __name__ == "__main__":
    main()
