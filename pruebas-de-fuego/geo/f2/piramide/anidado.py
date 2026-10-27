"""ORE 0072 F2·3·2 · PC3 en local: (1) el lector propio da lo mismo que la librería oficial de PMTiles;
(2) **anidado**: un punto que se ve en z sigue en z+1 (si no, al acercarse desaparece: parpadeo).

  python pruebas-de-fuego/geo/f2/piramide/anidado.py [pmtiles/trip_3m_puntos.pmtiles]
"""
import gzip
import os
import random
import sys

import mapbox_vector_tile
from pmtiles.reader import MmapSource, Reader

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lector_pmtiles import Lector  # noqa: E402

ruta = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "pmtiles", "trip_3m_puntos.pmtiles")
f = open(ruta, "rb")
oficial = Reader(MmapSource(f))
cab = oficial.header()


def leer(desde, largo):
    f.seek(desde)
    return f.read(largo)


propio = Lector(leer)
capa = os.path.basename(ruta).split(".")[0]


def ids(z, x, y, dentro=False):
    """Los ids de la tesela; con `dentro`, sólo los de dentro de sus bordes (no los del margen, que son de las
    teselas de al lado y en z+1 caen en las hijas de otra)."""
    b = propio.tesela(z, x, y)
    if not b:
        return set()
    if propio.cabecera["tesela_gzip"]:
        b = gzip.decompress(b)
    d = mapbox_vector_tile.decode(b).get(capa, {})
    ext = d.get("extent", 4096)
    return {ft["properties"].get("id") for ft in d.get("features", [])
            if not dentro or (0 <= ft["geometry"]["coordinates"][0] < ext and 0 <= ft["geometry"]["coordinates"][1] < ext)}


# (1) el lector: las mismas teselas que la librería oficial, en una muestra de cada zoom
lon0, lat0, lon1, lat1 = cab["min_lon_e7"] / 1e7, cab["min_lat_e7"] / 1e7, cab["max_lon_e7"] / 1e7, cab["max_lat_e7"] / 1e7
import math  # noqa: E402


def xy(lon, lat, z):
    n = 2 ** z
    return int((lon + 180) / 360 * n), int((1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n)


random.seed(1)
iguales = distintas = 0
for z in range(cab["min_zoom"], cab["max_zoom"] + 1):
    (xa, ya), (xb, yb) = xy(lon0, lat1, z), xy(lon1, lat0, z)
    for _ in range(20):
        x, y = random.randint(xa, xb), random.randint(ya, yb)
        if oficial.get(z, x, y) == propio.tesela(z, x, y):
            iguales += 1
        else:
            distintas += 1
print("lector: %d teselas iguales a la librería oficial, %d distintas" % (iguales, distintas))

# (2) anidado: en una muestra de teselas de cada zoom, los ids de z que no están en sus 4 hijas de z+1
for z in range(max(cab["min_zoom"], 8), cab["max_zoom"]):
    (xa, ya), (xb, yb) = xy(lon0, lat1, z), xy(lon1, lat0, z)
    vistos = perdidos = 0
    for _ in range(8):
        x, y = random.randint(xa, xb), random.randint(ya, yb)
        padre = ids(z, x, y, dentro=True) - {None}
        hijas = set()
        for dx in (0, 1):
            for dy in (0, 1):
                hijas |= ids(z + 1, 2 * x + dx, 2 * y + dy)
        vistos += len(padre)
        perdidos += len(padre - hijas)
    print("z%-2d → z%-2d: %7d puntos vistos, %5d desaparecen al acercarse (%.2f %%)"
          % (z, z + 1, vistos, perdidos, 100 * perdidos / max(1, vistos)))
