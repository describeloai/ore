#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
drive-de-mentira.py — un Google Drive de mentira (ADR 0082, paso 5).

  python drive-de-mentira.py PUERTO

No hay emulador de Drive: esto está escrito de lo que X0 midió contra el de
verdad (docs/products/0082-origin-google-drive.md §3.1), con lo justo de lo que
usa `ore-gdrive`. Lo que se mide aquí es lo que X0 vio; lo que X0 no vio (las
unidades compartidas, el `429` real, un token de cambios caducado) no está.

Drive, en PUERTO (exige `Authorization: Bearer $DRIVE_TOKEN`, salvo el IAM):
  POST /v1/projects/-/serviceAccounts/{cuenta}:generateAccessToken   (cualquier Bearer)
                                           la autosuplantación: devuelve $DRIVE_TOKEN
  GET  /drive/v3/files?q=…                 hijos de una carpeta (`'ID' in parents and
                                           trashed=false`, y por nombre) en páginas de
                                           $PAGINA; la papelera no se ve (X0 C5)
  GET  /drive/v3/files/{id}                los metadatos (`403` para SINPERMISO)
  GET  /drive/v3/files/{id}?alt=media      los bytes (Range, también por el final: X0 B3);
                                           un nativo, `400` (X0 D1); `If-Match` se ignora (X0 B4)
  GET  /drive/v3/files/{id}/revisions/{r}?alt=media
                                           sólo la revisión actual; otra, `404` (X0 B2)
  GET  /drive/v3/files/{id}/export?mimeType=…
                                           un nativo, ENTERO: el Range se ignora; el `txt`
                                           de un Doc lleva BOM, el `csv` de una Hoja CRLF y
                                           sin salto final (X0 D3); `docx`/`xlsx` no son
                                           deterministas (X0 D3) y aquí tampoco
  GET  /drive/v3/changes/startPageToken    el token de ahora
  GET  /drive/v3/changes?pageToken=…       lo que cambió desde él, de $PAGINA en $PAGINA
Cada $CADA_429 peticiones (0: nunca), alternando, `429` con `Retry-After: 1` y
`403 userRateLimitExceeded`.

Para sembrar (sin token, sólo en 127.0.0.1; `raiz=OTRA` siembra fuera de la carpeta
compartida, como un fichero que la cuenta ve en otro sitio):
  PUT    /_mentira?ruta=a/b.csv[&dup=1][&raiz=OTRA]   una revisión nueva (cambian md5 y
                                           headRevisionId); con `dup=1` un fichero más
                                           con el mismo nombre
  POST   /_mentira/nativo?ruta=…&clase=doc|hoja|pres  un nativo (el cuerpo es su texto)
  POST   /_mentira/atajo?ruta=…&destino=ID            un acceso directo
  POST   /_mentira/tocar?ruta=…                       sólo metadatos: sube `version`
  POST   /_mentira/renombrar?ruta=…&a=nombre
  POST   /_mentira/mover?ruta=…&a=carpeta/            a otra carpeta bajo la raíz
  POST   /_mentira/papelera?ruta=…
  POST   /_mentira/caducar                            los tokens de cambios de antes dejan de valer
                                           (`400`): SUPUESTO, X0 no vio qué contesta Drive
  DELETE /_mentira?ruta=…                             borrado de verdad (`removed`)
  GET    /_mentira/id?ruta=…                          el ID de un fichero
  GET    /_mentira/cuentas                            {"peticiones":…, "429":…, "403":…, "sin_token":…}

Escucha en 127.0.0.1 e imprime `listo`. La carpeta compartida es `RAIZ`.
"""
import hashlib
import json
import os
import re
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, unquote, urlparse

PUERTO = int(sys.argv[1]) if len(sys.argv) > 1 else 8792
TOKEN = os.environ.get("DRIVE_TOKEN", "mentira")
PAGINA = int(os.environ.get("PAGINA", "2"))
CADA_429 = int(os.environ.get("CADA_429", "0"))

CARPETA = "application/vnd.google-apps.folder"
ACCESO = "application/vnd.google-apps.shortcut"
NATIVOS = {
    "doc": "application/vnd.google-apps.document",
    "hoja": "application/vnd.google-apps.spreadsheet",
    "pres": "application/vnd.google-apps.presentation",
}
CERROJO = threading.Lock()
CUENTAS = {"peticiones": 0, "429": 0, "403": 0, "sin_token": 0}
ARCHIVOS = {}
BORRADOS = set()
CAMBIOS = []  # (seq, id)
SEQ = 0
CADUCADO = 0
N = 0


def ahora():
    return time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime())


def nuevo(nombre, mime, padre, contenido=None, **mas):
    global N
    N += 1
    f = {"id": f"i{N}", "name": nombre, "mime": mime, "padre": padre, "contenido": contenido,
         "rev": 1, "version": 1, "modificado": ahora(), "papelera": False, "destino": None}
    f.update(mas)
    ARCHIVOS[f["id"]] = f
    cambiado(f["id"], contenido=False)
    return f


def cambiado(i, contenido):
    """Un cambio: sube `version` (también con sólo metadatos: X0 B5) y entra en el feed."""
    global SEQ
    f = ARCHIVOS.get(i)
    if f and i not in BORRADOS:
        f["version"] += 1
        if contenido:
            f["rev"] += 1
        f["modificado"] = ahora()
    SEQ += 1
    CAMBIOS.append((SEQ, i))


for carpeta in ("RAIZ", "OTRA", "SINPERMISO"):
    ARCHIVOS[carpeta] = {"id": carpeta, "name": carpeta, "mime": CARPETA, "padre": "", "contenido": None,
                         "rev": 1, "version": 1, "modificado": ahora(), "papelera": False, "destino": None}


def hijo(padre, nombre, solo_uno=True):
    r = [f for f in ARCHIVOS.values()
         if f["padre"] == padre and f["name"] == nombre and not f["papelera"] and f["id"] not in BORRADOS]
    return r[0] if r and solo_uno else r


def carpetas(ruta, raiz="RAIZ"):
    """La carpeta de `a/b/` bajo `raiz`, creando lo que falte."""
    p = raiz
    for seg in [s for s in ruta.split("/") if s]:
        c = hijo(p, seg)
        p = c["id"] if c and c["mime"] == CARPETA else nuevo(seg, CARPETA, p)["id"]
    return p


def buscar(ruta, raiz="RAIZ"):
    dir_, _, nombre = ruta.rstrip("/").rpartition("/")
    p = carpetas(dir_, raiz)
    return hijo(p, nombre) if nombre else ARCHIVOS[p]


def md5(f):
    return hashlib.md5(f["contenido"]).hexdigest()


def es_nativo(f):
    return f["mime"] in NATIVOS.values()


def ficha(f):
    d = {"id": f["id"], "name": f["name"], "mimeType": f["mime"], "modifiedTime": f["modificado"],
         "version": str(f["version"]), "trashed": f["papelera"]}
    if f["padre"]:
        d["parents"] = [f["padre"]]
    if f["mime"] == ACCESO:
        d["shortcutDetails"] = {"targetId": f["destino"], "targetMimeType": "text/csv"}
    elif es_nativo(f):
        d["size"] = "1024"  # X0 D4: no es el tamaño de nada
    elif f["mime"] != CARPETA:
        d["size"] = str(len(f["contenido"]))
        d["md5Checksum"] = md5(f)
        d["headRevisionId"] = f"rev-{f['id']}-{f['rev']}"
    return d


def exportado(f, mime):
    """Lo que Drive exporta de un nativo (X0 D2, D3)."""
    t = f["contenido"].decode("utf-8")
    if mime == "text/plain":
        return b"\xef\xbb\xbf" + t.rstrip("\n").encode("utf-8")
    if mime == "text/csv":
        return t.rstrip("\n").replace("\n", "\r\n").encode("utf-8")
    if mime == "application/pdf":
        return b"%PDF-1.4\n% " + t.encode("utf-8") + b"\n%%EOF\n"
    # docx / xlsx: un zip con marcas de tiempo, distinto cada vez (X0 D3)
    return b"PK\x03\x04" + str(time.time_ns()).encode() + t.encode("utf-8")


def rango(cab, n):
    """`bytes=a-b`, `-n`, `a-` → (inicio, fin exclusivo), o None si no cabe."""
    m = re.fullmatch(r"bytes=(\d*)-(\d*)", cab or "")
    if not m or (not m[1] and not m[2]):
        return (0, n)
    if not m[1]:
        return (max(0, n - int(m[2])), n)
    a = int(m[1])
    b = n if not m[2] else min(n, int(m[2]) + 1)
    return None if a >= n or a >= b else (a, b)


def desescapar(s):
    return re.sub(r"\\(.)", r"\1", s)


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def responder(self, estado, cuerpo=b"", tipo="application/json", cab=None):
        if not isinstance(cuerpo, bytes):
            cuerpo = json.dumps(cuerpo).encode("utf-8")
        self.send_response(estado)
        self.send_header("Content-Type", tipo)
        self.send_header("Content-Length", str(len(cuerpo)))
        for k, v in (cab or {}).items():
            self.send_header(k, v)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(cuerpo)

    def error(self, estado, razon, msg, cab=None):
        self.responder(estado, {"error": {"code": estado, "message": msg,
                                          "errors": [{"message": msg, "domain": "global", "reason": razon}]}}, cab=cab)

    def cuerpo(self):
        n = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(n) if n else b""

    def do_GET(self):
        self.atender()

    do_PUT = do_POST = do_DELETE = do_GET

    def atender(self):
        u = urlparse(self.path)
        q = {k: v[0] for k, v in parse_qs(u.query, keep_blank_values=True).items()}
        with CERROJO:
            if u.path.startswith("/_mentira"):
                if self.client_address[0] != "127.0.0.1":
                    return self.error(403, "forbidden", "sólo en local")
                return self.sembrar(u.path, q)
            if u.path.startswith("/v1/projects/"):  # el IAM de la autosuplantación
                self.cuerpo()
                return self.responder(200, {"accessToken": TOKEN, "expireTime": "2099-01-01T00:00:00Z"})
            CUENTAS["peticiones"] += 1
            if self.headers.get("Authorization") != f"Bearer {TOKEN}":
                CUENTAS["sin_token"] += 1
                return self.error(401, "authError", "Invalid Credentials")
            if CADA_429 and CUENTAS["peticiones"] % CADA_429 == 0:
                if (CUENTAS["429"] + CUENTAS["403"]) % 2 == 0:
                    CUENTAS["429"] += 1
                    return self.error(429, "rateLimitExceeded", "Rate Limit Exceeded", {"Retry-After": "1"})
                CUENTAS["403"] += 1
                return self.error(403, "userRateLimitExceeded", "User Rate Limit Exceeded")
            self.drive(u.path, q)

    # ── Drive ──────────────────────────────────────────────────────────────
    def drive(self, ruta, q):
        if ruta == "/drive/v3/files":
            return self.listar(unquote(q.get("q", "")), q)
        m = re.fullmatch(r"/drive/v3/files/([^/]+)(?:/(export)|/revisions/([^/]+))?", ruta)
        if m:
            return self.fichero(m[1], m[2], m[3], q)
        if ruta == "/drive/v3/changes/startPageToken":
            return self.responder(200, {"startPageToken": str(SEQ)})
        if ruta == "/drive/v3/changes":
            return self.feed(q)
        self.error(404, "notFound", f"no route {ruta}")

    def listar(self, consulta, q):
        m = re.fullmatch(r"'((?:\\.|[^'\\])*)' in parents and trashed=false"
                         r"(?: and mimeType='([^']*)' and name='((?:\\.|[^'\\])*)')?", consulta)
        if not m:
            return self.error(400, "invalid", f"Invalid Value: {consulta}")
        padre = desescapar(m[1])
        if padre == "SINPERMISO":
            return self.error(403, "forbidden", "The user does not have sufficient permissions for this file.")
        hijos = sorted((f for f in ARCHIVOS.values()
                        if f["padre"] == padre and not f["papelera"] and f["id"] not in BORRADOS),
                       key=lambda f: f["name"])
        if m[2] is not None:
            hijos = [f for f in hijos if f["mime"] == m[2] and f["name"] == desescapar(m[3])]
        desde = int(q.get("pageToken", "p0")[1:]) if q.get("pageToken") else 0
        pagina = hijos[desde:desde + PAGINA]
        r = {"files": [ficha(f) for f in pagina]}
        if desde + PAGINA < len(hijos):
            r["nextPageToken"] = f"p{desde + PAGINA}"
        self.responder(200, r)

    def fichero(self, i, accion, rev, q):
        if i == "SINPERMISO":
            return self.error(403, "forbidden", "The user does not have sufficient permissions for this file.")
        f = ARCHIVOS.get(i)
        if not f or i in BORRADOS:
            return self.error(404, "notFound", f"File not found: {i}.")
        if accion == "export":
            if not es_nativo(f):
                return self.error(403, "fileNotExportable", "Export only supports Docs Editors files.")
            return self.responder(200, exportado(f, q.get("mimeType", "")), q.get("mimeType", "application/octet-stream"))
        if rev is not None:
            if f["mime"] == CARPETA or es_nativo(f) or rev != f"rev-{f['id']}-{f['rev']}":
                return self.error(404, "notFound", f"Revision not found: {rev}.")
        elif q.get("alt") != "media":
            return self.responder(200, ficha(f))
        if f["mime"] == CARPETA or f["mime"] == ACCESO:
            return self.error(403, "fileNotDownloadable", "Only files with binary content can be downloaded.")
        if es_nativo(f):
            return self.error(400, "badRequest", 'Unsupported alt type "media" for non byte stream request.')
        datos = f["contenido"]
        r = rango(self.headers.get("Range"), len(datos))
        if r is None:
            return self.error(416, "requestedRangeNotSatisfiable", "Requested range not satisfiable")
        if self.headers.get("Range"):
            a, b = r
            return self.responder(206, datos[a:b], f["mime"],
                                  {"Content-Range": f"bytes {a}-{b - 1}/{len(datos)}"})
        self.responder(200, datos, f["mime"])

    def feed(self, q):
        try:
            desde, _, pos = q.get("pageToken", "").partition(".")
            desde, pos = int(desde), int(pos or 0)
        except ValueError:
            return self.error(400, "invalid", "Invalid Value")
        if desde < CADUCADO:
            return self.error(400, "invalid", "Invalid Value: the page token has expired")
        ultimo = {}
        for s, i in CAMBIOS:
            if s > desde:
                ultimo[i] = s
        orden = [i for i, _ in sorted(ultimo.items(), key=lambda x: x[1])]
        pagina = orden[pos:pos + PAGINA]
        cs = []
        for i in pagina:
            if i in BORRADOS:
                cs.append({"fileId": i, "removed": True})
            else:
                cs.append({"fileId": i, "removed": False, "file": ficha(ARCHIVOS[i])})
        r = {"changes": cs}
        if pos + PAGINA < len(orden):
            r["nextPageToken"] = f"{desde}.{pos + PAGINA}"
        else:
            r["newStartPageToken"] = str(SEQ)
        self.responder(200, r)

    # ── la siembra ─────────────────────────────────────────────────────────
    def sembrar(self, ruta, q):
        cuerpo = self.cuerpo()
        raiz = q.get("raiz", "RAIZ")
        if ruta == "/_mentira/cuentas":
            return self.responder(200, CUENTAS)
        if ruta == "/_mentira/caducar":
            global SEQ, CADUCADO
            SEQ += 1
            CADUCADO = SEQ
            return self.responder(200, {})
        r = q.get("ruta", "")
        if ruta == "/_mentira/id":
            f = buscar(r, raiz)
            return self.responder(200, {"id": f["id"]}) if f else self.error(404, "notFound", r)
        if self.command == "PUT" and ruta == "/_mentira":
            dir_, _, nombre = r.rpartition("/")
            p = carpetas(dir_, raiz)
            f = None if q.get("dup") else hijo(p, nombre)
            if f:
                f["contenido"] = cuerpo
                cambiado(f["id"], contenido=True)
            else:
                nuevo(nombre, q.get("tipo", "text/csv"), p, cuerpo)
            return self.responder(201, {})
        if ruta == "/_mentira/nativo":
            dir_, _, nombre = r.rpartition("/")
            p = carpetas(dir_, raiz)
            f = hijo(p, nombre)
            if f:
                f["contenido"] = cuerpo
                cambiado(f["id"], contenido=True)
            else:
                nuevo(nombre, NATIVOS[q.get("clase", "doc")], p, cuerpo)
            return self.responder(201, {})
        if ruta == "/_mentira/atajo":
            dir_, _, nombre = r.rpartition("/")
            nuevo(nombre, ACCESO, carpetas(dir_, raiz), destino=q.get("destino"))
            return self.responder(201, {})
        f = buscar(r, raiz) if r else None
        if not f:
            return self.error(404, "notFound", f"`{r}` no está")
        if ruta == "/_mentira/tocar":
            cambiado(f["id"], contenido=False)
        elif ruta == "/_mentira/renombrar":
            f["name"] = q["a"]
            cambiado(f["id"], contenido=False)
        elif ruta == "/_mentira/mover":
            f["padre"] = carpetas(q["a"], "RAIZ")
            cambiado(f["id"], contenido=False)
        elif ruta == "/_mentira/papelera":
            f["papelera"] = True
            cambiado(f["id"], contenido=False)
        elif self.command == "DELETE" and ruta == "/_mentira":
            fuera, pendientes = [f["id"]], [f["id"]]
            while pendientes:
                p = pendientes.pop()
                for i, x in ARCHIVOS.items():
                    if x["padre"] == p and i not in fuera:
                        fuera.append(i)
                        pendientes.append(i)
            for i in fuera:
                BORRADOS.add(i)
                cambiado(i, contenido=False)
        else:
            return self.error(404, "notFound", ruta)
        self.responder(200, {})


class Servidor(ThreadingHTTPServer):
    def handle_error(self, request, client_address):
        pass  # un cliente que cierra su conexión viva no es un error


if __name__ == "__main__":
    s = Servidor(("127.0.0.1", PUERTO), H)
    s.daemon_threads = True
    print("listo", flush=True)
    s.serve_forever()
