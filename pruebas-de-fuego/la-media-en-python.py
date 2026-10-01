"""LA MEDIA EN PYTHON (0049 B3·5), sin clúster.

Un `ore-serve` de mentira (`items`, `item`, y el `307` de `content`) y un servidor
de bytes de mentira (lo que sería `ore-medios` o el lago: rangos, permisos que
caducan, 412, flujos cortados). Se comprueba el SDK:

   1  items(): el listado por cursor, perezoso
   2  stat(): el ítem fresco y si es el actual
   3  open(): leer, seek desde el final con un Range, tell
   4  una lectura entera se verifica y da el sha256 visto
   5  el token de ORE va a la celda y NUNCA a la URL de los bytes
   6  un permiso caducado (401) se renueva con la misma versión y se sigue
   7  la versión ya no está (412) → MediaCambiado
   8  el digest no casa, o el flujo se corta → MediaCorrupto
   9  read_bytes() de uno grande, por rangos en paralelo
  10  leer_varios(): muchos a la vez, el error de uno es un valor
  11  cerrar a medias no baja el resto

    PYTHONUTF8=1 python pruebas-de-fuego/la-media-en-python.py
"""
import hashlib
import http.server
import json
import os
import random
import sys
import threading
import urllib.parse

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(RAIZ, "puesto", "python"))

random.seed(49)
A = b"%PDF-" + random.randbytes(3_000_000)
OBJETOS = {"a.pdf": A, "b.pdf": b"%PDF-otro", "cambia.pdf": b"%PDF-viejo",
           "corrupto.pdf": b"%PDF-bytes", "cortado.pdf": b"%PDF-" + b"x" * 5000}
SHA = {k: hashlib.sha256(v).hexdigest() for k, v in OBJETOS.items()}


def ref(path, digest=None):
    return {"uri": "ore://legal.archivo.contratos/%s?v=v1" % path, "collection": "legal.archivo.contratos",
            "path": path, "version": "v1", "digest": digest, "size": len(OBJETOS[path]),
            "content_type": "application/pdf", "checksum": "etag:x", "state": "actual", "extra": "se ignora"}


SERVE, BYTES = [], []          # (metodo, ruta, cabeceras) que llegaron a cada uno
PERMISOS = {}                  # permiso → (path, usos que le quedan)
ESTADO = {"n": 0}
CONTADOS = {"enviados": 0}
RAMA = {"r": None}             # la rama que la ficha del puesto dice (c13)


class Celda(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        SERVE.append(("GET", self.path, dict(self.headers)))
        u = urllib.parse.urlparse(self.path)
        q = dict(urllib.parse.parse_qsl(u.query))
        if u.path == "/puestos/p1":
            return self._json(200, {"rama": RAMA["r"]} if RAMA["r"] else {})
        if u.path.endswith("/items"):
            todos = [ref("a.pdf"), ref("b.pdf", "sha256:" + SHA["b.pdf"]), ref("cambia.pdf")]
            if q.get("cursor") == "c2":
                return self._json(200, {"as_of": "7", "items": todos[2:], "cursor": None})
            return self._json(200, {"as_of": "7", "items": todos[:2], "cursor": "c2"})
        if u.path.endswith("/item"):
            return self._json(200, dict(ref(q["path"]), current=True))
        if u.path.endswith("/content"):
            path = q["path"]
            ESTADO["n"] += 1
            p = "p%d" % ESTADO["n"]
            # El primer permiso de a.pdf vale para UNA petición: obliga a renovar.
            PERMISOS[p] = (path, 1 if (path == "a.pdf" and ESTADO["n"] == 1) else 10_000)
            digest = {"corrupto.pdf": "sha256:" + "0" * 64}.get(path)
            cuerpo = {"url": "http://127.0.0.1:%d/contenido?permiso=%s" % (BYTES_PUERTO, p),
                      "desde": "medios", "version": q.get("version") or "v1", "ttl_s": 300,
                      "item": ref(path, digest)}
            return self._json(307, cuerpo, {"location": cuerpo["url"]})
        self._json(404, {"type": "media/no-existe", "detail": self.path})

    def _json(self, codigo, cuerpo, extra=None):
        b = json.dumps(cuerpo).encode()
        self.send_response(codigo)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b)))
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(b)

    def log_message(self, *a):
        pass


class Bytes(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        BYTES.append(("GET", self.path, dict(self.headers)))
        q = dict(urllib.parse.parse_qsl(urllib.parse.urlparse(self.path).query))
        p = q.get("permiso")
        if p not in PERMISOS or PERMISOS[p][1] <= 0:
            return self._error(401, "media/permiso")
        path, usos = PERMISOS[p]
        PERMISOS[p] = (path, usos - 1)
        if path == "cambia.pdf":
            return self._error(412, "media/cambiado")
        datos = OBJETOS[path]
        rango = self.headers.get("Range")
        codigo, a, z = 200, 0, len(datos) - 1
        if rango:
            x, y = rango.split("=", 1)[1].split("-")
            a, z, codigo = int(x), (int(y) if y else len(datos) - 1), 206
        trozo = datos[a:z + 1]
        self.send_response(codigo)
        self.send_header("content-length", str(len(trozo)))
        if path == "cortado.pdf":
            trozo = trozo[: len(trozo) // 2]   # promete el entero y da la mitad
        if codigo == 206:
            self.send_header("content-range", "bytes %d-%d/%d" % (a, z, len(datos)))
        self.end_headers()
        try:
            for i in range(0, len(trozo), 16384):
                self.wfile.write(trozo[i:i + 16384])
                CONTADOS["enviados"] += len(trozo[i:i + 16384])
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            pass

    def _error(self, codigo, tipo):
        b = json.dumps({"type": tipo, "status": codigo, "detail": tipo}).encode()
        self.send_response(codigo)
        self.send_header("content-type", "application/problem+json")
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def log_message(self, *a):
        pass


celda = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Celda)
bytes_ = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Bytes)
BYTES_PUERTO = bytes_.server_port
for s in (celda, bytes_):
    threading.Thread(target=s.serve_forever, daemon=True).start()
os.environ["ORE_SERVE"] = "http://127.0.0.1:%d" % celda.server_port
os.environ["PUESTO"] = "p1"

import ore  # noqa: E402
from ore import medios  # noqa: E402

ore.puesto._proveedor = lambda: {"authorization": "Bearer secreto-de-ore"}
fallos = 0


def bien(m):
    print("  ✓", m)


def mal(m):
    global fallos
    fallos += 1
    print("  ✗", m)


def caso(n, f):
    try:
        f()
    except AssertionError as e:
        mal("%s · %s" % (n, e))
    except Exception as e:  # noqa: BLE001
        mal("%s · %s: %s" % (n, type(e).__name__, e))


print("la media en python")
c = ore.coleccion("legal.archivo.contratos")


def c1():
    its = list(c.items())
    assert [i.ref.path for i in its] == ["a.pdf", "b.pdf", "cambia.pdf"], its
    assert sum(1 for m, r, _ in SERVE if "/items" in r) == 2
    bien("1 · items(): tres ítems en dos páginas, por cursor (MediaRef ignora lo desconocido)")


def c2():
    it = c.stat(path="a.pdf")
    assert it.actual is True and it.ref.size == len(A)
    bien("2 · stat(): fresco y actual")


def c3():
    BYTES.clear()
    it = c.stat(path="a.pdf")
    with it.open() as f:
        assert f.read(5) == b"%PDF-"
        f.seek(-10, 2)
        cola = f.read()
        assert cola == A[-10:], cola
        assert f.tell() == len(A)
    rangos = [h.get("Range") for _, _, h in BYTES]
    assert "bytes=%d-" % (len(A) - 10) in rangos, rangos
    bien("3 · open(): read, seek desde el final con un Range (%d peticiones de bytes), tell" % len(BYTES))


def c4():
    it = c.stat(path="a.pdf")
    with it.open() as f:
        assert f.read() == A
    assert it.sha256_visto == SHA["a.pdf"], it.sha256_visto
    bien("4 · una lectura entera se verifica y da el sha256 visto")


def c5():
    malos = [h for _, _, h in BYTES if any(k.lower() in ("authorization", "x-ore-puesto") for k in h)]
    buenos = [h for _, _, h in SERVE
              if {k.lower(): v for k, v in h.items()}.get("authorization") == "Bearer secreto-de-ore"]
    assert not malos, "la URL de los bytes recibió el token: %s" % malos[:1]
    assert buenos and len(buenos) == len(SERVE)
    bien("5 · el token de ORE va a la celda (%d) y nunca a los bytes (%d)" % (len(SERVE), len(BYTES)))


def c6():
    renovadas = [r for _, r, _ in SERVE if "/content" in r and "a.pdf" in r]
    assert len(renovadas) >= 2 and all("version=v1" in r for r in renovadas), renovadas
    bien("6 · el primer permiso caducó a la primera: se pidió otro con version=v1 y se siguió")


def c7():
    try:
        c.stat(path="cambia.pdf").read_bytes()
    except ore.MediaCambiado as e:
        assert e.status == 412
        bien("7 · la versión ya no está: MediaCambiado (412)")
        return
    raise AssertionError("debía ser MediaCambiado")


def c8():
    for path in ("corrupto.pdf", "cortado.pdf"):
        it = c.stat(path=path)
        if path == "corrupto.pdf":
            it.ref = medios.dataclasses.replace(it.ref, digest="sha256:" + "0" * 64)
        try:
            it.read_bytes()
        except ore.MediaCorrupto:
            continue
        raise AssertionError("%s debía ser MediaCorrupto" % path)
    bien("8 · el digest no casa, o el flujo se corta: MediaCorrupto")


def c9():
    medios.EN_PARALELO_DESDE, medios.TROZO = 50_000, 32_768
    BYTES.clear()
    it = c.stat(path="a.pdf")
    assert it.read_bytes(hilos=4) == A
    rangos = [h.get("Range") for _, _, h in BYTES if h.get("Range")]
    assert len(rangos) >= len(A) // 32_768, rangos
    medios.EN_PARALELO_DESDE, medios.TROZO = 32 << 20, 8 << 20
    bien("9 · read_bytes() de uno grande: %d rangos en paralelo, verificado" % len(rangos))


def c10():
    r = {it.ref.path: (d, e) for it, d, e in ore.leer_varios(c.items(), hilos=4)}
    assert r["a.pdf"][0] == A and r["b.pdf"][0] == OBJETOS["b.pdf"]
    assert isinstance(r["cambia.pdf"][1], ore.MediaCambiado)
    bien("10 · leer_varios(): 3 a la vez, el 412 de uno es un valor")


def c11():
    CONTADOS["enviados"] = 0
    with c.stat(path="a.pdf").open() as f:
        f.read(10)
    import time
    time.sleep(0.3)
    assert CONTADOS["enviados"] < len(A), CONTADOS
    bien("11 · cerrar a medias: el servidor envió %d de %d bytes" % (CONTADOS["enviados"], len(A)))


def c12():
    # Un ítem cuyo listado no dice su tamaño (el hallazgo de B3·6 en un puesto
    # de victor): seek desde el final antes de leer nada, y verificar al final.
    it = c.stat(path="a.pdf")
    it.ref = medios.dataclasses.replace(it.ref, size=None)
    with it.open() as f:
        f.seek(-10, 2)
        assert f.read() == A[-10:]
    it = c.stat(path="a.pdf")
    it.ref = medios.dataclasses.replace(it.ref, size=None)
    with it.open() as f:
        assert f.read(5) == b"%PDF-"
        f.seek(-3, 2)
        assert f.read() == A[-3:]
    bien("12 · sin tamaño en el listado: se aprende de la respuesta (o de un byte) y el seek desde el final va")


def c13():
    # La rama del puesto viaja en las peticiones de la media, preguntada una vez.
    RAMA["r"] = "r1/trabajo"
    SERVE.clear()
    try:
        c2_ = ore.coleccion("legal.archivo.contratos")
        c2_.stat(path="a.pdf")
        c2_.stat(path="b.pdf")
    finally:
        RAMA["r"] = None
    fichas = [r for _, r, _ in SERVE if r == "/puestos/p1"]
    h = {k.lower(): v for k, v in SERVE[-1][2].items()}
    assert h.get("x-ore-rama") == "r1/trabajo", h
    assert len(fichas) == 1, fichas
    bien("13 · la rama del puesto va en x-ore-rama (la ficha, preguntada una vez)")


for n, f in enumerate([c1, c2, c3, c4, c5, c6, c7, c8, c9, c10, c11, c12, c13], 1):
    caso(n, f)
celda.shutdown()
bytes_.shutdown()
print("todo bien" if fallos == 0 else "%d fallos" % fallos)
sys.exit(1 if fallos else 0)
