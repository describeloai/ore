"""ORE 0072 F2·3·2 · lanza PC3 (`pc3_job.py` + `lector_pmtiles.py`) en un Job de victor e imprime la tabla.

  python pruebas-de-fuego/geo/f2/piramide/pc3.py
"""
import importlib.util
import json
import os
import time

S = os.path.dirname(os.path.abspath(__file__)) + os.sep
G = os.path.dirname(os.path.dirname(S.rstrip(os.sep))) + os.sep
spec = importlib.util.spec_from_file_location("vara", G + "vara.py")
vara = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vara)

# el lector, como módulo dentro del Job
codigo = ("import sys, types\nm = types.ModuleType('lector_pmtiles')\nexec(%r, m.__dict__)\nsys.modules['lector_pmtiles'] = m\n"
          % open(S + "lector_pmtiles.py", encoding="utf-8").read()) + open(S + "pc3_job.py", encoding="utf-8").read()
fecha = time.strftime("%Y%m%d-%H%M")
log = vara.job("pc3-piramide-%d" % (int(time.time()) % 10**6), ["python3", "-u", "-c", codigo],
               imagen=vara.PUESTO, plazo=1800, memoria="1Gi")
ruta = S + "pc3-%s.log" % fecha
open(ruta, "w", encoding="utf-8", newline="\n").write(log)
print("%-34s %3s | %-36s | %s" % ("capa", "z", "fría: vista · p95 tesela · GET · KB", "servidor (3 vistas): vista ms · p95 ms · GET"))
for l in log.splitlines():
    if l.startswith("PC3|vista|"):
        d = json.loads(l.split("|", 2)[2])
        f, s = d["fria"], d["servidor"]
        print("%-34s %3d | %5d ms · %4d ms · %3d · %5d KB | %s · %s · %s" % (
            d["capa"], d["z"], f["vista_ms"], f["p95_ms"], f["gets"], f["kb"], s["vista_ms"], s["p95_ms"], s["gets"]))
    elif l.startswith(("PC3|falla", "##")) or "Traceback" in l:
        print(l[:400])
print("registro:", ruta)
