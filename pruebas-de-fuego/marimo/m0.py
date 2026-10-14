"""M0 · marimo dentro de un puesto (sonda en t-victor, red del puesto).

Mide: arranque del servidor (con y sin update-check, sin internet), memoria
(servidor solo, + kernel, + pandas/duckdb), latencia de una celda por SSE+POST
(el transporte que cabría por la correa), una cadena reactiva, DuckDB, que el
SDK `ore` se importa en el kernel, la salida a internet y el viaje
pod → ore-serve (el coste de cada salto de un relevo).
"""
import http.client
import json
import os
import statistics
import subprocess
import sys
import threading
import time
import urllib.request

import psutil

PORT = 2718
DIR = "/tmp/nb"
NB = "nb.py"
R = {}


def log(*a):
    print(*a, flush=True)


def get(path, timeout=5):
    with urllib.request.urlopen(f"http://127.0.0.1:{PORT}{path}", timeout=timeout) as r:
        return r.status, r.read()


def post(path, body, sid):
    req = urllib.request.Request(
        f"http://127.0.0.1:{PORT}{path}",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Marimo-Session-Id": sid},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.status, r.read()


def rss_arbol(pid):
    try:
        p = psutil.Process(pid)
        procs = [p] + p.children(recursive=True)
        return sum(x.memory_info().rss for x in procs) / 2**20, len(procs)
    except psutil.NoSuchProcess:
        return 0, 0


def arrancar(extra):
    os.makedirs(DIR, exist_ok=True)
    with open(os.path.join(DIR, NB), "w") as f:
        f.write(
            "import marimo\napp = marimo.App()\n\n@app.cell\ndef _():\n    return\n\n"
            "if __name__ == '__main__':\n    app.run()\n"
        )
    env = dict(os.environ, HOME="/tmp", PYTHONPATH="/opt/ore:/capa", MARIMO_SKIP_UPDATE_CHECK="")
    cfg = "/tmp/.config/marimo"
    os.makedirs(cfg, exist_ok=True)
    with open(os.path.join(cfg, "marimo.toml"), "w") as f:
        f.write('[server]\ntransport = "sse"\n\n[runtime]\nauto_instantiate = false\n')
    orden = ["marimo", "edit", "--headless", "--host", "127.0.0.1", "--port", str(PORT),
             "--no-token", "--no-sandbox", "--no-skew-protection", DIR] + extra
    t0 = time.time()
    p = subprocess.Popen(orden, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    salida = []
    threading.Thread(target=lambda: [salida.append(l) for l in p.stdout], daemon=True).start()
    while time.time() - t0 < 180:
        try:
            if get("/api/status", 2)[0] == 200:
                break
        except Exception:
            time.sleep(0.05)
    return p, time.time() - t0, salida


class Flujo:
    """El flujo SSE de una sesión: cada mensaje con su hora de llegada."""

    def __init__(self, sid):
        self.sid = sid
        self.msgs = []
        self.cond = threading.Condition()
        c = http.client.HTTPConnection("127.0.0.1", PORT, timeout=300)
        c.request("GET", f"/sse?session_id={sid}&file={NB}")
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
                    m = {"op": "?", "raw": buf[:200]}
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
                        return self.msgs[i][0], i
                rest = fin - time.time()
                if rest <= 0:
                    raise TimeoutError(f"sin mensaje; ops vistas: {[m['op'] for _, m in self.msgs[-15:]]}")
                self.cond.wait(rest)


def idle_de(cid):
    def pred(m):
        d = m.get("data") or {}
        return m.get("op") == "cell-op" and d.get("cell_id") == cid and d.get("status") == "idle"
    return pred


def correr(flujo, cid, codigo):
    n = len(flujo.msgs)
    t0 = time.time()
    post("/api/kernel/run", {"cellIds": [cid], "codes": [codigo]}, flujo.sid)
    t1, i = flujo.esperar(idle_de(cid), n)
    # la salida de la celda (último cell-op con output)
    out = None
    for _, m in flujo.msgs[n:i + 1]:
        d = m.get("data") or {}
        if m.get("op") == "cell-op" and d.get("cell_id") == cid and d.get("output"):
            out = d["output"]
    return (t1 - t0) * 1000, out


def pct(xs):
    xs = sorted(xs)
    return {"p50": round(statistics.median(xs), 1), "p95": round(xs[int(len(xs) * 0.95) - 1], 1),
            "min": round(xs[0], 1), "n": len(xs)}


def salida_a_internet():
    res = {}
    for url in ["https://pypi.org/simple/marimo/", "https://github.com", "https://cdn.jsdelivr.net",
                "https://api.github.com/repos/marimo-team/marimo/releases/latest"]:
        t0 = time.time()
        try:
            urllib.request.urlopen(url, timeout=5)
            res[url] = "ALCANZA"
        except Exception as e:  # noqa: BLE001
            res[url] = f"no ({type(e).__name__}, {time.time() - t0:.1f}s)"
    return res


def viaje_a_ore_serve(n=50):
    xs = []
    for _ in range(n):
        t0 = time.time()
        try:
            urllib.request.urlopen(os.environ.get("ORE_SERVE", "http://ore-serve:8080") + "/salud", timeout=5).read()
            xs.append((time.time() - t0) * 1000)
        except Exception as e:  # noqa: BLE001
            return f"error: {e}"
    return pct(xs)


def main():
    import marimo
    R["marimo"] = marimo.__version__
    R["python"] = sys.version.split()[0]
    t0 = time.time()
    subprocess.run([sys.executable, "-c", "import marimo"], check=True)
    R["import_marimo_s"] = round(time.time() - t0, 2)
    R["internet"] = salida_a_internet()
    R["pod_a_ore_serve_ms"] = viaje_a_ore_serve()

    # ── A · arranque, sin internet, con y sin la comprobación de versión ──
    for nombre, extra in [("sin_update_check", ["--skip-update-check"]), ("con_update_check", [])]:
        tiempos = []
        for _ in range(3):
            p, t, salida = arrancar(extra)
            tiempos.append(round(t, 2))
            time.sleep(1)
            rss, nproc = rss_arbol(p.pid)
            p.terminate()
            p.wait(10)
            time.sleep(0.5)
        R[f"arranque_{nombre}_s"] = tiempos
        R[f"rss_servidor_solo_mib_{nombre}"] = round(rss, 1)
        R[f"log_{nombre}"] = [l.rstrip() for l in salida if l.strip()][-6:]

    # ── B · sesión: kernel listo, memoria ──
    p, t, salida = arrancar(["--skip-update-check"])
    R["rss_servidor_antes_de_sesion_mib"] = round(rss_arbol(p.pid)[0], 1)
    t0 = time.time()
    f = Flujo("s1")
    tk, _ = f.esperar(lambda m: m.get("op") == "kernel-ready")
    R["sse_hasta_kernel_ready_ms"] = round((tk - t0) * 1000)
    post("/api/kernel/instantiate", {"objectIds": [], "values": [], "autoRun": True}, "s1")
    time.sleep(2)
    rss, nproc = rss_arbol(p.pid)
    R["rss_servidor_mas_kernel_mib"] = round(rss, 1)
    R["procesos"] = nproc
    R["ops_vistas"] = sorted({m.get("op") for _, m in f.msgs})

    # ── C · latencia de una celda trivial por SSE + POST ──
    ms_, out = correr(f, "triv", "x = 1 + 1\nx")
    R["celda_primera_ms"] = round(ms_, 1)
    R["celda_salida_ejemplo"] = out
    xs = [correr(f, "triv", f"x = {i} + 1\nx")[0] for i in range(40)]
    R["celda_trivial_ms"] = pct(xs)

    # cadena reactiva: a → b → c; cambiar a re-ejecuta b y c solas
    correr(f, "ca", "a = 1")
    correr(f, "cb", "b = a * 2")
    correr(f, "cc", "c = b + 1\nc")
    xs = []
    for i in range(20):
        n = len(f.msgs)
        t0 = time.time()
        post("/api/kernel/run", {"cellIds": ["ca"], "codes": [f"a = {i}"]}, "s1")
        t1, _ = f.esperar(idle_de("cc"), n)
        xs.append((t1 - t0) * 1000)
    R["cadena_reactiva_3_celdas_ms"] = pct(xs)

    # el SDK en el kernel, y lo pesado de la imagen
    _, out = correr(f, "sdk", "import ore\nore.__file__")
    R["sdk_en_kernel"] = out
    ms_, out = correr(f, "imp", "import pandas, pyarrow, duckdb, polars")
    R["import_pandas_pyarrow_duckdb_polars_ms"] = round(ms_)
    ms_, out = correr(f, "duck", "duckdb.sql('select sum(range) from range(100000000)').fetchall()")
    R["duckdb_suma_100M_ms"] = round(ms_)
    ms_, out = correr(f, "df", "import numpy as np\ndf = pandas.DataFrame({'k': np.arange(1_000_000) % 100, 'v': np.random.rand(1_000_000)})\ndf")
    R["dataframe_1M_salida_ms"] = round(ms_)
    R["dataframe_1M_salida_mimetype"] = (out or {}).get("mimetype")
    R["dataframe_1M_salida_bytes"] = len(json.dumps(out or {}))
    rss, nproc = rss_arbol(p.pid)
    R["rss_con_datos_mib"] = round(rss, 1)

    # mo.sql sobre DuckDB, en el kernel
    ms_, out = correr(f, "mosql", "import marimo as mo\nr = mo.sql('select k, avg(v) from df group by k')\nr")
    R["mo_sql_group_by_1M_ms"] = round(ms_)
    R["mo_sql_salida_mimetype"] = (out or {}).get("mimetype")
    if (out or {}).get("mimetype", "").endswith("error"):
        R["mo_sql_error"] = str(out.get("data"))[:400]

    # M0·e · guardar es un POST explicito del frontend: ¿que escribe y donde?
    codigos = ["a = 1", "b = a * 2", "c = b + 1\nc"]
    try:
        st, cuerpo = post("/api/kernel/save", {"cellIds": ["ca", "cb", "cc"], "codes": codigos,
                          "names": ["_", "_", "_"], "configs": [{}, {}, {}],
                          "filename": os.path.join(DIR, NB), "persist": True}, "s1")
        R["guardar"] = {"status": st, "respuesta": cuerpo[:200].decode(errors="replace")}
    except Exception as e:  # noqa: BLE001
        R["guardar"] = {"error": str(e), "cuerpo": getattr(e, "read", lambda: b"")()[:300].decode(errors="replace")}
    with open(os.path.join(DIR, NB)) as fh:
        R["fichero_guardado"] = fh.read()[:600]

    # guardar: ¿qué escribe marimo en disco? (el fichero vuelve al árbol por quien sea)
    R["fichero_tras_sesion_bytes"] = os.path.getsize(os.path.join(DIR, NB))
    R["ficheros_en_dir"] = sorted(os.listdir(DIR))
    p.terminate()
    p.wait(10)

    print("M0-RESULTADO " + json.dumps(R, ensure_ascii=False, default=str))


if __name__ == "__main__":
    main()
