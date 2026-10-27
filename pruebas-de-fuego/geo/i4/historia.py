"""ORE 0072 I4·2 · una tabla con historia en el banco, para ver al Job seguirla (`e2e.py`): 350 000 viajes de
SpatialBench en cuatro ficheros por tiempo (A el más viejo, B1, B2, C) y cuatro instantáneas:

  S0  A + B1 + B2                 (la base)
  S1  + C                         (una carga)
  S2  borrar 500 viajes de B1     (copy-on-write: B1 sale, B1' entra)
  S3  borrar lo más viejo         (retención: A sale entero)

Corre en python:3.12-slim con PyIceberg y DuckDB; el token por el entorno (no se imprime). Imprime
`INSTANTANEAS|[…]` con el `metadata.json` de cada una, en orden.

  docker run --rm -e TOKEN=… -e RAIZ=i4/e2e-… -v <i4>:/i4 python:3.12-slim sh -c \\
    "pip -q install 'pyiceberg[gcsfs,sql-sqlite]' duckdb==1.5.6 pyarrow && python -I /i4/historia.py"
"""
import json
import os
import time

import duckdb
import pyarrow.parquet as pq
from pyiceberg.catalog.sql import SqlCatalog
from pyiceberg.expressions import In

PROYECTO = "project-8853a180-450d-47be-b83"
CUBO = PROYECTO + "-banco-geo"
HTTPS = "https://storage.googleapis.com/" + CUBO
RAIZ = os.environ["RAIZ"]
TOK = os.environ["TOKEN"]
props = {"gcs.oauth2.token": TOK, "gcs.project-id": PROYECTO,
         "gcs.oauth2.token-expires-at": str(int((time.time() + 3000) * 1000))}

con = duckdb.connect()
for e in ("spatial", "httpfs"):
    con.execute("install " + e)
    con.execute("load " + e)
con.execute("create secret b (type http, bearer_token '%s', scope '%s/')" % (TOK.replace("'", "''"), HTTPS))
con.execute(f"""create table v as select t_tripkey, t_pickuptime, t_fare, t_pickuploc
                from read_parquet('{HTTPS}/spatialbench/sf100/trip.parquet') limit 350000""")
con.execute("create table w as select *, row_number() over (order by t_pickuptime, t_tripkey) n from v")
trozos = {"a": (0, 100000), "b1": (100000, 200000), "b2": (200000, 300000), "c": (300000, 350000)}
import gcsfs  # noqa: E402

fs = gcsfs.GCSFileSystem(token=TOK)
uris = {}
for k, (a, b) in trozos.items():
    local = f"/tmp/{k}.parquet"
    con.execute(f"copy (select t_tripkey, t_pickuptime, t_fare, t_pickuploc from w where n > {a} and n <= {b}) "
                f"to '{local}' (format parquet, row_group_size 100000)")
    uris[k] = f"gs://{CUBO}/{RAIZ}/datos/{k}.parquet"
    fs.put(local, uris[k].removeprefix("gs://"))
corte_a = con.execute("select max(t_pickuptime) from w where n <= 100000").fetchone()[0]
claves_b1 = [r[0] for r in con.execute("select t_tripkey from w where n > 100000 and n <= 200000 "
                                         "using sample reservoir(500 rows) repeatable (7)").fetchall()]

cat = SqlCatalog("e2e", uri="sqlite:////tmp/cat.db", warehouse=f"gs://{CUBO}/{RAIZ}/tablas", **props)
cat.create_namespace_if_not_exists("i4")
t = cat.create_table("i4.viajes", schema=pq.read_schema("/tmp/a.parquet"))
pasos = []
t.add_files([uris["a"], uris["b1"], uris["b2"]])
t = cat.load_table("i4.viajes")
pasos.append(t.metadata_location)
t.add_files([uris["c"]])
t = cat.load_table("i4.viajes")
pasos.append(t.metadata_location)
t.delete(In("t_tripkey", set(claves_b1)))
t = cat.load_table("i4.viajes")
pasos.append(t.metadata_location)
# A es lo más viejo: todo lo de antes del corte (incluido) sale con su fichero
t.delete("t_pickuptime <= '%s'" % str(corte_a).replace(" ", "T"))
t = cat.load_table("i4.viajes")
pasos.append(t.metadata_location)
print("INSTANTANEAS|" + json.dumps(pasos), flush=True)
print("FILAS|" + json.dumps([cat.load_table("i4.viajes").scan().to_arrow().num_rows]), flush=True)
