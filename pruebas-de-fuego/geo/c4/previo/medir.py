"""ORE 0068 C4·2 · El previo del generador de DuckDB: cuánto tarda cada forma de SQL de Enrich, con los
límites de un puesto Lightweight (2 CPU, 4 GiB), antes de escribir el generador.

Datos sintéticos con forma de España en EPSG:25830 (metros, 600 × 600 km), generados aquí:
  municipios   8 000 polígonos de ~500 vértices (círculos de una rejilla con desplazamiento);
  secciones   36 000 polígonos de ~65 vértices, con su población;
  competidores 100 000 puntos;
  clientes     N puntos (`CLIENTES`, por defecto 100 000 y 1 000 000).
Una línea `C4P|caso|n=…|s=…|filas=…|plan=…` por medida. Cada una con su límite (`LIMITE_S`).
"""
import os
import threading
import time

import duckdb

LIMITE = int(os.environ.get("LIMITE_S", "600"))
TAMANOS = [int(x) for x in os.environ.get("CLIENTES", "100000,1000000").split(",")]
SOLO = set(filter(None, os.environ.get("CASOS", "").split(",")))
T0 = time.time()

c = duckdb.connect()
c.execute("set extension_directory = '/opt/ore/duckdb'")
c.execute("load spatial")
c.execute("set threads = 2")
c.execute("set memory_limit = '3GB'")
c.execute("set temp_directory = '/d/tmp'")
print("C4P|version|%s" % c.execute("select version()").fetchone()[0], flush=True)


def linea(caso, **kv):
    print("C4P|%s|%s" % (caso, " ".join("%s=%s" % (k, v) for k, v in kv.items())), flush=True)


t = time.time()
c.execute("""
create table municipios as
select i ine, 'm' || i nombre, (random() * 50000)::int poblacion,
       ST_Buffer(ST_Point(x * 6700 + random() * 2000, y * 6700 + random() * 2000), 4200, 125) geom
from (select i, i % 90 x, i // 90 y from range(8000) r(i))""")
c.execute("""
create table secciones as
select i id, (random() * 2000)::int poblacion, random() * 30000 renta_media,
       ST_Buffer(ST_Point(x * 3160 + random() * 500, y * 3160 + random() * 500), 1900, 16) geom
from (select i, i % 190 x, i // 190 y from range(36000) r(i))""")
c.execute("""
create table competidores as
select i id, 'c' || i nombre, (random() * 1e6)::int ventas, ST_Point(random() * 600000, random() * 600000) geom
from range(100000) r(i)""")
linea("datos", s="%.1f" % (time.time() - t),
      vertices_municipio=c.execute("select avg(ST_NPoints(geom))::int from municipios").fetchone()[0],
      vertices_seccion=c.execute("select avg(ST_NPoints(geom))::int from secciones").fetchone()[0])

# Las formas de SQL, con `t` = los clientes con su `__ore_k`.
CASOS = {
    # value por más solape: el punto no tiene área, así que con varias gana la de menor valor.
    "within": """
      select t.__ore_k, arg_min(l.nombre, l.nombre) municipio
      from t join municipios l on ST_Within(t.geom, l.geom) group by 1""",
    "count500": """
      select t.__ore_k, count(*) n
      from t join competidores l on ST_DWithin(t.geom, l.geom, 500) group by 1""",
    "nearest-lateral": """
      select t.__ore_k, x.nombre, x.d from t, lateral (
        select l.nombre, ST_Distance(t.geom, l.geom) d from competidores l
        where ST_DWithin(t.geom, l.geom, 5000) order by d, l.nombre limit 1) x""",
    "nearest-join": """
      select t.__ore_k, arg_min(l.nombre, ST_Distance(t.geom, l.geom)) nombre,
             min(ST_Distance(t.geom, l.geom)) d
      from t join competidores l on ST_DWithin(t.geom, l.geom, 5000) group by 1""",
    "extensive": """
      select t.__ore_k,
             sum(l.poblacion * ST_Area(ST_Intersection(t.b, l.geom)) / ST_Area(l.geom)) poblacion_500m,
             sum(l.renta_media * ST_Area(ST_Intersection(t.b, l.geom)))
               / sum(ST_Area(ST_Intersection(t.b, l.geom))) renta_500m
      from (select __ore_k, ST_Buffer(geom, 500) b from t) t
      join secciones l on ST_Intersects(t.b, l.geom) group by 1""",
}
# La consulta entera de D-C4-2: el objetivo, todas sus filas, con LEFT JOIN de cada CTE.
CASOS["entera"] = """
  with c1 as (%s), c2 as (%s), c3 as (%s), c4 as (%s)
  select t.* exclude (__ore_k), c1.municipio, coalesce(c2.n, 0) competidores_500m,
         c3.nombre competidor_cercano, c3.d dist_competidor, c4.poblacion_500m, c4.renta_500m
  from t left join c1 using (__ore_k) left join c2 using (__ore_k)
         left join c3 using (__ore_k) left join c4 using (__ore_k)""" % (
    CASOS["within"], CASOS["count500"], CASOS["nearest-join"], CASOS["extensive"])

OPERADORES = ("SPATIAL_JOIN", "PIECEWISE_MERGE_JOIN", "NESTED_LOOP_JOIN", "BLOCKWISE_NL_JOIN",
              "HASH_JOIN", "CROSS_PRODUCT", "IE_JOIN")


def plan(sql):
    try:
        p = "\n".join(r[1] for r in c.execute("explain " + sql).fetchall())
    except Exception as e:  # noqa: BLE001
        return "error:" + str(e)[:80].replace(" ", "_")
    return ",".join(o for o in OPERADORES if o in p) or "-"


for n in TAMANOS:
    c.execute("create or replace table clientes as select i cliente_id, "
              "ST_Point(random() * 600000, random() * 600000) geom from range(%d) r(i)" % n)
    c.execute("create or replace view t as select *, row_number() over () __ore_k from clientes")
    for caso, sql in CASOS.items():
        if SOLO and caso not in SOLO:
            continue
        r = {}
        q = "select count(*) from (%s)" % sql

        def ir():
            try:
                r["n"] = c.execute(q).fetchone()[0]
            except Exception as e:  # noqa: BLE001
                r["e"] = str(e).splitlines()[0][:200].replace(" ", "_")

        t = time.time()
        h = threading.Thread(target=ir, daemon=True)
        h.start()
        h.join(LIMITE)
        if h.is_alive():
            c.interrupt()
            h.join(60)
            linea(caso, n=n, estado="limite", s=LIMITE, plan=plan(sql))
            continue
        s = time.time() - t
        if "e" in r:
            linea(caso, n=n, estado="error", s="%.1f" % s, error=r["e"])
            continue
        linea(caso, n=n, estado="bien", s="%.1f" % s, filas=r["n"], plan=plan(sql),
              filas_por_s="%.0f" % (n / s if s else 0))
linea("fin", s="%.0f" % (time.time() - T0))
