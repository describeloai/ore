"""ORE 0072 F2·3·2 · PC3 (dentro de un Job de victor): servir la pirámide desde GCS.

Por cada capa y zoom, las teselas de una vista de 1600×900 (512 px, como MapLibre) TODAS A LA VEZ, leídas del
`.pmtiles` del banco por rango (conexiones con keep-alive por hilo, como un servidor). Dos casos:
  · fría: un lector nuevo (cabecera + raíz + hojas + teselas): el primer visitante de un servidor recién arrancado;
  · servidor: el lector vivo (cabecera, raíz y hojas ya en memoria), en otra zona: sólo las teselas.
Sale `PC3|vista|{json}` con la vista entera, el p50/p95 por tesela, los GET y los KB.
"""
import http.client
import json
import math
import os
import sys
import threading
import time

sys.path.insert(0, "/opt/ore")
import ore  # noqa: E402

BUCKET = "project-8853a180-450d-47be-b83-banco-geo"
TOKEN = ore._token_de_google()
_local = threading.local()


def di(que, **kw):
    print("PC3|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)


def conexion():
    c = getattr(_local, "c", None)
    if c is None:
        c = _local.c = http.client.HTTPSConnection("storage.googleapis.com", timeout=30)
    return c


def lectora(objeto):
    ruta = "/%s/piramide/%s" % (BUCKET, objeto)

    def leer(desde, largo):
        for intento in range(2):
            try:
                c = conexion()
                c.request("GET", ruta, headers={"authorization": "Bearer " + TOKEN,
                                                "range": "bytes=%d-%d" % (desde, desde + largo - 1)})
                r = c.getresponse()
                b = r.read()
                if r.status not in (200, 206):
                    raise RuntimeError("GET %s: %s %s" % (objeto, r.status, b[:200]))
                return b
            except (http.client.HTTPException, OSError):
                _local.c = None
                if intento:
                    raise
    return leer


def tesela_de(lon, lat, z):
    n = 2 ** z
    return (lon + 180) / 360 * n, (1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n


def vista(lon, lat, z, ancho=1600, alto=900):
    cx, cy = tesela_de(lon, lat, z)
    mx, my = ancho / 512 / 2, alto / 512 / 2
    return [(z, x, y) for x in range(math.floor(cx - mx), math.floor(cx + mx) + 1)
            for y in range(math.floor(cy - my), math.floor(cy + my) + 1)]


def pct(v, p):
    v = sorted(v)
    return round(v[min(len(v) - 1, int(round(p * (len(v) - 1))))]) if v else None


def una_vista(lector, teselas):
    gets0 = lector.gets
    ms, kb, vacias = [None] * len(teselas), [0] * len(teselas), [0]

    def pedir(i, t):
        t0 = time.time()
        b = lector.tesela(*t)
        ms[i] = (time.time() - t0) * 1000
        if b:
            kb[i] = len(b) / 1024
        else:
            vacias[0] += 1

    # la cabecera la pide la primera (como el cliente: hasta tenerla no se sabe dónde están las teselas)
    t0 = time.time()
    hilos = [threading.Thread(target=pedir, args=(i, t)) for i, t in enumerate(teselas)]
    for h in hilos:
        h.start()
    for h in hilos:
        h.join()
    return {"vista_ms": round((time.time() - t0) * 1000), "teselas": len(teselas), "vacias": vacias[0],
            "p50_ms": pct(ms, 0.5), "p95_ms": pct(ms, 0.95), "max_ms": pct(ms, 1),
            "gets": lector.gets - gets0, "kb": round(sum(kb)), "kb_max": round(max(kb), 1)}


CAPAS = json.loads(os.environ.get("PC3_CAPAS", "null")) or [
    ["trip_3m_puntos-0.pmtiles", [-73.98, 40.75], list(range(9, 17))],
    ["edificios_densidad-0.pmtiles", [-3.70, 40.42], list(range(9, 13))],
    ["edificios-0.pmtiles", [-3.70, 40.42], [13, 14]],
    ["trip_3m_lineas_densidad-0.pmtiles", [-73.98, 40.75], list(range(9, 14))],
]


def main():
    # el lector tiene un candado sólo para contar: la caché de hojas es un dict (en CPython, seguro)
    from lector_pmtiles import Lector
    for objeto, (lon, lat), zooms in CAPAS:
        try:
            vivo = Lector(lectora(objeto))
            for z in zooms:
                # fría: un lector nuevo, la vista del centro
                fria = una_vista(Lector(lectora(objeto)), vista(lon, lat, z))
                # servidor: el lector vivo, en una zona de al lado cada vez (sin teselas repetidas)
                if vivo.cabecera is None:
                    vivo.tesela(0, 0, 0)
                d = 0.05 * 2 ** (13 - z)
                servidor = [una_vista(vivo, vista(lon + d * k, lat, z)) for k in (1, 2, 3)]
                di("vista", capa=objeto, z=z, fria=fria,
                   servidor={"vista_ms": [s["vista_ms"] for s in servidor], "p95_ms": [s["p95_ms"] for s in servidor],
                             "gets": [s["gets"] for s in servidor], "kb": [s["kb"] for s in servidor]})
        except Exception as e:  # noqa: BLE001
            di("falla", capa=objeto, error=str(e)[:300])
    di("fin")


main()
