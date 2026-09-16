# -*- coding: utf-8 -*-
"""El plano de control del gateway de modelos, DE BANCO (0027 E1).

Lo que `ore-serve` le pide a Bastion B3 al escribir o retirar un `Model`:

    POST   /admin/tenants/{celda}/models   {"model": id}   → 201
    DELETE /admin/tenants/{celda}/models/{id}             → 204 · 404 si no estaba
    GET    /admin/tenants                                  → los tenants con lo suscrito (la prueba mira ahi)
    GET    /admin/health                                   → {ok, backends[{id, model, sovereignty, inflight, up}], tenants}
    POST   /admin/backends  {id, url, model, sovereignty}  → 201 (aqui nace `up`: el banco no sondea)
    DELETE /admin/backends/{id}                            → 204
    GET    /admin/usage?tenant=&from=&to=                  → [{day, tenant, model, requests, prompt_tokens, completion_tokens, usd}]

Y lo que `GET /modelos` le pregunta desde E3 (⑥): que backends estan arriba y
que gasto la celda. Nada mas: no enruta, no cobra, no comprueba tokens. Es el
contrato tal como lo fija `crates/bastion-gateway` (README, plano de control),
para que `los-modelos.sh` corra sin Bastion. Con `BASTION_GATEWAY=<binario>` la
prueba usa el de verdad.

Dos cosas que el de verdad no tiene, solo para el banco:

    POST   /banco/uso  {day, tenant, model, requests, prompt_tokens, completion_tokens, usd}
                        → 201: una fila de uso, como si alguien hubiera llamado
    --como-backend      contesta 200 a `/v1/models`: lo que el gateway de verdad
                        sondea para dar un backend por `up`

    uso:  python pruebas-de-fuego/gateway-de-banco.py --port 9871
          python pruebas-de-fuego/gateway-de-banco.py --port 8871 --como-backend
"""
import argparse
import json
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote

TENANTS = {}
BACKENDS = {}
USO = []
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
            with LOCK:
                bs = [dict(b, inflight=0, up=True) for b in BACKENDS.values()]
            return self._json(200, {"ok": True, "backends": bs, "tenants": {t: {} for t in TENANTS}})
        if p == ["admin", "backends"]:
            with LOCK:
                return self._json(200, list(BACKENDS.values()))
        if p == ["admin", "usage"]:
            q = dict(kv.split("=", 1) for kv in self.path.split("?", 1)[1].split("&") if "=" in kv) if "?" in self.path else {}
            de, hasta, tenant = q.get("from", "1970-01-01"), q.get("to", "9999-12-31"), q.get("tenant")
            with LOCK:
                filas = [u for u in USO if de <= u["day"] < hasta and (not tenant or u["tenant"] == tenant)]
            return self._json(200, filas)
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
        if p == ["admin", "backends"]:
            with LOCK:
                BACKENDS[cuerpo["id"]] = {k: cuerpo.get(k) for k in ("id", "url", "model", "sovereignty")}
            return self._json(201, cuerpo)
        if p == ["banco", "uso"]:
            with LOCK:
                USO.append({k: cuerpo.get(k, 0) for k in ("day", "tenant", "model", "requests", "prompt_tokens", "completion_tokens", "usd")})
            return self._json(201, cuerpo)
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
        if len(p) == 3 and p[:2] == ["admin", "backends"]:
            with LOCK:
                habia = BACKENDS.pop(p[2], None)
            return self._json(204) if habia else self._json(404, {"error": "no such backend"})
        self._json(404, {"error": "not here"})


class Backend(BaseHTTPRequestHandler):
    """Lo minimo que el gateway de verdad sondea: `/v1/models` → 200."""
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        pass

    def do_GET(self):
        body = json.dumps({"object": "list", "data": []}).encode()
        self.send_response(200 if self.path.startswith("/v1/models") else 404)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=9871)
    ap.add_argument("--como-backend", action="store_true", help="un vLLM de mentira: solo /v1/models")
    a = ap.parse_args()
    print("%s de banco · :%d" % ("backend" if a.como_backend else "gateway", a.port), flush=True)
    ThreadingHTTPServer(("127.0.0.1", a.port), Backend if a.como_backend else H).serve_forever()
