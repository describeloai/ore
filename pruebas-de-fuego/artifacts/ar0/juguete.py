"""ORE 0074 AR0 · previos P1, P2, P4: un repositorio de juguete que apunta cada petición.

Guarda lo que recibe por PUT (Maven) y por POST multipart (la API de subida de PyPI), y lo sirve por
GET, para que también haga de repositorio del que resolver. Apunta en registro.jsonl método, ruta,
estado, tamaño y si llegó `Authorization` (el esquema y el usuario, nunca la contraseña).
"""
import base64, email.parser, email.policy, json, os, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

RAIZ = os.environ.get("RAIZ", "/datos")
REGISTRO = os.path.join(RAIZ, "registro.jsonl")
USUARIO, CLAVE = "publicar", os.environ.get("CLAVE", "token-de-juguete")


def apunta(**k):
    with open(REGISTRO, "a") as f:
        f.write(json.dumps(k) + "\n")


class Juguete(BaseHTTPRequestHandler):
    def auth(self):
        a = self.headers.get("Authorization")
        if not a:
            return None, False
        esquema, _, valor = a.partition(" ")
        if esquema.lower() != "basic":
            return esquema, False
        u, _, c = base64.b64decode(valor).decode().partition(":")
        return f"basic:{u}", (u in (USUARIO, "__token__") and c == CLAVE)

    def responde(self, codigo, cuerpo=b"", extra=None, tipo="application/octet-stream"):
        self.send_response(codigo)
        self.send_header("Content-Type", tipo)
        if codigo == 401:
            self.send_header("WWW-Authenticate", 'Basic realm="ore"')
        self.send_header("Content-Length", str(len(cuerpo)))
        self.end_headers()
        if cuerpo and self.command != "HEAD":
            self.wfile.write(cuerpo)
        apunta(metodo=self.command, ruta=self.path, estado=codigo, auth=extra, **(self._k or {}))

    def ruta(self):
        return os.path.join(RAIZ, "repo", self.path.split("?")[0].lstrip("/"))

    def do_GET(self):
        self._k = None
        quien, _ = self.auth()
        p = self.ruta()
        if os.path.isdir(p):
            # El índice simple de PyPI (PEP 503) a mano: un enlace por fichero.
            enlaces = "".join(f'<a href="{n}">{n}</a>\n' for n in sorted(os.listdir(p)))
            return self.responde(200, f"<html><body>\n{enlaces}</body></html>".encode(), quien, "text/html")
        if os.path.isfile(p):
            return self.responde(200, open(p, "rb").read(), quien)
        self.responde(404, b"", quien)

    do_HEAD = do_GET

    def do_PUT(self):
        # AR1·0: lo que el cliente anuncia antes del cuerpo, y cuánto tarda en mandarlo.
        t0 = time.monotonic()
        n = int(self.headers.get("Content-Length", 0))
        cuerpo = self.rfile.read(n)
        self._k = {"bytes": n, "expect": self.headers.get("Expect"),
                   "transfer_encoding": self.headers.get("Transfer-Encoding"),
                   "agente": (self.headers.get("User-Agent") or "")[:60],
                   "ms_cuerpo": round((time.monotonic() - t0) * 1000)}
        quien, bien = self.auth()
        if not bien:
            return self.responde(401, b"", quien)
        p = self.ruta()
        if os.path.exists(p) and not p.endswith(("maven-metadata.xml", ".sha1", ".md5", ".sha256", ".sha512")):
            return self.responde(409, b"ya publicada", quien)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        open(p, "wb").write(cuerpo)
        self.responde(201, b"", quien)

    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0))
        cuerpo = self.rfile.read(n)
        quien, bien = self.auth()
        msg = email.parser.BytesParser(policy=email.policy.HTTP).parsebytes(
            b"Content-Type: " + self.headers.get("Content-Type", "").encode() + b"\r\n\r\n" + cuerpo)
        campos, fichero = {}, None
        for parte in msg.iter_parts():
            nombre = parte.get_param("name", header="content-disposition")
            if parte.get_filename():
                fichero = (parte.get_filename(), parte.get_payload(decode=True))
            else:
                campos[nombre] = parte.get_content().strip()[:80]
        self._k = {"bytes": n, "campos": sorted(campos), "accion": campos.get(":action"),
                   "nombre": campos.get("name"), "version": campos.get("version"),
                   "fichero": fichero and fichero[0], "sha256": campos.get("sha256_digest")}
        if not bien:
            return self.responde(401, b"", quien)
        norm = campos.get("name", "x").lower().replace("_", "-").replace(".", "-")
        d = os.path.join(RAIZ, "repo", "simple", norm)
        os.makedirs(d, exist_ok=True)
        if fichero:
            if os.path.exists(os.path.join(d, fichero[0])):
                return self.responde(409, b"ya publicada", quien)
            open(os.path.join(d, fichero[0]), "wb").write(fichero[1])
        self.responde(200, b"", quien)

    def log_message(self, *a):
        pass


os.makedirs(os.path.join(RAIZ, "repo"), exist_ok=True)
print("juguete en :8080", flush=True)
ThreadingHTTPServer(("0.0.0.0", 8080), Juguete).serve_forever()
