# -*- coding: utf-8 -*-
"""El plano de control del gateway de modelos, DE BANCO (0027 E1).

Lo que `ore-serve` le pide a Bastion B3 al escribir o retirar un `Model`:

    POST   /admin/tenants/{celda}/models   {"model": id}   → 201
    DELETE /admin/tenants/{celda}/models/{id}             → 204 · 404 si no estaba
    GET    /admin/tenants                                  → los tenants con lo suscrito (la prueba mira ahi)
    GET    /admin/health                                   → 200

Y nada mas: no enruta, no cobra, no comprueba tokens. Es el contrato tal como lo
fija `crates/bastion-gateway` (README, plano de control), para que `los-modelos.sh`
corra sin Bastion. Con `BASTION_GATEWAY=<binario>` la prueba usa el de verdad.

    uso:  python pruebas-de-fuego/gateway-de-banco.py --port 9871
"""
import argparse
import json
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote

TENANTS = {}
LOCK = threading.Lock()


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        sys.stderr.write("banco %s\n" % (fmt % args))

    def _json(self, code, obj=None):
        body = b"" if obj is None else json.dumps(obj).encode()
        self.send_response(code)
        if body:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        if body:
            self.wfile.write(body)

    def _partes(self):
        return [unquote(p) for p in self.path.split("?")[0].split("/") if p]

    def do_GET(self):
        p = self._partes()
        if p == ["admin", "health"]:
            return self._json(200, {"ok": True, "backends": [], "tenants": {t: {} for t in TENANTS}})
        if p == ["admin", "tenants"]:
            with LOCK:
                return self._json(200, list(TENANTS.values()))
        if len(p) == 3 and p[:2] == ["admin", "tenants"]:
            with LOCK:
                t = TENANTS.get(p[2])
            return self._json(200, t) if t else self._json(404, {"error": "no such tenant"})
        self._json(404, {"error": "not here"})

    def do_POST(self):
        p = self._partes()
        n = int(self.headers.get("Content-Length", "0"))
        cuerpo = json.loads(self.rfile.read(n) or b"{}")
        if len(p) == 4 and p[:2] == ["admin", "tenants"] and p[3] == "models":
            with LOCK:
                t = TENANTS.setdefault(p[2], {"id": p[2], "sovereignty": "eu-dc", "allowed_models": []})
                if cuerpo.get("model") and cuerpo["model"] not in t["allowed_models"]:
                    t["allowed_models"].append(cuerpo["model"])
            return self._json(201, t)
        self._json(404, {"error": "not here"})

    def do_DELETE(self):
        p = self._partes()
        if len(p) == 5 and p[:2] == ["admin", "tenants"] and p[3] == "models":
            with LOCK:
                t = TENANTS.get(p[2])
                if not t or p[4] not in t["allowed_models"]:
                    return self._json(404, {"error": "not subscribed"})
                t["allowed_models"].remove(p[4])
            return self._json(204)
        self._json(404, {"error": "not here"})


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=9871)
    a = ap.parse_args()
    print("gateway de banco · :%d" % a.port, flush=True)
    ThreadingHTTPServer(("127.0.0.1", a.port), H).serve_forever()
