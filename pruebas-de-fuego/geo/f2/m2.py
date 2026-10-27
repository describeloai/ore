"""ORE 0072 F2·0 · M2: el salto. El motor de laboratorio sirviendo por HTTP (`servir`) y, delante, una puerta
que hace lo que `ore-serve` (`preview.rs` `al_motor`): un hilo por conexión y una conexión nueva al motor por
tesela, `connection: close`. La carga (`carga`): 1 y 32 lectores contra el motor solo y contra la puerta, las
mismas teselas al azar (z12–z16 sobre Madrid), con los pies ya en caché. Lo que añade el salto es la diferencia.
Sobre los edificios de F1·3 (`F2_ML`). Emite «F2M|…» y «M2|…». Lo lanza `correr.py m2`.
"""
import json
import os
import socket
import subprocess
import sys
import time
import urllib.request

sys.path.insert(0, "/opt/ore")
os.environ.setdefault("ORE_SERVE", "http://ore-serve.%s.svc.cluster.local:8080" % os.environ.get("NS", "t-victor"))
import ore  # noqa: E402

BANCO = "https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo"
SEG = os.environ.get("F2_SEGUNDOS", "30")


def di(que, **kw):
    print("M2|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)


tok = ore._token_de_google()
req = urllib.request.Request(BANCO + "/computo/f2/f2-motor", headers={"authorization": "Bearer " + tok})
open("/tmp/f2-motor", "wb").write(urllib.request.urlopen(req, timeout=120).read())
os.chmod("/tmp/f2-motor", 0o755)

con = ore._duckdb()
con.execute("load httpfs")
meta = ore._iceberg(os.environ["F2_ML"]).replace("iceberg_scan(", "iceberg_metadata(", 1)
FS = [p for p, st in con.execute(f"select file_path, status from {meta}").fetchall()
      if str(p).endswith(".parquet") and str(st) != "DELETED"]

env = dict(os.environ, F2_FICHEROS=",".join(FS))
motor = subprocess.Popen(["/tmp/f2-motor", "servir", "8097"], env=env, stdout=subprocess.PIPE, text=True)
puerta = subprocess.Popen(["/tmp/f2-motor", "puerta", "8098", "127.0.0.1:8097"], stdout=subprocess.PIPE, text=True)
for puerto in (8097, 8098):
    for _ in range(100):
        try:
            socket.create_connection(("127.0.0.1", puerto), timeout=1).close()
            break
        except OSError:
            time.sleep(0.2)
di("listos", ficheros=len(FS))


def carga(destino, lectores, seg):
    p = subprocess.run(["/tmp/f2-motor", "carga", destino, str(lectores), str(seg)], capture_output=True, text=True)
    for l in p.stdout.splitlines():
        if l.startswith("F2M|"):
            print(l, flush=True)


carga("127.0.0.1:8097", 32, 20)  # calentar: los pies en caché
for lectores in (1, 32):
    for nombre, destino in (("motor", "127.0.0.1:8097"), ("puerta", "127.0.0.1:8098")):
        di("ronda", via=nombre, lectores=lectores)
        carga(destino, lectores, SEG)
motor.kill()
puerta.kill()
di("fin")
