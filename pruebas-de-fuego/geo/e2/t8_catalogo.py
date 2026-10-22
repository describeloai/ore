# ORE 0072 E2·0 T8 · El catálogo visual sobre el 3D de Google, pegado al relieve.
# Cada tesela se ancla con CLAMP_TO_GROUND (Google pone su origen sobre SU suelo) y sus vértices llevan sólo la
# DIFERENCIA de altura respecto al ancla, sacada de Copernicus GLO-30 (+ un margen). Coordenadas del glTF: ENU
# (x este, y norte, z arriba) en metros desde el ancla.
import json, math, struct, sys
import numpy as np, rasterio, h3
from scipy.spatial import Voronoi

DEM = rasterio.open(r'C:\tmp\e2-t8\dem\N40W004.tif')
Z = DEM.read(1).astype('f8')
T = DEM.transform
MARGEN = 4.0
OUT = r'C:\tmp\e2-t8\web' + '\\'


def cota(lat, lng):
    """Bilineal sobre el DEM (lat, lng arrays)."""
    c = (np.asarray(lng) - T.c) / T.a - 0.5
    r = (np.asarray(lat) - T.f) / T.e - 0.5
    c0 = np.clip(np.floor(c).astype(int), 0, Z.shape[1] - 2); r0 = np.clip(np.floor(r).astype(int), 0, Z.shape[0] - 2)
    fc = c - c0; fr = r - r0
    return (Z[r0, c0] * (1 - fc) * (1 - fr) + Z[r0, c0 + 1] * fc * (1 - fr) + Z[r0 + 1, c0] * (1 - fc) * fr + Z[r0 + 1, c0 + 1] * fc * fr)


def enu(lat, lng, lat0, lng0):
    return (np.asarray(lng) - lng0) * 111320.0 * math.cos(math.radians(lat0)), (np.asarray(lat) - lat0) * 110574.0


# ── rampas ──
VIRIDIS = np.array([[68, 1, 84], [59, 82, 139], [33, 145, 140], [94, 201, 98], [253, 231, 37]], 'f8')
RDBU = np.array([[178, 24, 43], [239, 138, 98], [247, 247, 247], [103, 169, 207], [33, 102, 172]], 'f8')


def rampa(v, stops):
    v = np.clip(np.asarray(v, 'f8'), 0, 1) * (len(stops) - 1)
    i = np.minimum(v.astype(int), len(stops) - 2); f = (v - i)[..., None]
    return stops[i] * (1 - f) + stops[i + 1] * f


def glb(nombre, pos, col, idx):
    pos = np.asarray(pos, 'f4'); col = np.asarray(col).astype('u1'); idx = np.asarray(idx, '<u4')
    bufs = [pos.tobytes(), col.tobytes(), idx.tobytes()]
    offs, blob = [], b''
    for b in bufs:
        offs.append(len(blob)); blob += b + b'\0' * ((4 - len(b) % 4) % 4)
    g = {"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
         "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "COLOR_0": 1}, "indices": 2, "material": 0, "mode": 4}]}],
         "accessors": [{"bufferView": 0, "componentType": 5126, "count": len(pos), "type": "VEC3", "min": pos.min(0).tolist(), "max": pos.max(0).tolist()},
                       {"bufferView": 1, "componentType": 5121, "normalized": True, "count": len(col), "type": "VEC4"},
                       {"bufferView": 2, "componentType": 5125, "count": len(idx), "type": "SCALAR"}],
         "bufferViews": [{"buffer": 0, "byteOffset": offs[i], "byteLength": len(bufs[i])} for i in range(3)],
         "buffers": [{"byteLength": len(blob)}],
         "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 1},
                        "doubleSided": True, "alphaMode": "BLEND" if (col[:, 3] < 255).any() else "OPAQUE", "extensions": {"KHR_materials_unlit": {}}}],
         "extensionsUsed": ["KHR_materials_unlit"]}
    j = json.dumps(g).encode(); j += b' ' * ((4 - len(j) % 4) % 4)
    out = struct.pack('<III', 0x46546C67, 2, 28 + len(j) + len(blob)) + struct.pack('<II', len(j), 0x4E4F534A) + j + struct.pack('<II', len(blob), 0x004E4942) + blob
    open(OUT + nombre, 'wb').write(out)
    return len(out)


class Malla:
    def __init__(s): s.p, s.c, s.i, s.n = [], [], [], 0

    def add(s, pos, col, idx):
        s.p.append(pos); s.c.append(col); s.i.append(idx + s.n); s.n += len(pos)

    def vacia(s): return s.n == 0

    def arrays(s): return np.concatenate(s.p), np.concatenate(s.c), np.concatenate(s.i)


def plano(m, lats, lngs, lat0, lng0, z0, color, densificar=True):
    """Un polígono convexo-ish (anillo sin cerrar) como abanico desde su centroide, densificado (puntos medios),
    con cada vértice a la altura del terreno relativa al ancla."""
    lats = np.asarray(lats); lngs = np.asarray(lngs)
    if densificar:
        la2 = (lats + np.roll(lats, -1)) / 2; lo2 = (lngs + np.roll(lngs, -1)) / 2
        lats = np.stack([lats, la2], 1).ravel(); lngs = np.stack([lngs, lo2], 1).ravel()
    cla, clo = lats.mean(), lngs.mean()
    la = np.concatenate([[cla], lats]); lo = np.concatenate([[clo], lngs])
    x, y = enu(la, lo, lat0, lng0)
    z = cota(la, lo) - z0 + MARGEN
    k = len(lats)
    idx = np.array([[0, 1 + i, 1 + (i + 1) % k] for i in range(k)]).ravel()
    m.add(np.stack([x, y, z], 1), np.repeat([color], len(x), 0), idx)


def prisma(m, lats, lngs, lat0, lng0, z0, altura, color):
    """Columna: base al suelo más bajo de sus esquinas, tapa plana a base + altura, paredes algo más oscuras."""
    lats = np.asarray(lats); lngs = np.asarray(lngs); k = len(lats)
    x, y = enu(lats, lngs, lat0, lng0)
    base = cota(lats, lngs).min() - z0 + MARGEN; top = base + altura
    cx, cy = x.mean(), y.mean()
    pos = np.concatenate([[[cx, cy, top]], np.stack([x, y, np.full(k, top)], 1), np.stack([x, y, np.full(k, base)], 1)])
    oscuro = np.array(color, 'f8'); oscuro[:3] *= 0.7
    col = np.concatenate([np.repeat([color], 1 + k, 0), np.repeat([oscuro], k, 0)])
    tapa = [[0, 1 + i, 1 + (i + 1) % k] for i in range(k)]
    paredes = []
    for i in range(k):
        a, b = 1 + i, 1 + (i + 1) % k; c, d = 1 + k + i, 1 + k + (i + 1) % k
        paredes += [[a, c, b], [b, c, d]]
    m.add(pos, col, np.array(tapa + paredes).ravel())


def teselas(nombre, celdas, lado_km, dibujar):
    """Agrupa las celdas (lista de (lat, lng, datos)) por teselas de lado_km, una malla por tesela anclada en su centro."""
    grupos = {}
    for c in celdas:
        k = (int(c[0] * 110.574 / lado_km), int(c[1] * 111.32 * math.cos(math.radians(c[0])) / lado_km))
        grupos.setdefault(k, []).append(c)
    manifiesto, total = [], 0
    for (ki, kj), cs in grupos.items():
        lat0 = float(np.mean([c[0] for c in cs])); lng0 = float(np.mean([c[1] for c in cs]))
        z0 = float(cota(lat0, lng0))
        m = Malla()
        for c in cs:
            dibujar(m, c, lat0, lng0, z0)
        if m.vacia(): continue
        f = f'{nombre}_{ki}_{kj}.glb'
        total += glb(f, *m.arrays())
        manifiesto.append({"f": f, "lat": lat0, "lng": lng0, "z0": z0})
    json.dump(manifiesto, open(OUT + nombre + '.json', 'w'))
    print(nombre, 'teselas', len(manifiesto), 'celdas', len(celdas), 'MB', round(total / 2**20, 1), flush=True)


def caja_h3(lat0, lng0, km, res):
    dla = km / 2 / 110.574; dlo = km / 2 / (111.32 * math.cos(math.radians(lat0)))
    poli = h3.LatLngPoly([(lat0 - dla, lng0 - dlo), (lat0 - dla, lng0 + dlo), (lat0 + dla, lng0 + dlo), (lat0 + dla, lng0 - dlo)])
    return h3.polygon_to_cells(poli, res)


def metrica(lat, lng, semilla=0):
    """Una métrica sintética con estructura (núcleos + ruido): 0..1."""
    rng = np.random.default_rng(abs(hash((round(lat, 5), round(lng, 5), semilla))) % 2**32)
    v = 0.0
    for (la, lo, s) in ((40.4168, -3.7038, 0.03), (40.45, -3.69, 0.02), (40.78, -3.90, 0.04), (40.39, -3.65, 0.025)):
        v += math.exp(-((lat - la) ** 2 + (lng - lo) ** 2) / (2 * s * s))
    return float(min(1.0, 0.75 * v + 0.25 * rng.random()))


ZONAS = {} if 'perf' in sys.argv else {'ciudad': (40.4168, -3.7038), 'sierra': (40.765, -3.885)}
caso = sys.argv[1] if len(sys.argv) > 1 else 'todo'

for zona, (la0, lo0) in ZONAS.items():
    # 1 · H3 res 9 plano, rampa secuencial
    cs = [(*h3.cell_to_latlng(c), c) for c in caja_h3(la0, lo0, 8, 9)]
    teselas(f'{zona}_h3', cs, 2, lambda m, c, a, b, z0: plano(m, *zip(*h3.cell_to_boundary(c[2])), a, b, z0, list(rampa(metrica(c[0], c[1]), VIRIDIS)) + [220]))
    # 2 · rejilla cuadrada 250 m, rampa divergente
    paso = 0.25; n = int(8 / paso)
    cs = []
    for i in range(n):
        for j in range(n):
            la = la0 + ((i + 0.5) * paso - 4) / 110.574; lo = lo0 + ((j + 0.5) * paso - 4) / (111.32 * math.cos(math.radians(la0)))
            cs.append((la, lo, paso))
    dla = paso / 2 / 110.574; dlo = paso / 2 / (111.32 * math.cos(math.radians(la0)))
    teselas(f'{zona}_rejilla', cs, 2, lambda m, c, a, b, z0: plano(m, [c[0] - dla, c[0] - dla, c[0] + dla, c[0] + dla], [c[1] - dlo, c[1] + dlo, c[1] + dlo, c[1] - dlo], a, b, z0,
                                                                  list(rampa(metrica(c[0], c[1], 1), RDBU)) + [210]))
    # 3 · coropleta: Voronoi de 400 «secciones» irregulares, recortadas a la caja
    rng = np.random.default_rng(3)
    semillas = np.stack([la0 + rng.uniform(-4, 4, 400) / 110.574, lo0 + rng.uniform(-4, 4, 400) / (111.32 * math.cos(math.radians(la0)))], 1)
    vor = Voronoi(semillas)
    cs = []
    for k, reg in enumerate(vor.point_region):
        r = vor.regions[reg]
        if -1 in r or len(r) < 3: continue
        pts = vor.vertices[r]
        if np.abs(pts[:, 0] - la0).max() > 4.2 / 110.574 or np.abs(pts[:, 1] - lo0).max() > 4.2 / 85: continue
        cs.append((semillas[k, 0], semillas[k, 1], pts))
    teselas(f'{zona}_coropleta', cs, 3, lambda m, c, a, b, z0: plano(m, c[2][:, 0], c[2][:, 1], a, b, z0, list(rampa(metrica(c[0], c[1], 2), VIRIDIS)) + [200]))
    # 4 · composición RGB: tres métricas a R, G y B (H3 res 9)
    cs = [(*h3.cell_to_latlng(c), c) for c in caja_h3(la0, lo0, 8, 9)]
    teselas(f'{zona}_rgb', cs, 2, lambda m, c, a, b, z0: plano(m, *zip(*h3.cell_to_boundary(c[2])), a, b, z0,
                                                              [255 * metrica(c[0], c[1], 3), 255 * metrica(c[0], c[1], 4), 255 * metrica(c[0], c[1], 5), 230]))
    # 5 · columnas H3 (res 9), altura y color por la métrica
    cs = [(*h3.cell_to_latlng(c), c) for c in caja_h3(la0, lo0, 8, 9)]
    def col(m, c, a, b, z0):
        v = metrica(c[0], c[1], 6)
        lats, lngs = zip(*h3.cell_to_boundary(c[2]))
        # un poco más estrecha que la celda, para que se vean separadas
        lats = c[0] + (np.array(lats) - c[0]) * 0.8; lngs = c[1] + (np.array(lngs) - c[1]) * 0.8
        prisma(m, lats, lngs, a, b, z0, 20 + 600 * v, list(rampa(v, VIRIDIS)) + [255])
    teselas(f'{zona}_columnas', cs, 2, col)
    # 6 · bloques: rejilla de 500 m extruida
    paso = 0.5; n = int(8 / paso)
    dla = paso / 2 / 110.574 * 0.85; dlo = paso / 2 / (111.32 * math.cos(math.radians(la0))) * 0.85
    cs = [(la0 + ((i + 0.5) * paso - 4) / 110.574, lo0 + ((j + 0.5) * paso - 4) / (111.32 * math.cos(math.radians(la0)))) for i in range(n) for j in range(n)]
    def bloque(m, c, a, b, z0):
        v = metrica(c[0], c[1], 7)
        prisma(m, [c[0] - dla, c[0] - dla, c[0] + dla, c[0] + dla], [c[1] - dlo, c[1] + dlo, c[1] + dlo, c[1] - dlo], a, b, z0, 30 + 900 * v, list(rampa(v, RDBU)) + [255])
    teselas(f'{zona}_bloques', cs, 4, bloque)

# 7 · rendimiento: columnas H3 res 11 (~200 k) y res 11 en un área mayor (~1 M), pegadas al relieve
def col_fina(m, c, a, b, z0):
    v = metrica(c[0], c[1], 8)
    lats, lngs = zip(*h3.cell_to_boundary(c[2]))
    prisma(m, lats, lngs, a, b, z0, 5 + 120 * v, list(rampa(v, VIRIDIS)) + [255])
for nombre, km in (('perf_200k', 20), ('perf_1m', 45)):
    cs = [(*h3.cell_to_latlng(c), c) for c in caja_h3(40.55, -3.75, km, 11)]
    teselas(nombre, cs, 6, col_fina)
