# Genera mallas glTF (.glb) con N hexágonos (o N puntos) en una sola malla, en metros locales (Y arriba).
import json, struct, numpy as np, sys

def glb(nombre, pos, col, idx, modo):
    bufs = [pos.astype('<f4').tobytes(), col.astype('u1').tobytes()] + ([idx.astype('<u4').tobytes()] if idx is not None else [])
    offs, blob = [], b''
    for b in bufs:
        offs.append(len(blob)); blob += b + b'\0' * ((4 - len(b) % 4) % 4)
    vistas = [{"buffer": 0, "byteOffset": offs[i], "byteLength": len(bufs[i])} for i in range(len(bufs))]
    acc = [{"bufferView": 0, "componentType": 5126, "count": len(pos), "type": "VEC3", "min": pos.min(0).tolist(), "max": pos.max(0).tolist()},
           {"bufferView": 1, "componentType": 5121, "normalized": True, "count": len(col), "type": "VEC4"}]
    prim = {"attributes": {"POSITION": 0, "COLOR_0": 1}, "material": 0, "mode": modo}
    if idx is not None:
        acc.append({"bufferView": 2, "componentType": 5125, "count": len(idx), "type": "SCALAR"}); prim["indices"] = 2
    g = {"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
         "meshes": [{"primitives": [prim]}], "accessors": acc, "bufferViews": vistas, "buffers": [{"byteLength": len(blob)}],
         "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 1},
                        "doubleSided": True, "extensions": {"KHR_materials_unlit": {}}}],
         "extensionsUsed": ["KHR_materials_unlit"]}
    j = json.dumps(g).encode(); j += b' ' * ((4 - len(j) % 4) % 4)
    out = struct.pack('<III', 0x46546C67, 2, 12 + 8 + len(j) + 8 + len(blob)) + struct.pack('<II', len(j), 0x4E4F534A) + j + struct.pack('<II', len(blob), 0x004E4942) + blob
    open(nombre, 'wb').write(out); print(nombre, len(out) // 1024, 'KB')

def colores(n, rng):
    v = rng.random(n)
    c = np.zeros((n, 4)); c[:, 0] = 255 * v; c[:, 1] = 80 + 120 * (1 - v); c[:, 2] = 40; c[:, 3] = 255
    return c

rng = np.random.default_rng(1)
ANCHO = 40000.0
for n in (10_000, 100_000, 300_000, 1_000_000):
    lado = int(np.ceil(np.sqrt(n))); paso = ANCHO / lado; r = paso * 0.45
    i = np.arange(n); cx = (i % lado - lado / 2) * paso; cz = (i // lado - lado / 2) * paso
    ang = np.pi / 3 * np.arange(6)
    vx = cx[:, None] + r * np.cos(ang)[None, :]; vz = cz[:, None] + r * np.sin(ang)[None, :]
    pos = np.stack([vx, np.zeros_like(vx), vz], -1).reshape(-1, 3)
    c = np.repeat(colores(n, rng), 6, axis=0)
    base = (np.arange(n) * 6)[:, None]
    tri = np.array([[0, 1, 2], [0, 2, 3], [0, 3, 4], [0, 4, 5]])
    idx = (base[:, None, :] + tri[None]).reshape(-1)
    glb(f'hex_{n}.glb', pos, c, idx, 4)
    if n >= 100_000:
        glb(f'pts_{n}.glb', np.stack([cx, np.zeros(n), cz], -1), colores(n, rng), None, 0)
