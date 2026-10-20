"""ORE 0068 C4·2 · El cotejo: el SQL que genera ORE (`/x/enrich.sql`), corrido en el DuckDB del puesto sobre
los datos de PostGIS (`/x/*.csv`), contra la respuesta de PostGIS (`/x/postgis.csv`), fila a fila.
Los números, con tolerancia relativa (`TOL`, 1e-6): el buffer lo calcula cada motor a su manera.
"""
import csv
import math
import os
import re
import sys
import time

import duckdb

TOL = float(os.environ.get("TOL", "1e-6"))
c = duckdb.connect()
c.execute("set extension_directory = '/opt/ore/duckdb'")
c.execute("load spatial")
for t, geo in (("municipios", "geom"), ("secciones", "geom"), ("competidores", "geom"), ("clientes", "ubicacion")):
    c.execute(f"create table {t} as select * replace (ST_GeomFromHEXEWKB({geo}) as {geo}) "
              f"from read_csv('/x/{t}.csv', header = true, all_varchar = false)")
sql = open("/x/enrich.sql").read()
# La sentencia del guion → su consulta, sobre las tablas de aquí.
sql = re.sub(r"^create or replace dataset \S+ as\s*", "", sql)
sql = re.sub(r"\bventas\.(\w+)", r"\1", sql)
t0 = time.time()
cur = c.execute(sql)
cols = [d[0] for d in cur.description]
ore = {r[0]: dict(zip(cols, r)) for r in cur.fetchall()}
print("COT|duckdb|%.1f s|%d filas" % (time.time() - t0, len(ore)))

pg = {r["cliente_id"]: r for r in csv.DictReader(open("/x/postgis.csv"))}
if set(pg) != set(ore):
    print("COT|⛔ filas distintas: postgis %d, ore %d" % (len(pg), len(ore)))
    sys.exit(1)


def igual(a, b):
    if b in ("", None):
        return a is None
    if a is None:
        return False
    try:
        x, y = float(a), float(b)
    except (TypeError, ValueError):
        return str(a) == b
    return math.isclose(x, y, rel_tol=TOL, abs_tol=TOL)


malas = {}
for k, fila in pg.items():
    for col, v in fila.items():
        if not igual(ore[k][col], v):
            malas.setdefault(col, []).append((k, ore[k][col], v))
for col in fila:
    n = len(malas.get(col, []))
    no_nulas = sum(1 for f in pg.values() if f[col] != "")
    print("COT|%s|%s|%d distintas de %d (%d con valor)" % ("bien" if n == 0 else "MAL", col, n, len(pg), no_nulas))
    for k, a, b in malas.get(col, [])[:3]:
        print("COT|  %s: ore=%r postgis=%r" % (k, a, b))
sys.exit(1 if malas else 0)
