"""MEDIDA · la vista del lado del código como SQL (antes de tocar OOS).

La pregunta: una View del lado del código (antes de promocionar) que sea TEXTO
SQL —cualquier `select`—, ¿qué se pierde y qué se puede seguir sabiendo de ella
sin que nadie lo declare? Hoy la View es una forma estructurada y cerrada
(`from` · `fields` · `where` · `groupBy` · `having`, v1alpha13).

  V1  las Views que existen (el repositorio): qué parte de la forma usan
  V2  un corpus de `select` típicos de vistas (Databricks, Snowflake, dbt):
      ¿cabe cada uno en la forma de hoy? y si no, ¿por qué?
  V3  ¿se sabe su ESQUEMA de salida sin datos? (DuckDB `describe` sobre las
      fuentes vacías, con sus columnas): columnas y tipos, y cuánto cuesta
  V4  ¿se sabe su LINAJE por columna? (el AST de DuckDB, `json_serialize_sql`):
      de qué columnas de qué fuente sale cada columna de salida, directa o
      derivada
  V5  lo que el AST no deja ver (`select *`, funciones de tabla, subconsultas)

Mide y dice; no falla. Necesita python con duckdb y pyyaml (si no, V1 cuenta
claves con una regex).
"""
import glob
import json
import os
import re
import sys
import time

import duckdb

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# ── las fuentes: una base `ventas` con tres datasets ─────────────────────────
FUENTES = {
    "pedidos": {"id": "BIGINT", "cliente_id": "BIGINT", "pais": "VARCHAR", "total": "DECIMAL(12,2)", "fecha": "DATE", "estado": "VARCHAR"},
    "clientes": {"id": "BIGINT", "nombre": "VARCHAR", "email": "VARCHAR", "segmento": "VARCHAR", "alta": "TIMESTAMP", "pais": "VARCHAR"},
    "lineas": {"pedido_id": "BIGINT", "producto": "VARCHAR", "cantidad": "INTEGER", "precio": "DECIMAL(10,2)"},
}

# ── V2 · el corpus: lo que la gente escribe en CREATE VIEW ───────────────────
CORPUS = [
    ("proyección", "select id, pais, total from ventas.s.pedidos"),
    ("renombrar", "select id as pedido, total as importe from ventas.s.pedidos"),
    ("filtro igualdad", "select id, total from ventas.s.pedidos where pais = 'ES' and estado in ('pagado', 'enviado')"),
    ("agregado", "select pais, count(*) as n, sum(total) as total from ventas.s.pedidos group by pais"),
    ("having", "select pais, count(*) as n from ventas.s.pedidos group by pais having count(*) >= 8"),
    ("distinct", "select distinct pais from ventas.s.clientes"),
    ("filtro rango", "select id, total from ventas.s.pedidos where fecha >= date '2026-01-01'"),
    ("filtro like", "select id, email from ventas.s.clientes where email like '%@empresa.com'"),
    ("filtro or", "select id from ventas.s.pedidos where pais = 'ES' or pais = 'PT'"),
    ("expresión", "select id, total * 1.21 as con_iva from ventas.s.pedidos"),
    ("case", "select id, case when total > 100 then 'grande' else 'normal' end as tamano from ventas.s.pedidos"),
    ("función", "select id, upper(nombre) as nombre, date_trunc('month', alta) as mes from ventas.s.clientes"),
    ("cast", "select id, cast(total as double) as total from ventas.s.pedidos"),
    ("join", "select p.id, c.nombre, p.total from ventas.s.pedidos p join ventas.s.clientes c on c.id = p.cliente_id"),
    ("left join + agregado", "select c.id, c.nombre, count(p.id) as pedidos, coalesce(sum(p.total), 0) as gastado from ventas.s.clientes c left join ventas.s.pedidos p on p.cliente_id = c.id group by c.id, c.nombre"),
    ("tres tablas", "select p.id, c.segmento, sum(l.cantidad * l.precio) as bruto from ventas.s.pedidos p join ventas.s.clientes c on c.id = p.cliente_id join ventas.s.lineas l on l.pedido_id = p.id group by p.id, c.segmento"),
    ("ventana", "select id, pais, total, rank() over (partition by pais order by total desc) as puesto from ventas.s.pedidos"),
    ("cte", "with grandes as (select * from ventas.s.pedidos where total > 100) select pais, count(*) as n from grandes group by pais"),
    ("union", "select id, pais from ventas.s.pedidos union all select id, pais from ventas.s.clientes"),
    ("subconsulta en where", "select id, nombre from ventas.s.clientes where id in (select cliente_id from ventas.s.pedidos where total > 500)"),
    ("select *", "select * from ventas.s.clientes where segmento = 'pyme'"),
    ("order/limit", "select id, total from ventas.s.pedidos order by total desc limit 10"),
    ("qualify", "select * from ventas.s.pedidos qualify row_number() over (partition by cliente_id order by fecha desc) = 1"),
    ("pivot duckdb", "pivot ventas.s.pedidos on pais using sum(total) group by estado"),
]

AGREGADOS = {"count", "count_star", "sum", "min", "max", "avg"}


def con_fuentes():
    con = duckdb.connect()
    con.execute("attach ':memory:' as ventas")
    con.execute("create schema ventas.s")
    for t, cols in FUENTES.items():
        con.execute("create table ventas.s.%s (%s)" % (t, ", ".join("%s %s" % c for c in cols.items())))
    return con


def ast(con, q):
    r = con.execute("select json_serialize_sql(?)", [q]).fetchone()[0]
    d = json.loads(r)
    if d.get("error"):
        return None, d.get("error_message")
    return d["statements"][0]["node"], None


def tablas_de(nodo):
    """Las BASE_TABLE del from (con su alias)."""
    out = []

    def ir(f):
        if not f:
            return
        t = f.get("type")
        if t == "BASE_TABLE":
            out.append((f.get("table_name"), f.get("alias") or f.get("table_name")))
        elif t == "JOIN":
            ir(f.get("left")); ir(f.get("right"))
        elif t == "SUBQUERY":
            out.append(("(subconsulta)", f.get("alias")))
        elif t:
            out.append(("(%s)" % t.lower(), f.get("alias")))
    ir(nodo.get("from_table"))
    return out


def refs(e, acc):
    """Las columnas que una expresión nombra: (calificador, columna)."""
    if isinstance(e, dict):
        if e.get("class") == "COLUMN_REF":
            n = e.get("column_names") or []
            acc.append((n[-2] if len(n) > 1 else None, n[-1]))
        if e.get("class") == "SUBQUERY":
            acc.append(("(subconsulta)", "?"))
        for v in e.values():
            refs(v, acc)
    elif isinstance(e, list):
        for v in e:
            refs(v, acc)
    return acc


def cabe_hoy(nodo):
    """¿Cabe en la forma de v1alpha13? Si no, por qué (el primer motivo)."""
    if nodo.get("type") != "SELECT_NODE":
        return False, "no es un select simple (%s)" % nodo.get("type", "?").lower()
    if nodo.get("cte_map", {}).get("map"):
        return False, "with (cte)"
    ts = tablas_de(nodo)
    if len(ts) != 1 or ts[0][0].startswith("("):
        return False, "join/varias fuentes" if len(ts) > 1 else "fuente que no es una tabla (%s)" % (ts[0][0] if ts else "?")
    for m in nodo.get("modifiers", []):
        # `distinct` a secas es `groupBy` sobre todo lo que sale (v1alpha8)
        if m.get("type") == "DISTINCT_MODIFIER" and not m.get("distinct_on_targets"):
            continue
        return False, "order by / limit / distinct on (%s)" % m.get("type", "").lower()
    if nodo.get("qualify"):
        return False, "qualify"
    for e in nodo.get("select_list", []):
        c = e.get("class")
        if c == "COLUMN_REF":
            continue
        if c == "STAR":
            return False, "select * (la forma nombra cada campo)"
        if c in ("AGGREGATE", "FUNCTION") and e.get("function_name") in AGREGADOS and not e.get("distinct") and all(x.get("class") == "COLUMN_REF" for x in e.get("children", [])):
            continue
        if c == "WINDOW":
            return False, "ventana"
        return False, "expresión en la proyección (%s)" % (e.get("function_name") or c.lower())
    w = nodo.get("where_clause")

    def w_ok(x):
        if x is None:
            return True, ""
        c, t = x.get("class"), x.get("type")
        if c == "CONJUNCTION" and t == "CONJUNCTION_AND":
            for h in x.get("children", []):
                ok, m = w_ok(h)
                if not ok:
                    return ok, m
            return True, ""
        if c == "CONJUNCTION":
            return False, "or en el where"
        if c == "COMPARISON" and t == "COMPARE_EQUAL":
            return True, ""
        if c == "COMPARISON":
            return False, "rango/desigualdad en el where (%s)" % t.lower()
        if c == "OPERATOR" and t in ("COMPARE_IN", "OPERATOR_IS_NULL", "OPERATOR_IS_NOT_NULL"):
            if any(ch.get("class") == "SUBQUERY" for ch in x.get("children", [])):
                return False, "subconsulta en el where"
            return True, ""
        if c == "SUBQUERY":
            return False, "subconsulta en el where"
        return False, "predicado %s en el where" % (x.get("function_name") or t or c).lower()
    ok, m = w_ok(w)
    if not ok:
        return False, m
    return True, ""


def linaje(con, nodo, q):
    """Cada columna de salida → las columnas de fuente de las que sale, y si
    es directa (una columna tal cual) o derivada. None si no se resuelve."""
    if nodo.get("type") != "SELECT_NODE" or nodo.get("cte_map", {}).get("map"):
        return None
    ts = tablas_de(nodo)
    if any(t.startswith("(") for t, _ in ts):
        return None
    alias = {a: t for t, a in ts}
    out = {}
    for i, e in enumerate(nodo.get("select_list", [])):
        if e.get("class") == "STAR":
            for t, _ in ts:
                for c in FUENTES[t]:
                    out[c] = ("directa", ["%s.%s" % (t, c)])
            continue
        nombre = e.get("alias") or (e.get("column_names") or ["col%d" % i])[-1]
        cols = []
        for cal, c in refs(e, []):
            if cal == "(subconsulta)":
                return None
            if cal:
                t = alias.get(cal)
            else:
                cand = [t for t, _ in ts if c in FUENTES.get(t, {})]
                t = cand[0] if len(cand) == 1 else None
            if not t or c not in FUENTES.get(t, {}):
                return None
            cols.append("%s.%s" % (t, c))
        out[nombre] = ("directa" if e.get("class") == "COLUMN_REF" else "derivada", sorted(set(cols)))
    return out


def v1():
    print("V1 · las Views del repositorio (qué parte de la forma usan)")
    try:
        import yaml
    except ImportError:
        yaml = None
    ficheros = [f for f in glob.glob(os.path.join(RAIZ, "**", "*.yaml"), recursive=True) if os.sep + "target" + os.sep not in f and "node_modules" not in f]
    cuenta = {"total": 0}
    desde = {}
    for f in ficheros:
        try:
            t = open(f, encoding="utf-8").read()
        except (OSError, UnicodeDecodeError):
            continue
        if not re.search(r"^kind:\s*View\s*$", t, re.M):
            continue
        cuenta["total"] += 1
        spec = None
        if yaml:
            try:
                spec = (yaml.safe_load(t) or {}).get("spec") or {}
            except Exception:  # noqa: BLE001
                spec = None
        if spec is None:
            spec = {k: True for k in re.findall(r"^  (\w+):", t, re.M)}
        for k in ("where", "groupBy", "having", "moved", "reserved"):
            if spec.get(k):
                cuenta[k] = cuenta.get(k, 0) + 1
        fr = spec.get("from") if isinstance(spec.get("from"), dict) else {}
        for k in ("table", "view", "dataset"):
            if k in fr:
                desde[k] = desde.get(k, 0) + 1
        fs = spec.get("fields") if isinstance(spec.get("fields"), dict) else {}
        if any(isinstance(v, str) and "(" in v for v in fs.values()):
            cuenta["agregado en fields"] = cuenta.get("agregado en fields", 0) + 1
    print("  · %d Views · from: %s · usan: %s" % (cuenta.pop("total"), desde, cuenta))


def main():
    v1()
    con = con_fuentes()
    print("V2 · el corpus (%d select típicos de vistas): ¿cabe en la forma de hoy?" % len(CORPUS))
    caben, motivos, filas = 0, {}, []
    for nombre, q in CORPUS:
        nodo, err = ast(con, q)
        if nodo is None:
            filas.append((nombre, "no analiza: %s" % err))
            motivos["el AST de DuckDB no lo da"] = motivos.get("el AST de DuckDB no lo da", 0) + 1
            continue
        ok, m = cabe_hoy(nodo)
        caben += ok
        if not ok:
            motivos[m.split(" (")[0]] = motivos.get(m.split(" (")[0], 0) + 1
        filas.append((nombre, "cabe" if ok else "NO · " + m))
    for n, r in filas:
        print("  · %-22s %s" % (n, r))
    print("  → caben %d de %d · lo que no: %s" % (caben, len(CORPUS), dict(sorted(motivos.items(), key=lambda x: -x[1]))))

    print("V3 · su esquema de salida, sin datos (describe sobre las fuentes vacías)")
    bien, ms = 0, []
    for nombre, q in CORPUS:
        t0 = time.perf_counter()
        try:
            cols = con.execute("describe " + q).fetchall()
            bien += 1
            ms.append((time.perf_counter() - t0) * 1000)
            if nombre in ("left join + agregado", "ventana", "pivot duckdb", "cte"):
                print("  · %-22s %s" % (nombre, [(c[0], c[1]) for c in cols]))
        except Exception as e:  # noqa: BLE001
            print("  · %-22s ERROR %s" % (nombre, " ".join(str(e).split())[:120]))
    ms.sort()
    print("  → %d de %d con esquema · %.1f ms la mediana, %.1f la peor" % (bien, len(CORPUS), ms[len(ms) // 2], ms[-1]))

    print("V4 · su linaje por columna (el AST, sin binder)")
    resueltas, total_v, directas, derivadas = 0, 0, 0, 0
    for nombre, q in CORPUS:
        nodo, _ = ast(con, q)
        if nodo is None:
            print("  · %-22s sin AST" % nombre)
            continue
        total_v += 1
        l = linaje(con, nodo, q)
        if l is None:
            print("  · %-22s no se resuelve (cte, union, subconsulta, función de tabla…)" % nombre)
            continue
        resueltas += 1
        directas += sum(1 for v in l.values() if v[0] == "directa")
        derivadas += sum(1 for v in l.values() if v[0] == "derivada")
        if nombre in ("join", "left join + agregado", "tres tablas", "case", "select *"):
            print("  · %-22s %s" % (nombre, {k: "%s ← %s" % (v[0], ", ".join(v[1])) for k, v in l.items()}))
    print("  → %d de %d vistas con linaje por columna (con un AST sin binder; el binder de DuckDB o un resolvedor en Rust llega más lejos) · %d columnas directas, %d derivadas" % (resueltas, total_v, directas, derivadas))

    print("V5 · las fuentes de cada una (lo que el analizador del árbol ya saca: `sql_del_arbol`)")
    for nombre, q in CORPUS:
        nodo, _ = ast(con, q)
        if nodo is not None and nombre in ("cte", "union", "subconsulta en where", "pivot duckdb"):
            ts = set()
            refs_tabla = re.findall(r"ventas\.s\.(\w+)", q)
            print("  · %-22s lee %s" % (nombre, sorted(set(refs_tabla))))


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    main()
