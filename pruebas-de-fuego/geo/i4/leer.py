"""ORE 0072 I4·2 · lee unas tablas del índice con DuckDB y su extensión Iceberg —un lector que no es nuestro, que
aplica los borrados por posición— y cuenta sus filas (los trozos 0) y la suma de `t_tripkey`. `MLS`: los
`metadata.json`, en JSON; el token por el entorno. Imprime `LEIDO|[ml, filas, suma]`.
"""
import json
import os

import duckdb

c = duckdb.connect()
for e in ("httpfs", "iceberg"):
    c.execute("install " + e)
    c.execute("load " + e)
c.execute("create secret g (type gcs, bearer_token '%s')" % os.environ["TOKEN"].replace("'", "''"))
for ml in json.loads(os.environ["MLS"]):
    r = c.execute("select count(*), sum(t_tripkey)::varchar from iceberg_scan('%s') where _ore_trozo = 0"
                  % ml.replace("'", "''")).fetchone()
    print("LEIDO|" + json.dumps([ml, r[0], r[1]]), flush=True)
