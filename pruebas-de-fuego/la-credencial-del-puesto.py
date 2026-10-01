"""LA CREDENCIAL DEL PUESTO (0049 B2·3), sin clúster.

Una celda corre en el proceso del agente y puede durar más que su token (300 s).
Se comprueba, contra un `ore-serve` de mentira que anota cabeceras y rutas:

  1  el SDK pide la cabecera al proveedor EN CADA petición: si el token se
     renueva a mitad de una celda, la siguiente petición lleva el nuevo;
  2  el latido late mientras dura la celda y para cuando acaba;
  3  un latido que falla (el servidor contesta 500) no tumba la celda.

    PYTHONUTF8=1 python pruebas-de-fuego/la-credencial-del-puesto.py
"""
import http.server
import os
import sys
import threading
import time

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(RAIZ, "puesto", "python"))

PEDIDAS = []
FALLAR = {"latido": False}


class H(http.server.BaseHTTPRequestHandler):
    def _anotar(self):
        PEDIDAS.append((self.command, self.path, self.headers.get("authorization")))

    def do_GET(self):
        self._anotar()
        self._ok(200, b'{"ok": true}')

    def do_POST(self):
        self._anotar()
        n = int(self.headers.get("content-length", 0) or 0)
        self.rfile.read(n)
        if self.path.endswith("/latido") and FALLAR["latido"]:
            return self._ok(500, b'{"error": "a proposito"}')
        self._ok(204 if self.path.endswith("/latido") else 200, b"" if self.path.endswith("/latido") else b"{}")

    def _ok(self, codigo, cuerpo):
        self.send_response(codigo)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(cuerpo)))
        self.end_headers()
        self.wfile.write(cuerpo)

    def log_message(self, *a):
        pass


srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
threading.Thread(target=srv.serve_forever, daemon=True).start()
os.environ["ORE_SERVE"] = "http://127.0.0.1:%d" % srv.server_port
os.environ["PUESTO"] = "p1"

import ore  # noqa: E402
import agente  # noqa: E402

fallos = 0


def mal(m):
    global fallos
    fallos += 1
    print("  ✗", m)


p = ore.Puesto()
emitidos = iter(range(1, 1000))
p._proveedor = lambda: {"authorization": "Bearer t%d" % next(emitidos)}

# 1 · cada petición, la cabecera vigente
p.pedir("GET", "/assets")
p.pedir("GET", "/assets")
tokens = [a for (_, r, a) in PEDIDAS if r == "/assets"]
if tokens == ["Bearer t1", "Bearer t2"]:
    print("  ✓ 1 · cada petición pide la cabecera al proveedor:", tokens)
else:
    mal("1 · las cabeceras fueron %s" % tokens)

# 2 · el latido, mientras dura la celda
agente.Latido.CADA = 0.2
PEDIDAS.clear()
with agente.Latido(p):
    time.sleep(0.75)
latidos = [r for (m, r, _) in PEDIDAS if m == "POST" and r == "/puestos/p1/latido"]
time.sleep(0.5)
despues = [r for (m, r, _) in PEDIDAS if m == "POST" and r == "/puestos/p1/latido"]
if len(latidos) >= 2 and len(despues) == len(latidos):
    print("  ✓ 2 · %d latidos durante la celda, ninguno después" % len(latidos))
else:
    mal("2 · latidos durante %d, después %d" % (len(latidos), len(despues)))

# 3 · un latido que falla no tumba la celda
FALLAR["latido"] = True
try:
    with agente.Latido(p):
        time.sleep(0.5)
        resultado = "la celda terminó"
    print("  ✓ 3 · con el latido en 500:", resultado)
except Exception as e:  # noqa: BLE001
    mal("3 · el latido tumbó la celda: %s" % e)

srv.shutdown()
print("todo bien" if fallos == 0 else "%d fallos" % fallos)
sys.exit(1 if fallos else 0)
