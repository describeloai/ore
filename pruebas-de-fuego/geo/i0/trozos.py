"""ORE 0072 I0·c · cuánto cuesta partir las geometrías grandes del índice (en 3857, por la rejilla de las teselas del
suelo), y cuánto ganan.

Una geometría cuya caja pasa de una tesela del suelo se parte por las celdas de esa rejilla (las que su caja
cubre y la geometría toca) con `clip_by_rect` de GEOS, lo mismo que haría el Job en Rust (`geo`/GEOS). Para cada
tipo de dato sintético: qué fracción se parte, cuántos trozos salen, cuánto tarda por millón y cuánto baja el
área de caja que una tesela tiene que mirar. Es una propiedad del algoritmo: se mide en local.

  python -I pruebas-de-fuego/geo/i0/trozos.py
"""
import math
import sys
import time

import numpy as np
import shapely

sys.stdout.reconfigure(encoding="utf-8")
R = np.random.default_rng(11)
TIERRA = 40075016.68557849
SUELO = 14
CELDA = TIERRA / 2 ** SUELO  # 2 446 m en 3857
UMBRAL = float(sys.argv[1]) if len(sys.argv) > 1 else 1.0  # se parte lo que pasa de UMBRAL celdas
X0, Y0 = -8240000.0, 4960000.0  # Nueva York en 3857


def lineas_de_viaje(n):
    """Viajes en línea recta (recogida → destino), longitudes log-normales con mediana ~3 km."""
    a = np.c_[R.normal(X0, 6000, n), R.normal(Y0, 6000, n)]
    ang = R.uniform(0, 2 * math.pi, n)
    lon = R.lognormal(math.log(3000), 0.8, n)
    b = a + np.c_[np.cos(ang), np.sin(ang)] * lon[:, None]
    return shapely.linestrings(np.stack([a, b], 1))


def carreteras(n, tramos=200, paso=250.0):
    """Carreteras: paseos aleatorios de 200 tramos de 250 m (~50 km)."""
    out = []
    for _ in range(n):
        d = np.cumsum(R.normal(0, 0.15, tramos))
        p = np.c_[np.cos(d), np.sin(d)].cumsum(0) * paso + [R.normal(X0, 20000), R.normal(Y0, 20000)]
        out.append(p)
    return shapely.linestrings(out)


def edificios(n):
    """Edificios: rectángulos de 10–60 m."""
    c = np.c_[R.normal(X0, 8000, n), R.normal(Y0, 8000, n)]
    m = R.uniform(10, 60, (n, 2))
    return shapely.box(c[:, 0], c[:, 1], c[:, 0] + m[:, 0], c[:, 1] + m[:, 1])


def municipios(n):
    """Áreas grandes: polígonos de 64 vértices con radio de 3–25 km."""
    out = []
    for _ in range(n):
        r = R.uniform(3000, 25000)
        t = np.linspace(0, 2 * math.pi, 64, endpoint=False)
        rr = r * (1 + 0.2 * R.normal(0, 1, 64))
        c = [R.normal(X0, 30000), R.normal(Y0, 30000)]
        out.append(np.c_[c[0] + rr * np.cos(t), c[1] + rr * np.sin(t)])
    return shapely.polygons(out)


def partir(gs):
    """Las que pasan de una celda, por las celdas que su caja cubre y la geometría toca."""
    b = shapely.bounds(gs)
    grande = ((b[:, 2] - b[:, 0]) > UMBRAL * CELDA) | ((b[:, 3] - b[:, 1]) > UMBRAL * CELDA)
    trozos = list(gs[~grande])
    for g, (x0, y0, x1, y1) in zip(gs[grande], b[grande]):
        i0, i1 = math.floor(x0 / CELDA), math.floor(x1 / CELDA)
        j0, j1 = math.floor(y0 / CELDA), math.floor(y1 / CELDA)
        ii, jj = np.meshgrid(np.arange(i0, i1 + 1), np.arange(j0, j1 + 1))
        ii, jj = ii.ravel(), jj.ravel()
        celdas = shapely.box(ii * CELDA, jj * CELDA, (ii + 1) * CELDA, (jj + 1) * CELDA)
        toca = shapely.intersects(g, celdas)
        for c in np.flatnonzero(toca):
            p = shapely.clip_by_rect(g, ii[c] * CELDA, jj[c] * CELDA, (ii[c] + 1) * CELDA, (jj[c] + 1) * CELDA)
            if not shapely.is_empty(p):
                trozos.append(p)
    return np.array(trozos, dtype=object), int(grande.sum())


def area_de_caja(gs):
    b = shapely.bounds(gs)
    return float(((b[:, 2] - b[:, 0]) * (b[:, 3] - b[:, 1])).sum())


print("# I0·c · partir por la rejilla de z%d (celda de %.0f m), lo que pasa de %g celdas · GEOS %s · %s" % (
    SUELO, CELDA, UMBRAL, shapely.geos_version_string, time.strftime("%Y-%m-%d %H:%M")))
print("| datos | n | se parten | trozos por geometría partida | filas del índice | µs por geometría | área de caja (antes → después) |")
print("|---|---|---|---|---|---|---|")
for nombre, gen, n in (("líneas de viaje", lineas_de_viaje, 200_000), ("carreteras (~50 km)", carreteras, 2_000),
                       ("edificios", edificios, 200_000), ("áreas de 3–25 km", municipios, 2_000)):
    gs = gen(n)
    t = time.time()
    ts, grandes = partir(gs)
    s = time.time() - t
    print("| %s | %d | %.1f %% | %s | %.2f× | %.1f | %.1f× menos |" % (
        nombre, n, grandes / n * 100, ("%.1f" % ((len(ts) - (n - grandes)) / grandes)) if grandes else "—",
        len(ts) / n, s / n * 1e6, area_de_caja(gs) / max(area_de_caja(ts), 1)), flush=True)
