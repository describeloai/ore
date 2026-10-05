# 0053 F4·3 · Lectura en vivo desde el puesto (test6): tres orígenes, sin copiar.
import json, time, urllib.request, urllib.error
import pyarrow as pa
import ore

s = ore.session

def leer(cuerpo):
    req = urllib.request.Request(s.servidor + "/federation/read",
                                 data=json.dumps(cuerpo).encode(), method="POST")
    req.add_header("content-type", "application/json")
    for k, v in (s._proveedor() if s._proveedor else s._cabeceras).items():
        req.add_header(k, v)
    if s.id:
        req.add_header("x-ore-puesto", s.id)
    t = time.time()
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            tabla = pa.ipc.open_stream(r.read()).read_all()
        print(f"✓ {cuerpo['tabla']}: {tabla.num_rows} filas · {tabla.num_columns} cols · {time.time()-t:.2f}s")
        return tabla
    except urllib.error.HTTPError as e:
        print(f"✗ {cuerpo['tabla']}: {e.code} {e.read().decode('utf-8','replace')[:300]}")

pg = leer({"tabla": "postgresql_20260921_2055.public.ai_insights", "limit": 5})
bq = leer({"tabla": "bigquery_20260927_1428.ventas.clientes", "limit": 5})
s3 = leer({"tabla": "s3_demo.nueva_carpeta.pedidos", "limit": 5})
# El tope: una tabla grande se corta en el presupuesto, no tumba nada.
grande = leer({"tabla": "bigquery_20260927_1428.ventas.ore_e2e_sintetica"})

for t in (pg, bq, s3):
    if t is not None:
        print(t.slice(0, 3).to_pandas())
