"""
La media en código, desde Python (ADR 0049 B3·5; el contrato, `docs/media.md`).

    c = ore.coleccion("legal.archivo.contratos")
    for item in c.items(prefijo="Nueva carpeta/"):    # el listado, por cursor
        with item.open() as f:                         # fijado a su versión
            cabecera = f.read(5)                       # b"%PDF-"
            f.seek(-1024, 2)                           # el pie, con un rango
    datos = c.stat(path="docs/a.pdf").read_bytes()     # entero, verificado
    for item, datos, error in ore.leer_varios(c.items(), hilos=16):
        ...

**El código nunca habla con el almacén ni con el origen** (D1): pide a la celda
`content` y la celda dice dónde están los bytes —la URL firmada del blob en el
lago (una mantenida) o `ore-medios` con un permiso (una virtual)—. Esa URL se lee
**sin el token de ORE**: no lo necesita, y no se le da.

Lo que B3·0 midió y decide aquí:

- **~125 ms por petición** al origen de una virtual (otra región, otra nube). Un
  `read` pequeño no puede ser una petición: se lee **en flujo** desde donde está el
  cursor, y sólo un `seek` abre otra (un `Range: bytes=n-`). Cerrar a medias corta
  la conexión y no baja el resto.
- **Un flujo, 59 MB/s; ocho rangos, 114 MB/s**: `read_bytes()` de un ítem grande
  baja por rangos en paralelo.
- **Muchos ítems pequeños** (132 ms cada uno): `leer_varios` los baja a la vez.
- **El `sha256` al paso** de una lectura entera: si el ítem lo dice, se coteja al
  final (`MediaCorrupto` si no casa); si no, `Item.sha256_visto` lo da.

Un permiso (o una URL firmada) caduca a los 5 min: el lector pide otro **con la
misma versión** y sigue donde iba. Si esa versión ya no se puede leer,
`MediaCambiado` —nunca bytes de otra—.

**Escribir** (B4b·3), en una colección escrita (`ore.crear_coleccion`):

    with ore.coleccion("legal.archivo.paginas").transaccion() as t:
        t.put("c1/p0.png", png)                  # bytes: con su Repr-Digest
        t.put("c1/original.pdf", "/tmp/c1.pdf")  # una ruta: en flujo
    # al salir: commit (el puntero, con su procedencia); con una excepción: abort

Los bytes van a `upload` —la subida de `ore-medios`, un portador de ESA
transacción— **sin el token de ORE**, como los de `open`. La celda calcula el
sha256 al paso, guarda el blob por su contenido y detecta el tipo por los
bytes: el que se declara sólo vale si los bytes no dicen nada. Dentro de un
transform sólo se escribe su `output`.
"""
import concurrent.futures as _cf
import dataclasses
import hashlib
import http.client
import io
import json
import urllib.error
import urllib.parse
import urllib.request

__all__ = ["coleccion", "Coleccion", "Item", "MediaRef", "leer_varios", "Transaccion", "MediaError",
           "MediaNoExiste", "MediaSinPermiso", "MediaCambiado", "MediaCorrupto", "MediaRango",
           "MediaNoEscribible", "MediaTransaccion"]

#: Lo que se pide de una vez cuando se baja un ítem grande por rangos.
TROZO = 8 << 20
#: A partir de cuánto `read_bytes()` baja por rangos en paralelo.
EN_PARALELO_DESDE = 32 << 20
#: Cuántas veces se pide otro permiso seguido antes de rendirse.
RENOVACIONES = 3
#: Cuántas veces se reintenta una subida cortada, o un commit que perdió la
#: carrera de la forja (B4b·3).
REINTENTOS = 3
#: Por debajo, lo que no se puede rebobinar se guarda en memoria; por encima, a
#: disco, mientras se calcula su sha256.
EN_MEMORIA = 8 << 20


# ── los errores del contrato (`docs/media.md` §3) ─────────────────────────────

class MediaError(Exception):
    """Un error del contrato: `tipo` (`media/…`), `status` y `detalle`."""

    def __init__(self, tipo, status, detalle):
        super().__init__("%s (%s): %s" % (tipo, status, detalle))
        self.tipo, self.status, self.detalle = tipo, status, detalle


class MediaNoExiste(MediaError, LookupError):
    pass


class MediaSinPermiso(MediaError, PermissionError):
    pass


class MediaCambiado(MediaError):
    """La versión fijada ya no se puede leer entera: nunca se dan bytes de otra."""


class MediaCorrupto(MediaError, IOError):
    """Los bytes no casan con `size` o `digest`, o el flujo se cortó a mitad."""


class MediaRango(MediaError, ValueError):
    pass


class MediaNoEscribible(MediaError, PermissionError):
    """La colección no es escrita: la llena su origen (`from`), no el código."""


class MediaTransaccion(MediaError, RuntimeError):
    """La transacción no está abierta: caducó, se cerró, o es de otra colección."""


_POR_TIPO = {
    "media/no-existe": MediaNoExiste,
    "media/sin-permiso": MediaSinPermiso,
    "media/no-declarada": MediaSinPermiso,
    "media/cambiado": MediaCambiado,
    "media/corrupto": MediaCorrupto,
    "media/rango": MediaRango,
    "media/sin-rangos": MediaRango,
    "media/permiso": MediaSinPermiso,
    "media/no-escribible": MediaNoEscribible,
    "media/transaccion": MediaTransaccion,
    "media/digest-no-casa": MediaCorrupto,
}


def _error(status, cuerpo, que):
    cuerpo = cuerpo if isinstance(cuerpo, dict) else {}
    tipo = cuerpo.get("type") or {404: "media/no-existe", 403: "media/sin-permiso",
                                  412: "media/cambiado", 416: "media/rango"}.get(status, "media/origen")
    detalle = cuerpo.get("detail") or cuerpo.get("error") or "%s contestó %s" % (que, status)
    return _POR_TIPO.get(tipo, MediaError)(tipo, status, detalle)


# ── la referencia ─────────────────────────────────────────────────────────────

@dataclasses.dataclass(frozen=True)
class MediaRef:
    """El valor de un `Media<c>` (OOS v1alpha17 `01` §3): dónde está un ítem y qué
    se sabe de él, sin sus bytes. Inmutable; un campo desconocido se ignora."""
    uri: str
    collection: str
    path: str
    version: str
    digest: str = None
    size: int = None
    content_type: str = None
    content_type_detected: str = None
    checksum: str = None
    annotations: dict = None
    modified: str = None
    state: str = None

    @classmethod
    def de_json(cls, d):
        campos = {f.name for f in dataclasses.fields(cls)}
        d = {k: v for k, v in (d or {}).items() if k in campos}
        if d.get("size") is not None:
            d["size"] = int(d["size"])
        return cls(**d)


# ── la colección ──────────────────────────────────────────────────────────────

def coleccion(nombre):
    """`ore.coleccion("base.schema.nombre")` (o `base.nombre`)."""
    return Coleccion(nombre)


class Coleccion:
    def __init__(self, nombre):
        from . import _corto, _partes
        #: El nombre en su forma corta (`base.nombre` en `default`): el que un
        #: transform declara, y el que se anota como leído.
        self.nombre_corto = _corto(nombre, "coleccion(): el nombre")
        self.base, self.schema, self.nombre = _partes(self.nombre_corto)
        self.ruta = "/media/%s/%s/%s" % (self.base, self.schema, self.nombre)
        #: La transacción que el listado leyó (B4·3): dentro de un transform,
        #: la que el servidor fijó al declararlo, aunque la colección cambie.
        self.as_of = None

    def __repr__(self):
        return "Coleccion(%s.%s.%s)" % (self.base, self.schema, self.nombre)

    def _pedir(self, op, consulta, que):
        from . import puesto, _rama_del_puesto, _lee
        # B4·3: leer una colección es leer, como `over()` y `sql()`: dentro de un
        # transform sólo sus `inputs` (PermissionError aquí, antes del 403 del
        # servidor), y fuera queda anotada en lo que la sesión leyó.
        _lee(self.nombre_corto)
        q = urllib.parse.urlencode({k: v for k, v in consulta.items() if v is not None})
        # B3·6: con la rama del puesto, como el resto del SDK (`over`, `sql`):
        # una colección declarada en la rama se ve desde su puesto. Se pregunta
        # una vez por colección (cuesta una petición a la ficha del puesto).
        if not hasattr(self, "_rama"):
            self._rama = _rama_del_puesto()
        codigo, r = puesto.pedir("GET", "%s/%s%s" % (self.ruta, op, "?" + q if q else ""),
                                 seguir=False, plazo=90, cabeceras=self._rama)
        return codigo, r

    def items(self, prefijo=None, estado=None, limite=1000):
        """El listado de una transacción, perezoso, por cursor: `Item`s sin bytes."""
        cursor = None
        while True:
            codigo, r = self._pedir("items", {"prefix": prefijo, "estado": estado,
                                              "limit": limite, "cursor": cursor}, "items")
            if codigo != 200:
                raise _error(codigo, r, "items(%s)" % self)
            if r.get("as_of"):
                self.as_of = r["as_of"]
            for d in r.get("items") or []:
                yield Item(self, MediaRef.de_json(d))
            cursor = r.get("cursor")
            if not cursor:
                return

    def stat(self, path=None, digest=None, version=None):
        """Un ítem, fresco: su `MediaRef` y si es la versión actual (`Item.actual`)."""
        codigo, r = self._pedir("item", {"path": path, "digest": digest, "version": version}, "stat")
        if codigo != 200:
            raise _error(codigo, r, "stat(%s)" % (path or digest))
        it = Item(self, MediaRef.de_json(r))
        it.actual = r.get("current")
        return it

    def _cabeceras(self):
        """La rama del puesto (B3·6), preguntada una vez por colección."""
        from . import _rama_del_puesto
        if not hasattr(self, "_rama"):
            self._rama = _rama_del_puesto()
        return self._rama

    def transaccion(self, ttl_s=3600):
        """**Una transacción para escribir en esta colección** (B4b·3). Como `with`:
        commit al salir, abort con una excepción. A mano: `t.put(…)`, `t.commit()`.
        Dentro de un transform, sólo sobre su `output`."""
        from . import _transform
        if _transform is not None and self.nombre_corto != _transform.output:
            raise PermissionError("`%s` no es el output de `%s` (`%s`): un transform sólo escribe lo que declara"
                                  % (self.nombre_corto, _transform.nombre, _transform.output))
        return Transaccion(self, ttl_s)

    def _donde(self, ref):
        """`content`: a dónde ir por los bytes de `ref`, fijado a su versión."""
        consulta = {"path": ref.path, "version": ref.version} if ref.path else {"digest": ref.digest}
        codigo, r = self._pedir("content", consulta, "content")
        if codigo not in (200, 307) or not (r or {}).get("url"):
            raise _error(codigo, r, "open(%s)" % ref.path)
        return r


# ── el ítem ───────────────────────────────────────────────────────────────────

class Item:
    """Un ítem de una colección: su `ref` y cómo leer sus bytes."""

    def __init__(self, col, ref):
        self.coleccion, self.ref = col, ref
        self.actual = None
        #: El sha256 que una lectura entera calculó, si el ítem no lo traía.
        self.sha256_visto = None

    def __repr__(self):
        return "Item(%s@%s)" % (self.ref.path, self.ref.version)

    def open(self):
        """Un fichero binario de sólo lectura, fijado a la versión del ítem: `read`,
        `seek`, `tell`. Con `with`: cerrar a medias no baja el resto."""
        return io.BufferedReader(_Lector(self), buffer_size=1 << 20)

    def read_bytes(self, hilos=8):
        """Los bytes enteros, verificados. Un ítem grande, por rangos en paralelo."""
        size = self.ref.size
        if not size or size < EN_PARALELO_DESDE or hilos <= 1:
            with self.open() as f:
                return f.read()
        acceso = _Acceso(self)
        tramos = [(a, min(size, a + TROZO) - 1) for a in range(0, size, TROZO)]
        with _cf.ThreadPoolExecutor(hilos) as ex:
            partes = list(ex.map(lambda t: acceso.rango(*t), tramos))
        datos = b"".join(partes)
        _verificar(self, len(datos), hashlib.sha256(datos).hexdigest())
        return datos

    def read_range(self, offset, length):
        """`length` bytes desde `offset` (`read_range` del contrato)."""
        return _Acceso(self).rango(offset, offset + length - 1)


def _verificar(item, leidos, visto):
    ref = item.ref
    if ref.size is not None and leidos != ref.size:
        raise MediaCorrupto("media/corrupto", 502, "%s: %d bytes de %d" % (ref.path, leidos, ref.size))
    if ref.digest and ref.digest.startswith("sha256:") and ref.digest[7:].lower() != visto:
        raise MediaCorrupto("media/corrupto", 502, "%s: sha256 %s, y el ítem dice %s"
                            % (ref.path, visto, ref.digest[7:]))
    if not ref.digest:
        item.sha256_visto = visto


# ── el acceso: dónde leer, y otra vez cuando caduca ───────────────────────────

# Los bytes tampoco siguen solos una redirección (el de `ore/__init__.py`).
from . import _SIN_SEGUIR as _ABRIDOR  # noqa: E402


class _Acceso:
    """La URL vigente de un ítem, fijada a su versión, y pedir otra si caduca."""

    def __init__(self, item):
        self.item = item
        self.url = None

    def vigente(self, otra=False):
        if self.url is None or otra:
            r = self.item.coleccion._donde(self.item.ref)
            self.url = r["url"]
            # La versión fijada: si el ítem no la decía, la de ahora, y ya no cambia.
            if r.get("version") and not self.item.ref.version:
                self.item.ref = dataclasses.replace(self.item.ref, version=r["version"])
            if (r.get("item") or {}).get("size") is not None and self.item.ref.size is None:
                self.item.ref = dataclasses.replace(self.item.ref, size=int(r["item"]["size"]))
        return self.url

    def abrir(self, desde=0, hasta=None):
        """La respuesta en flujo desde `desde` (y hasta `hasta`). Un permiso o una URL
        caducada (401/403) se renueva con la misma versión."""
        rango = None
        if desde or hasta is not None:
            rango = "bytes=%d-%s" % (desde, "" if hasta is None else hasta)
        for intento in range(RENOVACIONES + 1):
            req = urllib.request.Request(self.vigente(otra=intento > 0))
            if rango:
                req.add_header("Range", rango)
            try:
                r = _ABRIDOR.open(req, timeout=60)
                self._aprender(r)
                return r
            except urllib.error.HTTPError as e:
                texto = e.read()
                try:
                    cuerpo = json.loads(texto)
                except ValueError:
                    cuerpo = {}
                if e.code in (401, 403) and intento < RENOVACIONES:
                    continue
                if e.code == 416 and hasta is None:
                    return None  # desde el final: no queda nada
                raise _error(e.code, cuerpo, "leer %s" % self.item.ref.path)
        raise MediaError("media/permiso", 401, "no se pudo renovar el acceso a %s" % self.item.ref.path)

    def _aprender(self, r):
        """El tamaño, de la respuesta, si el ítem no lo decía (B3·6): el total de
        `Content-Range` (`bytes a-b/total`) o, de una lectura entera, `Content-Length`."""
        if self.item.ref.size is not None:
            return
        total = None
        cr = r.headers.get("Content-Range") or ""
        if "/" in cr and cr.rsplit("/", 1)[1].strip().isdigit():
            total = int(cr.rsplit("/", 1)[1])
        elif r.status == 200 and (r.headers.get("Content-Length") or "").isdigit():
            total = int(r.headers["Content-Length"])
        if total is not None:
            self.item.ref = dataclasses.replace(self.item.ref, size=total)

    def tamano(self):
        """El tamaño del ítem; si no se sabe, se pregunta un byte (`bytes=0-0`)."""
        if self.item.ref.size is None:
            r = self.abrir(0, 0)
            if r is not None:
                r.close()
        return self.item.ref.size

    def rango(self, a, z):
        r = self.abrir(a, z)
        if r is None:
            return b""
        with r:
            try:
                datos = r.read()
            except http.client.IncompleteRead as e:
                raise MediaCorrupto("media/corrupto", 502, "%s: el flujo se cortó (%d bytes)"
                                    % (self.item.ref.path, len(e.partial)))
        return datos


class _Lector(io.RawIOBase):
    """El fichero de `Item.open()`: un flujo desde el cursor; un `seek` lo cierra y
    abre otro desde ahí. La lectura entera de principio a fin se verifica."""

    def __init__(self, item):
        super().__init__()
        self.item = item
        self.acceso = _Acceso(item)
        self.acceso.vigente()
        self.pos = 0
        self.flujo = None
        self.pos_flujo = None
        # Se verifica lo que se leyó de una vez desde 0 sin saltos.
        self.hash = hashlib.sha256()
        self.entero = True

    def readable(self):
        return True

    def seekable(self):
        return True

    def tell(self):
        return self.pos

    def _size(self):
        return self.item.ref.size

    def seek(self, offset, whence=io.SEEK_SET):
        if whence == io.SEEK_SET:
            nueva = offset
        elif whence == io.SEEK_CUR:
            nueva = self.pos + offset
        elif whence == io.SEEK_END:
            # Sin tamaño conocido se pregunta (un byte): el final no se adivina.
            if self.acceso.tamano() is None:
                raise MediaRango("media/rango", 416, "el origen no dice el tamaño: no se cuenta desde el final")
            nueva = self._size() + offset
        else:
            raise ValueError("whence")
        if nueva < 0:
            raise ValueError("seek antes del principio")
        if nueva != self.pos:
            self.entero = False
        self.pos = nueva
        return self.pos

    def _cerrar_flujo(self):
        if self.flujo is not None:
            try:
                self.flujo.close()
            except Exception:  # noqa: BLE001 — cerrar a medias corta, y ya
                pass
        self.flujo = None
        self.pos_flujo = None

    def readinto(self, b):
        if self.closed:
            raise ValueError("el fichero está cerrado")
        if self._size() is not None and self.pos >= self._size():
            self._al_final()
            return 0
        if self.flujo is None or self.pos_flujo != self.pos:
            self._cerrar_flujo()
            self.flujo = self.acceso.abrir(self.pos)
            if self.flujo is None:
                return 0
            self.pos_flujo = self.pos
        try:
            n = self.flujo.readinto(b)
        except http.client.IncompleteRead:
            # Menos bytes de los que la respuesta prometió: el flujo se cortó
            # (la celda corta así lo que no casa, B3·1).
            self._cerrar_flujo()
            raise MediaCorrupto("media/corrupto", 502, "%s: el flujo se cortó en %d"
                                % (self.item.ref.path, self.pos))
        if not n:
            self._cerrar_flujo()
            if self._size() is not None and self.pos < self._size():
                raise MediaCorrupto("media/corrupto", 502, "%s: el flujo se cortó en %d de %d"
                                    % (self.item.ref.path, self.pos, self._size()))
            self._al_final()
            return 0
        if self.entero:
            self.hash.update(memoryview(b)[:n])
        self.pos += n
        self.pos_flujo = self.pos
        return n

    def _al_final(self):
        if self.entero:
            self.entero = False
            _verificar(self.item, self.pos, self.hash.hexdigest())

    def close(self):
        self._cerrar_flujo()
        super().close()


# ── muchos a la vez ───────────────────────────────────────────────────────────

def leer_varios(items, hilos=16):
    """Los bytes de muchos ítems, `hilos` a la vez (cada petición al origen de una
    virtual cuesta ~125 ms: de uno en uno, 8 por segundo; con 16, unos 120).
    Da `(item, datos, None)` o `(item, None, error)` según terminan: el error de
    uno no para los demás. `items` puede ser perezoso (`c.items()`)."""
    items = iter(items)
    with _cf.ThreadPoolExecutor(hilos) as ex:
        vivos = {}

        def lanzar():
            for it in items:
                vivos[ex.submit(it.read_bytes, 1)] = it
                if len(vivos) >= hilos * 2:
                    return

        lanzar()
        while vivos:
            hecho, _ = _cf.wait(list(vivos), return_when=_cf.FIRST_COMPLETED)
            for f in hecho:
                it = vivos.pop(f)
                try:
                    yield it, f.result(), None
                except Exception as e:  # noqa: BLE001 — el error es un valor
                    yield it, None, e
            lanzar()


# ── escribir: la transacción (B4b·3) ──────────────────────────────────────────

def _fuente(datos):
    """`datos` → (un fichero rebobinable en su inicio, su largo, su sha256, cerrar?).
    Bytes, una ruta, o un fichero; uno que no se puede rebobinar se copia antes
    (a memoria o a disco), porque la subida dice su largo y se reintenta."""
    import os
    import tempfile
    if isinstance(datos, (bytes, bytearray, memoryview)):
        b = bytes(datos)
        return io.BytesIO(b), len(b), hashlib.sha256(b).digest(), True
    if isinstance(datos, (str, os.PathLike)):
        f = open(datos, "rb")
        return (f,) + _medir(f) + (True,)
    if hasattr(datos, "read"):
        rebobinable = getattr(datos, "seekable", lambda: False)()
        if rebobinable:
            return (datos,) + _medir(datos) + (False,)
        copia = tempfile.SpooledTemporaryFile(max_size=EN_MEMORIA)
        while True:
            trozo = datos.read(1 << 20)
            if not trozo:
                break
            copia.write(trozo)
        copia.seek(0)
        return (copia,) + _medir(copia) + (True,)
    raise TypeError("put(): `datos` son bytes, una ruta o un fichero, no %s" % type(datos).__name__)


def _medir(f):
    """El largo y el sha256 desde donde está `f`, que vuelve a donde estaba."""
    desde = f.tell()
    h, n = hashlib.sha256(), 0
    while True:
        trozo = f.read(1 << 20)
        if not trozo:
            break
        h.update(trozo)
        n += len(trozo)
    f.seek(desde)
    return n, h.digest()


class Transaccion:
    """**Una transacción abierta** sobre una colección escrita (B4b·3): `put` sube
    ítems, `commit` los deja escritos —y el puntero, con su procedencia—, `abort`
    no deja nada. `upload` es un portador: no se enseña."""

    def __init__(self, col, ttl_s=3600):
        from . import puesto
        self.coleccion = col
        self.subidos = []
        self.resultado = None
        self.cerrada = False
        codigo, r = puesto.pedir("POST", col.ruta + "/transactions", {"ttl_s": ttl_s},
                                 plazo=60, cabeceras=col._cabeceras())
        if codigo != 201:
            raise _error(codigo, r, "transaccion(%s)" % col)
        self.id = r["transaction"]
        self._upload = r["upload"]

    def __repr__(self):
        return "Transaccion(%s, %s%s)" % (self.coleccion, self.id, ", cerrada" if self.cerrada else "")

    def __enter__(self):
        return self

    def __exit__(self, tipo, valor, traza):
        if self.cerrada:
            return False
        if tipo is None:
            self.commit()
        else:
            try:
                self.abort()
            except Exception:  # noqa: BLE001 — la excepción de dentro es la que importa
                pass
        return False

    def _abierta(self, que):
        if self.cerrada:
            raise MediaTransaccion("media/transaccion", 409, "%s: la transacción %s ya se cerró" % (que, self.id))

    def put(self, path, datos, tipo=None):
        """Sube un ítem a `path` (relativo a la colección). `datos`: bytes, una ruta o
        un fichero. `tipo`, el declarado: vale si los bytes no dicen nada. Devuelve su
        `MediaRef` (el digest y el tipo que la celda vio)."""
        self._abierta("put(%s)" % path)
        f, largo, sha, cerrar = _fuente(datos)
        desde = f.tell()
        try:
            return self._subir(path, f, desde, largo, sha, tipo)
        finally:
            if cerrar:
                f.close()

    def _subir(self, path, f, desde, largo, sha, tipo):
        import base64
        u = urllib.parse.urlsplit(self._upload)
        destino = "%s?%s&path=%s" % (u.path, u.query, urllib.parse.quote(path, safe="/"))
        cabeceras = {"content-length": str(largo),
                     "repr-digest": "sha-256=:%s:" % base64.b64encode(sha).decode()}
        if tipo:
            cabeceras["content-type"] = tipo
        ultimo = None
        for intento in range(REINTENTOS + 1):
            f.seek(desde)
            # Sin el token de ORE: `upload` ya es el permiso, como la URL de `open`.
            c = (http.client.HTTPSConnection if u.scheme == "https" else http.client.HTTPConnection)(
                u.hostname, u.port, timeout=300)
            try:
                c.request("PUT", destino, body=_Trozo(f, largo), headers=cabeceras)
                r = c.getresponse()
                texto = r.read()
            except (OSError, http.client.HTTPException) as e:
                # Cortada: se vuelve a subir. Es idempotente —el mismo contenido al
                # mismo camino es la misma fila, y el blob ya está si llegó—.
                ultimo = e
                continue
            finally:
                c.close()
            try:
                cuerpo = json.loads(texto) if texto.strip() else {}
            except ValueError:
                cuerpo = {}
            if r.status != 201:
                raise _error(r.status, cuerpo, "put(%s)" % path)
            ref = MediaRef.de_json(cuerpo)
            self.subidos.append(ref)
            return ref
        raise MediaError("media/origen", 503, "put(%s): la subida se cortó %d veces: %s"
                         % (path, REINTENTOS + 1, ultimo))

    def put_varios(self, pares, hilos=8):
        """Muchos `put` a la vez: `pares` da `(path, datos)` o `(path, datos, tipo)`, y
        esto da `(path, ref, error)` según acaban —el error de uno es un valor y no
        para a los demás, como en `leer_varios`—. Hay `2 × hilos` en vuelo como mucho:
        lo que `pares` produce no se lee entero de antemano."""
        self._abierta("put_varios")
        pares = iter(pares)
        with _cf.ThreadPoolExecutor(max_workers=hilos) as ex:
            vuelo = {}

            def lanzar():
                for par in pares:
                    path, datos, *tipo = par
                    vuelo[ex.submit(self.put, path, datos, *tipo)] = path
                    if len(vuelo) >= 2 * hilos:
                        return

            lanzar()
            while vuelo:
                hecho, _ = _cf.wait(vuelo, return_when=_cf.FIRST_COMPLETED)
                for fut in hecho:
                    path = vuelo.pop(fut)
                    try:
                        yield path, fut.result(), None
                    except Exception as e:  # noqa: BLE001 — el de uno es un valor
                        yield path, None, e
                lanzar()

    def commit(self):
        """Deja escrito lo subido —el puntero, con su procedencia— y la cierra. Si otro
        confirmó a la vez (la forja: 409 sin `type`), vuelve a confirmar sobre lo nuevo.
        Devuelve `{transaccion, items, cambios, commit, metadata_location, …}`."""
        import time
        self._abierta("commit")
        espera = 0.5
        for intento in range(REINTENTOS + 1):
            codigo, r = self._cerrar("commit")
            if codigo == 200:
                self.cerrada, self.resultado = True, r or {}
                return self.resultado
            carrera = codigo == 409 and not (r or {}).get("type")
            if not carrera or intento == REINTENTOS:
                raise _error(codigo, r, "commit(%s)" % self.id)
            time.sleep(espera)
            espera *= 2

    def abort(self):
        """No deja nada de lo subido, y la cierra."""
        if self.cerrada:
            return
        codigo, r = self._cerrar("abort")
        self.cerrada = True
        if codigo not in (204, 404):
            raise _error(codigo, r, "abort(%s)" % self.id)

    def _cerrar(self, op):
        from . import puesto
        return puesto.pedir("POST", "%s/transactions/%s/%s" % (self.coleccion.ruta, self.id, op), {},
                            plazo=300, cabeceras=self.coleccion._cabeceras())


class _Trozo:
    """Lo que `http.client` lee como cuerpo: `largo` bytes de `f`, ni uno más."""

    def __init__(self, f, largo):
        self.f, self.queda = f, largo

    def read(self, n=-1):
        if self.queda <= 0:
            return b""
        n = self.queda if n is None or n < 0 else min(n, self.queda)
        b = self.f.read(n)
        self.queda -= len(b)
        return b
