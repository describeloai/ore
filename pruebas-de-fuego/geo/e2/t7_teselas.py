# ORE 0072 E2·0 T7 · N hexágonos repartidos en K modelos glTF (uno por tesela, en coordenadas locales
# centradas en su tesela) y su manifiesto (posición lat/lng de cada modelo).
import json, math, sys, numpy as np
sys.argv = ['x']
exec(open('t6_glb.py').read().split('rng = np.random.default_rng(1)')[0])  # glb() y colores()

LAT0, LNG0 = 40.4168, -3.7038
rng = np.random.default_rng(2)

def caso(nombre, n, k, paso=40.0):
    lado = int(math.ceil(math.sqrt(n))); t = int(math.sqrt(k)); por = int(math.ceil(lado / t))
    r = paso * 0.45; ang = np.pi / 3 * np.arange(6); tri = np.array([[0, 1, 2], [0, 2, 3], [0, 3, 4], [0, 4, 5]])
    manifiesto = []
    for ti in range(t):
        for tj in range(t):
            ii, jj = np.meshgrid(np.arange(ti * por, min((ti + 1) * por, lado)), np.arange(tj * por, min((tj + 1) * por, lado)))
            ii, jj = ii.ravel(), jj.ravel()
            m = len(ii)
            if m == 0: continue
            cx_t = ((ti + 0.5) * por - lado / 2) * paso; cz_t = ((tj + 0.5) * por - lado / 2) * paso
            cx = (ii - lado / 2) * paso - cx_t; cz = (jj - lado / 2) * paso - cz_t
            vx = cx[:, None] + r * np.cos(ang)[None]; vz = cz[:, None] + r * np.sin(ang)[None]
            pos = np.stack([vx, np.zeros_like(vx), vz], -1).reshape(-1, 3)
            col = np.repeat(colores(m, rng), 6, axis=0)
            idx = ((np.arange(m) * 6)[:, None, None] + tri[None]).reshape(-1)
            f = f'{nombre}_{ti}_{tj}.glb'
            glb(f, pos, col, idx, 4)
            lat = LAT0 - cz_t / 111320.0; lng = LNG0 + cx_t / (111320.0 * math.cos(math.radians(LAT0)))
            manifiesto.append({"f": f, "lat": lat, "lng": lng, "n": m})
    json.dump(manifiesto, open(nombre + '.json', 'w'))
    print(nombre, 'modelos', len(manifiesto), 'elementos', sum(x['n'] for x in manifiesto))

caso('m1_k4', 1_000_000, 4)
caso('m1_k16', 1_000_000, 16)
caso('m1_k64', 1_000_000, 64)
caso('m3_k16', 3_000_000, 16)
caso('m3_k64', 3_000_000, 64)
caso('m01_k1', 100_000, 1)   # base de las variantes ligeras
