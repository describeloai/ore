"""ORE 0072 F2·0 · M1: el polígono en Rust. El motor de laboratorio (`motor/`, el binario en el bucket del
banco) sobre los edificios que dejó F1·3 por el SDK (`F2_ML`: su `metadata_location`): Madrid de z8 a z16, en
frío y con los pies en caché, y la concurrencia; con la simplificación RDP y con Visvalingam que conserva la
topología. Luego el contraste: la misma tesela por `ST_AsMVT` de DuckDB (con la poda por `geom_bbox`) y las dos
decodificadas aquí (elementos, vértices, anillos por orientación, lo que no cumple la spec 2.1). Emite «F2M|…»
(el motor) y «M1|…» (el contraste). Lo lanza `correr.py m1`.
"""
import json
import math
import os
import subprocess
import sys
import time
import urllib.request

sys.path.insert(0, "/opt/ore")
os.environ.setdefault("ORE_SERVE", "http://ore-serve.%s.svc.cluster.local:8080" % os.environ.get("NS", "t-victor"))
import ore  # noqa: E402
from ore import geo  # noqa: E402

BANCO = "https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo"
ML = os.environ["F2_ML"]


def di(que, **kw):
    print("M1|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)


tok = ore._token_de_google()
req = urllib.request.Request(BANCO + "/computo/f2/f2-motor", headers={"authorization": "Bearer " + tok})
open("/tmp/f2-motor", "wb").write(urllib.request.urlopen(req, timeout=120).read())
os.chmod("/tmp/f2-motor", 0o755)

con = ore._duckdb()
for e in ("spatial", "httpfs"):
    con.execute("load " + e)
con.execute("set geometry_always_xy = true")
con.execute("set threads = 8")
scan = ore._iceberg(ML)
meta = scan.replace("iceberg_scan(", "iceberg_metadata(", 1)
FS = [p for p, st in con.execute(f"select file_path, status from {meta}").fetchall()
      if str(p).endswith(".parquet") and str(st) != "DELETED"]
di("ficheros", n=len(FS), duckdb=con.execute("select version()").fetchone()[0])

for simplifica, seg in (("rdp", os.environ.get("F2_SEGUNDOS", "30")), ("vw", "5")):
    env = dict(os.environ, F2_FICHEROS=",".join(FS), F2_SIMPLIFICA=simplifica, F2_SEGUNDOS=seg)
    p = subprocess.run(["/tmp/f2-motor", "medir"], env=env, capture_output=True, text=True, timeout=3600)
    for l in p.stdout.splitlines():
        if l.startswith("F2M|"):
            k, v = l.split("|", 2)[1:]
            d = json.loads(v)
            d["simplifica"] = simplifica
            print("F2M|%s|%s" % (k, json.dumps(d)), flush=True)
    if p.returncode != 0:
        di("motor_falla", simplifica=simplifica, codigo=p.returncode, err=p.stderr[-800:])
    if simplifica == "rdp":
        os.makedirs("/tmp/rdp", exist_ok=True)
        for f in os.listdir("/tmp/teselas"):
            os.replace("/tmp/teselas/" + f, "/tmp/rdp/" + f)


# ── El decodificador (protobuf a mano): lo que hay en la tesela y si cumple la spec 2.1 ──────────────
def varints(b, i, fin):
    while i < fin:
        v, s = 0, 0
        while True:
            c = b[i]
            i += 1
            v |= (c & 0x7F) << s
            s += 7
            if c < 0x80:
                break
        yield v, i


def campos(b, i=0, fin=None):
    fin = len(b) if fin is None else fin
    while i < fin:
        k, i = next(varints(b, i, fin))
        n, t = k >> 3, k & 7
        if t == 0:
            v, i = next(varints(b, i, fin))
            yield n, v, None
        elif t == 2:
            ln, i = next(varints(b, i, fin))
            yield n, i, i + ln
            i += ln
        elif t == 5:
            i += 4
        elif t == 1:
            i += 8
        else:
            raise ValueError("tipo %d" % t)


def decodificar(b):
    out = {"capas": 0, "elementos": 0, "vertices": 0, "exteriores": 0, "agujeros": 0, "anillos_nulos": 0,
           "tipos": {}, "ids_repetidos": 0, "sin_id": 0}
    for n, a, z in campos(b):
        if n != 3:
            continue
        out["capas"] += 1
        ids = set()
        for m, fa, fz in campos(b, a, z):
            if m != 2:
                continue
            out["elementos"] += 1
            fid, tipo, geom = None, 0, []
            for q, ga, gz in campos(b, fa, fz):
                if q == 1:
                    fid = ga
                elif q == 3:
                    tipo = ga
                elif q == 4:
                    geom = [v for v, _ in varints(b, ga, gz)]
            out["tipos"][tipo] = out["tipos"].get(tipo, 0) + 1
            if fid is None:
                out["sin_id"] += 1
            elif fid in ids:
                out["ids_repetidos"] += 1
            else:
                ids.add(fid)
            if tipo != 3:
                continue
            x = y = 0
            k = 0
            anillo = []
            while k < len(geom):
                cmd, cnt = geom[k] & 7, geom[k] >> 3
                k += 1
                if cmd in (1, 2):
                    for _ in range(cnt):
                        dx, dy = geom[k], geom[k + 1]
                        k += 2
                        x += (dx >> 1) ^ -(dx & 1)
                        y += (dy >> 1) ^ -(dy & 1)
                        if cmd == 1:
                            anillo = []
                        anillo.append((x, y))
                        out["vertices"] += 1
                elif cmd == 7:
                    s = sum(anillo[j][0] * anillo[(j + 1) % len(anillo)][1] - anillo[(j + 1) % len(anillo)][0] * anillo[j][1]
                            for j in range(len(anillo)))
                    if s > 0:
                        out["exteriores"] += 1
                    elif s < 0:
                        out["agujeros"] += 1
                    else:
                        out["anillos_nulos"] += 1
    return out


def tesela_de(lon, lat, z):
    n = 2 ** z
    return int((lon + 180) / 360 * n), int((1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n)


def caja(z, x, y, margen=64 / 4096):
    n = 2 ** z
    lat = lambda t: math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * t / n))))
    x0, x1 = x / n * 360 - 180, (x + 1) / n * 360 - 180
    y0, y1 = lat(y + 1), lat(y)
    mx, my = (x1 - x0) * margen, (y1 - y0) * margen
    return x0 - mx, y0 - my, x1 + mx, y1 + my


rel = ore._con_geo(scan, {"geo": {"geom": {"crs": "EPSG:4326", "covering": "geom_bbox"}}})
for z in range(10, 17):
    x, y = tesela_de(-3.7038, 40.4168, z)
    q = (f"select ST_AsMVT({{'id': id, 'geom': g}}, 'edificios', 4096, 'geom', 'id') from ("
         f"select id, ST_AsMVTGeom(ST_Transform(geom, 'EPSG:4326', 'EPSG:3857', true), "
         f"ST_Extent(ST_TileEnvelope({z}, {x}, {y})), 4096, 64, true) g "
         f"from {rel} where {geo.intersects_box('geom', *caja(z, x, y))}) where g is not null")
    out = {"z": z}
    try:
        ts = []
        for _ in range(2):
            t0 = time.time()
            b = con.execute(q).fetchone()[0] or b""
            ts.append(time.time() - t0)
        out["duckdb"] = {"frio_ms": round(ts[0] * 1000), "ms": round(ts[1] * 1000), "kb": round(len(b) / 1024, 1),
                         **decodificar(bytes(b))}
    except Exception as e:
        out["duckdb"] = {"error": str(e)[:300]}
    nuestra = "/tmp/rdp/%d_%d_%d.mvt" % (z, x, y)
    if os.path.exists(nuestra):
        b = open(nuestra, "rb").read()
        out["motor"] = {"kb": round(len(b) / 1024, 1), **decodificar(b)}
    di("contraste", **out)
di("fin")
