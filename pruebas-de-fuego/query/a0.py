"""Query A0 · ¿Pesa bien el plan de DuckDB? (docs/query.md §3.7)

Por cada caso: `EXPLAIN (FORMAT JSON)` —lo que el optimizador estima, sin ejecutar, y cuánto tarda en
decirlo— y `EXPLAIN ANALYZE (FORMAT JSON)` —lo que de verdad salió—, operador a operador. Una línea
JSON por caso: la cardinalidad estimada y la real del operador mayor y de la raíz, y el plan resumido
(los JSON quedan como casos de prueba de A2).

  ESCALON=grande MEMORIA_MB=38912 python3 a0.py        (lo lanza medir.sh a0)

Sobre el mismo motor que Q0 (`consultas.motor()`: el del puesto, leyendo el Parquet del banco).
"""
import json
import time

import consultas as q0

C = q0.motor()
C.execute("create or replace view trip as select * from read_parquet('%s') where t_tripkey <= 60000000" % q0.TRIP)
C.execute("create or replace view trip6 as select * from read_parquet('%s') where t_tripkey <= 6000000" % q0.TRIP)

GEN = """select range as t_tripkey, cast(hash(range, 1) % 1500000 as bigint) as t_custkey,
  round(cast(hash(range, 5) % 30000 as double) / 1000, 3) as t_distance,
  -74.25 + cast(hash(range, 7) % 500000 as double) / 1000000 as t_pickup_lon
from range({n})"""

# (nombre, sql, analizar: si se ejecuta para ver lo real)
CASOS = [(n, sql.format(T="trip"), True) for n, _, sql in q0.CONSULTAS]
CASOS += [
    ("genera_3m", "create or replace temp table g3 as " + GEN.format(n=3_000_000), True),
    ("genera_30m", "create or replace temp table g30 as " + GEN.format(n=30_000_000), True),
    ("genera_300m", "create or replace temp table g300 as " + GEN.format(n=300_000_000), False),
    ("junta_que_multiplica", "select count(*) from trip6 t join trip6 u on t.t_custkey = u.t_custkey", True),
    ("junta_que_filtra",
     "select count(*) from trip t join customer c on c.c_custkey = t.t_custkey where c.c_nation = 'SPAIN'", True),
    ("escribe_lo_leido", "create or replace temp table w as select t_custkey, t_fare from trip where t_fare > 20", True),
]


def nodos(n, out):
    """Aplana el árbol del plan: (operador, estimada, real)."""
    if isinstance(n, list):
        for x in n:
            nodos(x, out)
        return out
    if not isinstance(n, dict):
        return out
    ex = n.get("extra_info") or {}
    est = ex.get("Estimated Cardinality") if isinstance(ex, dict) else None
    try:
        est = int(str(est).replace("~", "")) if est is not None else None
    except ValueError:
        est = None
    nombre = n.get("name") or n.get("operator_name") or n.get("operator_type")
    if nombre:
        out.append({"op": str(nombre).strip(), "est": est, "real": n.get("operator_cardinality")})
    for h in n.get("children") or []:
        nodos(h, out)
    return out


def plan(sql, analizar):
    t = time.time()
    filas = C.execute(("explain (analyze, format json) " if analizar else "explain (format json) ") + sql).fetchall()
    ms = round((time.time() - t) * 1000)
    return json.loads(filas[0][-1]), ms


for nombre, sql, analizar in CASOS:
    r = {"caso": nombre}
    try:
        p, r["explain_ms"] = plan(sql, False)
        est = nodos(p, [])
        r["est_raiz"] = est[0]["est"] if est else None
        r["est_max"] = max((x["est"] or 0 for x in est), default=0)
        if analizar:
            pa, r["analyze_ms"] = plan(sql, True)
            reales = nodos(pa, [])
            r["real_max"] = max((x["real"] or 0 for x in reales), default=0)
            r["plan_real"] = reales
        r["plan"] = est
    except Exception as e:  # el caso falla, la medida sigue
        r["error"] = str(e)[:300]
    print(json.dumps(r, ensure_ascii=False), flush=True)
