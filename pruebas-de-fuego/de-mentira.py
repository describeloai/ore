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
                                     de Iceberg de la medida W3.6c; no comprueba SigV4
                                     en cabeceras, pero SÍ una URL prefirmada (0046
                                     E9·2): su firma con el secreto `S3_SECRETO`
                                     (`mentira`), su caducidad (403 las dos), y
                                     `response-content-type`/`-disposition`.
  python de-mentira.py vllm PUERTO   `/v1/chat/completions` como el vLLM de mentira de
                                     E0 (0027): contesta por fila con un objeto JSON
                                     `{"categoriaEs": …}` traducido de una tabla corta; a
                                     una fila cuyo texto lleve `rompe` contesta prosa sin
                                     objeto (la fila que el modelo no contesta bien), y
                                     pide `Authorization` si `EXIGE_TOKEN=1`.

Cada uno imprime `listo` cuando escucha. Son de prueba: no se aceptan fuera de la
máquina (escuchan en 127.0.0.1).
"""
import base64
import datetime
import hashlib
import hmac
import json
import os
import re
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, quote, unquote, urlparse

MODO = sys.argv[1] if len(sys.argv) > 1 else "s3"
PUERTO = int(sys.argv[2]) if len(sys.argv) > 2 else 0

OBJETOS = {}
TIPOS = {}  # clave → content-type, como lo subió el cliente
FECHAS = {}  # clave → LastModified (ISO), al subirlo o al copiarlo sobre sí mismo


def ahora_iso():
    import datetime
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"
PARTES = {}  # uploadId → {partNumber: bytes}


def crc64nvme(b):
    """La huella que S3 da con `x-amz-checksum-mode: ENABLED` (CRC-64/NVME, en
    base64 de sus 8 bytes): lo que una colección fija de cada ítem (0046 E7)."""
    c = 0xFFFFFFFFFFFFFFFF
    for x in b:
        c ^= x
        for _ in range(8):
            c = (c >> 1) ^ (0x9A6C9329AC4BC9B5 if c & 1 else 0)
    return base64.b64encode((c ^ 0xFFFFFFFFFFFFFFFF).to_bytes(8, "big")).decode()


def cabeceras_de_objeto(h, b):
    """ETag y huella, como S3 en `HEAD` y `GET` (un bucket sin versiones: la
    versión es `null`)."""
    h.send_header("etag", '"%s"' % hashlib.md5(b).hexdigest())
    h.send_header("x-amz-checksum-crc64nvme", crc64nvme(b))
    h.send_header("x-amz-checksum-type", "FULL_OBJECT")
    h.send_header("x-amz-version-id", "null")
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
        # `CopyObject` (la copia sobre sí mismo con `REPLACE` es como se toca un
        # objeto en S3: cambia `LastModified`, no los bytes).
        fuente = self.headers.get("x-amz-copy-source")
        if fuente:
            src = unquote(fuente).lstrip("/").split("/", 1)[1]
            with CERROJO:
                if src not in OBJETOS:
                    self.send_response(404)
                    self.send_header("content-length", "0")
                    self.end_headers()
                    return
                OBJETOS[clave] = OBJETOS[src]
                TIPOS[clave] = TIPOS.get(src, "")
                FECHAS[clave] = ahora_iso()
            b = b"<CopyObjectResult><ETag>\"x\"</ETag></CopyObjectResult>"
            self.send_response(200)
            self.send_header("content-length", str(len(b)))
            self.end_headers()
            self.wfile.write(b)
            return
        if "uploadId" in q:
            with CERROJO:
                PARTES.setdefault(q["uploadId"][0], {})[int(q["partNumber"][0])] = cuerpo
            self.send_response(200)
            self.send_header("ETag", '"p%s"' % q["partNumber"][0])
            self.send_header("content-length", "0")
            self.end_headers()
            return
        # El checksum que el cliente dice, cotejado ANTES de guardar: lo que
        # R2 y S3 hacen con `x-amz-checksum-sha256` (400 `BadDigest`).
        dicho = self.headers.get("x-amz-checksum-sha256")
        if dicho and dicho != base64.b64encode(hashlib.sha256(cuerpo).digest()).decode():
            self.send_response(400)
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
            TIPOS[clave] = self.headers.get("content-type", "")
            FECHAS[clave] = ahora_iso()
        self.send_response(200)
        self.send_header("ETag", '"x"')
        self.send_header("content-length", "0")
        self.end_headers()

    def _prefirma(self):
        """None si la URL prefirmada vale; si no, por qué (SigV4 en la consulta,
        hecho aparte de `ore_s3::firma::prefirmar` para cotejarla de verdad)."""
        u = urlparse(self.path)
        q = parse_qs(u.query, keep_blank_values=True)
        marca = q["X-Amz-Date"][0]
        t = datetime.datetime.strptime(marca, "%Y%m%dT%H%M%SZ").replace(tzinfo=datetime.timezone.utc)
        if datetime.datetime.now(datetime.timezone.utc) > t + datetime.timedelta(seconds=int(q["X-Amz-Expires"][0])):
            return "Request has expired"
        _, fecha, region, servicio, _ = q["X-Amz-Credential"][0].split("/")
        e = lambda s: quote(s, safe="-_.~")
        pares = sorted((e(k), e(v)) for k, vs in q.items() for v in vs if k != "X-Amz-Signature")
        consulta = "&".join("%s=%s" % kv for kv in pares)
        canon = "GET\n%s\n%s\nhost:%s\n\nhost\nUNSIGNED-PAYLOAD" % (
            quote(unquote(u.path), safe="/-_.~"), consulta, self.headers.get("Host", ""))
        sts = "AWS4-HMAC-SHA256\n%s\n%s/%s/%s/aws4_request\n%s" % (
            marca, fecha, region, servicio, hashlib.sha256(canon.encode()).hexdigest())
        h = lambda k, m: hmac.new(k, m.encode(), hashlib.sha256).digest()
        k = h(("AWS4" + os.environ.get("S3_SECRETO", "mentira")).encode(), fecha)
        for parte in (region, servicio, "aws4_request"):
            k = h(k, parte)
        if not hmac.compare_digest(h(k, sts).hex(), q.get("X-Amz-Signature", [""])[0]):
            return "SignatureDoesNotMatch"
        return None

    def do_GET(self):
        clave, q = self._clave()
        if "X-Amz-Signature" in q:
            motivo = self._prefirma()
            if motivo:
                b = ("<Error><Code>AccessDenied</Code><Message>%s</Message></Error>" % motivo).encode()
                self.send_response(403)
                self.send_header("content-length", str(len(b)))
                self.end_headers()
                self.wfile.write(b)
                return
        firmado = {k: q[k][0] for k in ("response-content-type", "response-content-disposition") if k in q}
        # `ListObjectVersions` (0046 E8·1b): sin versiones, una por clave, `null`.
        if "versions" in q:
            pref = q.get("prefix", [""])[0]
            with CERROJO:
                claves = sorted((k, OBJETOS[k]) for k in OBJETOS if k.startswith(pref))
            xml = "<ListVersionsResult><IsTruncated>false</IsTruncated>" + "".join(
                "<Version><Key>%s</Key><VersionId>null</VersionId><IsLatest>true</IsLatest>"
                "<LastModified>%s</LastModified><ETag>&quot;%s&quot;</ETag><Size>%d</Size></Version>"
                % (k, FECHAS.get(k, "2026-01-01T00:00:00.000Z"), hashlib.md5(b).hexdigest(), len(b))
                for k, b in claves) + "</ListVersionsResult>"
            b = xml.encode()
            self.send_response(200)
            self.send_header("content-type", "application/xml")
            self.send_header("content-length", str(len(b)))
            self.end_headers()
            self.wfile.write(b)
            return
        if "list-type" in q:
            pref = q.get("prefix", [""])[0]
            with CERROJO:
                claves = sorted(k for k in OBJETOS if k.startswith(pref))
            xml = "<ListBucketResult>" + "".join(
                "<Contents><Key>%s</Key><Size>%d</Size><LastModified>%s</LastModified></Contents>"
                % (k, len(OBJETOS.get(k, b"")), FECHAS.get(k, "2026-01-01T00:00:00.000Z"))
                for k in claves) + "<IsTruncated>false</IsTruncated></ListBucketResult>"
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
            if "response-content-type" in firmado:
                self.send_header("content-type", firmado["response-content-type"])
            self.send_header("content-length", str(len(trozo)))
            self.end_headers()
            self.wfile.write(trozo)
            return
        self.send_response(200)
        tipo = firmado.get("response-content-type") or TIPOS.get(clave)
        if tipo:
            self.send_header("content-type", tipo)
        if "response-content-disposition" in firmado:
            self.send_header("content-disposition", firmado["response-content-disposition"])
        cabeceras_de_objeto(self, b)
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_HEAD(self):
        clave, _ = self._clave()
        with CERROJO:
            b = OBJETOS.get(clave)
        self.send_response(200 if b is not None else 404)
        if b is not None:
            cabeceras_de_objeto(self, b)
        self.send_header("content-length", str(len(b) if b is not None else 0))
        self.end_headers()

    def do_DELETE(self):
        clave, q = self._clave()
        with CERROJO:
            if "uploadId" in q:
                PARTES.pop(q["uploadId"][0], None)
            else:
                OBJETOS.pop(clave, None)
                FECHAS.pop(clave, None)
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
