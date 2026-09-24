"""PROTOTIPO DESECHABLE · P6: el servidor de SQL por el canal de verdad.

Un ore-serve de verdad (el banco de `medida-el-catalogo-como-resolutor.py`) con
un puesto abierto. Entre medias, lo que cambiaria:

  el agente     escucha `GET /puestos/{id}/lsp/agente` y REPARTE: lo de SQL
                (id `sql:n`, una uri `.sql`, o `initialize` con
                `initializationOptions.lenguaje = sql`) al servidor de SQL
                (`servidor.py`, un hijo por stdio); lo demas a un "pyright" de
                mentira. Lo que contestan sale por `POST /lsp/salida` en lotes
                de 20 ms (como el agente de verdad).
  el editor     DOS clientes sobre el mismo flujo `GET /lsp/consola`, como
                quedaria la consola con un cliente por lenguaje: el de Python
                (ids numericos) y el de SQL (ids `sql:n`); lotes de 25 ms; el
                de SQL manda el texto ANTES de pedir completion (el arreglo).

Mide: la ida y vuelta de una completion y de un diagnostico de punta a punta,
y que nada se cruce (lo de SQL no llega al pyright, cada cliente solo recibe lo
suyo).

    python canal.py --indice assets.json
"""
import importlib.util
import json
import os
import statistics
import subprocess
import sys
import threading
import time
import urllib.request

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__))
PF = os.path.dirname(AQUI)
INDICE = sys.argv[sys.argv.index("--indice") + 1]
sys.argv[:] = [sys.argv[0], "--filas", "100"]
_sp = importlib.util.spec_from_file_location("resolutor", PF + "/medida-el-catalogo-como-resolutor.py")
R = importlib.util.module_from_spec(_sp)
_sp.loader.exec_module(R)


def fila(k, v="", nota=""):
    print("     %-46s %-20s %s" % (k, v, nota))


def med(xs):
    return statistics.median(xs) if xs else 0


def p95(xs):
    xs = sorted(xs)
    return xs[max(0, int(len(xs) * 0.95) - 1)] if xs else 0


def flujo(url, cab, cada, listo):
    req = urllib.request.Request(url, headers=dict(cab, accept="text/event-stream"))
    with urllib.request.urlopen(req, timeout=600) as r:
        listo.set()
        ev = None
        for l in r:
            l = l.decode("utf-8", "replace").rstrip("\r\n")
            if l.startswith("event: "):
                ev = l[7:]
            elif l.startswith("data: ") and ev == "lsp":
                cada(l[6:])


class Lotes:
    """Junta mensajes y los manda cada `cada_ms` (el editor, 25; el agente, 20)."""

    def __init__(self, url, cab, cada_ms):
        self.url, self.cab, self.cada = url, cab, cada_ms / 1000
        self.cola, self.cv = [], threading.Lock()
        threading.Thread(target=self._bucle, daemon=True).start()

    def poner(self, m):
        with self.cv:
            self.cola.append(json.dumps(m) if not isinstance(m, str) else m)

    def _bucle(self):
        while True:
            time.sleep(self.cada)
            with self.cv:
                lote, self.cola = self.cola, []
            if lote:
                R.http("POST", self.url, {"mensajes": lote}, self.cab)


def es_sql(m):
    if isinstance(m.get("id"), str) and m["id"].startswith("sql:"):
        return True
    uri = ((m.get("params") or {}).get("textDocument") or {}).get("uri", "")
    if uri.endswith(".sql"):
        return True
    return m.get("method") == "initialize" and ((m.get("params") or {}).get("initializationOptions") or {}).get("lenguaje") == "sql"


class Agente:
    """El agente con el reparto: SQL a `servidor.py`, lo demas a un pyright de mentira."""

    def __init__(self, base, puesto):
        self.salida = Lotes(base + "/puestos/%s/lsp/salida" % puesto, R.AGENTE, 20)
        self.sql = subprocess.Popen([sys.executable, os.path.join(AQUI, "servidor.py"), "--indice", INDICE],
                                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.al_pyright = []
        threading.Thread(target=self._leer_sql, daemon=True).start()
        listo = threading.Event()
        threading.Thread(target=flujo, args=(base + "/puestos/%s/lsp/agente" % puesto, R.AGENTE, self.recibir, listo), daemon=True).start()
        listo.wait(10)

    def recibir(self, dato):
        m = json.loads(dato)
        if es_sql(m):
            b = dato.encode("utf-8")
            self.sql.stdin.write(b"Content-Length: %d\r\n\r\n" % len(b) + b)
            self.sql.stdin.flush()
        else:
            self.al_pyright.append(m)
            if "id" in m and "method" in m:  # el pyright de mentira contesta a todo
                self.salida.poner({"jsonrpc": "2.0", "id": m["id"], "result": {"items": [{"label": "desde_pyright"}]}})

    def _leer_sql(self):
        f = self.sql.stdout
        while True:
            largo = None
            while True:
                l = f.readline()
                if not l:
                    return
                l = l.strip()
                if not l:
                    break
                if l.lower().startswith(b"content-length:"):
                    largo = int(l.split(b":")[1])
            self.salida.poner(f.read(largo).decode("utf-8"))


class Editor:
    """Un cliente de la consola: su lenguaje, sus ids, filtra lo que no es suyo."""

    def __init__(self, base, puesto, lenguaje):
        self.lenguaje = lenguaje
        self.entrada = Lotes(base + "/puestos/%s/lsp" % puesto, R.ANA, 25)
        self.sig, self.resp, self.diags, self.ajenos = 1, {}, [], 0
        self.cv = threading.Condition()

    def id(self):
        i = self.sig
        self.sig += 1
        return ("sql:%d" % i) if self.lenguaje == "sql" else i

    def llega(self, m):
        mio_id = isinstance(m.get("id"), str) == (self.lenguaje == "sql")
        with self.cv:
            if "id" in m and "method" not in m:
                if mio_id:
                    self.resp[m["id"]] = (time.perf_counter(), m)
                elif m["id"] in self.resp or (isinstance(m["id"], str) and self.lenguaje != "sql" and m["id"].startswith("sql:")):
                    pass
            elif m.get("method") == "textDocument/publishDiagnostics":
                if m["params"]["uri"].endswith(".sql") == (self.lenguaje == "sql"):
                    self.diags.append((time.perf_counter(), m["params"]))
            self.cv.notify_all()

    def pedir(self, metodo, params, plazo=10):
        i = self.id()
        t0 = time.perf_counter()
        self.entrada.poner({"jsonrpc": "2.0", "id": i, "method": metodo, "params": params})
        with self.cv:
            self.cv.wait_for(lambda: i in self.resp, timeout=plazo)
            t, m = self.resp.get(i, (None, None))
        return (m or {}).get("result"), ((t - t0) * 1000 if t else None)

    def notificar(self, metodo, params):
        self.entrada.poner({"jsonrpc": "2.0", "method": metodo, "params": params})


def main():
    print("P6 · EL CANAL DE VERDAD  (ore-serve + el reparto en el agente + dos clientes en el editor)")
    b = R.Banco()
    try:
        b.levantar()
        base, P = b.directo, R.PUESTO
        ag = Agente(base, P)
        py, sq = Editor(base, P, "python"), Editor(base, P, "sql")
        listo = threading.Event()

        def a_los_dos(dato):
            m = json.loads(dato)
            py.llega(m)
            sq.llega(m)
            # lo que un cliente recibe y no es suyo, se cuenta en el otro
        threading.Thread(target=flujo, args=(base + "/puestos/%s/lsp/consola" % P, R.ANA, a_los_dos, listo), daemon=True).start()
        listo.wait(10)
        time.sleep(0.5)
        r, t = sq.pedir("initialize", {"processId": None, "rootUri": "file:///trabajo", "capabilities": {},
                                       "initializationOptions": {"lenguaje": "sql"}})
        fila("initialize de SQL", "%.0f ms" % (t or -1), "serverInfo: %s" % ((r or {}).get("serverInfo") or {}).get("name"))
        r, t = py.pedir("initialize", {"processId": None, "rootUri": "file:///trabajo", "capabilities": {}})
        fila("initialize de Python (al pyright de mentira)", "%.0f ms" % (t or -1), "")
        py.notificar("textDocument/didOpen", {"textDocument": {"uri": "file:///trabajo/a.py", "languageId": "python", "version": 1, "text": "x = 1"}})
        uri = "file:///trabajo/consulta.sql"
        sq.notificar("textDocument/didOpen", {"textDocument": {"uri": uri, "languageId": "sql", "version": 1, "text": ""}})
        # teclear con el arreglo: el texto sale antes de pedir; completion en cada tecla
        objetivo = "select p.category, count(*) from standard_test.products p where p.max_prce > 1 group by all"
        comp, v = [], 1
        for k in range(1, len(objetivo) + 1):
            v += 1
            texto = objetivo[:k]
            sq.notificar("textDocument/didChange", {"textDocument": {"uri": uri, "version": v}, "contentChanges": [{"text": texto}]})
            if objetivo[k - 1].isalnum() or objetivo[k - 1] in "._":
                r, t = sq.pedir("textDocument/completion", {"textDocument": {"uri": uri}, "position": {"line": 0, "character": k}})
                if t:
                    comp.append(t)
            time.sleep(0.08)
        t_fin = time.perf_counter()
        with sq.cv:
            sq.cv.wait_for(lambda: any(d.get("version") == v for _, d in sq.diags), timeout=10)
        final = [d for t, d in sq.diags if d.get("version") == v]
        t_diag = [(t - t_fin) * 1000 for t, d in sq.diags if d.get("version") == v]
        fila("completion de punta a punta", "%.0f ms p50" % med(comp), "%.0f ms p95 · %d peticiones" % (p95(comp), len(comp)))
        fila("el diagnostico tras la ultima tecla", "%.0f ms" % (t_diag[0] if t_diag else -1),
             "(300 ms de demora del servidor incluidos) · %s" % (final[0]["diagnostics"][0]["message"][:60] if final and final[0]["diagnostics"] else "sin error"))
        r, t = py.pedir("textDocument/completion", {"textDocument": {"uri": "file:///trabajo/a.py"}, "position": {"line": 0, "character": 1}})
        fila("una completion de Python, a la vez", "%.0f ms" % (t or -1), "items: %s" % [x["label"] for x in (r or {}).get("items", [])])
        sql_en_pyright = [m for m in ag.al_pyright if es_sql(m)]
        fila("mensajes de SQL que llegaron al pyright", "%d" % len(sql_en_pyright), "de %d que recibio" % len(ag.al_pyright))
        fila("diagnosticos de un .sql en el cliente de Python", "%d" % len(py.diags), "")
        cruzadas = sum(1 for i in py.resp if isinstance(i, str)) + sum(1 for i in sq.resp if not isinstance(i, str))
        fila("respuestas que acabaron en el cliente ajeno", "%d" % cruzadas, "")
        ag.sql.kill()
    finally:
        b.cerrar()


if __name__ == "__main__":
    main()
