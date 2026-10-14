"""M0·d · el SDK de ORE dentro de un kernel de marimo, en un puesto REAL.

Corre en el pod de un puesto (no en una sonda): el servidor de marimo es un
proceso aparte del agente y su kernel, un subproceso de ése. Mide:
  1. si el kernel hereda la identidad del puesto (no: el agente la pone en su
     propio proceso) y si basta con el token del pod (`x-ore-pod`);
  2. `over()` y `sql()` sobre un dataset del lago, y su salida en el editor;
  3. ore-serve como catálogo PyIceberg (`RestCatalog`) desde el kernel, con
     credenciales prestadas, y si marimo lo descubre como fuente de datos;
  4. `write()` bajo re-ejecución reactiva: misma entrada → ¿snapshot nuevo?
"""
import http.client
import json
import os
import re
import statistics
import subprocess
import sys
import threading
import time
import urllib.request

PORT = 2719
DIR = "/tmp/m0d"
NB = "m0d.py"
LEER = os.environ.get("M0D_LEER", "bq.ventas.ore_e2e_sintetica")
ESCRIBIR = os.environ.get("M0D_ESCRIBIR", "sandbox.m0d_marimo")
R = {}


def post(path, body, sid):
    req = urllib.request.Request(
        f"http://127.0.0.1:{PORT}{path}", data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Marimo-Session-Id": sid}, method="POST")
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.status, r.read()


def rss_arbol(pid):
    import psutil
    p = psutil.Process(pid)
    return round(sum(x.memory_info().rss for x in [p] + p.children(recursive=True)) / 2**20, 1)


class Flujo:
    def __init__(self, sid):
        self.sid, self.msgs, self.cond = sid, [], threading.Condition()
        c = http.client.HTTPConnection("127.0.0.1", PORT, timeout=900)
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
                    m = {"op": "?"}
                buf = ""
                with self.cond:
                    self.msgs.append((time.time(), m))
                    self.cond.notify_all()

    def esperar(self, pred, desde=0, timeout=600):
        fin = time.time() + timeout
        with self.cond:
            while True:
                for i in range(desde, len(self.msgs)):
                    if pred(self.msgs[i][1]):
                        return self.msgs[i][0], i
                if fin - time.time() <= 0:
                    raise TimeoutError([m.get("op") for _, m in self.msgs[-10:]])
                self.cond.wait(fin - time.time())


def idle(cid):
    return lambda m: m.get("op") == "cell-op" and (m.get("data") or {}).get("cell_id") == cid \
        and (m.get("data") or {}).get("status") == "idle"


def correr(f, cid, codigo, esperar=None):
    """Corre una celda; espera a que quede ociosa `esperar` (ella misma por
    defecto: con una dependiente, la última de la cadena). Devuelve ms, la
    salida y lo que la celda escribió por consola."""
    n = len(f.msgs)
    t0 = time.time()
    post("/api/kernel/run", {"cellIds": [cid], "codes": [codigo]}, f.sid)
    fin = esperar or cid
    t1, i = f.esperar(idle(fin), n)
    salidas, consola = {}, []
    for _, m in f.msgs[n:i + 1]:
        d = m.get("data") or {}
        if m.get("op") == "cell-op":
            if d.get("output"):
                salidas[d["cell_id"]] = d["output"]
            for c in (d.get("console") or []) if isinstance(d.get("console"), list) else [d.get("console")]:
                if c and c.get("data"):
                    consola.append(str(c["data"])[:300])
    return round((t1 - t0) * 1000), salidas, consola


def texto(salida):
    if not salida:
        return None
    d = salida.get("data")
    d = d if isinstance(d, str) else json.dumps(d)
    return re.sub(r"<[^>]+>", "", d).replace("&#x27;", "'").replace("&quot;", '"')[:500]


def main():
    R["puesto"] = os.environ.get("PUESTO")
    os.makedirs(DIR, exist_ok=True)
    with open(os.path.join(DIR, NB), "w") as fh:
        fh.write("import marimo\napp = marimo.App()\n\nif __name__ == '__main__':\n    app.run()\n")
    cfg = "/tmp/.config/marimo"
    os.makedirs(cfg, exist_ok=True)
    with open(os.path.join(cfg, "marimo.toml"), "w") as fh:
        fh.write('[server]\ntransport = "sse"\n\n[runtime]\nauto_instantiate = false\n\n'
                 '[completion]\ncopilot = false\n\n[ai]\nenabled = false\n')
    env = dict(os.environ, HOME="/tmp", PYTHONPATH="/opt/ore:/capa")
    t0 = time.time()
    srv = subprocess.Popen([sys.executable, "-m", "marimo", "edit", "--headless", "--host", "127.0.0.1",
                            "--port", str(PORT), "--no-token", "--no-sandbox", "--no-skew-protection",
                            "--skip-update-check", DIR], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    while True:
        try:
            urllib.request.urlopen(f"http://127.0.0.1:{PORT}/api/status", timeout=2)
            break
        except Exception:
            if time.time() - t0 > 120:
                raise
            time.sleep(0.05)
    R["arranque_s"] = round(time.time() - t0, 2)
    try:
        f = Flujo("d1")
        f.esperar(lambda m: m.get("op") == "kernel-ready")
        post("/api/kernel/instantiate", {"objectIds": [], "values": [], "autoRun": True}, "d1")

        # ── 1 · la identidad ──
        ms, s, _ = correr(f, "id0", "import ore\nore.session.pedir('GET', '/v1/config')[0]")
        R["1_sin_arranque_status"] = texto(s.get("id0"))
        ms, s, _ = correr(f, "boot", (
            "def cabeceras_del_pod():\n"
            "    with open('/var/run/ore/pod/token') as t:\n"
            "        return {'x-ore-pod': t.read().strip()}\n"
            "ore.session._proveedor = cabeceras_del_pod\n"
            "ore.session.pedir('GET', '/v1/config')[0]"))
        R["1_con_token_del_pod_status"] = texto(s.get("boot"))
        R["1_con_token_del_pod_ms"] = ms

        # ── 2 · leer ──
        ms, s, _ = correr(f, "over", f"df = ore.over('{LEER}')\ndf")
        R["2_over_ms"] = ms
        R["2_over_salida_bytes"] = len(json.dumps(s.get("over") or {}))
        R["2_over_salida_mimetype"] = (s.get("over") or {}).get("mimetype")
        ms, s, _ = correr(f, "filas", "(len(df), list(df.columns)[:8])")
        R["2_filas_columnas"] = texto(s.get("filas"))
        ms, s, _ = correr(f, "sql", f"r = ore.sql('select count(*) as n from {LEER}')\nr")
        R["2_sql_ms"] = ms
        R["2_sql_salida"] = texto(s.get("sql"))[:200] if s.get("sql") else None
        R["2_rss_mib"] = rss_arbol(srv.pid)

        # ── 3 · ore-serve como catálogo PyIceberg ──
        base, ns, tabla = LEER.split(".")
        ms, s, cons = correr(f, "cat", (
            "import os, time\n"
            "from pyiceberg.catalog.rest import RestCatalog\n"
            "_t = time.time()\n"
            f"cat = RestCatalog('ore', uri=os.environ['ORE_SERVE'], prefix='{base}', **{{\n"
            "    'header.x-ore-pod': cabeceras_del_pod()['x-ore-pod'],\n"
            "    'header.x-ore-puesto': os.environ['PUESTO'],\n"
            "    'header.X-Iceberg-Access-Delegation': 'vended-credentials'})\n"
            "_ns = cat.list_namespaces()\n"
            f"_tb = cat.load_table(('{ns}', '{tabla}'))\n"
            "_arrow = _tb.scan(limit=1000).to_arrow()\n"
            "(len(_ns), _arrow.num_rows, round(time.time() - _t, 2))"))
        R["3_pyiceberg_ms"] = ms
        R["3_pyiceberg_salida"] = texto(s.get("cat"))
        R["3_pyiceberg_consola"] = cons[-3:]
        fuentes = [m for _, m in f.msgs if m.get("op") in ("data-source-connections", "datasets")]
        R["3_marimo_ve_fuentes"] = [json.dumps(m)[:400] for m in fuentes[-2:]]

        # ── 4 · write() y la reactividad ──
        correr(f, "n", "n = 1000")
        cod_w = ("import pandas as pd\n"
                 f"res = ore.write('{ESCRIBIR}', pd.DataFrame({{'i': range(n), 'doble': [2 * x for x in range(n)]}}))\n"
                 "{k: res.get(k) for k in ('rows', 'snapshot', 'repeated', 'operation')}")
        ms, s, cons = correr(f, "w", cod_w)
        R["4a_primera_escritura_ms"] = ms
        R["4a_salida"] = texto(s.get("w")) or cons[-2:]
        # la MISMA entrada: cambiar `n` al mismo valor re-ejecuta `w` sola
        ms, s, cons = correr(f, "n", "n = 1000", esperar="w")
        R["4b_reactiva_misma_entrada_ms"] = ms
        R["4b_salida"] = texto(s.get("w")) or cons[-2:]
        # entrada distinta → snapshot nuevo
        ms, s, cons = correr(f, "n", "n = 2000", esperar="w")
        R["4c_reactiva_otra_entrada_ms"] = ms
        R["4c_salida"] = texto(s.get("w")) or cons[-2:]
        R["rss_final_mib"] = rss_arbol(srv.pid)
    finally:
        srv.terminate()
        srv.wait(15)
    print("M0D-RESULTADO " + json.dumps(R, ensure_ascii=False, default=str))


if __name__ == "__main__":
    main()
