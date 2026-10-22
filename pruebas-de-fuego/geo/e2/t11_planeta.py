# ORE 0072 E2·0 T11b · Los 600 M de viajes de SpatialBench sobre el 3D de Google, con nivel de detalle: agregados H3
# res 2–5 en todo el planeta, res 6–7 y los viajes individuales en tres zonas densas. Relieve: Copernicus GLO-90 (la
# versión 1/4, reducida a 75 px por grado con máximo, y la 1/4 entera en las zonas), con envolvente.
import glob, json, math, os, struct, sys, time
from concurrent.futures import ThreadPoolExecutor
import numpy as np, pyarrow.parquet as pq, h3, rasterio
from scipy.ndimage import maximum_filter

os.environ.update({"GDAL_DISABLE_READDIR_ON_OPEN": "EMPTY_DIR", "CPL_VSIL_CURL_ALLOWED_EXTENSIONS": ".tif", "AWS_NO_SIGN_REQUEST": "YES"})
BASE = os.environ.get('E2_BASE', 'C:/tmp/e2-t12/')  # datos/ (los parquet de previo_f) y web/ (la salida)
OUT = BASE + 'web/'
os.makedirs(OUT, exist_ok=True)
src = open('C:/ORE-pg/pruebas-de-fuego/geo/e2/t9_usa.py', encoding='utf-8').read()
exec(src[src.index('# ── la Tierra curva'):src.index('def capa_h3')])  # ecef, enu, rampa, glb, subdividir (glb escribe en OUT)
ZONAS = {"africa": (7.0, 19.0, 10.0, 22.0), "china": (44.0, 116.0, 46.0, 118.0), "sinai": (28.0, 33.0, 30.0, 36.0)}

def leer_tabla(nombre):
    fs = glob.glob(BASE + f'datos/{nombre}/*.parquet')
    return pq.read_table(fs).to_pandas()

# ── relieve ──
def nombre_dem(la, lo):
    return f"Copernicus_DSM_COG_30_{'N' if la >= 0 else 'S'}{abs(la):02d}_00_{'E' if lo >= 0 else 'W'}{abs(lo):03d}_00_DEM"

def leer_dem(k, n):
    la, lo = k; nm = nombre_dem(la, lo)
    try:
        with rasterio.open(f'/vsicurl/https://copernicus-dem-90m.s3.amazonaws.com/{nm}/{nm}.tif') as r:
            a = r.read(1, out_shape=(1, 300, 300)).astype('f4')
        return k, (a if n == 300 else a.reshape(n, 300 // n, n, 300 // n).max(axis=(1, 3)))
    except Exception:
        return k, None

def cargar_dem(nombre, claves, n):
    ruta = BASE + nombre
    if os.path.exists(ruta):
        return dict(np.load(ruta))
    t0 = time.time()
    with ThreadPoolExecutor(32) as ex:
        d = {f'{la}_{lo}': a for (la, lo), a in ex.map(lambda k: leer_dem(k, n), claves) if a is not None}
    np.savez(ruta, **d)
    print(nombre, len(claves), 'pedidas', len(d), 'con tierra', round(time.time() - t0), 's', flush=True)
    return d

_ENV = {}
def cota(lat, lng, dem, clave, ventana):
    if ventana > 1:
        if clave not in _ENV:
            _ENV[clave] = {k: maximum_filter(a, size=ventana, mode='nearest') for k, a in dem.items()}
        dem = _ENV[clave]
    lat = np.asarray(lat, 'f8'); lng = np.asarray(lng, 'f8'); z = np.zeros(lat.shape)
    fla = np.floor(lat).astype(int); flo = np.floor(lng).astype(int)
    for k in set(zip(fla.ravel().tolist(), flo.ravel().tolist())):
        a = dem.get(f'{k[0]}_{k[1]}')
        if a is None: continue
        n = a.shape[0]; m = (fla == k[0]) & (flo == k[1])
        r = (k[0] + 1 - lat[m]) * n - 0.5; c = (lng[m] - k[1]) * n - 0.5
        r0 = np.clip(np.floor(r).astype(int), 0, n - 2); c0 = np.clip(np.floor(c).astype(int), 0, n - 2)
        fr = np.clip(r - r0, 0, 1); fc = np.clip(c - c0, 0, 1)
        z[m] = a[r0, c0] * (1 - fr) * (1 - fc) + a[r0, c0 + 1] * (1 - fr) * fc + a[r0 + 1, c0] * fr * (1 - fc) + a[r0 + 1, c0 + 1] * fr * fc
    return np.maximum(z, 0)

t5 = leer_tabla('sb_r5')
cen5 = np.array([h3.cell_to_latlng(h3.int_to_str(int(c))) for c in t5['celda']])
claves_g = sorted(set(zip(np.floor(cen5[:, 0]).astype(int).tolist(), np.floor(cen5[:, 1]).astype(int).tolist())))
DEM_G = cargar_dem('dem_75.npz', claves_g, 75)
claves_z = sorted({(la, lo) for (a, b, c, d) in ZONAS.values() for la in range(int(a), int(c) + 1) for lo in range(int(b), int(d) + 1)})
DEM_Z = cargar_dem('dem_300.npz', claves_z, 300)

NMAX = float(t5['n'].max())
def color_de(n, res):
    # densidad por km² normalizada en escala logarítmica (las celdas de cada resolución, comparables)
    area = h3.average_hexagon_area(res, 'km^2')
    d = np.log1p(np.asarray(n, 'f8') / area)
    return rampa(d / np.log1p(NMAX / h3.average_hexagon_area(5, 'km^2')))

def tesela_de(lat, lng, z):
    t = 360.0 / 2 ** z
    return np.floor(lng / t).astype(int), np.floor(lat / t).astype(int)

NIVELES = [  # res, z, subdivisión, ventana, margen, dem
    (2, 5, 4, 25, 100, 'g'), (3, 6, 3, 15, 80, 'g'), (4, 7, 2, 9, 60, 'g'), (5, 9, 1, 5, 40, 'g'), (6, 10, 0, 9, 30, 'z'), (7, 12, 0, 5, 20, 'z')]
arbol = {'niveles': [], 'zonas': list(ZONAS.values())}
for (res, z, nsub, ventana, margen, cual) in NIVELES:
    t0 = time.time()
    t = leer_tabla(f'sb_r{res}')
    cs = [h3.int_to_str(int(c)) for c in t['celda']]
    cen = np.array([h3.cell_to_latlng(c) for c in cs])
    col = color_de(t['n'].values, res)
    tx, ty = tesela_de(cen[:, 0], cen[:, 1], z)
    bary, tri = subdividir(nsub)
    dem = DEM_G if cual == 'g' else DEM_Z
    orden = np.lexsort((ty, tx)); lista, total, i = [], 0, 0
    while i < len(orden):
        j = i
        while j < len(orden) and tx[orden[j]] == tx[orden[i]] and ty[orden[j]] == ty[orden[i]]: j += 1
        ks = orden[i:j]; i = j
        lat0, lng0 = float(cen[ks, 0].mean()), float(cen[ks, 1].mean())
        P, C, I, nv = [], [], [], 0
        for k in ks:
            b = np.array(h3.cell_to_boundary(cs[k]))
            # celdas que cruzan el antimeridiano: fuera (no saben de longitudes partidas)
            if np.ptp(b[:, 1]) > 90: continue
            for q in range(len(b)):
                ll = bary @ np.stack([cen[k], b[q], b[(q + 1) % len(b)]])
                P.append(ll); C.append(np.repeat([list(col[k]) + [255]], len(ll), 0)); I.append(tri + nv); nv += len(ll)
        if not P: continue
        ll = np.concatenate(P)
        h = cota(ll[:, 0], ll[:, 1], dem, (cual, ventana), ventana) + margen
        f = f'p{res}_{tx[ks[0]]}_{ty[ks[0]]}.glb'
        total += glb(f, enu(ll[:, 0], ll[:, 1], h, lat0, lng0), np.concatenate(C), np.concatenate(I).ravel())
        lista.append([f, round(lat0, 5), round(lng0, 5)])
    arbol['niveles'].append({'res': res, 'z': z, 'teselas': lista, 'solo_zonas': cual == 'z'})
    print(f'res {res}: celdas {len(cs)} viajes {int(t["n"].sum())} teselas {len(lista)} MB {total / 2**20:.0f} {time.time() - t0:.0f} s', flush=True)

# ── los viajes individuales (zonas): discos de 60 m en el nivel res 7, con tope por tesela ──
pts = leer_tabla('sb_puntos')
la, lo, tar = pts['y'].values, pts['x'].values, pts['tarifa'].values
TOPE = int(os.environ.get('T12_TOPE', '300000'))
tx, ty = tesela_de(la, lo, 12)
ANG = np.pi / 3 * np.arange(6)
orden = np.lexsort((ty, tx)); lista, total, i, dibujados = [], 0, 0, 0
while i < len(orden):
    j = i
    while j < len(orden) and tx[orden[j]] == tx[orden[i]] and ty[orden[j]] == ty[orden[i]]: j += 1
    ii = orden[i:j]; key = (tx[orden[i]], ty[orden[i]]); i = j
    if len(ii) > TOPE: ii = ii[np.random.default_rng(1).choice(len(ii), TOPE, replace=False)]
    m = len(ii); dibujados += m
    lat0, lng0 = float(la[ii].mean()), float(lo[ii].mean())
    r = 25.0
    dla = r * np.sin(ANG)[None] / 110574.0; dlo = r * np.cos(ANG)[None] / (111320.0 * np.cos(np.radians(la[ii]))[:, None])
    h = cota(la[ii], lo[ii], DEM_Z, ('z', 1), 1)[:, None] + 25
    pos = enu((la[ii][:, None] + dla).ravel(), (lo[ii][:, None] + dlo).ravel(), np.repeat(h, 6, 1).ravel(), lat0, lng0)
    c = rampa(np.clip(tar[ii] / 60, 0, 1), MAGMA)
    col = np.concatenate([np.repeat(c, 6, 0), np.full((m * 6, 1), 255)], 1)
    idx = (np.arange(m)[:, None, None] * 6 + np.array([[0, 1, 2], [0, 2, 3], [0, 3, 4], [0, 4, 5]])[None]).reshape(-1)
    f = f'v_{key[0]}_{key[1]}.glb'; total += glb(f, pos, col, idx); lista.append([f, round(lat0, 5), round(lng0, 5)])
arbol['individuos'] = {'7': {'z': 12, 'teselas': lista}}
print(f'viajes en las zonas {len(la)} dibujados {dibujados} teselas {len(lista)} MB {total / 2**20:.0f}', flush=True)
json.dump(arbol, open(OUT + 'arbol.json', 'w'))
print('listo', flush=True)
