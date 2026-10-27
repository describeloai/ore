"""ORE 0074 AR2·0 · dos índices PyPI de juguete: «ORE» (:8091) y un «PyPI» falso (:8092).

Nunca el PyPI real. Cada índice sirve PEP 503 desde `<RAIZ>/<indice>/<nombre>/<fichero>`, con
`#sha256=` y `data-requires-python`, e IGNORA `Range` (como `ore-medios` hoy). «ORE» pide Basic
`capa:<CLAVE>` y contesta 401 con `WWW-Authenticate`; su comportamiento se cambia en caliente
escribiendo `<RAIZ>/modo-ore.txt` (`normal`, `401`, `403`, `500`). Apunta cada petición en
`<RAIZ>/registro.jsonl`: índice, método, ruta, estado, si llegó `Authorization` (el usuario, nunca
la contraseña), `Accept`, `Range` y, en un `POST`, el ORDEN de las partes del multipart.
"""
import base64, hashlib, json, os, re, sys, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

RAIZ = sys.argv[1]
CLAVE = os.environ.get("CLAVE", "ore_art_juguete")
REGISTRO = os.path.join(RAIZ, "registro.jsonl")
CERROJO = threading.Lock()


def apunta(**k):
    with CERROJO, open(REGISTRO, "a", encoding="utf-8") as f:
        f.write(json.dumps(k) + "\n")


def normal(n):
    return re.sub(r"[-_.]+", "-", n).lower()


def hecho(indice):
    class Indice(BaseHTTPRequestHandler):
        def modo(self):
            if indice != "ore":
                return "normal"
            try:
                return open(os.path.join(RAIZ, "modo-ore.txt")).read().strip() or "normal"
            except OSError:
                return "normal"

        def quien(self):
            a = self.headers.get("Authorization") or ""
            if not a.lower().startswith("basic "):
                return None, False
            u, _, c = base64.b64decode(a[6:]).decode().partition(":")
            return u, c == CLAVE

        def responde(self, codigo, cuerpo=b"", tipo="text/plain", extra=None):
            self.send_response(codigo)
            self.send_header("Content-Type", tipo)
            if codigo == 401:
                self.send_header("WWW-Authenticate", 'Basic realm="ore"')
            self.send_header("Content-Length", str(len(cuerpo)))
            self.end_headers()
            if cuerpo and self.command != "HEAD":
                self.wfile.write(cuerpo)
            u, _ = self.quien()
            apunta(indice=indice, metodo=self.command, ruta=self.path, estado=codigo, usuario=u,
                   accept=self.headers.get("Accept"), range=self.headers.get("Range"),
                   agente=(self.headers.get("User-Agent") or "")[:40], **(extra or {}))

        def puerta(self):
            """None si pasa; si no, ya contestó."""
            m = self.modo()
            if m in ("403", "500"):
                self.responde(int(m))
                return False
            if indice == "ore":
                u, bien = self.quien()
                if m == "401" or not bien:
                    self.responde(401)
                    return False
            return True

        def do_GET(self):
            if not self.puerta():
                return
            partes = [p for p in self.path.split("?")[0].split("/") if p]
            base = os.path.join(RAIZ, indice)
            if len(partes) == 2 and partes[0] == "simple":
                d = os.path.join(base, normal(partes[1]))
                if not os.path.isdir(d):
                    return self.responde(404)
                enlaces = []
                for f in sorted(os.listdir(d)):
                    if f.endswith(".rp"):
                        continue
                    sha = hashlib.sha256(open(os.path.join(d, f), "rb").read()).hexdigest()
                    rp = os.path.join(d, f + ".rp")
                    attr = ""
                    if os.path.exists(rp):
                        attr = ' data-requires-python="%s"' % open(rp).read().strip().replace(">", "&gt;").replace("<", "&lt;")
                    enlaces.append('<a href="/files/%s/%s#sha256=%s"%s>%s</a><br/>' % (normal(partes[1]), f, sha, attr, f))
                html = "<!DOCTYPE html><html><body>\n" + "\n".join(enlaces) + "\n</body></html>"
                return self.responde(200, html.encode(), "text/html")
            if len(partes) == 3 and partes[0] == "files":
                p = os.path.join(base, partes[1], partes[2])
                if os.path.isfile(p):
                    return self.responde(200, open(p, "rb").read(), "application/octet-stream")
            self.responde(404)

        do_HEAD = do_GET

        def do_POST(self):
            n = int(self.headers.get("Content-Length", 0))
            cuerpo = self.rfile.read(n)
            # El orden de las partes, leído a mano (como lo leería un lector en flujo).
            limite = re.search(r'boundary="?([^";]+)"?', self.headers.get("Content-Type", "")).group(1).encode()
            orden, campos, fichero = [], {}, None
            for parte in cuerpo.split(b"--" + limite)[1:]:
                if parte.startswith(b"--"):
                    break
                cabeza, _, valor = parte.partition(b"\r\n\r\n")
                nombre = re.search(rb'name="([^"]*)"', cabeza).group(1).decode()
                fn = re.search(rb'filename="([^"]*)"', cabeza)
                valor = valor[:-2] if valor.endswith(b"\r\n") else valor
                if fn:
                    orden.append(nombre + "<fichero>")
                    fichero = (fn.group(1).decode(), valor)
                else:
                    orden.append(nombre)
                    campos.setdefault(nombre, valor.decode(errors="replace")[:80])
            extra = {"bytes": n, "orden": orden, "nombre": campos.get("name"),
                     "version": campos.get("version"), "fichero": fichero and fichero[0],
                     "filetype": campos.get("filetype"), "requires_python": campos.get("requires_python"),
                     "chunked": self.headers.get("Transfer-Encoding"), "expect": self.headers.get("Expect")}
            if not self.puerta():
                return
            d = os.path.join(RAIZ, indice, normal(campos.get("name", "x")))
            os.makedirs(d, exist_ok=True)
            p = os.path.join(d, fichero[0])
            if os.path.exists(p):
                return self.responde(409, b"File already exists", extra=extra)
            open(p, "wb").write(fichero[1])
            if campos.get("requires_python"):
                open(p + ".rp", "w").write(campos["requires_python"])
            self.responde(200, extra=extra)

        def log_message(self, *a):
            pass

    return Indice


for indice, puerto in (("ore", 8091), ("pypi", 8092)):
    os.makedirs(os.path.join(RAIZ, indice), exist_ok=True)
    s = ThreadingHTTPServer(("127.0.0.1", puerto), hecho(indice))
    threading.Thread(target=s.serve_forever, daemon=True).start()
print("índices en :8091 (ore) y :8092 (pypi)", flush=True)
threading.Event().wait()
