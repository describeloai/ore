"""Query Q4·4 · La escalera de Q0 como datasets del lago de victor (una vez, antes de medir).

Un trabajador de Query sólo lee datasets del lago, con la credencial que `ore-serve` presta para
cada uno: el Parquet crudo del banco que leía Q0 no lo puede leer (y está bien que no). Esto copia
`trip` a 6, 60 y 600 M filas —el mismo corte que Q0, `t_tripkey <= N·10⁶`— y `customer`.

Se pega en una celda de una **sesión Heavyweight** (Spark: 600 M filas no caben en una sesión
ligera) de un repositorio de victor, con la base `query` ya creada en la consola. Tarda del orden
de un cuarto de hora (600 M filas, ~39 GB); lo escribe la persona, con su identidad.
"""
import time

import ore
from ore import spark as ore_spark

BASE = "query"
BANCO = "gs://project-8853a180-450d-47be-b83-banco-geo/spatialbench/sf100/"

s = ore_spark.sesion()
trip = s.read.parquet(BANCO + "trip.parquet")
for millones in (6, 60, 600):
    t0 = time.time()
    ore.write("%s.trip_%dm" % (BASE, millones), trip.where("t_tripkey <= %d" % (millones * 1_000_000)))
    print("trip_%dm · %.0f s" % (millones, time.time() - t0), flush=True)
t0 = time.time()
ore.write("%s.customer" % BASE, s.read.parquet(BANCO + "customer.parquet"))
print("customer · %.0f s" % (time.time() - t0), flush=True)
