# ORE 0072 E2·0 T11a · El techo de la GPU: teselas de 1 M de cuadrados (2 triángulos, color por vértice), plano a cota fija.
import json, math, struct, numpy as np
OUT = 'C:/tmp/e2-t11/web/'
def glb(nombre, pos, col, idx):
    bufs = [pos.astype('<f4').tobytes(), col.astype('u1').tobytes(), idx.astype('<u4').tobytes()]
    offs, blob = [], b''
    for b in bufs:
        offs.append(len(blob)); blob += b + b'\0' * ((4 - len(b) % 4) % 4)
    g = {"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
         "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "COLOR_0": 1}, "indices": 2, "material": 0, "mode": 4}]}],
         "accessors": [{"bufferView": 0, "componentType": 5126, "count": len(pos), "type": "VEC3", "min": pos.min(0).tolist(), "max": pos.max(0).tolist()},
                       {"bufferView": 1, "componentType": 5121, "normalized": True, "count": len(col), "type": "VEC4"},
                       {"bufferView": 2, "componentType": 5125, "count": len(idx), "type": "SCALAR"}],
         "bufferViews": [{"buffer": 0, "byteOffset": offs[i], "byteLength": len(bufs[i])} for i in range(3)], "buffers": [{"byteLength": len(blob)}],
         "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 1}, "doubleSided": True,
                        "alphaMode": "OPAQUE", "extensions": {"KHR_materials_unlit": {}}}], "extensionsUsed": ["KHR_materials_unlit"]}
    j = json.dumps(g).encode(); j += b' ' * ((4 - len(j) % 4) % 4)
    open(OUT + nombre, 'wb').write(struct.pack('<III', 0x46546C67, 2, 28 + len(j) + len(blob)) + struct.pack('<II', len(j), 0x4E4F534A) + j + struct.pack('<II', len(blob), 0x004E4942) + blob)
LADO, PASO = 1000, 30.0          # 1 M de celdas de 30 m: una tesela de 30 km
rng = np.random.default_rng(1)
i, j = np.meshgrid(np.arange(LADO), np.arange(LADO)); i = i.ravel(); j = j.ravel()
x0 = (i - LADO / 2) * PASO; y0 = (j - LADO / 2) * PASO; s = PASO * 0.42
pos = np.stack([np.stack([x0 - s, y0 - s], 1), np.stack([x0 + s, y0 - s], 1), np.stack([x0 + s, y0 + s], 1), np.stack([x0 - s, y0 + s], 1)], 1).reshape(-1, 2)
pos = np.concatenate([pos, np.zeros((len(pos), 1))], 1)
idx = (np.arange(len(i))[:, None] * 4 + np.array([0, 1, 2, 0, 2, 3])[None]).ravel()
LAT0, LNG0 = 40.0, -4.6
man = []
for t in range(30):
    ti, tj = t % 6, t // 6
    lat = LAT0 + tj * 30000 / 110574.0; lng = LNG0 + ti * 30000 / (111320.0 * math.cos(math.radians(LAT0)))
    v = rng.random(len(i))
    c = np.stack([255 * v, 255 * (1 - v), 128 + 0 * v, 255 + 0 * v], 1)
    glb(f'c{t}.glb', pos, np.repeat(c, 4, 0), idx)
    man.append([f'c{t}.glb', lat, lng])
json.dump(man, open(OUT + 'techo.json', 'w'))
print('ok', len(man))
