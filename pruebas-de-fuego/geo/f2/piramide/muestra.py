"""ORE 0072 F2·3·2·1 · una muestra de teselas de un PMTiles con la librería oficial (z, x, y y el largo de sus
bytes; -1 si no está), para contrastar el lector de `ore-teselas` (la prueba ignorada
`un_pmtiles_de_tippecanoe_da_las_teselas_de_la_libreria_oficial`).

  python pruebas-de-fuego/geo/f2/piramide/muestra.py pmtiles/trip_3m_puntos.pmtiles > muestra.json
"""
import json
import math
import random
import sys

from pmtiles.reader import MmapSource, Reader

f = open(sys.argv[1], "rb")
r = Reader(MmapSource(f))
c = r.header()
lon0, lat0, lon1, lat1 = c["min_lon_e7"] / 1e7, c["min_lat_e7"] / 1e7, c["max_lon_e7"] / 1e7, c["max_lat_e7"] / 1e7


def xy(lon, lat, z):
    n = 2 ** z
    return int((lon + 180) / 360 * n), int((1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n)


random.seed(7)
out = []
for z in range(c["min_zoom"], c["max_zoom"] + 1):
    (xa, ya), (xb, yb) = xy(lon0, lat1, z), xy(lon1, lat0, z)
    # dentro de la caja de la pirámide y un poco fuera (las que no están también cuentan)
    for _ in range(40):
        x = random.randint(max(0, xa - 1), min(2 ** z - 1, xb + 1))
        y = random.randint(max(0, ya - 1), min(2 ** z - 1, yb + 1))
        b = r.get(z, x, y)
        out.append({"z": z, "x": x, "y": y, "largo": len(b) if b else -1})
json.dump(out, sys.stdout)
