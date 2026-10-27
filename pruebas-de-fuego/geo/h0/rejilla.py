"""ORE 0072 H0·B2 (y la medida de H2) · la rejilla Hilbert y la profundidad, simuladas.

Una propiedad del orden, no del hierro: se simula en local con puntos sintéticos de una ciudad (5 M, grupos de
100 000 como `GRUPO_DE_FILAS`). Para cada orden: filas leídas por tesela z14 frente a las que tiene, grupos
tocados, y la profundidad (cuántos grupos cubren, de media, un punto con datos), para ver si la profundidad
predice lo leído.

  python pruebas-de-fuego/geo/h0/rejilla.py
"""
import math
import time

import numpy as np
import sys

sys.stdout.reconfigure(encoding="utf-8")

R = np.random.default_rng(7)
N, GRUPO, Z = 5_000_000, 100_000, 14
EXT = (-3.90, 40.30, -3.50, 40.60)  # una ciudad (~34 × 33 km)


def puntos(n):
    """60 % en un centro denso, 25 % en cinco barrios, 15 % repartido."""
    a, b, c = int(n * .6), int(n * .25), n - int(n * .6) - int(n * .25)
    cx, cy = -3.70, 40.42
    centro = np.c_[R.normal(cx, .025, a), R.normal(cy, .02, a)]
    subs = R.uniform([EXT[0] + .05, EXT[1] + .05], [EXT[2] - .05, EXT[3] - .05], (5, 2))
    k = R.integers(0, 5, b)
    barrios = subs[k] + R.normal(0, .01, (b, 2))
    resto = R.uniform(EXT[:2], EXT[2:], (c, 2))
    p = np.r_[centro, barrios, resto]
    p[:, 0] = p[:, 0].clip(EXT[0], EXT[2])
    p[:, 1] = p[:, 1].clip(EXT[1], EXT[3])
    return p[R.permutation(n)]  # llegan sin orden, como de una fuente


def hilbert(x, y, orden):
    """xy → d (Wikipedia `xy2d`) en numpy uint64; `orden` bits por eje (≤ 32)."""
    x, y = x.astype(np.uint64), y.astype(np.uint64)
    lado = np.uint64(1) << np.uint64(orden)
    d = np.zeros_like(x)
    s = lado >> np.uint64(1)
    while s > 0:
        rx = ((x & s) > 0).astype(np.uint64)
        ry = ((y & s) > 0).astype(np.uint64)
        d += s * s * ((np.uint64(3) * rx) ^ ry)
        # rotar el cuadrante
        m = ry == 0
        v = m & (rx == 1)
        x = np.where(v, lado - np.uint64(1) - x, x)
        y = np.where(v, lado - np.uint64(1) - y, y)
        x, y = np.where(m, y, x), np.where(m, x, y)
        s >>= np.uint64(1)
    return d


def clave(p, ext, orden):
    lado = (1 << orden) - 1
    cx = ((p[:, 0] - ext[0]) / (ext[2] - ext[0]) * lado).clip(0, lado)
    cy = ((p[:, 1] - ext[1]) / (ext[3] - ext[1]) * lado).clip(0, lado)
    return hilbert(np.floor(cx), np.floor(cy), orden)


def grupos(p):
    """Las cajas y filas de cada grupo de `GRUPO` filas, en el orden en que llegan."""
    out = []
    for i in range(0, len(p), GRUPO):
        g = p[i:i + GRUPO]
        out.append((g[:, 0].min(), g[:, 1].min(), g[:, 0].max(), g[:, 1].max(), len(g)))
    return np.array(out)


def ordenar(p, ext, orden):
    return p[np.argsort(clave(p, ext, orden), kind="stable")]


def por_tramos(p, n_tramos, ext_de_cada=True, orden=16):
    """Cada tramo ordenado por su cuenta (lo de `hilbert_por_tramos`, o n `append`)."""
    out = []
    for t in np.array_split(p, n_tramos):
        e = (t[:, 0].min(), t[:, 1].min(), t[:, 0].max(), t[:, 1].max()) if ext_de_cada else EXT
        out.append(ordenar(t, e, orden))
    return out


def teselas(p, n):
    """n teselas z14 con datos (por un punto al azar de los datos)."""
    m = 2 ** Z
    i = R.integers(0, len(p), n)
    lon, lat = p[i, 0], p[i, 1]
    x = np.floor((lon + 180) / 360 * m)
    y = np.floor((1 - np.arcsinh(np.tan(np.radians(lat))) / math.pi) / 2 * m)
    t = np.unique(np.c_[x, y], axis=0)
    x0 = t[:, 0] / m * 360 - 180
    x1 = (t[:, 0] + 1) / m * 360 - 180
    lat_de = lambda yy: np.degrees(np.arctan(np.sinh(math.pi * (1 - 2 * yy / m))))
    return np.c_[x0, lat_de(t[:, 1] + 1), x1, lat_de(t[:, 1])]


def medir(nombre, gs, p, ts):
    # filas en la tesela (contadas sobre los puntos) y filas leídas (las de los grupos que toca)
    orden = np.lexsort((p[:, 1], p[:, 0]))
    xs = p[orden, 0]
    leidas, tiene, tocados = [], [], []
    for x0, y0, x1, y1 in ts:
        a, b = np.searchsorted(xs, [x0, x1])
        ys = p[orden[a:b], 1]
        n = int(((ys >= y0) & (ys <= y1)).sum())
        t = (gs[:, 0] <= x1) & (gs[:, 2] >= x0) & (gs[:, 1] <= y1) & (gs[:, 3] >= y0)
        leidas.append(gs[t, 4].sum())
        tiene.append(max(n, 1))
        tocados.append(int(t.sum()))
    r = np.array(leidas) / np.array(tiene)
    # la profundidad: de media, cuántos grupos cubren un punto con datos
    q = p[R.integers(0, len(p), 4000)]
    cubren = ((gs[None, :, 0] <= q[:, None, 0]) & (gs[None, :, 2] >= q[:, None, 0]) &
              (gs[None, :, 1] <= q[:, None, 1]) & (gs[None, :, 3] >= q[:, None, 1])).sum(1)
    print("| %s | %.1f | %.0f× | %.0f× | %d de %d | %.0f k |" % (
        nombre, cubren.mean(), np.median(r), np.percentile(r, 95), np.median(tocados), len(gs),
        np.median(leidas) / 1000), flush=True)
    return cubren.mean(), np.median(r)


def main():
    t = time.time()
    p = puntos(N)
    ts = teselas(p, 1500)
    print("# H0·B2 · %d puntos, grupos de %d, %d teselas z%d · %s" % (N, GRUPO, len(ts), Z, time.strftime("%Y-%m-%d %H:%M")))
    print("| orden | profundidad | leídas/tiene p50 | p95 | grupos tocados p50 | leídas p50 |")
    print("|---|---|---|---|---|---|")
    mundo = (-180.0, -90.0, 180.0, 90.0)
    filas = []
    filas.append(medir("sin orden", grupos(p), p, ts))
    filas.append(medir("Hilbert 2¹⁶ sobre la tabla (hoy)", grupos(ordenar(p, EXT, 16)), p, ts))
    filas.append(medir("Hilbert 2¹⁶ fija, mundo", grupos(ordenar(p, mundo, 16)), p, ts))
    filas.append(medir("Hilbert 2³² fija, mundo", grupos(ordenar(p, mundo, 32)), p, ts))
    for n in (5, 10, 50):
        gs = np.vstack([grupos(t) for t in por_tramos(p, n)])
        filas.append(medir("%d tramos/`append` ordenados por su cuenta" % n, gs, p, ts))
    # niveles (LSM): 3 niveles, cada uno ordenado y sin solaparse por dentro (90 % / 9 % / 1 %)
    cortes = [int(N * .9), int(N * .99)]
    niveles = [ordenar(x, mundo, 32) for x in np.split(p, cortes)]
    filas.append(medir("3 niveles (90/9/1 %), 2³² fija", np.vstack([grupos(x) for x in niveles]), p, ts))
    pr = np.array(filas)
    c = np.corrcoef(np.log(pr[:, 0]), np.log(pr[:, 1]))[0, 1]
    print("\ncorrelación log(profundidad) ~ log(leídas/tiene): %.3f · %.0f s" % (c, time.time() - t))


main()
