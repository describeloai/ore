"""ORE 0068 C2·3 2b·3 · Lo que leen los SQL transforms del laboratorio: dos datasets geo en EPSG:25830,
escritos con el SDK pesado como ana (`geo.lugares`, 30 puntos; `geo.zonas`, 4 buffers de 150 m).
Corre en `ore-puesto-spark:local` en la red del banco (`medir.sh`).
"""
import os
import sys

os.environ.update({"ORE_MOTOR": "heavyweight", "ORE_SERVE": "http://127.0.0.1:8080",
                   "ORE_SUJETO": "persona:ana", "ORE_MEMORIA_MB": "4096"})
sys.path.insert(0, "/opt/ore")
import ore  # noqa: E402
from ore import spark as pesado  # noqa: E402

ore.session._cabeceras = {"x-ore-sujeto": "persona:ana"}
s = pesado.sesion()
a = ore.write("geo.lugares", s.sql("select id, concat('p', id) nombre, "
                                   "ST_SetSRID(ST_Point(440000 + id * 40, 4474000 + (id % 3) * 40), 25830) geom "
                                   "from range(30)"))
b = ore.write("geo.zonas", s.sql("select id, ST_SetSRID(ST_Buffer(ST_Point(440000 + id * 400, 4474000), 150), 25830) zona "
                                 "from range(4)"))
print("sembrado · geo.lugares %d filas · geo.zonas %d filas" % (a["rows"], b["rows"]))
