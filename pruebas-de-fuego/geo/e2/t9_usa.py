# ORE 0072 E2·0 T9 · Los 48 estados sobre el 3D de Google: H3 res 3–6 (resolución por distancia), 1 M de puntos y
# su 10 % de más «ingresos» como columnas. Relieve: Copernicus GLO-90 leído por trozos (la versión 1/4 de cada
# COG, ~360 m) y guardado en relieve.npz. Curvatura: cada vértice en ECEF y luego en el ENU de su tesela.
import json, math, os, struct, sys, time
from concurrent.futures import ThreadPoolExecutor
import numpy as np, h3, rasterio
from shapely.geometry import Polygon, Point
from shapely.prepared import prep

os.environ.update({"GDAL_DISABLE_READDIR_ON_OPEN": "EMPTY_DIR", "CPL_VSIL_CURL_ALLOWED_EXTENSIONS": ".tif", "AWS_NO_SIGN_REQUEST": "YES"})
OUT = r'C:\tmp\e2-t9\web' + '\\'
os.makedirs(OUT, exist_ok=True)

CONUS = [(-124.7, 48.4), (-124.1, 46.3), (-124.5, 42.8), (-124.3, 40.3), (-123.0, 38.0), (-122.4, 37.2), (-120.6, 34.6), (-118.5, 34.0),
         (-117.1, 32.5), (-114.7, 32.7), (-111.0, 31.3), (-108.2, 31.3), (-106.5, 31.8), (-104.5, 29.6), (-103.0, 29.0), (-101.4, 29.8),
         (-99.5, 27.5), (-97.4, 25.9), (-97.4, 27.8), (-94.0, 29.6), (-90.0, 29.2), (-89.0, 30.3), (-85.0, 29.7), (-83.0, 29.0), (-82.7, 27.5),
         (-81.5, 25.2), (-80.1, 25.8), (-80.6, 28.4), (-81.4, 30.7), (-81.0, 32.0), (-78.0, 33.8), (-75.5, 35.2), (-76.0, 37.0), (-74.0, 39.5),
         (-73.8, 40.6), (-71.0, 41.4), (-70.0, 41.8), (-70.6, 43.0), (-67.0, 44.8), (-67.8, 47.1), (-69.2, 47.4), (-71.5, 45.0), (-74.7, 45.0),
         (-76.5, 44.2), (-79.0, 43.3), (-79.0, 42.8), (-83.0, 42.0), (-82.5, 45.3), (-84.5, 46.5), (-88.4, 48.3), (-95.2, 49.0), (-123.0, 49.0),
         (-122.8, 48.2)]
POLI = Polygon(CONUS)
PP = prep(POLI)

# ── el relieve: una vez, por trozos, y guardado ──
REL = OUT + '..\\relieve.npz'
if os.path.exists(REL):
    RELIEVE = dict(np.load(REL))
else:
    claves = [(la, lo) for la in range(24, 50) for lo in range(-125, -66) if POLI.intersects(Polygon([(lo, la), (lo + 1, la), (lo + 1, la + 1), (lo, la + 1)]))]
    def leer(k):
        la, lo = k
        n = f"Copernicus_DSM_COG_30_{'N' if la >= 0 else 'S'}{abs(la):02d}_00_{'W' if lo < 0 else 'E'}{abs(lo):03d}_00_DEM"
        try:
            with rasterio.open(f'/vsicurl/https://copernicus-dem-90m.s3.amazonaws.com/{n}/{n}.tif') as r:
                return k, r.read(1, out_shape=(1, 300, 300)).astype('f4')
        except Exception:
            return k, None  # mar: la tesela no existe
    t0 = time.time()
    with ThreadPoolExecutor(24) as ex:
        RELIEVE = {f'{la}_{lo}': a for (la, lo), a in ex.map(leer, claves) if a is not None}
    np.savez_compressed(REL, **RELIEVE)
    print('relieve', len(claves), 'teselas pedidas', len(RELIEVE), 'con tierra', round(time.time() - t0), 's', flush=True)


def cota(lat, lng):
    lat = np.asarray(lat, 'f8'); lng = np.asarray(lng, 'f8'); z = np.zeros(lat.shape)
    fla = np.floor(lat).astype(int); flo = np.floor(lng).astype(int)
    for k in set(zip(fla.ravel().tolist(), flo.ravel().tolist())):
        a = RELIEVE.get(f'{k[0]}_{k[1]}')
        if a is None: continue
        m = (fla == k[0]) & (flo == k[1])
        r = (k[0] + 1 - lat[m]) * 300 - 0.5; c = (lng[m] - k[1]) * 300 - 0.5
        r0 = np.clip(np.floor(r).astype(int), 0, 298); c0 = np.clip(np.floor(c).astype(int), 0, 298)
        fr = np.clip(r - r0, 0, 1); fc = np.clip(c - c0, 0, 1)
        z[m] = a[r0, c0] * (1 - fr) * (1 - fc) + a[r0, c0 + 1] * (1 - fr) * fc + a[r0 + 1, c0] * fr * (1 - fc) + a[r0 + 1, c0 + 1] * fr * fc
    return np.maximum(z, 0)


# La envolvente: la cota máxima en una ventana de v píxeles (~360 m cada uno), por tesela. Para celdas grandes, cuyos
# triángulos miden kilómetros, la cota puntual deja asomar las crestas entre vértices.
from scipy.ndimage import maximum_filter
_ENV = {}
def cota_env(lat, lng, v):
    global RELIEVE
    if v not in _ENV:
        _ENV[v] = {k: maximum_filter(a, size=v, mode='nearest') for k, a in RELIEVE.items()}
    orig = RELIEVE; RELIEVE = _ENV[v]
    try:
        return cota(lat, lng)
    finally:
        RELIEVE = orig


# ── la Tierra curva: geodésicas → ECEF → ENU del ancla ──
A_, E2 = 6378137.0, 6.69437999014e-3
def ecef(lat, lng, h):
    la, lo = np.radians(lat), np.radians(lng)
    n = A_ / np.sqrt(1 - E2 * np.sin(la) ** 2)
    return np.stack([(n + h) * np.cos(la) * np.cos(lo), (n + h) * np.cos(la) * np.sin(lo), (n * (1 - E2) + h) * np.sin(la)], -1)
def enu(lat, lng, h, lat0, lng0):
    p = ecef(lat, lng, h) - ecef(np.array([lat0]), np.array([lng0]), np.array([0.0]))
    la, lo = math.radians(lat0), math.radians(lng0)
    e = np.array([-math.sin(lo), math.cos(lo), 0]); n = np.array([-math.sin(la) * math.cos(lo), -math.sin(la) * math.sin(lo), math.cos(la)])
    u = np.array([math.cos(la) * math.cos(lo), math.cos(la) * math.sin(lo), math.sin(la)])
    return np.stack([p @ e, p @ n, p @ u], -1)


# ── la métrica: «población» con forma de EE. UU. ──
CIUDADES = [(40.71, -74.0, 20), (34.05, -118.24, 13), (41.88, -87.63, 9.5), (29.76, -95.37, 7), (33.45, -112.07, 5), (39.95, -75.17, 6),
            (29.42, -98.49, 2.5), (32.72, -117.16, 3.3), (32.78, -96.8, 7.5), (37.77, -122.42, 7.7), (47.61, -122.33, 4), (39.74, -104.99, 3),
            (33.75, -84.39, 6), (25.76, -80.19, 6), (42.36, -71.06, 4.9), (42.33, -83.05, 4.3), (44.98, -93.27, 3.6), (38.63, -90.2, 2.8),
            (38.9, -77.04, 6.3), (27.95, -82.46, 3.2), (45.52, -122.68, 2.5), (36.17, -115.14, 2.3), (39.1, -94.58, 2.2), (35.23, -80.84, 2.7),
            (40.76, -111.89, 1.3), (36.16, -86.78, 2), (29.95, -90.07, 1.3), (35.47, -97.52, 1.4), (39.1, -84.51, 2.2), (40.44, -80.0, 2.3)]
CI = np.array(CIUDADES)
def densidad(lat, lng):
    lat = np.asarray(lat)[..., None]; lng = np.asarray(lng)[..., None]
    d2 = (lat - CI[:, 0]) ** 2 + ((lng - CI[:, 1]) * np.cos(np.radians(lat))) ** 2
    return (CI[:, 2] * np.exp(-d2 / (2 * 0.6 ** 2))).sum(-1) + 0.05

VIRIDIS = np.array([[68, 1, 84], [59, 82, 139], [33, 145, 140], [94, 201, 98], [253, 231, 37]], 'f8')
MAGMA = np.array([[0, 0, 4], [81, 18, 124], [183, 55, 121], [252, 137, 97], [252, 253, 191]], 'f8')
def rampa(v, s=VIRIDIS):
    v = np.clip(np.asarray(v, 'f8'), 0, 1) * (len(s) - 1); i = np.minimum(v.astype(int), len(s) - 2); f = (v - i)[..., None]
    return s[i] * (1 - f) + s[i + 1] * f


def glb(nombre, pos, col, idx=None, modo=4):
    pos = np.asarray(pos, 'f4'); col = np.asarray(col).astype('u1')
    bufs = [pos.tobytes(), col.tobytes()] + ([np.asarray(idx, '<u4').tobytes()] if idx is not None else [])
    offs, blob = [], b''
    for b in bufs:
        offs.append(len(blob)); blob += b + b'\0' * ((4 - len(b) % 4) % 4)
    prim = {"attributes": {"POSITION": 0, "COLOR_0": 1}, "material": 0, "mode": modo}
    acc = [{"bufferView": 0, "componentType": 5126, "count": len(pos), "type": "VEC3", "min": pos.min(0).tolist(), "max": pos.max(0).tolist()},
           {"bufferView": 1, "componentType": 5121, "normalized": True, "count": len(col), "type": "VEC4"}]
    if idx is not None:
        acc.append({"bufferView": 2, "componentType": 5125, "count": len(idx), "type": "SCALAR"}); prim["indices"] = 2
    g = {"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}], "meshes": [{"primitives": [prim]}],
         "accessors": acc, "bufferViews": [{"buffer": 0, "byteOffset": offs[i], "byteLength": len(bufs[i])} for i in range(len(bufs))],
         "buffers": [{"byteLength": len(blob)}],
         "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 1},
                        "doubleSided": True, "alphaMode": "OPAQUE", "extensions": {"KHR_materials_unlit": {}}}],
         "extensionsUsed": ["KHR_materials_unlit"]}
    j = json.dumps(g).encode(); j += b' ' * ((4 - len(j) % 4) % 4)
    open(OUT + nombre, 'wb').write(struct.pack('<III', 0x46546C67, 2, 28 + len(j) + len(blob)) + struct.pack('<II', len(j), 0x4E4F534A) + j
                                   + struct.pack('<II', len(blob), 0x004E4942) + blob)
    return 28 + len(j) + len(blob)


def subdividir(n):
    """Puntos baricéntricos y triángulos de un triángulo dividido en N = 2^n por lado."""
    N = 2 ** n
    pts = [(i, j) for i in range(N + 1) for j in range(N + 1 - i)]
    ix = {p: k for k, p in enumerate(pts)}
    tri = []
    for i in range(N):
        for j in range(N - i):
            tri.append([ix[(i, j)], ix[(i + 1, j)], ix[(i, j + 1)]])
            if j < N - i - 1:
                tri.append([ix[(i + 1, j)], ix[(i + 1, j + 1)], ix[(i, j + 1)]])
    b = np.array(pts, 'f8') / N
    return np.stack([1 - b.sum(1), b[:, 0], b[:, 1]], 1), np.array(tri)


def capa_h3(nombre, res, n_sub, margen, caja_deg, ventana=1):
    t0 = time.time()
    celdas = [c for c in h3.polygon_to_cells(h3.LatLngPoly([(la, lo) for lo, la in CONUS]), res)]
    cen = np.array([h3.cell_to_latlng(c) for c in celdas])
    vivas = cota(cen[:, 0], cen[:, 1]) > 0
    celdas = [c for c, v in zip(celdas, vivas) if v]; cen = cen[vivas]
    dens = densidad(cen[:, 0], cen[:, 1])
    color = rampa(np.log1p(dens) / np.log1p(dens.max()))
    bary, tri = subdividir(n_sub)
    grupos = {}
    for k, c in enumerate(celdas):
        grupos.setdefault((int(cen[k, 0] // caja_deg), int(cen[k, 1] // caja_deg)), []).append(k)
    manifiesto, total = [], 0
    for (gi, gj), ks in grupos.items():
        lat0, lng0 = float(cen[ks, 0].mean()), float(cen[ks, 1].mean())
        P, C, I, nv = [], [], [], 0
        for k in ks:
            b = np.array(h3.cell_to_boundary(celdas[k]))
            for t in range(len(b)):
                A, B = b[t], b[(t + 1) % len(b)]
                tri_ll = np.stack([cen[k], A, B])  # abanico desde el centro
                ll = bary @ tri_ll
                P.append(ll); C.append(np.repeat([list(color[k]) + [255]], len(ll), 0)); I.append(tri + nv); nv += len(ll)
        ll = np.concatenate(P)
        h = (cota_env(ll[:, 0], ll[:, 1], ventana) if ventana > 1 else cota(ll[:, 0], ll[:, 1])) + margen
        pos = enu(ll[:, 0], ll[:, 1], h, lat0, lng0)
        f = f'{nombre}_{gi}_{gj}.glb'
        total += glb(f, pos, np.concatenate(C), np.concatenate(I).ravel())
        manifiesto.append({"f": f, "lat": lat0, "lng": lng0})
    json.dump(manifiesto, open(OUT + nombre + '.json', 'w'))
    print(nombre, 'celdas', len(celdas), 'teselas', len(manifiesto), 'MB', round(total / 2**20, 1), round(time.time() - t0), 's', flush=True)


capa_h3('h3r3', 3, 3, 60, 6, 21)
capa_h3('h3r4', 4, 2, 50, 6, 17)
capa_h3('h3r5', 5, 1, 40, 4, 13)
capa_h3('h3r6', 6, 0, 30, 2.5, 9)
if 'solo_h3' in sys.argv: sys.exit(0)

# ── 1 M de «tiendas» (puntos) y su 10 % de más ingresos (columnas) ──
rng = np.random.default_rng(9)
N = 1_000_000
w = CI[:, 2] / CI[:, 2].sum()
k = rng.choice(len(CI), size=int(N * 0.85), p=w)
la = CI[k, 0] + rng.normal(0, 0.45, len(k)); lo = CI[k, 1] + rng.normal(0, 0.55, len(k))
minx, miny, maxx, maxy = POLI.bounds
ru = np.stack([rng.uniform(miny, maxy, N), rng.uniform(minx, maxx, N)], 1)
la = np.concatenate([la, ru[:, 0]]); lo = np.concatenate([lo, ru[:, 1]])
dentro = np.array([PP.contains(Point(x, y)) for x, y in zip(lo, la)])
la, lo = la[dentro][:N], lo[dentro][:N]
z = cota(la, lo); tierra = z > 0; la, lo, z = la[tierra], lo[tierra], z[tierra]
ingresos = rng.lognormal(0, 1, len(la)) * densidad(la, lo) ** 0.5
print('tiendas', len(la), flush=True)

def puntos(nombre, sel, caja_deg):
    grupos = {}
    for i in np.flatnonzero(sel):
        grupos.setdefault((int(la[i] // caja_deg), int(lo[i] // caja_deg)), []).append(i)
    man, total = [], 0
    for (gi, gj), ii in grupos.items():
        ii = np.array(ii); lat0, lng0 = float(la[ii].mean()), float(lo[ii].mean())
        pos = enu(la[ii], lo[ii], z[ii] + 40, lat0, lng0)
        col = np.concatenate([rampa(np.clip(ingresos[ii] / 30, 0, 1), MAGMA), np.full((len(ii), 1), 255)], 1)
        f = f'{nombre}_{gi}_{gj}.glb'; total += glb(f, pos, col, None, 0); man.append({"f": f, "lat": lat0, "lng": lng0})
    json.dump(man, open(OUT + nombre + '.json', 'w'))
    print(nombre, int(sel.sum()), 'teselas', len(man), 'MB', round(total / 2**20, 1), flush=True)
puntos('tiendas', np.ones(len(la), bool), 4)

# el 10 % de más ingresos: columnas hexagonales de ~600 m, altura por ingresos
top = ingresos >= np.quantile(ingresos, 0.9)
grupos = {}
for i in np.flatnonzero(top):
    grupos.setdefault((int(la[i] // 4), int(lo[i] // 4)), []).append(i)
man, total = [], 0
ang = np.pi / 3 * np.arange(6)
for (gi, gj), ii in grupos.items():
    ii = np.array(ii); lat0, lng0 = float(la[ii].mean()), float(lo[ii].mean())
    r = 600.0
    dla = r * np.sin(ang)[None] / 110574.0; dlo = r * np.cos(ang)[None] / (111320.0 * np.cos(np.radians(la[ii]))[:, None])
    base = z[ii][:, None] + 30; alto = base + 500 + 4000 * np.clip(ingresos[ii] / 40, 0, 1)[:, None]
    m = len(ii)
    lat_v = np.concatenate([la[ii][:, None] + dla, la[ii][:, None] + dla], 1); lng_v = np.concatenate([lo[ii][:, None] + dlo, lo[ii][:, None] + dlo], 1)
    h_v = np.concatenate([np.repeat(alto, 6, 1), np.repeat(base, 6, 1)], 1)
    pos = enu(lat_v.ravel(), lng_v.ravel(), h_v.ravel(), lat0, lng0)
    c = rampa(np.clip(ingresos[ii] / 40, 0, 1), MAGMA)
    col = np.concatenate([np.repeat(c, 6, 0).reshape(m, 6, 3), np.repeat(c * 0.7, 6, 0).reshape(m, 6, 3)], 1).reshape(-1, 3)
    col = np.concatenate([col, np.full((len(col), 1), 255)], 1)
    t1 = [[0, 1, 2], [0, 2, 3], [0, 3, 4], [0, 4, 5]]
    paredes = sum([[[a, 6 + a, (a + 1) % 6], [(a + 1) % 6, 6 + a, 6 + (a + 1) % 6]] for a in range(6)], [])
    tri = np.array(t1 + paredes)
    idx = (np.arange(m)[:, None, None] * 12 + tri[None]).reshape(-1)
    f = f'top_{gi}_{gj}.glb'; total += glb(f, pos, col, idx); man.append({"f": f, "lat": lat0, "lng": lng0})
json.dump(man, open(OUT + 'top.json', 'w'))
print('top', int(top.sum()), 'teselas', len(man), 'MB', round(total / 2**20, 1), flush=True)
