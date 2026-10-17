"""ORE 0067 · BigQuery: la misma tabla por REST (texto) y por la Storage Read (Arrow).

Dice filas/s de cada vía y si la geometría es la misma: el WKT tal cual y, si
difiere, comparado como geometría (shapely, `equals_exact` a 0).

    ORE_GCP_TOKEN=... python bigquery.py <binario> bigquery://<proyecto>/<dataset> tabla geo [tabla geo...]
"""
import io
import json
import subprocess
import sys
import time

import pyarrow.ipc as ipc
import shapely

binario, url, *resto = sys.argv[1:]
for tabla, geo in zip(resto[::2], resto[1::2]):
    pet = {"url": url, "objeto": tabla, "proyeccion": {"id": "id", geo: geo}}
    t0 = time.time()
    r = subprocess.run([binario, "leer"], input=json.dumps({**pet, "formato": "arrow"}).encode(), capture_output=True)
    sg_a = time.time() - t0
    if r.returncode:
        print(tabla, "arrow rc", r.returncode, r.stderr.decode()[:300])
        continue
    t = ipc.open_stream(io.BytesIO(r.stdout)).read_all()
    arrow = dict(zip(t.column("id").to_pylist(), t.column(geo).to_pylist()))
    t0 = time.time()
    r = subprocess.run([binario, "leer"], input=json.dumps(pet).encode(), capture_output=True)
    sg_r = time.time() - t0
    if r.returncode:
        print(tabla, "rest rc", r.returncode, r.stderr.decode()[:300])
        continue
    rest = {}
    for linea in r.stdout.decode().splitlines():
        f = json.loads(linea)
        rest[f["id"]] = f[geo]
    distintas_texto = [k for k in rest if rest[k] != arrow.get(k)]
    distintas = [k for k in distintas_texto
                 if (rest[k] is None) != (arrow.get(k) is None)
                 or (rest[k] is not None and not shapely.from_wkt(rest[k]).equals_exact(shapely.from_wkt(arrow[k]), 0))]
    print(f"{tabla}: {len(arrow)} filas · Storage Read {sg_a:.1f} s = {len(arrow) / sg_a:,.0f} filas/s, "
          f"{t.schema.field(geo).type} · REST {sg_r:.1f} s = {len(rest) / sg_r:,.0f} filas/s · "
          f"filas iguales {len(rest) == len(arrow) and set(rest) == set(arrow)} · "
          f"WKT distinto en {len(distintas_texto)}, geometría distinta en {len(distintas)}"
          + (f" (p. ej. {distintas_texto[0]}: {rest[distintas_texto[0]][:60]} | {arrow[distintas_texto[0]][:60]})" if distintas_texto else ""))
