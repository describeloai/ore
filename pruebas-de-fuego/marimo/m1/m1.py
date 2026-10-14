"""M1·a · la latencia de una celda, desde donde se llame.

`python3 m1.py <base>` con <base> = `http://127.0.0.1:2718/m` (en el pod, sin
relevo), `http://127.0.0.1:8080/m` (en el relevo: relevo → túnel → marimo) o
`http://localhost:18080/m` (desde fuera, por `kubectl port-forward`). Abre su
propia sesión sobre `medida.py` (que exista en el directorio de marimo) por
SSE, como el navegador, y cronometra POST → `idle` de la celda.
"""
import http.client
import json
import re
import statistics
import sys
import threading
import time
import urllib.parse
import urllib.request

BASE = sys.argv[1].rstrip("/")
SID = sys.argv[2] if len(sys.argv) > 2 else "m1-%d" % int(time.time())
FICHERO = sys.argv[3] if len(sys.argv) > 3 else "medida.py"
U = urllib.parse.urlsplit(BASE)
# La protección de versión de marimo: el navegador lee el token del HTML y lo
# manda en cada POST; aquí, igual.
_HTML = urllib.request.urlopen(BASE + "/?file=" + FICHERO, timeout=30).read().decode()
TOKEN = re.search(r'data-token="([^"]+)"', _HTML).group(1)


def post(path, body):
    req = urllib.request.Request(BASE + path, data=json.dumps(body).encode(), method="POST",
                                 headers={"Content-Type": "application/json", "Marimo-Session-Id": SID, "Marimo-Server-Token": TOKEN})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.status


class Flujo:
    def __init__(self):
        self.msgs, self.cond = [], threading.Condition()
        c = http.client.HTTPConnection(U.hostname, U.port, timeout=600)
        c.request("GET", U.path + f"/sse?session_id={SID}&file={FICHERO}")
        self.r = c.getresponse()
        threading.Thread(target=self._leer, daemon=True).start()

    def _leer(self):
        buf = ""
        while True:
            linea = self.r.readline()
            if not linea:
                return
            linea = linea.decode()
            if linea.startswith("data:"):
                buf += linea[5:].strip()
            elif linea.strip() == "" and buf:
                try:
                    m = json.loads(buf)
                except Exception:
                    m = {}
                buf = ""
                with self.cond:
                    self.msgs.append((time.time(), m))
                    self.cond.notify_all()

    def esperar(self, pred, desde=0, timeout=120):
        fin = time.time() + timeout
        with self.cond:
            while True:
                for i in range(desde, len(self.msgs)):
                    if pred(self.msgs[i][1]):
                        return self.msgs[i][0]
                if time.time() > fin:
                    raise TimeoutError
                self.cond.wait(fin - time.time())


def idle(cid):
    return lambda m: m.get("op") == "cell-op" and (m.get("data") or {}).get("cell_id") == cid \
        and (m.get("data") or {}).get("status") == "idle"


def correr(f, cid, codigo, fin=None):
    n = len(f.msgs)
    t0 = time.time()
    post("/api/kernel/run", {"cellIds": [cid], "codes": [codigo]})
    return (f.esperar(idle(fin or cid), n) - t0) * 1000


def pct(xs):
    xs = sorted(xs)
    return {"p50": round(statistics.median(xs), 1), "p95": round(xs[int(len(xs) * 0.95) - 1], 1),
            "min": round(xs[0], 1), "n": len(xs)}


def main():
    R = {"base": BASE}
    t0 = time.time()
    f = Flujo()
    f.esperar(lambda m: m.get("op") == "kernel-ready")
    R["sse_hasta_kernel_ready_ms"] = round((time.time() - t0) * 1000)
    post("/api/kernel/instantiate", {"objectIds": [], "values": [], "autoRun": True})
    xs = []
    for _ in range(20):
        t = time.time()
        urllib.request.urlopen(BASE + "/api/status", timeout=10).read()
        xs.append((time.time() - t) * 1000)
    R["get_status_ms"] = pct(xs)
    correr(f, "t", "x = 0")
    R["celda_trivial_ms"] = pct([correr(f, "t", f"x = {i}\nx") for i in range(40)])
    correr(f, "a", "a = 1")
    correr(f, "b", "b = a * 2")
    correr(f, "c", "c = b + 1\nc")
    R["cadena_3_celdas_ms"] = pct([correr(f, "a", f"a = {i}", fin="c") for i in range(20)])
    print("M1-RESULTADO " + json.dumps(R))


if __name__ == "__main__":
    main()
