"""ORE 0072 F2·3·2·4 · la pirámide que pide `ore-serve` (sin columna id) frente a la del banco con id: si en cada
tesela hay los mismos puntos en el mismo sitio, la selección es la misma, y el anidado medido con ids (0 % de
parpadeo, `anidado.py`) vale para la sin id. Además, la tesela máxima de cada una y un anidado sin ids, por
posición (un punto de z dentro de su tesela, en el mismo píxel de z en alguna hija de z+1): ⚠️ ése CUENTA DE MÁS,
por el redondeo (4096 de extent contra 256 píxeles); medido el 2026-10-26, 1–6 % por posición frente a 0 % por ids
sobre la misma selección. La medida es la de ids.

  python pruebas-de-fuego/geo/f2/piramide/cotejo_sin_id.py sin_id.pmtiles con_id.pmtiles [capa]
"""
import gzip
import os
import sys
from collections import Counter

import mapbox_vector_tile
from pmtiles.reader import MmapSource, all_tiles

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lector_pmtiles import Lector  # noqa: E402


def abrir(ruta):
    f = open(ruta, "rb")

    def leer(desde, largo):
        f.seek(desde)
        return f.read(largo)
    lec = Lector(leer)
    lec.lista = [zxy for zxy, _ in all_tiles(MmapSource(open(ruta, "rb")))]
    lec.tesela(0, 0, 0)
    return lec


sin, con = abrir(sys.argv[1]), abrir(sys.argv[2])
capa = sys.argv[3] if len(sys.argv) > 3 else "trip_3m_puntos"


def puntos(lec, z, x, y):
    """Los puntos de dentro de la tesela, en coordenadas de mundo de z+8 (el píxel de z, en enteros)."""
    b = lec.tesela(z, x, y)
    if not b:
        return [], 0
    t = len(b)
    if lec.cabecera["tesela_gzip"]:
        b = gzip.decompress(b)
    d = mapbox_vector_tile.decode(b, default_options={"y_coord_down": True}).get(capa, {})
    ext = d.get("extent", 4096)
    out = []
    for ft in d.get("features", []):
        g = ft["geometry"]["coordinates"]
        for px, py in (g if isinstance(g[0], list) else [g]):
            if 0 <= px < ext and 0 <= py < ext:
                out.append(((x * ext + px) * 256 // ext, (y * ext + py) * 256 // ext))
    return out, t


def teselas(lec, z):
    return [(x, y) for (zz, x, y) in lec.lista if zz == z]


zmax = min(sin.cabecera["maxzoom"], con.cabecera["maxzoom"])
for z in range(0, zmax + 1):
    ts = sorted(set(teselas(sin, z)) | set(teselas(con, z)))
    iguales = distintas = n_sin = n_con = 0
    tmax_sin = tmax_con = 0
    for (x, y) in ts:
        a, ta = puntos(sin, z, x, y)
        b, tb = puntos(con, z, x, y)
        tmax_sin, tmax_con = max(tmax_sin, ta), max(tmax_con, tb)
        n_sin += len(a)
        n_con += len(b)
        if Counter(a) == Counter(b):
            iguales += 1
        else:
            distintas += 1
    # anidado por posición: cada punto de z (dentro) en el píxel de z de alguna hija de z+1
    perdidos = vistos = 0
    if z < zmax:
        for (x, y) in ts[:: max(1, len(ts) // 40)]:
            padre, _ = puntos(sin, z, x, y)
            hijos = set()
            for dx in (0, 1):
                for dy in (0, 1):
                    h, _ = puntos(sin, z + 1, 2 * x + dx, 2 * y + dy)
                    hijos |= {(px // 2, py // 2) for px, py in h}
            vistos += len(padre)
            perdidos += sum(1 for p in padre if p not in hijos)
    print("z%-2d %6d teselas · iguales %6d distintas %4d · puntos %8d / %8d · máx %6d / %6d B · anidado %s"
          % (z, len(ts), iguales, distintas, n_sin, n_con, tmax_sin, tmax_con,
             "%.2f %% de %d" % (100.0 * perdidos / vistos, vistos) if vistos else "-"))
print("tamaño de la pirámide: %d / %d B" % (os.path.getsize(sys.argv[1]), os.path.getsize(sys.argv[2])))
