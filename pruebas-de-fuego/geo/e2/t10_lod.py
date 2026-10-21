# ORE 0072 E2·0 T10 · Teselado por nivel de detalle sobre el 3D de Google: un árbol de teselas en lat/lng (tamaño
# 360/2^z grados), un nivel por resolución H3, el ancla de cada tesela en el centroide de sus celdas. El cliente pide
# sólo las teselas cuyo ancla cae en la vista. Marcadores con tamaño para los individuos.
import json, math, os, struct, sys, time
from concurrent.futures import ThreadPoolExecutor
import numpy as np, h3, rasterio
from shapely.geometry import Polygon, Point
from shapely.prepared import prep
from scipy.ndimage import maximum_filter

os.environ.update({"GDAL_DISABLE_READDIR_ON_OPEN": "EMPTY_DIR", "CPL_VSIL_CURL_ALLOWED_EXTENSIONS": ".tif", "AWS_NO_SIGN_REQUEST": "YES"})
BASE = r'C:\tmp\e2-t10' + '\\'
OUT = BASE + 'web\\'
os.makedirs(OUT, exist_ok=True)
exec(open(r'C:\ORE-pg\pruebas-de-fuego\geo\e2\t9_usa.py', encoding='utf-8').read().split('# ── el relieve')[0].split('OUT = ')[0])  # imports
CONUS = [(-124.7, 48.4), (-124.1, 46.3), (-124.5, 42.8), (-124.3, 40.3), (-123.0, 38.0), (-122.4, 37.2), (-120.6, 34.6), (-118.5, 34.0),
         (-117.1, 32.5), (-114.7, 32.7), (-111.0, 31.3), (-108.2, 31.3), (-106.5, 31.8), (-104.5, 29.6), (-103.0, 29.0), (-101.4, 29.8),
         (-99.5, 27.5), (-97.4, 25.9), (-97.4, 27.8), (-94.0, 29.6), (-90.0, 29.2), (-89.0, 30.3), (-85.0, 29.7), (-83.0, 29.0), (-82.7, 27.5),
         (-81.5, 25.2), (-80.1, 25.8), (-80.6, 28.4), (-81.4, 30.7), (-81.0, 32.0), (-78.0, 33.8), (-75.5, 35.2), (-76.0, 37.0), (-74.0, 39.5),
         (-73.8, 40.6), (-71.0, 41.4), (-70.0, 41.8), (-70.6, 43.0), (-67.0, 44.8), (-67.8, 47.1), (-69.2, 47.4), (-71.5, 45.0), (-74.7, 45.0),
         (-76.5, 44.2), (-79.0, 43.3), (-79.0, 42.8), (-83.0, 42.0), (-82.5, 45.3), (-84.5, 46.5), (-88.4, 48.3), (-95.2, 49.0), (-123.0, 49.0),
         (-122.8, 48.2)]
POLI = Polygon(CONUS); PP = prep(POLI)
ZONAS_FINAS = {'nueva_york': (39.6, -75.6, 41.9, -72.2), 'denver': (38.9, -106.6, 40.6, -104.4), 'los_angeles': (33.4, -119.2, 34.8, -116.6)}

# ── relieve: GLO-90 a 1/4 (~360 m) para todo, y a resolución completa (90 m) en las zonas finas ──
def leer(k, completo):
    la, lo = k
    n = f"Copernicus_DSM_COG_30_N{la:02d}_00_W{abs(lo):03d}_00_DEM"
    try:
        with rasterio.open(f'/vsicurl/https://copernicus-dem-90m.s3.amazonaws.com/{n}/{n}.tif') as r:
            a = r.read(1) if completo else r.read(1, out_shape=(1, 300, 300))
            return k, a.astype('f4')
    except Exception:
        return k, None

def cargar_relieve(nombre, claves, completo):
    ruta = BASE + nombre
    if os.path.exists(ruta):
        return dict(np.load(ruta))
    t0 = time.time()
    with ThreadPoolExecutor(24) as ex:
        d = {f'{la}_{lo}': a for (la, lo), a in ex.map(lambda k: leer(k, completo), claves) if a is not None}
    np.savez(ruta, **d)
    print(nombre, len(claves), 'pedidas', len(d), 'con tierra', round(time.time() - t0), 's', flush=True)
    return d

GRUESO = cargar_relieve('relieve_360.npz', [(la, lo) for la in range(24, 50) for lo in range(-125, -66)
                                             if POLI.intersects(Polygon([(lo, la), (lo + 1, la), (lo + 1, la + 1), (lo, la + 1)]))], False)
FINO = cargar_relieve('relieve_90.npz', sorted({(la, lo) for (a, b, c, d) in ZONAS_FINAS.values()
                                                 for la in range(int(math.floor(a)), int(math.floor(c)) + 1) for lo in range(int(math.floor(b)), int(math.floor(d)) + 1)}), True)
_ENV = {}

def cota(lat, lng, ventana=1, fino=False):
    fuente = FINO if fino else GRUESO
    clave = (fino, ventana)
    if ventana > 1:
        if clave not in _ENV:
            _ENV[clave] = {k: maximum_filter(a, size=ventana, mode='nearest') for k, a in fuente.items()}
        fuente = _ENV[clave]
    lat = np.asarray(lat, 'f8'); lng = np.asarray(lng, 'f8'); z = np.zeros(lat.shape)
    fla = np.floor(lat).astype(int); flo = np.floor(lng).astype(int)
    for k in set(zip(fla.ravel().tolist(), flo.ravel().tolist())):
        a = fuente.get(f'{k[0]}_{k[1]}')
        if a is None: continue
        n = a.shape[0]
        m = (fla == k[0]) & (flo == k[1])
        r = (k[0] + 1 - lat[m]) * n - 0.5; c = (lng[m] - k[1]) * n - 0.5
        r0 = np.clip(np.floor(r).astype(int), 0, n - 2); c0 = np.clip(np.floor(c).astype(int), 0, n - 2)
        fr = np.clip(r - r0, 0, 1); fc = np.clip(c - c0, 0, 1)
        z[m] = a[r0, c0] * (1 - fr) * (1 - fc) + a[r0, c0 + 1] * (1 - fr) * fc + a[r0 + 1, c0] * fr * (1 - fc) + a[r0 + 1, c0 + 1] * fr * fc
    return np.maximum(z, 0)

# (ecef, enu, densidad, rampa, glb, subdividir: los de T9)
src = open(r'C:\ORE-pg\pruebas-de-fuego\geo\e2\t9_usa.py', encoding='utf-8').read()
exec(src[src.index('# ── la Tierra curva'):src.index('def capa_h3')].replace("OUT + nombre", "OUT + nombre"))

# ── los niveles ──
NIVELES = [  # (res H3, z de la tesela, subdivisión, ventana de envolvente, margen, sólo zonas finas)
    (3, 6, 3, 21, 60, False),
    (4, 7, 2, 17, 50, False),
    (5, 9, 1, 13, 40, False),
    (6, 10, 0, 9, 30, False),
    (7, 12, 0, 5, 20, True),
]

def tesela(lat, lng, z):
    t = 360.0 / 2 ** z
    return np.floor(lng / t).astype(int), np.floor(lat / t).astype(int)

def celdas_de(res, solo_finas):
    if not solo_finas:
        return list(h3.polygon_to_cells(h3.LatLngPoly([(la, lo) for lo, la in CONUS]), res))
    s = set()
    for (a, b, c, d) in ZONAS_FINAS.values():
        s |= set(h3.polygon_to_cells(h3.LatLngPoly([(a, b), (a, d), (c, d), (c, b)]), res))
    return list(s)

manifiesto = {'niveles': []}
for (res, z, nsub, ventana, margen, solo) in NIVELES:
    t0 = time.time()
    cs = celdas_de(res, solo)
    cen = np.array([h3.cell_to_latlng(c) for c in cs])
    viva = cota(cen[:, 0], cen[:, 1]) > 0
    cs = [c for c, v in zip(cs, viva) if v]; cen = cen[viva]
    dens = densidad(cen[:, 0], cen[:, 1])
    color = rampa(np.log1p(dens) / np.log1p(densidad(np.array([40.71]), np.array([-74.0]))[0]))
    tx, ty = tesela(cen[:, 0], cen[:, 1], z)
    bary, tri = subdividir(nsub)
    orden = np.lexsort((ty, tx))
    lista, total = [], 0
    i = 0
    while i < len(orden):
        j = i
        while j < len(orden) and tx[orden[j]] == tx[orden[i]] and ty[orden[j]] == ty[orden[i]]: j += 1
        ks = orden[i:j]; i = j
        lat0, lng0 = float(cen[ks, 0].mean()), float(cen[ks, 1].mean())
        P, C, I, nv = [], [], [], 0
        for k in ks:
            b = np.array(h3.cell_to_boundary(cs[k]))
            for t in range(len(b)):
                ll = bary @ np.stack([cen[k], b[t], b[(t + 1) % len(b)]])
                P.append(ll); C.append(np.repeat([list(color[k]) + [255]], len(ll), 0)); I.append(tri + nv); nv += len(ll)
        ll = np.concatenate(P)
        h = cota(ll[:, 0], ll[:, 1], ventana, fino=solo) + margen
        f = f'r{res}_{tx[ks[0]]}_{ty[ks[0]]}.glb'
        total += glb(f, enu(ll[:, 0], ll[:, 1], h, lat0, lng0), np.concatenate(C), np.concatenate(I).ravel())
        lista.append([f, round(lat0, 5), round(lng0, 5)])
    manifiesto['niveles'].append({'res': res, 'z': z, 'teselas': lista, 'solo_zonas': solo})
    print(f'res {res} z {z}: celdas {len(cs)} teselas {len(lista)} MB {total / 2**20:.1f} {time.time() - t0:.0f} s', flush=True)

# ── individuos: 1 M de tiendas (como T9) ──
rng = np.random.default_rng(9)
N = 1_000_000
w = CI[:, 2] / CI[:, 2].sum()
k = rng.choice(len(CI), size=int(N * 0.85), p=w)
la = np.concatenate([CI[k, 0] + rng.normal(0, 0.45, len(k)), rng.uniform(24, 49.5, N)])
lo = np.concatenate([CI[k, 1] + rng.normal(0, 0.55, len(k)), rng.uniform(-125, -66.5, N)])
dentro = np.array([PP.contains(Point(x, y)) for x, y in zip(lo, la)])
la, lo = la[dentro][:N], lo[dentro][:N]
zz = cota(la, lo); tierra = zz > 0; la, lo, zz = la[tierra], lo[tierra], zz[tierra]
ingresos = rng.lognormal(0, 1, len(la)) * densidad(la, lo) ** 0.5
print('tiendas', len(la), flush=True)
ANG = np.pi / 3 * np.arange(6)

def marcadores(nombre, sel, z, radio, alto_min, alto_max, base_extra):
    """Columnas hexagonales de `radio` m en las tiendas `sel`, agrupadas en teselas de nivel z."""
    ii_all = np.flatnonzero(sel)
    tx, ty = tesela(la[ii_all], lo[ii_all], z)
    orden = np.lexsort((ty, tx)); lista, total = [], 0
    i = 0
    while i < len(orden):
        j = i
        while j < len(orden) and tx[orden[j]] == tx[orden[i]] and ty[orden[j]] == ty[orden[i]]: j += 1
        ii = ii_all[orden[i:j]]; t_ = (tx[orden[i]], ty[orden[i]]); i = j
        lat0, lng0 = float(la[ii].mean()), float(lo[ii].mean()); m = len(ii)
        dla = radio * np.sin(ANG)[None] / 110574.0; dlo = radio * np.cos(ANG)[None] / (111320.0 * np.cos(np.radians(la[ii]))[:, None])
        base = zz[ii][:, None] + base_extra
        alto = base + alto_min + (alto_max - alto_min) * np.clip(ingresos[ii] / 40, 0, 1)[:, None]
        lat_v = np.concatenate([la[ii][:, None] + dla] * 2, 1); lng_v = np.concatenate([lo[ii][:, None] + dlo] * 2, 1)
        h_v = np.concatenate([np.repeat(alto, 6, 1), np.repeat(base, 6, 1)], 1)
        pos = enu(lat_v.ravel(), lng_v.ravel(), h_v.ravel(), lat0, lng0)
        c = rampa(np.clip(ingresos[ii] / 40, 0, 1), MAGMA)
        col = np.concatenate([np.repeat(c, 6, 0).reshape(m, 6, 3), np.repeat(c * 0.7, 6, 0).reshape(m, 6, 3)], 1).reshape(-1, 3)
        col = np.concatenate([col, np.full((len(col), 1), 255)], 1)
        tri = np.array([[0, 1, 2], [0, 2, 3], [0, 3, 4], [0, 4, 5]] + sum([[[a, 6 + a, (a + 1) % 6], [(a + 1) % 6, 6 + a, 6 + (a + 1) % 6]] for a in range(6)], []))
        f = f'{nombre}_{t_[0]}_{t_[1]}.glb'
        total += glb(f, pos, col, (np.arange(m)[:, None, None] * 12 + tri[None]).reshape(-1))
        lista.append([f, round(lat0, 5), round(lng0, 5)])
    print(nombre, int(sel.sum()), 'teselas', len(lista), f'MB {total / 2**20:.1f}', flush=True)
    return lista

top = ingresos >= np.quantile(ingresos, 0.9)
manifiesto['individuos'] = {
    '5': {'z': 9, 'teselas': marcadores('top', top, 9, 900, 600, 5000, 40)},          # nivel res 5: el 10 % como columnas
    '6': {'z': 10, 'teselas': marcadores('t400', np.ones(len(la), bool), 10, 400, 150, 1500, 30)},   # nivel res 6: todas, 400 m
    '7': {'z': 12, 'teselas': marcadores('t120', np.any([(la >= a) & (la <= c) & (lo >= b) & (lo <= d) for (a, b, c, d) in ZONAS_FINAS.values()], 0), 12, 120, 40, 450, 20)},  # nivel res 7: las de las zonas finas, 120 m
}
json.dump(manifiesto, open(OUT + 'arbol.json', 'w'))
print('listo', flush=True)
