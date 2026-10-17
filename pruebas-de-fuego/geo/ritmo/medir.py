"""ORE 0067 · el ritmo de copia de PostGIS, en local.

Lee cada tabla entera por `ore-read-postgres leer` (formato arrow) con un
binario dado y dice filas/s, MB y una huella del contenido que no depende de
cómo viaja la geometría (hex en texto o EWKB en binario): así el antes y el
después se comparan fila a fila.

    python medir.py <binario> <url> tabla [tabla...]
"""
import hashlib
import io
import json
import subprocess
import sys
import time

import pyarrow as pa
import pyarrow.ipc as ipc

binario, url, *tablas = sys.argv[1:]
for t in tablas:
    cols = {"lugares": ["id", "nombre", "categoria", "geom"],
            "lugares_geog": ["id", "nombre", "categoria", "geog"],
            "carreteras": ["id", "nombre", "tipo", "geom"],
            "lugares_x10": ["id", "nombre", "categoria", "geom"]}[t]
    pet = {"url": url, "objeto": f"public.{t}", "proyeccion": {c: c for c in cols}, "formato": "arrow"}
    t0 = time.time()
    r = subprocess.run([binario, "leer"], input=json.dumps(pet).encode(), capture_output=True)
    sg = time.time() - t0
    if r.returncode:
        print(t, "rc", r.returncode, r.stderr.decode()[:400])
        continue
    tabla = ipc.open_stream(io.BytesIO(r.stdout)).read_all()
    h = hashlib.md5()
    filas = sorted(
        tuple(v.hex().upper() if isinstance(v, bytes) else v for v in fila.values())
        for fila in tabla.to_pylist()
    )
    for f in filas:
        h.update(repr(f).encode())
    tipos = {c: str(tabla.schema.field(c).type) for c in cols}
    print(f"{t}: {tabla.num_rows} filas en {sg:.2f} s = {tabla.num_rows / sg:,.0f} filas/s, "
          f"{len(r.stdout) / 1e6:.1f} MB, md5 {h.hexdigest()[:12]}, geo {tipos[cols[-1]]}")
