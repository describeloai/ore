#!/usr/bin/env python3
"""
MEDIDA · CREATE VIEW en el puesto (ADR 0040 paso 5, antes de escribirlo).

El contrato de una vista v1alpha14 (`spec.columns`) lo puede sacar DuckDB del
SELECT sin ejecutarlo: `DESCRIBE <select>`. Aquí se mide qué tipos da DuckDB
para las expresiones de siempre sobre datasets con los físicos de 0032 (como
los registra un puesto: `"__ore_dataset"."p.n"`), cuánto cuesta, y si describe
sin leer una fila.

  D1  los tipos que DuckDB da, expresión por expresión
  D2  DESCRIBE no lee: sobre una tabla de 10 M filas, el mismo tiempo que sobre 0
  D3  nulabilidad: lo que DESCRIBE dice de `null` (y si sirve para algo)
  D4  lo que no se puede describir: una columna sin nombre, dos con el mismo,
      un nombre que no está

Uso:  python pruebas-de-fuego/medida-create-view.py
"""
import sys
import time

import duckdb
import pyarrow as pa

sys.stdout.reconfigure(encoding="utf-8", errors="replace")

con = duckdb.connect()
con.execute("set TimeZone = 'UTC'")
con.execute('create schema "__ore_dataset"')
# Los físicos de 0032 tal como un dataset los trae (Integer=int64, Decimal=
# decimal(38,9)?, String=utf8, DateTimeTz=timestamp[us, UTC], Date, Boolean, Float).
ventas = pa.table({
    "id": pa.array([1, 2, 3], pa.int64()),
    "pais": pa.array(["ES", "FR", "ES"], pa.string()),
    "total": pa.array([1, 2, 3], pa.decimal128(38, 2)),
    "cuando": pa.array([0, 1, 2], pa.timestamp("us", tz="UTC")),
    "dia": pa.array([0, 1, 2], pa.date32()),
    "ok": pa.array([True, False, None], pa.bool_()),
    "peso": pa.array([1.5, 2.5, None], pa.float64()),
})
con.register("_v", ventas)
con.execute('create view "__ore_dataset"."ventas.pedidos" as select * from _v')

EXPRESIONES = [
    ("id", "id"),
    ("pais", "pais"),
    ("total", "total"),
    ("cuando", "cuando"),
    ("dia", "dia"),
    ("ok", "ok"),
    ("peso", "peso"),
    ("count(*)", "count(*) as n"),
    ("count(distinct pais)", "count(distinct pais) as n"),
    ("sum(id)", "sum(id) as s"),
    ("sum(total)", "sum(total) as s"),
    ("avg(id)", "avg(id) as m"),
    ("avg(total)", "avg(total) as m"),
    ("min(cuando)", "min(cuando) as m"),
    ("max(dia)", "max(dia) as m"),
    ("id * 2", "id * 2 as x"),
    ("id / 2", "id / 2 as x"),
    ("id // 2", "id // 2 as x"),
    ("total * 2", "total * 2 as x"),
    ("total / 3", "total / 3 as x"),
    ("round(total, 1)", "round(total, 1) as x"),
    ("cast(id as integer)", "cast(id as integer) as x"),
    ("cast(id as varchar)", "cast(id as varchar) as x"),
    ("upper(pais)", "upper(pais) as x"),
    ("pais || '-'", "pais || '-' as x"),
    ("case when ok then 1 else 0 end", "case when ok then 1 else 0 end as x"),
    ("coalesce(peso, 0)", "coalesce(peso, 0) as x"),
    ("date_trunc('month', cuando)", "date_trunc('month', cuando) as x"),
    ("cuando::date", "cuando::date as x"),
    ("now()", "now() as x"),
    ("current_date", "current_date as x"),
    ("1", "1 as x"),
    ("1.5", "1.5 as x"),
    ("'a'", "'a' as x"),
    ("null", "null as x"),
    ("row_number() over ()", "row_number() over () as x"),
    ("list(pais)", "list(pais) as x"),
    ("{'a': 1}", "{'a': 1} as x"),
]

print("D1 · los tipos que DuckDB da (DESCRIBE, sin ejecutar)")
tipos = {}
for nombre, expr in EXPRESIONES:
    agrega = any(f in expr for f in ("count(", "sum(", "avg(", "min(", "max(", "list("))
    q = 'select %s from "__ore_dataset"."ventas.pedidos"' % expr
    if agrega and "over" not in expr:
        q += ""
    try:
        d = con.execute("describe " + q).fetchall()
        t = d[0][1]
        nulo = d[0][2]
    except Exception as e:
        t, nulo = "✗ " + str(e).splitlines()[0][:70], ""
    tipos[nombre] = t
    print("  %-34s %-28s null=%s" % (nombre, t, nulo))

print()
print("D2 · DESCRIBE no lee filas")
grande = pa.table({"id": pa.array(range(10_000_000), pa.int64()), "g": pa.array([i % 7 for i in range(10_000_000)], pa.int64())})
con.register("_g", grande)
con.execute('create view "__ore_dataset"."ventas.grande" as select * from _g')
for n, tabla in (("0 filas", 'select * from _g limit 0'), ("10 M filas", 'select * from "__ore_dataset"."ventas.grande"')):
    q = "select g, count(*) as n, sum(id) as s from (%s) group by g" % tabla
    t0 = time.perf_counter()
    for _ in range(20):
        con.execute("describe " + q).fetchall()
    ms = (time.perf_counter() - t0) * 1000 / 20
    print("  describe sobre %-12s %.2f ms" % (n, ms))
t0 = time.perf_counter()
con.execute('select g, count(*) as n, sum(id) as s from "__ore_dataset"."ventas.grande" group by g').fetchall()
print("  y ejecutarla de verdad          %.1f ms" % ((time.perf_counter() - t0) * 1000))

print()
print("D3 · nulabilidad: DESCRIBE dice YES en todo lo que viene de una vista")
d = con.execute('describe select id, count(*) as n from "__ore_dataset"."ventas.pedidos" group by id').fetchall()
print("  " + ", ".join("%s null=%s" % (r[0], r[2]) for r in d))

print()
print("D4 · lo que no se puede describir bien")
for q in ('select id + 1 from "__ore_dataset"."ventas.pedidos"',
          'select id, id from "__ore_dataset"."ventas.pedidos"',
          'select nada from "__ore_dataset"."ventas.pedidos"',
          'select * from "__ore_dataset"."ventas.nada"'):
    try:
        d = con.execute("describe " + q).fetchall()
        print("  %-58s → %s" % (q[:58], [r[0] for r in d]))
    except Exception as e:
        print("  %-58s → ✗ %s" % (q[:58], str(e).splitlines()[0][:90]))
print()
print("duckdb", duckdb.__version__)
