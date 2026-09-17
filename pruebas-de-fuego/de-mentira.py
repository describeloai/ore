#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
de-mentira.py — dos servidores de mentira para probar la invocación sin red ni GPU.

  python de-mentira.py s3   PUERTO   un S3 en memoria: PUT (honra `If-None-Match: *`
                                     con 412), GET, HEAD, DELETE y `?list-type=2&prefix=`.
                                     Lo justo para `ore-store-r2`; no comprueba SigV4.
  python de-mentira.py vllm PUERTO   `/v1/chat/completions` como el vLLM de mentira de
                                     E0 (0027): contesta por fila con un objeto JSON
                                     `{"categoriaEs": …}` traducido de una tabla corta; a
                                     una fila cuyo texto lleve `rompe` contesta prosa sin
                                     objeto (la fila que el modelo no contesta bien), y
                                     pide `Authorization` si `EXIGE_TOKEN=1`.

Cada uno imprime `listo` cuando escucha. Son de prueba: no se aceptan fuera de la
máquina (escuchan en 127.0.0.1).
"""
import json
import os
import re
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, unquote, urlparse

MODO = sys.argv[1] if len(sys.argv) > 1 else "s3"
PUERTO = int(sys.argv[2]) if len(sys.argv) > 2 else 0

OBJETOS = {}
CERROJO = threading.Lock()


class S3(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _clave(self):
        u = urlparse(self.path)
        partes = unquote(u.path).lstrip("/").split("/", 1)
        return (partes[1] if len(partes) > 1 else ""), parse_qs(u.query)

    def do_PUT(self):
        clave, _ = self._clave()
        n = int(self.headers.get("content-length", "0"))
        cuerpo = self.rfile.read(n)
        with CERROJO:
            if self.headers.get("If-None-Match") == "*" and clave in OBJETOS:
                self.send_response(412)
                self.end_headers()
                return
            OBJETOS[clave] = cuerpo
        self.send_response(200)
        self.send_header("ETag", '"x"')
        self.end_headers()

    def do_GET(self):
        clave, q = self._clave()
        if "list-type" in q:
            pref = q.get("prefix", [""])[0]
            with CERROJO:
                claves = sorted(k for k in OBJETOS if k.startswith(pref))
            xml = "<ListBucketResult>" + "".join("<Contents><Key>%s</Key></Contents>" % k for k in claves) + "</ListBucketResult>"
            b = xml.encode()
            self.send_response(200)
            self.send_header("content-type", "application/xml")
            self.send_header("content-length", str(len(b)))
            self.end_headers()
            self.wfile.write(b)
            return
        with CERROJO:
            b = OBJETOS.get(clave)
        if b is None:
            self.send_response(404)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_HEAD(self):
        clave, _ = self._clave()
        with CERROJO:
            hay = clave in OBJETOS
        self.send_response(200 if hay else 404)
        self.end_headers()

    def do_DELETE(self):
        clave, _ = self._clave()
        with CERROJO:
            OBJETOS.pop(clave, None)
        self.send_response(204)
        self.end_headers()


TRADUCE = {
    "beleza_saude": "Belleza y salud",
    "informatica_acessorios": "Informática y accesorios",
    "automotivo": "Automoción",
    "cama_mesa_banho": "Cama, mesa y baño",
    "moveis_decoracao": "Muebles y decoración",
    "esporte_lazer": "Deporte y ocio",
    "perfumaria": "Perfumería",
    "utilidades_domesticas": "Menaje",
    "telefonia": "Telefonía",
    "relogios_presentes": "Relojes y regalos",
}
LLAMADAS = []


class VLLM(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _json(self, codigo, obj):
        b = json.dumps(obj).encode("utf-8")
        self.send_response(codigo)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_GET(self):
        if self.path == "/v1/models":
            return self._json(200, {"data": [{"id": "de-mentira/v2-lite"}]})
        if self.path == "/llamadas":
            with CERROJO:
                return self._json(200, {"llamadas": len(LLAMADAS), "ultima": LLAMADAS[-1] if LLAMADAS else None})
        self._json(404, {"error": "no"})

    def do_POST(self):
        if os.environ.get("EXIGE_TOKEN") == "1" and not self.headers.get("authorization", "").startswith("Bearer "):
            return self._json(401, {"error": "cell is not identified"})
        n = int(self.headers.get("content-length", "0"))
        cuerpo = json.loads(self.rfile.read(n).decode("utf-8"))
        if self.path != "/v1/chat/completions":
            return self._json(404, {"error": "no"})
        usuario = next((m["content"] for m in cuerpo.get("messages", []) if m.get("role") == "user"), "")
        sistema = next((m["content"] for m in cuerpo.get("messages", []) if m.get("role") == "system"), "")
        with CERROJO:
            LLAMADAS.append({"model": cuerpo.get("model"), "temperature": cuerpo.get("temperature"), "max_tokens": cuerpo.get("max_tokens"),
                             "system": sistema[:200], "user": usuario[:80]})
        if "rompe" in usuario:
            contenido = "Lo siento, no puedo clasificar esto."
        else:
            m = re.search(r"productCategoryName: *(\S+)", usuario)
            clave = m.group(1) if m else ""
            contenido = json.dumps({"categoriaEs": TRADUCE.get(clave, clave.replace("_", " ").capitalize())}, ensure_ascii=False)
        self._json(200, {
            "id": "de-mentira", "object": "chat.completion", "model": cuerpo.get("model"),
            "choices": [{"index": 0, "message": {"role": "assistant", "content": contenido}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": len((sistema + usuario).split()), "completion_tokens": len(contenido.split()),
                      "total_tokens": len((sistema + usuario).split()) + len(contenido.split())},
        })


if __name__ == "__main__":
    srv = ThreadingHTTPServer(("127.0.0.1", PUERTO), S3 if MODO == "s3" else VLLM)
    print("listo %d" % srv.server_address[1], flush=True)
    srv.serve_forever()
