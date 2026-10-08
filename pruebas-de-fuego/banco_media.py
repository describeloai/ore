"""EL BANCO DE LA MEDIA (0049 B4·3/B4b·3 P0): una celda de mentira, sin clúster.

Dos servidores, como en la celda de verdad:

- **`Celda`**, lo que haría `ore-serve`: la ficha del puesto (su rama), la media
  (`items`, `item`, el `307` de `content`), `put` por transacciones
  (`transactions`, `commit`, `abort`), declarar y retirar un transform, y
  `/documentos/MediaCollection/…` (leer y escribir el documento);
- **`Medios`**, lo que haría `ore-medios` en su puerto del puesto (o el lago): los
  bytes con rangos y permisos que caducan, `412`, flujos cortados; y la subida
  (`PUT /subida?permiso=&path=`), con el sha256 al paso, el tipo por los bytes
  y el `Repr-Digest` cotejado.

Y, para los ficheros que dan ficheros (0049 B9), una **entrada** cuyo origen se
cambia (`ENTRADA`, la colección `DERIVAR`) y el **índice de cada colección
escrita** (`ESCRITAS`) con lo que `ore-medios` hace al sellar: el linaje de cada
fichero, una entrada que reemplaza la salida anterior de su origen, las marcas
(que no son ítems), `retire_sources` y `retire`, y lo que no cuadra.

Todo lo que llega queda en `SERVE` y `BYTES` (método, ruta, cabeceras), para
comprobar lo que el SDK manda —y lo que NO manda: el token de ORE nunca va a los
bytes ni a la subida—. Los modos (`MODOS`) provocan lo que la red y la forja
hacen: cortar una subida, perder la carrera de un commit.

    import banco_media as banco
    banco.arrancar()          # y después `import ore`
"""
import base64
import hashlib
import json
import os
import random
import re
import sys
import threading
import http.server
import urllib.parse

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(RAIZ, "puesto", "python"))

random.seed(49)
A = b"%PDF-" + random.randbytes(3_000_000)
OBJETOS = {"a.pdf": A, "b.pdf": b"%PDF-otro", "cambia.pdf": b"%PDF-viejo",
           "corrupto.pdf": b"%PDF-bytes", "cortado.pdf": b"%PDF-" + b"x" * 5000}
SHA = {k: hashlib.sha256(v).hexdigest() for k, v in OBJETOS.items()}

#: Las colecciones que el banco tiene: la de leer es mantenida (no se escribe).
MANTENIDAS = {"legal.archivo.contratos"}

SERVE, BYTES = [], []          # (metodo, ruta, cabeceras) que llegaron a cada uno
PERMISOS = {}                  # permiso de leer → (path, usos que le quedan)
ESTADO = {"n": 0}
CONTADOS = {"enviados": 0}
RAMA = {"r": None}             # la rama que la ficha del puesto dice
DOCUMENTOS = {}                # (kind, base, schema, nombre) → YAML
TRANSACCIONES = {}             # t → {coleccion, permiso, items: {path: ref}, cerrada}
LAGO = {}                      # sha256 → bytes: los blobs subidos
PUNTEROS = {}                  # colección → transacción confirmada (un entero)
TRANSFORMS = []                # ("POST", cuerpo) | ("DELETE", None)
MODOS = {"cortar_subidas": 0,  # las próximas N subidas se cortan sin contestar
         "conflictos": 0}      # los próximos N commits pierden la carrera (409)
PUERTOS = {}

# 0049 B9 · la entrada de los casos de `derivar` y el índice de las escritas.
DERIVAR = "conformidad.default.derivar"
ENTRADA = {}                   # camino → bytes: el origen de DERIVAR, que los casos cambian
ESCRITAS = {}                  # colección → {"tx", "items": {camino: ref}, "marcas": {origen: entrada}}


def ref(path, digest=None):
    return {"uri": "ore://legal.archivo.contratos/%s?v=v1" % path, "collection": "legal.archivo.contratos",
            "path": path, "version": "v1", "digest": digest, "size": len(OBJETOS[path]),
            "content_type": "application/pdf", "checksum": "etag:x", "state": "actual", "extra": "se ignora"}


def tipo_por_bytes(b):
    """Lo mismo que `ore_core::medios::tipo_por_bytes`, lo justo para el banco."""
    for magia, tipo in ((b"%PDF", "application/pdf"), (b"\x89PNG\r\n\x1a\n", "image/png"),
                        (b"\xff\xd8\xff", "image/jpeg")):
        if b.startswith(magia):
            return tipo
    return None


def _json(h, codigo, cuerpo, extra=None):
    b = b"" if cuerpo is None else json.dumps(cuerpo).encode()
    h.send_response(codigo)
    if cuerpo is not None:
        h.send_header("content-type", "application/json")
    h.send_header("content-length", str(len(b)))
    for k, v in (extra or {}).items():
        h.send_header(k, v)
    h.end_headers()
    h.wfile.write(b)


def _problema(h, codigo, tipo, detalle=None):
    _json(h, codigo, {"type": tipo, "status": codigo, "detail": detalle or tipo})


def _cuerpo(h):
    n = int(h.headers.get("content-length") or 0)
    t = h.rfile.read(n) if n else b""
    return json.loads(t) if t.strip() else {}


# Quien abrió el puesto, como lo dice `ore-iam` (la 048): lo que crea es suyo.
QUIEN_CREA = "user:ana"


def _doc_de(ruta):
    """`/documentos/{kind}/{b}/{n}` o `/documentos/{kind}/{b}/{s}/{n}` → clave."""
    p = ruta.split("/")[2:]
    if len(p) == 3:
        return (p[0], p[1], "default", p[2])
    if len(p) == 4:
        return tuple(p)
    return None


class Celda(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        SERVE.append(("GET", self.path, dict(self.headers)))
        u = urllib.parse.urlparse(self.path)
        q = dict(urllib.parse.parse_qsl(u.query))
        if u.path == "/puestos/p1":
            return _json(self, 200, {"rama": RAMA["r"]} if RAMA["r"] else {})
        if u.path.startswith("/documentos/"):
            d = DOCUMENTOS.get(_doc_de(u.path))
            return _json(self, 200, {"yaml": d}) if d else _json(self, 404, {"error": "no hay ningún documento"})
        col = ".".join(u.path.split("/")[2:5]) if u.path.startswith("/media/") else None
        if col == DERIVAR or col in ESCRITAS:
            return self._b9(col, u.path.rsplit("/", 1)[1], q)
        if u.path.endswith("/items"):
            todos = [ref("a.pdf"), ref("b.pdf", "sha256:" + SHA["b.pdf"]), ref("cambia.pdf")]
            if q.get("cursor") == "c2":
                return _json(self, 200, {"as_of": "7", "items": todos[2:], "cursor": None})
            return _json(self, 200, {"as_of": "7", "items": todos[:2], "cursor": "c2"})
        if u.path.endswith("/item"):
            return _json(self, 200, dict(ref(q["path"]), current=True))
        if u.path.endswith("/content"):
            path = q["path"]
            ESTADO["n"] += 1
            p = "p%d" % ESTADO["n"]
            # El primer permiso de a.pdf vale para UNA petición: obliga a renovar.
            PERMISOS[p] = (path, 1 if (path == "a.pdf" and ESTADO["n"] == 1) else 10_000)
            digest = {"corrupto.pdf": "sha256:" + "0" * 64}.get(path)
            cuerpo = {"url": "http://127.0.0.1:%d/contenido?permiso=%s" % (PUERTOS["medios"], p),
                      "desde": "medios", "version": q.get("version") or "v1", "ttl_s": 300,
                      "item": ref(path, digest)}
            return _json(self, 307, cuerpo, {"location": cuerpo["url"]})
        _problema(self, 404, "media/no-existe", self.path)

    def do_PUT(self):
        SERVE.append(("PUT", self.path, dict(self.headers)))
        u = urllib.parse.urlparse(self.path)
        if not u.path.startswith("/documentos/"):
            return _problema(self, 404, "media/no-existe", self.path)
        clave = _doc_de(u.path)
        texto = _cuerpo(self).get("yaml", "")
        dueno = re.search(r"^  owner: (\S+)$", texto, re.M)
        if not dueno:
            # Como `PUT /documentos` (0052 · Ownership): sin `owner`, el que ya
            # tenía si se reescribe, y si nace, quien lo crea.
            previo = re.search(r"^  owner: (\S+)$", DOCUMENTOS.get(clave, ""), re.M)
            texto = texto.replace("\nspec:\n", "\nspec:\n  owner: %s\n" % (
                previo.group(1) if previo else QUIEN_CREA), 1)
        if dueno and not re.fullmatch(r"(team|user):[a-z][a-z0-9-]*", dueno.group(1)):
            # Como `ore_core::pertenencia::es_handle` (OOS2009).
            return _json(self, 422, {"error": "no compila", "diagnosticos": [
                {"codigo": "OOS2009", "mensaje": "`owner: %s` no es un handle" % dueno.group(1)}]})
        if "media: nada" in texto or "name: rota" in texto:
            return _json(self, 422, {"error": "no compila", "diagnosticos": [
                {"codigo": "OOS1004", "mensaje": "`media` no es un medio"}]})
        nueva = clave not in DOCUMENTOS
        DOCUMENTOS[clave] = texto
        kind, b, s, n = clave
        # Sólo las de la suite (`conformidad.…`): las demás pruebas del banco
        # cuentan con el `commit` de siempre.
        if kind == "MediaCollection" and b == "conformidad" and "\n  from:" not in texto:
            ESCRITAS.setdefault("%s.%s.%s" % (b, s, n), {"tx": 0, "items": {}, "marcas": {}})
        _json(self, 201 if nueva else 200, {"kind": kind, "namespace": b, "schema": s, "name": n,
                                            "fichero": "packages/%s/%s/collections/%s.yaml" % (b, s, n),
                                            "commit": "c0ffee", "nueva": nueva})

    def do_POST(self):
        SERVE.append(("POST", self.path, dict(self.headers)))
        u = urllib.parse.urlparse(self.path)
        p = u.path.split("/")
        cuerpo = _cuerpo(self)
        if u.path == "/puestos/p1/transform":
            TRANSFORMS.append(("POST", cuerpo))
            return _json(self, 200, {"transform": cuerpo.get("nombre")})
        if len(p) >= 6 and p[1] == "media" and p[5] == "transactions":
            col = ".".join(p[2:5])
            if len(p) == 6:
                return self._abrir(col, cuerpo)
            if len(p) == 8 and p[7] in ("commit", "abort"):
                return self._cerrar(col, p[6], p[7], cuerpo)
        _problema(self, 404, "media/no-existe", self.path)

    def do_DELETE(self):
        SERVE.append(("DELETE", self.path, dict(self.headers)))
        if self.path == "/puestos/p1/transform":
            TRANSFORMS.append(("DELETE", None))
            return _json(self, 200, {"transform": False})
        _problema(self, 404, "media/no-existe", self.path)

    def _abrir(self, col, cuerpo):
        if col in MANTENIDAS:
            return _problema(self, 409, "media/no-escribible", "`%s` se mantiene desde su `from`" % col)
        t = "t-%d" % (len(TRANSACCIONES) + 1)
        permiso = hashlib.sha256(t.encode()).hexdigest()
        TRANSACCIONES[t] = {"coleccion": col, "permiso": permiso, "items": {}, "cerrada": False}
        _json(self, 201, {"transaction": t, "collection": col, "ttl_s": int(cuerpo.get("ttl_s") or 3600),
                          "upload": "http://127.0.0.1:%d/subida?permiso=%s" % (PUERTOS["medios"], permiso),
                          "expires_ms": 0})

    def _cerrar(self, col, t, op, cuerpo=None):
        tx = TRANSACCIONES.get(t)
        if not tx or tx["cerrada"] or tx["coleccion"] != col:
            return _problema(self, 404, "media/transaccion", "no hay ninguna transacción abierta `%s`" % t)
        if op == "abort":
            tx["cerrada"] = True
            return _json(self, 204, None)
        if MODOS["conflictos"] > 0:
            # La forja perdió la carrera: 409 sin `type`, y la transacción sigue.
            MODOS["conflictos"] -= 1
            return _json(self, 409, {"error": "la rama se adelantó mientras se escribía: vuelve a confirmar"})
        if col in ESCRITAS:
            return self._sellar(col, t, tx, cuerpo or {})
        tx["cerrada"] = True
        n = PUNTEROS.get(col, 0) + 1
        PUNTEROS[col] = n
        _json(self, 200, {"transaccion": n, "metadata_location": "gs://lago/%s/%d.json" % (col, n),
                          "items": {"actuales": len(tx["items"]), "retirados": 0, "perdidos": 0},
                          "cambios": {"entran": len(tx["items"]), "cambian": 0, "iguales": 0},
                          "commit": "c0ffee%d" % n, "procedencia": {"puesto": "p1", "transaccion": t}})

    # ── 0049 B9 ──────────────────────────────────────────────────────────────

    def _b9(self, col, op, q):
        if col == DERIVAR:
            refs = {c: ref_de_entrada(c) for c in sorted(ENTRADA)}
            if op == "items":
                return _json(self, 200, {"as_of": "1", "items": list(refs.values()), "cursor": None})
            if op == "item":
                r = refs.get(q.get("path"))
                return _json(self, 200, dict(r, current=True)) if r else _problema(self, 404, "media/no-existe")
            if op == "content":
                c = q.get("path")
                if c not in ENTRADA:
                    return _problema(self, 404, "media/no-existe")
                OBJETOS["derivar:" + c] = ENTRADA[c]
                ESTADO["n"] += 1
                p = "p%d" % ESTADO["n"]
                PERMISOS[p] = ("derivar:" + c, 10_000)
                cuerpo = {"url": "http://127.0.0.1:%d/contenido?permiso=%s" % (PUERTOS["medios"], p),
                          "desde": "medios", "version": refs[c]["version"], "ttl_s": 300, "item": refs[c]}
                return _json(self, 307, cuerpo, {"location": cuerpo["url"]})
            return _problema(self, 404, "media/no-existe", op)
        st = ESCRITAS[col]
        if op == "items":
            return _json(self, 200, {"as_of": str(st["tx"]), "items": [st["items"][c] for c in sorted(st["items"])],
                                     "cursor": None})
        if op == "item":
            r = st["items"].get(q.get("path"))
            return _json(self, 200, dict(r, current=True)) if r else _problema(self, 404, "media/no-existe")
        if op == "derivations":
            return _json(self, 200, {"as_of": str(st["tx"]), "derivations": registro_de(st), "cursor": None})
        return _problema(self, 404, "media/no-existe", op)

    def _sellar(self, col, t, tx, cuerpo):
        """Lo que hace `ore-medios` al confirmar una escrita (B9·2), en pequeño."""
        st = ESCRITAS[col]
        r = sellar(st, tx["items"], cuerpo)
        if r[0] != 200:
            return _problema(self, r[0], r[1], r[2])   # la transacción sigue abierta
        tx["cerrada"] = True
        _json(self, 200, r[1])

    def log_message(self, *a):
        pass


def ref_de_entrada(camino):
    datos = ENTRADA[camino]
    h = hashlib.sha256(datos).hexdigest()
    return {"uri": "ore://%s/%s?v=%s" % (DERIVAR, camino, h[:12]), "collection": DERIVAR, "path": camino,
            "version": h[:12], "digest": "sha256:" + h, "size": len(datos),
            "content_type": tipo_por_bytes(datos) or "application/octet-stream", "checksum": "crc32c:00000000",
            "state": "actual"}


def origen_de(r):
    s = r.get("source") or {}
    return s.get("digest") or s.get("uri")


def registro_de(st):
    por = {}
    for c in sorted(st["items"]):
        r = st["items"][c]
        if r.get("source"):
            e = por.setdefault(origen_de(r), {"source": {k: r["source"][k] for k in ("uri", "digest")},
                                              "derivation": r.get("derivation"), "state": "files", "files": []})
            e["files"].append({"path": c, "anchor": r["source"].get("anchor")})
    for o, m in st["marcas"].items():
        por.setdefault(o, dict(m, files=[]))
    return [por[o] for o in sorted(por)]


def sellar(st, subidos, cuerpo):
    """`(200, respuesta)` o `(status, type, detalle)`."""
    ds = cuerpo.get("derivations") or []
    idos = cuerpo.get("retire_sources") or []
    retirar = cuerpo.get("retire") or []
    mal = lambda m: (422, "media/derivacion", m)  # noqa: E731
    linaje, origenes = {}, set()
    for d in ds:
        src = d.get("source") or {}
        if not src.get("uri") or not (d.get("derivation") or {}).get("key"):
            return mal("`source.uri` y `derivation.key` son obligatorias")
        o = src.get("digest") or src["uri"]
        if o in origenes or o in idos:
            return mal("el origen `%s` está dos veces" % src["uri"])
        origenes.add(o)
        fs = d.get("files") or []
        estado = d.get("state")
        if estado == "files" and not fs:
            return mal("`state: files` sin ficheros")
        if estado in ("empty", "error") and fs:
            return mal("`state: %s` no lleva ficheros" % estado)
        if estado == "error" and not d.get("error"):
            return mal("`state: error` sin `error`")
        if estado not in ("files", "empty", "error"):
            return mal("`state`")
        for f in fs:
            c = f["path"]
            if c not in subidos:
                return mal("`%s` no se subió en esta transacción" % c)
            if c in linaje:
                return mal("`%s` sale de dos orígenes" % c)
            if c in retirar:
                return mal("`%s` está en `files` y en `retire`" % c)
            linaje[c] = (o, {"uri": src["uri"], "digest": src.get("digest"), "anchor": f.get("anchor")},
                         d["derivation"])
    for c in retirar:
        if c in subidos:
            return mal("`%s` se sube y se retira" % c)
        if c not in st["items"]:
            return (404, "media/no-existe", "`%s` no es un ítem actual" % c)
    items, marcas = dict(st["items"]), dict(st["marcas"])
    entran = cambian = iguales = 0
    for c, r in subidos.items():
        nuevo = dict(r)
        nuevo.pop("stored", None)
        if c in linaje:
            nuevo["source"], nuevo["derivation"] = linaje[c][1], linaje[c][2]
        previo = items.get(c)
        if previo and previo.get("source") and c in linaje and origen_de(previo) != linaje[c][0]                 and origen_de(previo) not in origenes and origen_de(previo) not in idos:
            return mal("`%s` sale de otro origen" % c)
        if previo is None:
            entran += 1
        elif previo.get("digest") == nuevo.get("digest"):
            iguales += 1
        else:
            cambian += 1
        items[c] = nuevo
    retirados = marcas_nuevas = 0
    for d in ds:
        o = (d["source"].get("digest") or d["source"]["uri"])
        quedan = {f["path"] for f in d.get("files") or []}
        for c in [c for c, r in st["items"].items() if r.get("source") and origen_de(r) == o
                  and c not in quedan and c not in subidos]:
            items.pop(c, None)
            retirados += 1
        if d["state"] == "files":
            marcas.pop(o, None)
        else:
            m = {"source": {"uri": d["source"]["uri"], "digest": d["source"].get("digest")},
                 "derivation": d["derivation"], "state": d["state"], "error": d.get("error")}
            if marcas.get(o) != m:
                marcas_nuevas += 1
            marcas[o] = m
    for o in idos:
        for c in [c for c, r in st["items"].items() if r.get("source") and origen_de(r) == o and c not in subidos]:
            items.pop(c, None)
            retirados += 1
        marcas.pop(o, None)
    for c in retirar:
        items.pop(c, None)
    if items == st["items"] and marcas == st["marcas"]:
        return (200, {"sin_cambios": True, "transaccion": st["tx"],
                      "metadata_location": "gs://lago/%d.json" % st["tx"]})
    st["tx"] += 1
    st["items"], st["marcas"] = items, marcas
    r = {"transaccion": st["tx"], "metadata_location": "gs://lago/%d.json" % st["tx"],
         "items": {"actuales": len(items)}, "cambios": {"entran": entran, "cambian": cambian, "iguales": iguales},
         "commit": "c0ffee%d" % st["tx"]}
    if ds or idos or retirar:
        r["derivations"] = {"written": len(ds), "files_retired": retirados, "marks": marcas_nuevas}
    return (200, r)


class Medios(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        BYTES.append(("GET", self.path, dict(self.headers)))
        q = dict(urllib.parse.parse_qsl(urllib.parse.urlparse(self.path).query))
        p = q.get("permiso")
        if p not in PERMISOS or PERMISOS[p][1] <= 0:
            return _problema(self, 401, "media/permiso")
        path, usos = PERMISOS[p]
        PERMISOS[p] = (path, usos - 1)
        if path == "cambia.pdf":
            return _problema(self, 412, "media/cambiado")
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

    def do_PUT(self):
        BYTES.append(("PUT", self.path, dict(self.headers)))
        u = urllib.parse.urlparse(self.path)
        q = dict(urllib.parse.parse_qsl(u.query))
        if u.path != "/subida":
            return _problema(self, 404, "media/no-existe", self.path)
        if self.headers.get("content-length") is None:
            return _problema(self, 411, "media/peticion", "una subida dice su `content-length`")
        datos = self.rfile.read(int(self.headers["content-length"]))
        if MODOS["cortar_subidas"] > 0:
            MODOS["cortar_subidas"] -= 1
            self.close_connection = True
            return  # sin contestar: el cliente ve la conexión cortada
        tx = next((t for t in TRANSACCIONES.values() if t["permiso"] == q.get("permiso") and not t["cerrada"]), None)
        if tx is None:
            return _problema(self, 401, "media/permiso")
        sha = hashlib.sha256(datos)
        pedido = self.headers.get("repr-digest")
        if pedido and pedido.split("=:", 1)[1].rstrip(":") != base64.b64encode(sha.digest()).decode():
            return _problema(self, 422, "media/digest-no-casa")
        hexd = sha.hexdigest()
        detectado = tipo_por_bytes(datos[:512])
        declarado = (self.headers.get("content-type") or "").split(";")[0].strip() or None
        guardado = hexd not in LAGO
        LAGO[hexd] = datos
        r = {"uri": "ore://%s/%s?v=%s" % (tx["coleccion"], q["path"], hexd), "collection": tx["coleccion"],
             "path": q["path"], "version": hexd, "digest": "sha256:" + hexd, "size": len(datos),
             "content_type": detectado or declarado or "application/octet-stream",
             "content_type_detected": detectado, "checksum": "crc32c:00000000",
             "transaction": next(k for k, v in TRANSACCIONES.items() if v is tx), "stored": guardado}
        tx["items"][q["path"]] = r
        _json(self, 201, r)

    def log_message(self, *a):
        pass


def arrancar():
    """Levanta los dos servidores y deja el entorno como el de un puesto."""
    celda = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Celda)
    medios = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Medios)
    PUERTOS["celda"], PUERTOS["medios"] = celda.server_port, medios.server_port
    for s in (celda, medios):
        threading.Thread(target=s.serve_forever, daemon=True).start()
    os.environ["ORE_SERVE"] = "http://127.0.0.1:%d" % celda.server_port
    os.environ["PUESTO"] = "p1"
    return celda, medios


# ── lo común de las pruebas ───────────────────────────────────────────────────

fallos = {"n": 0}


def bien(m):
    print("  ✓", m)


def mal(m):
    fallos["n"] += 1
    print("  ✗", m)


def caso(n, f):
    try:
        f()
    except AssertionError as e:
        mal("%s · %s" % (n, e))
    except Exception as e:  # noqa: BLE001
        mal("%s · %s: %s" % (n, type(e).__name__, e))


def cabeceras(h):
    return {k.lower(): v for k, v in h.items()}
