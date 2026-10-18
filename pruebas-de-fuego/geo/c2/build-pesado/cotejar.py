"""ORE 0068 C2·3 2b·3 · Lo que el build pesado escribió en `geo.conteo`, contra lo mismo calculado sin
Spark (shapely, con la resolución de Sedona: 8 segmentos por cuadrante). Corre tras `medir.sh`, en la
red del banco, como ana.
"""
import os
import sys

os.environ.update({"ORE_MOTOR": "heavyweight", "ORE_SERVE": "http://127.0.0.1:8080",
                   "ORE_SUJETO": "persona:ana", "ORE_MEMORIA_MB": "4096"})
sys.path.insert(0, "/opt/ore")
import ore  # noqa: E402
from shapely.geometry import Point  # noqa: E402

ore.session._cabeceras = {"x-ore-sujeto": "persona:ana"}
lugares = [Point(440000 + i * 40, 4474000 + (i % 3) * 40) for i in range(30)]
esperado = {z: sum(Point(440000 + z * 400, 4474000).buffer(150, quad_segs=8).contains(p) for p in lugares)
            for z in range(4)}
esperado = {k: v for k, v in esperado.items() if v}
escrito = {r.id: r.n for r in ore.over("geo.conteo").collect()}
centros = ore.over("geo.centros").selectExpr("id", "ST_SRID(centro) s", "round(ST_X(centro)) x", "area").orderBy("id").collect()
print("conteo · escrito %s · esperado %s · %s" % (escrito, esperado, "IGUAL" if escrito == esperado else "DISTINTO"))
print("centros · %s" % [(c.id, c.s, c.x, c.area) for c in centros])
