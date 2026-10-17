"""ADR 0067 G2·5 · Los fijos de la matriz de adopción: unas 200 geometrías de todos los subtipos, en
EPSG:25830 y en OGC:CRS84, con su WKB en hexadecimal y lo que cada lector tiene que ver al leerlas
(filas, md5 de los WKB en orden de `id`, bbox). Una fuente para los dos sitios: el arnés local
(`examples/v5-fijos.rs` los sella con el escritor de ORE) y el Job de victor (los escribe con
`ore.write`). Deterministas: se regeneran igual.

    docker run --rm -v <este directorio>:/v5 <imagen puesto-python> python /v5/fijos.py /v5/fijos.json
"""
import hashlib
import json
import sys

import shapely
from shapely.geometry import (GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
                              Point, Polygon)

N_POR_TIPO = 25


def piezas(x0, y0, d, i):
    """Una geometría de cada subtipo, desplazada por `i`; `d` es la escala del CRS."""
    x, y = x0 + i * d, y0 + (i % 5) * d
    cuadro = Polygon([(x, y), (x + d, y), (x + d, y + d), (x, y + d)],
                     [[(x + d / 4, y + d / 4), (x + d / 2, y + d / 4), (x + d / 2, y + d / 2), (x + d / 4, y + d / 2)]])
    return [
        ("Point", Point(x, y)),
        ("LineString", LineString([(x, y), (x + d, y + d / 2), (x + 2 * d, y)])),
        ("Polygon", cuadro),
        ("MultiPoint", MultiPoint([(x, y), (x + d, y + d)])),
        ("MultiLineString", MultiLineString([[(x, y), (x + d, y)], [(x, y + d), (x + d, y + d)]])),
        ("MultiPolygon", MultiPolygon([cuadro, Polygon([(x + 2 * d, y), (x + 3 * d, y), (x + 3 * d, y + d)])])),
        ("GeometryCollection", GeometryCollection([Point(x, y), LineString([(x, y), (x + d, y)])])),
    ]


def fijo(crs, x0, y0, d):
    filas = []
    for i in range(N_POR_TIPO):
        for sub, g in piezas(x0, y0, d, i):
            filas.append({"id": len(filas), "subtipo": sub, "wkb": shapely.to_wkb(g, hex=True, output_dimension=2)})
    # Un vacío y un nulo, al final.
    filas.append({"id": len(filas), "subtipo": "Point(vacío)", "wkb": shapely.to_wkb(shapely.Point(), hex=True)})
    filas.append({"id": len(filas), "subtipo": "nulo", "wkb": None})
    wkbs = [bytes.fromhex(f["wkb"]) for f in filas if f["wkb"]]
    no_vacias = [shapely.from_wkb(w) for w in wkbs]
    no_vacias = [g for g in no_vacias if not g.is_empty]
    bbox = list(shapely.total_bounds(no_vacias))
    return {"crs": crs, "filas": filas, "esperado": {
        "filas": len(filas), "nulos": 1,
        "md5_wkb": hashlib.md5(b"".join(wkbs)).hexdigest(),
        "bbox": [round(v, 6) for v in bbox],
        "subtipos": sorted({f["subtipo"] for f in filas if f["wkb"] and "vacío" not in f["subtipo"]})}}


if __name__ == "__main__":
    salida = sys.argv[1] if len(sys.argv) > 1 else "fijos.json"
    d = {"EPSG:25830": fijo("EPSG:25830", 440000.0, 4474000.0, 100.0),
         "OGC:CRS84": fijo("OGC:CRS84", -3.75, 40.38, 0.001)}
    json.dump(d, open(salida, "w", encoding="utf-8"), indent=0, ensure_ascii=False)
    for k, v in d.items():
        print(k, v["esperado"])
