"""ORE 0067 G3·1 · El corpus del códec de entrada (`geo-corpus.json`), con shapely como oráculo.

Cada caso es una geometría en las cinco formas en que llega de un origen (WKB en hex, EWKB en hex
con SRID 4326, WKT, EWKT y GeoJSON) y el WKB ISO que tiene que salir. Los siete subtipos, con
agujero, con Z, y el punto vacío.

    docker run --rm -v <crates/ore-base/tests>:/t <imagen puesto-python> python /t/geo-corpus.py
"""
import json

import shapely
from shapely.geometry import (GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
                              Point, Polygon, mapping)

c = Polygon([(0, 0), (4, 0), (4, 4), (0, 4)], [[(1, 1), (2, 1), (2, 2), (1, 2)]])
geos = [
    Point(-3.7038, 40.4168),
    Point(440000.5, 4474000.25, 650.0),
    LineString([(0, 0), (1, 1), (2, 0)]),
    LineString([(0, 0, 1), (1, 1, 2)]),
    c,
    MultiPoint([(0, 0), (1, 1)]),
    MultiLineString([[(0, 0), (1, 0)], [(0, 1), (1, 1)]]),
    MultiPolygon([c, Polygon([(10, 10), (11, 10), (11, 11)])]),
    GeometryCollection([Point(1, 2), LineString([(0, 0), (1, 1)])]),
    Point(),
]
casos = []
for g in geos:
    d = 3 if g.has_z else 2
    iso = shapely.to_wkb(g, hex=True, output_dimension=d, byte_order=1, flavor="iso")
    ewkb = shapely.to_wkb(shapely.set_srid(g, 4326), hex=True, output_dimension=d, include_srid=True, byte_order=0)
    wkt = shapely.to_wkt(g, rounding_precision=-1, output_dimension=d)
    formas = {"wkb_hex": iso, "wkb_be_hex": shapely.to_wkb(g, hex=True, output_dimension=d, byte_order=0, flavor="iso"),
              "ewkb_hex": ewkb, "wkt": wkt, "ewkt": "SRID=4326;" + wkt}
    if not g.is_empty:
        formas["geojson"] = json.dumps(mapping(g))
    casos.append({"subtipo": g.geom_type + (" Z" if g.has_z else ""), "iso": iso.upper(), "formas": formas})
json.dump(casos, open("/t/geo-corpus.json", "w"), indent=1)
print(len(casos), "casos")
