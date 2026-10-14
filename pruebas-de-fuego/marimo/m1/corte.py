"""M1·b · ¿cuánto vive un flujo SSE de marimo por la puerta pública?

La puerta (`plazo-de-la-puerta`, 19-la-puerta-del-tunel.yaml) corta toda petición a los
300 s, y un SSE de marimo dura lo que la sesión. Abre uno, lo deja quieto y anota cada
`keep-alive`, cada evento y cuándo y cómo acaba (con `close` de marimo o cortado).
`python corte.py <base> <fichero>` con M1_TOKEN en el entorno.
"""
import http.client
import json
import os
import sys
import time
import urllib.parse

BASE, FICHERO = sys.argv[1].rstrip("/"), sys.argv[2]
U = urllib.parse.urlsplit(BASE)
cab = {"User-Agent": "Mozilla/5.0 (m1-lab)", "x-m1-token": os.environ.get("M1_TOKEN", "")}
c = http.client.HTTPSConnection(U.hostname, U.port, timeout=900)
t0 = time.time()
c.request("GET", U.path + "/sse?session_id=corte-%d&file=%s" % (int(t0), FICHERO), headers=cab)
r = c.getresponse()
print("status", r.status, "cf-ray", r.getheader("cf-ray"), flush=True)
latidos, eventos, cierre = [], 0, None
try:
    while True:
        linea = r.readline()
        if not linea:
            break
        linea = linea.decode(errors="replace").rstrip("\n")
        if linea.startswith(":"):
            latidos.append(round(time.time() - t0, 1))
        elif linea.startswith("event: close"):
            cierre = "close"
        elif linea.startswith("data:"):
            eventos += 1
except Exception as e:  # noqa: BLE001
    cierre = "excepcion: %s" % e
dur = round(time.time() - t0, 1)
print("CORTE-RESULTADO " + json.dumps({"duro_s": dur, "como": cierre or "cortado sin close",
                                       "eventos": eventos, "latidos": len(latidos),
                                       "intervalo_latido_s": round((latidos[-1] - latidos[0]) / max(1, len(latidos) - 1), 1) if len(latidos) > 1 else None}))
