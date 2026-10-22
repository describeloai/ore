"""X0 de HBase, ruta REST: cuanto tarda un scan completo de TestTable (10^6 filas, 4 regiones).

    python sonda_rest.py [json|proto] [batch] [filas_max]

Mide filas por segundo, bytes por la red y lo que pasa con un scanner que se deja abierto.
"""
import http.client
import json
import sys
import time

modo = sys.argv[1] if len(sys.argv) > 1 else "json"
batch = int(sys.argv[2]) if len(sys.argv) > 2 else 1000
maximo = int(sys.argv[3]) if len(sys.argv) > 3 else 10**9
TABLA = "TestTable"
ACCEPT = {"json": "application/json", "proto": "application/x-protobuf"}[modo]

c = http.client.HTTPConnection("127.0.0.1", 8080, timeout=120)
t0 = time.time()
c.request("PUT", f"/{TABLA}/scanner", body=f'<Scanner batch="{batch}"/>', headers={"Content-Type": "text/xml"})
r = c.getresponse()
r.read()
assert r.status == 201, r.status
loc = r.getheader("Location").split("8080", 1)[1]
filas = bytes_ = 0
while filas < maximo:
    c.request("GET", loc, headers={"Accept": ACCEPT})
    r = c.getresponse()
    cuerpo = r.read()
    if r.status == 204:
        break
    assert r.status == 200, (r.status, cuerpo[:200])
    bytes_ += len(cuerpo)
    if modo == "json":
        filas += len(json.loads(cuerpo)["Row"])
    else:
        filas += cuerpo.count(b"\x0a\x0a") or 0  # solo orientativo: la cuenta exacta es del cliente protobuf
c.request("DELETE", loc)
c.getresponse().read()
dt = time.time() - t0
print(f"{modo} batch={batch}: {filas} filas en {dt:.1f}s = {filas/dt:,.0f} filas/s, {bytes_/1e6:.0f} MB por la red")
