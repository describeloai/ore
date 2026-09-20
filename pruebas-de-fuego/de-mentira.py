#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
de-mentira.py — dos servidores de mentira para probar la invocación sin red ni GPU.

  python de-mentira.py s3   PUERTO   un S3 en memoria: PUT (honra `If-None-Match: *`
                                     con 412), GET (con `Range`), HEAD, DELETE,
                                     `?list-type=2&prefix=` y la subida multiparte
                                     (`?uploads`, `?partNumber&uploadId`, completar y
                                     abortar) que pyarrow y DuckDB usan al escribir.
                                     Lo justo para `ore-store-r2` y para los escritores
                                     de Iceberg de la medida W3.6c; no comprueba SigV4.
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
PARTES = {}  # uploadId → {partNumber: bytes}
CERROJO = threading.Lock()


class S3(BaseHTTPRequestHandler):
    # HTTP/1.1: los SDK de S3 (pyarrow, DuckDB) mandan `Expect: 100-continue`
    # y reutilizan la conexion; todas las respuestas llevan content-length.
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def _cuerpo(self):
        """El cuerpo tal cual, o desenvuelto: los SDK de S3 mandan las partes con
        `Transfer-Encoding: chunked` y dentro `Content-Encoding: aws-chunked`
        (tramas `tamaño-hex[;firma]\r\n datos \r\n` y un tráiler con el checksum)."""
        if self.headers.get("Transfer-Encoding", "").lower() == "chunked":
            trozos = []
            while True:
                linea = self.rfile.readline().strip()
                n = int(linea.split(b";")[0] or b"0", 16)
                if n == 0:
                    while self.rfile.readline().strip():
                        pass
                    break
                trozos.append(self.rfile.read(n))
                self.rfile.readline()
            crudo = b"".join(trozos)
        else:
            crudo = self.rfile.read(int(self.headers.get("content-length", "0")))
        if "aws-chunked" in self.headers.get("Content-Encoding", ""):
            datos, i = b"", 0
            while i < len(crudo):
                fin = crudo.index(b"\r\n", i)
                n = int(crudo[i:fin].split(b";")[0] or b"0", 16)
                if n == 0:
                    break
                datos += crudo[fin + 2:fin + 2 + n]
                i = fin + 2 + n + 2
            return datos
        return crudo

    def _clave(self):
        u = urlparse(self.path)
        partes = unquote(u.path).lstrip("/").split("/", 1)
        return (partes[1] if len(partes) > 1 else ""), parse_qs(u.query, keep_blank_values=True)

    def do_POST(self):
        clave, q = self._clave()
        self._cuerpo()
        if "uploads" in q:
            uid = "u%d" % len(PARTES)
            with CERROJO:
                PARTES[uid] = {}
            b = ("<InitiateMultipartUploadResult><Bucket>b</Bucket><Key>%s</Key><UploadId>%s</UploadId></InitiateMultipartUploadResult>" % (clave, uid)).encode()
        elif "uploadId" in q:
            uid = q["uploadId"][0]
            with CERROJO:
                partes = PARTES.pop(uid, {})
                OBJETOS[clave] = b"".join(partes[k] for k in sorted(partes))
            b = ("<CompleteMultipartUploadResult><Key>%s</Key><ETag>\"x\"</ETag></CompleteMultipartUploadResult>" % clave).encode()
        else:
            self.send_response(400); self.send_header("content-length", "0"); self.end_headers(); return
        self.send_response(200)
        self.send_header("content-type", "application/xml")
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_PUT(self):
        clave, q = self._clave()
        cuerpo = self._cuerpo()
        if "uploadId" in q:
            with CERROJO:
                PARTES.setdefault(q["uploadId"][0], {})[int(q["partNumber"][0])] = cuerpo
            self.send_response(200)
            self.send_header("ETag", '"p%s"' % q["partNumber"][0])
            self.send_header("content-length", "0")
            self.end_headers()
            return
        with CERROJO:
            if self.headers.get("If-None-Match") == "*" and clave in OBJETOS:
                self.send_response(412)
                self.send_header("content-length", "0")
                self.end_headers()
                return
            OBJETOS[clave] = cuerpo
        self.send_response(200)
        self.send_header("ETag", '"x"')
        self.send_header("content-length", "0")
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
            self.send_header("content-length", "0")
            self.end_headers()
            return
        rango = self.headers.get("Range", "")
        if rango.startswith("bytes="):
            a, _, z = rango[6:].partition("-")
            a = int(a or 0); z = min(int(z), len(b) - 1) if z else len(b) - 1
            trozo = b[a:z + 1]
            self.send_response(206)
            self.send_header("content-range", "bytes %d-%d/%d" % (a, z, len(b)))
            self.send_header("content-length", str(len(trozo)))
            self.end_headers()
            self.wfile.write(trozo)
            return
        self.send_response(200)
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_HEAD(self):
        clave, _ = self._clave()
        with CERROJO:
            b = OBJETOS.get(clave)
        self.send_response(200 if b is not None else 404)
        self.send_header("content-length", str(len(b) if b is not None else 0))
        self.end_headers()

    def do_DELETE(self):
        clave, q = self._clave()
        with CERROJO:
            if "uploadId" in q:
                PARTES.pop(q["uploadId"][0], None)
            else:
                OBJETOS.pop(clave, None)
        self.send_response(204)
        self.send_header("content-length", "0")
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
