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

__all__ = ["coleccion", "Coleccion", "Item", "MediaRef", "leer_varios", "MediaError",
           "MediaNoExiste", "MediaSinPermiso", "MediaCambiado", "MediaCorrupto", "MediaRango"]

#: Lo que se pide de una vez cuando se baja un ítem grande por rangos.
TROZO = 8 << 20
#: A partir de cuánto `read_bytes()` baja por rangos en paralelo.
EN_PARALELO_DESDE = 32 << 20
#: Cuántas veces se pide otro permiso seguido antes de rendirse.
RENOVACIONES = 3


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


_POR_TIPO = {
    "media/no-existe": MediaNoExiste,
    "media/sin-permiso": MediaSinPermiso,
    "media/no-declarada": MediaSinPermiso,
    "media/cambiado": MediaCambiado,
    "media/corrupto": MediaCorrupto,
    "media/rango": MediaRango,
    "media/sin-rangos": MediaRango,
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
        self.base, self.schema, self.nombre = _partes(_corto(nombre, "coleccion(): el nombre"))
        self.ruta = "/media/%s/%s/%s" % (self.base, self.schema, self.nombre)

    def __repr__(self):
        return "Coleccion(%s.%s.%s)" % (self.base, self.schema, self.nombre)

    def _pedir(self, op, consulta, que):
        from . import puesto, _rama_del_puesto
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
