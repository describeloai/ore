#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
graph-de-mentira.py — un Microsoft Graph de mentira para SharePoint (ADR 0061 O5).

  python graph-de-mentira.py PUERTO_GRAPH PUERTO_DESCARGA

No hay emulador de Graph: esto está escrito de la documentación (investigación
§8), con lo justo de lo que usa `ore-graph` y las trampas que la documentación
nombra. Lo que no se sabe sin un tenant (si `versions` trae la actual, qué
contesta pedir la actual por su id…) va como se supone y marcado «supuesto».

Graph, en PUERTO_GRAPH, exige `Authorization: Bearer $GRAPH_TOKEN`:
  GET /v1.0/sites/{host}:/{ruta}            el sitio por ruta (id `host,guid,guid`)
  GET /v1.0/sites/{host}                    el sitio raíz
  GET /v1.0/sites/{id}/drives               sus bibliotecas
  GET /v1.0/drives/{d}/root[:/{ruta}:]      un elemento (y `/children`, en
  GET /v1.0/drives/{d}/items/{id}           páginas de $PAGINA con @odata.nextLink)
  GET …/items/{id}/versions                 de la nueva a la vieja, con la actual (supuesto)
  GET …/items/{id}/content                  302 al host de descarga (o 304 con if-none-match)
  GET …/items/{id}/versions/{v}/content     302; la actual, 400 (supuesto: «no se puede»)

La descarga, en PUERTO_DESCARGA (otro origen, como `*.sharepoint.com`): la URL
caduca a los 60 s, **rechaza un `Authorization`** (`401`: el token de Graph no
debe llegar aquí) y sirve `Range` (`206`, `416`). La de `/content` sirve lo
vigente **al bajar**, no al pedirla (el peor caso: no está fijada a nada).

Negativas: un sitio que no está en $CONCEDIDOS da `403 accessDenied` (la app no
tiene la concesión `Sites.Selected` de ese sitio); lo que no está, `404
itemNotFound`; cada $CADA_429 peticiones (0: nunca), `429 activityLimitReached`
con `Retry-After: 1`.

Para sembrar (sin token, sólo en 127.0.0.1):
  PUT    /_mentira?sitio=sites/Finanzas&biblioteca=Documentos&ruta=a/b.pdf[&tipo=package|remote]
         cuerpo = los bytes: una versión nueva (1.0, 2.0…), cambia eTag y cTag
  POST   /_mentira/tocar?…&ruta=…           sólo los metadatos: cambia el eTag, no el cTag
  DELETE /_mentira/version?…&ruta=…&version=1.0   recorta una versión vieja
  DELETE /_mentira?…&ruta=…                 borra el fichero
  GET    /_mentira/cuentas                  {"peticiones":…, "429":…, "descargas":…, "con_token":…}

Escucha en 127.0.0.1 e imprime `listo`. El quickXorHash se coteja al arrancar con
los vectores de rclone (`crates/ore-objetos/src/quickxor-vectores.txt`).
"""
import base64
import json
import os
import re
import sys
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, quote, unquote, urlparse

PG = int(sys.argv[1]) if len(sys.argv) > 1 else 8790
PD = int(sys.argv[2]) if len(sys.argv) > 2 else 8791
TOKEN = os.environ.get("GRAPH_TOKEN", "mentira")
PAGINA = int(os.environ.get("PAGINA", "2"))
CADA_429 = int(os.environ.get("CADA_429", "0"))
CONCEDIDOS = set(os.environ.get("CONCEDIDOS", "sites/Finanzas").split(","))
VIDA_URL = 60

CERROJO = threading.Lock()
CUENTAS = {"peticiones": 0, "429": 0, "descargas": 0, "con_token": 0}


def xor_de(b: bytes) -> int:
    """El XOR de todos los bytes, plegando el entero en C (no byte a byte)."""
    if not b:
        return 0
    largo = 1
    while largo < len(b):
        largo *= 2
    n = int.from_bytes(b, "little")
    while largo > 1:
        largo //= 2
        n = (n & ((1 << (8 * largo)) - 1)) ^ (n >> (8 * largo))
    return n


def quickxor(datos: bytes) -> str:
    # El byte i va desplazado 11*i bits en un registro de 160: todos los de la
    # misma posición módulo 160 van igual, así que se juntan antes por XOR.
    r = bytearray(20)
    for resto in range(min(160, len(datos))):
        b = xor_de(datos[resto::160])
        bit = (11 * resto) % 160
        i, d = bit // 8, bit % 8
        r[i] ^= (b << d) & 0xFF
        if d:
            r[(i + 1) % 20] ^= b >> (8 - d)
    for i, b in enumerate(len(datos).to_bytes(8, "little")):
        r[12 + i] ^= b
    return base64.b64encode(bytes(r)).decode()


def cotejar_quickxor():
    aqui = os.path.dirname(os.path.abspath(__file__))
    p = os.path.join(aqui, "..", "crates", "ore-objetos", "src", "quickxor-vectores.txt")
    n = 0
    for l in open(p, encoding="utf-8"):
        if l.startswith("#") or not l.strip():
            continue
        _, h, q = l.split()
        datos = b"" if h == "-" else bytes.fromhex(h)
        if quickxor(datos) != q:
            sys.exit(f"el quickXorHash de mentira no casa con rclone ({len(datos)} bytes)")
        n += 1
    return n


def ahora_iso(t=None):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(t or time.time()))


class Elemento:
    def __init__(self, nombre, padre, tipo):
        self.id = "01" + uuid.uuid4().hex[:30].upper()
        self.nombre = nombre
        self.padre = padre  # el id de la carpeta, o None (la raíz)
        self.tipo = tipo  # carpeta | fichero | package | remote
        self.guid = "{" + str(uuid.uuid4()).upper() + "}"
        self.versiones = []  # [(etiqueta, bytes, epoch, quickxor)], de la vieja a la nueva
        self.meta = 1  # cuántas veces cambió (contenido o metadatos): el eTag
        self.contenido = 1  # cuántas veces cambió el contenido: el cTag
        self.modificado = time.time()


class Biblioteca:
    def __init__(self, nombre, sitio):
        self.id = "b!" + base64.urlsafe_b64encode(uuid.uuid4().bytes * 2).decode().rstrip("=")
        self.nombre = nombre
        self.sitio = sitio
        self.raiz = Elemento("root", None, "carpeta")
        self.elementos = {self.raiz.id: self.raiz}

    def hijos(self, e):
        return sorted(
            (x for x in self.elementos.values() if x.padre == e.id), key=lambda x: x.nombre
        )

    def ruta_de(self, e):
        partes = []
        while e.padre is not None:
            partes.append(e.nombre)
            e = self.elementos[e.padre]
        return "/".join(reversed(partes))

    def por_ruta(self, ruta, crear=False):
        e = self.raiz
        for p in [x for x in ruta.split("/") if x]:
            h = next((x for x in self.hijos(e) if x.nombre.lower() == p.lower()), None)
            if h is None:
                if not crear:
                    return None
                h = Elemento(p, e.id, "carpeta")
                self.elementos[h.id] = h
            e = h
        return e


class Sitio:
    def __init__(self, ruta, bibliotecas):
        self.ruta = ruta
        self.id = None  # se completa con el host de la primera petición
        self.guids = (str(uuid.uuid4()), str(uuid.uuid4()))
        self.bibliotecas = {n: Biblioteca(n, self) for n in bibliotecas}


SITIOS = {
    "": Sitio("", ["Documents"]),
    "sites/Finanzas": Sitio("sites/Finanzas", ["Documentos", "Activos del sitio"]),
    "sites/RRHH": Sitio("sites/RRHH", ["Documentos"]),
}
DESCARGAS = {}  # token → (biblioteca, elemento, etiqueta | None, caduca)


def id_de_sitio(s, host):
    if s.id is None:
        s.id = f"{host},{s.guids[0]},{s.guids[1]}"
    return s.id


def sitio_por_id(i):
    return next((s for s in SITIOS.values() if s.id == i), None)


def biblioteca_por_id(i):
    for s in SITIOS.values():
        for b in s.bibliotecas.values():
            if b.id == i:
                return b
    return None


class Fallo(Exception):
    def __init__(self, estado, codigo, mensaje, cabeceras=None):
        self.estado, self.codigo, self.mensaje = estado, codigo, mensaje
        self.cabeceras = cabeceras or {}


def concedido(s):
    if s.ruta not in CONCEDIDOS:
        raise Fallo(403, "accessDenied", "Access denied")


def item_json(b, e, base):
    j = {
        "id": e.id,
        "name": e.nombre,
        "eTag": f'"{e.guid},{e.meta}"',
        "lastModifiedDateTime": ahora_iso(e.modificado),
        "parentReference": {
            "driveId": b.id,
            "driveType": "documentLibrary",
            "id": e.padre,
            "path": f"/drives/{b.id}/root:" + ("/" + b.ruta_de(b.elementos[e.padre]) if e.padre and e.padre != b.raiz.id else ""),
        },
    }
    if e.padre is None:
        j["root"] = {}
        j.pop("parentReference")
    if e.tipo == "carpeta":
        j["folder"] = {"childCount": len(b.hijos(e))}
        j["size"] = 0
    else:
        datos = e.versiones[-1][1]
        j["size"] = len(datos)
        j["cTag"] = f'"c:{e.guid},{e.contenido}"'
        if e.tipo == "fichero":
            j["file"] = {
                "mimeType": "application/octet-stream",
                "hashes": {"quickXorHash": e.versiones[-1][3]},
            }
        elif e.tipo == "package":
            j["package"] = {"type": "oneNote"}
        elif e.tipo == "remote":
            j["remoteItem"] = {"id": "01REMOTO", "parentReference": {"driveId": "b!otro"}}
    return j


class Graph(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, formato, *a):
        if os.environ.get("DEPURA"):
            sys.stderr.write(f"{time.time():.3f} {self.address_string()} {formato % a}\n")

    def responder(self, estado, cuerpo=b"", cabeceras=None, tipo="application/json"):
        if isinstance(cuerpo, (dict, list)):
            cuerpo = json.dumps(cuerpo).encode()
        self.send_response(estado)
        for k, v in (cabeceras or {}).items():
            self.send_header(k, v)
        if cuerpo or estado not in (204, 302, 304):
            self.send_header("Content-Type", tipo)
        self.send_header("Content-Length", str(len(cuerpo)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(cuerpo)

    def fallo(self, f):
        self.responder(
            f.estado,
            {"error": {"code": f.codigo, "message": f.mensaje, "innerError": {"request-id": str(uuid.uuid4())}}},
            f.cabeceras,
        )

    def cuerpo(self):
        n = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(n) if n else b""

    # ── sembrar ──────────────────────────────────────────────────────────────
    def sembrar(self):
        u = urlparse(self.path)
        q = {k: v[0] for k, v in parse_qs(u.query).items()}
        if u.path == "/_mentira/cuentas":
            return self.responder(200, CUENTAS)
        s = SITIOS.get(q.get("sitio", ""))
        b = s and s.bibliotecas.get(q.get("biblioteca", ""))
        if not b:
            return self.responder(404, {"error": "sitio o biblioteca"})
        ruta = q.get("ruta", "")
        with CERROJO:
            if self.command == "PUT":
                datos = self.cuerpo()
                padre, _, nombre = ruta.rpartition("/")
                c = b.por_ruta(padre, crear=True)
                e = next((x for x in b.hijos(c) if x.nombre == nombre), None)
                if e is None:
                    e = Elemento(nombre, c.id, q.get("tipo", "fichero"))
                    b.elementos[e.id] = e
                else:
                    e.meta += 1
                    e.contenido += 1
                e.versiones.append((f"{len(e.versiones) + 1}.0", datos, time.time(), quickxor(datos)))
                e.modificado = time.time()
                return self.responder(201, {"id": e.id, "version": e.versiones[-1][0]})
            e = b.por_ruta(ruta)
            if e is None:
                return self.responder(404, {"error": "ruta"})
            if u.path == "/_mentira/tocar":
                e.meta += 1
                return self.responder(200, {"eTag": e.meta})
            if u.path == "/_mentira/version":
                e.versiones = [v for v in e.versiones if v[0] != q.get("version")]
                return self.responder(204)
            del b.elementos[e.id]
            return self.responder(204)

    # ── Graph ────────────────────────────────────────────────────────────────
    def do_PUT(self):
        self.sembrar()

    def do_POST(self):
        self.sembrar()

    def do_DELETE(self):
        self.sembrar()

    def do_GET(self):
        if self.path.startswith("/_mentira"):
            return self.sembrar()
        with CERROJO:
            CUENTAS["peticiones"] += 1
            n = CUENTAS["peticiones"]
        try:
            if self.headers.get("Authorization") != f"Bearer {TOKEN}":
                raise Fallo(401, "InvalidAuthenticationToken", "Access token is empty or invalid.")
            if CADA_429 and n % CADA_429 == 0:
                CUENTAS["429"] += 1
                raise Fallo(429, "activityLimitReached", "The request has been throttled", {"Retry-After": "1"})
            with CERROJO:
                self.graph()
        except Fallo as f:
            self.fallo(f)

    def graph(self):
        u = urlparse(self.path)
        camino = unquote(u.path)
        q = {k: v[0] for k, v in parse_qs(u.query).items()}
        base = f"http://127.0.0.1:{PG}"
        if not camino.startswith("/v1.0/"):
            raise Fallo(400, "BadRequest", "Invalid version")
        camino = camino[len("/v1.0"):]

        # el sitio
        m = re.fullmatch(r"/sites/([^/:,]+)(?::/(.*?))?/?", camino)
        if m:
            host, ruta = m.group(1), (m.group(2) or "").strip("/")
            s = next((x for x in SITIOS.values() if x.ruta.lower() == ruta.lower()), None)
            if s is None:
                raise Fallo(404, "itemNotFound", "Requested site could not be found")
            concedido(s)
            return self.responder(200, {
                "id": id_de_sitio(s, host),
                "name": s.ruta.split("/")[-1] or "root",
                "webUrl": f"https://{host}/{s.ruta}",
            })
        m = re.fullmatch(r"/sites/([^/]+)/drives", camino)
        if m:
            s = sitio_por_id(m.group(1))
            if s is None:
                raise Fallo(404, "itemNotFound", "Requested site could not be found")
            concedido(s)
            return self.responder(200, {"value": [
                {"id": b.id, "name": b.nombre, "driveType": "documentLibrary"}
                for b in s.bibliotecas.values()
            ]})

        m = re.fullmatch(r"/drives/([^/]+)/(root(?::/([^:]*):?)?|items/([^/]+))(/.*)?", camino)
        if not m:
            raise Fallo(400, "invalidRequest", f"Invalid request: {camino}")
        b = biblioteca_por_id(m.group(1))
        if b is None:
            raise Fallo(404, "itemNotFound", "The drive could not be found")
        concedido(b.sitio)
        if m.group(4):
            e = b.elementos.get(m.group(4))
        elif m.group(3) is not None:
            e = b.por_ruta(m.group(3))
        else:
            e = b.raiz
        if e is None:
            raise Fallo(404, "itemNotFound", "The resource could not be found.")
        resto = (m.group(5) or "").rstrip("/")
        if resto in ("", ":"):
            return self.responder(200, item_json(b, e, base))
        if resto == "/children":
            if e.tipo != "carpeta":
                raise Fallo(400, "invalidRequest", "Item is not a folder")
            hijos = b.hijos(e)
            desde = int(q.get("$skiptoken", "0"))
            top = min(int(q.get("$top", "200")), PAGINA)
            j = {"value": [item_json(b, x, base) for x in hijos[desde:desde + top]]}
            if desde + top < len(hijos):
                j["@odata.nextLink"] = (
                    f"{base}/v1.0/drives/{b.id}/items/{e.id}/children?$top={top}&$skiptoken={desde + top}"
                )
            return self.responder(200, j)
        if e.tipo == "carpeta":
            raise Fallo(400, "invalidRequest", "Item is a folder")
        if resto == "/versions":
            return self.responder(200, {"value": [
                {"id": v[0], "lastModifiedDateTime": ahora_iso(v[2]), "size": len(v[1])}
                for v in reversed(e.versiones)
            ]})
        if resto == "/content":
            cn = self.headers.get("If-None-Match")
            if cn and cn in (f'"{e.guid},{e.meta}"', f'"c:{e.guid},{e.contenido}"'):
                return self.responder(304)
            return self.redirigir(b, e, None)
        m2 = re.fullmatch(r"/versions/([^/]+)/content", resto)
        if m2:
            v = m2.group(1)
            if v == e.versiones[-1][0]:
                raise Fallo(400, "invalidRequest", "The current version cannot be downloaded by id; use /content")
            if not any(x[0] == v for x in e.versiones):
                raise Fallo(404, "itemNotFound", "The version could not be found.")
            return self.redirigir(b, e, v)
        raise Fallo(400, "invalidRequest", f"Invalid request: {resto}")

    def redirigir(self, b, e, etiqueta):
        tok = uuid.uuid4().hex
        DESCARGAS[tok] = (b, e.id, etiqueta, time.time() + VIDA_URL)
        self.responder(302, b"", {"Location": f"http://127.0.0.1:{PD}/_layouts/15/download.aspx?UniqueId={e.guid}&tempauth={tok}"})


class Descarga(Graph):
    def do_GET(self):
        with CERROJO:
            CUENTAS["descargas"] += 1
            if self.headers.get("Authorization"):
                CUENTAS["con_token"] += 1
                return self.responder(401, {"error": {"code": "invalidRequest", "message": "Authorization not allowed on a pre-authenticated URL"}})
            tok = parse_qs(urlparse(self.path).query).get("tempauth", [""])[0]
            d = DESCARGAS.get(tok)
            if d is None or d[3] < time.time():
                return self.responder(401, {"error": {"code": "unauthenticated", "message": "Token expired"}})
            b, i, etiqueta, _ = d
            e = b.elementos.get(i)
            if e is None:
                return self.responder(404, {"error": {"code": "itemNotFound", "message": "gone"}})
            vs = [v for v in e.versiones if etiqueta is None or v[0] == etiqueta]
            if not vs:
                return self.responder(404, {"error": {"code": "itemNotFound", "message": "gone"}})
            datos = vs[-1][1]
        r = self.headers.get("Range")
        if not r:
            return self.responder(200, datos, tipo="application/octet-stream")
        m = re.fullmatch(r"bytes=(\d*)-(\d*)", r.strip())
        if not m or (not m.group(1) and not m.group(2)):
            return self.responder(200, datos, tipo="application/octet-stream")
        n = len(datos)
        if m.group(1):
            a = int(m.group(1))
            z = min(int(m.group(2)), n - 1) if m.group(2) else n - 1
        else:
            a, z = max(0, n - int(m.group(2))), n - 1
        if a >= n or a > z:
            return self.responder(416, b"", {"Content-Range": f"bytes */{n}"})
        self.responder(206, datos[a:z + 1], {"Content-Range": f"bytes {a}-{z}/{n}"}, tipo="application/octet-stream")


def main():
    n = cotejar_quickxor()
    g = ThreadingHTTPServer(("127.0.0.1", PG), Graph)
    d = ThreadingHTTPServer(("127.0.0.1", PD), Descarga)
    threading.Thread(target=d.serve_forever, daemon=True).start()
    print(f"listo (quickXorHash cotejado con {n} vectores)", flush=True)
    g.serve_forever()


if __name__ == "__main__":
    main()
