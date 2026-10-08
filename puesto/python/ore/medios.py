"""
Media in code, from Python (ADR 0049 B3·5; the contract, `docs/media.md`).

    c = ore.collection("legal.archive.contracts")
    for item in c.items(prefix="New folder/"):        # the listing, by cursor
        with item.open() as f:                         # pinned to its version
            head = f.read(5)                           # b"%PDF-"
            f.seek(-1024, 2)                           # the tail, with a range
    data = c.stat(path="docs/a.pdf").read_bytes()      # whole, verified
    for item, data, error in ore.read_many(c.items(), threads=16):
        ...

    with ore.collection("legal.archive.pages").transaction() as t:
        t.put("c1/p0.png", png)                        # bytes, or a path, or a file
    # on exit: commit; on an exception: abort

    ore.collection("legal.archive.contracts").apply(fn, output="legal.archive.texts")

The code never talks to the store or the origin: it asks the cell where the
bytes are, and reads that URL without ORE's token. Errors are `MediaError`
subclasses (`MediaNotFound`, `MediaForbidden`, `MediaChanged`, `MediaCorrupt`,
`MediaRangeError`, `MediaNotWritable`, `MediaTransactionError`); the old Spanish
names are aliases of the same classes.

---

La media en código, desde Python (ADR 0049 B3·5; el contrato, `docs/media.md`).

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

from . import _Alias, _Result, _avisar, _en, _kw  # noqa: E402 — los alias (S1)

__all__ = ["collection", "Collection", "Item", "MediaRef", "read_many", "Transaction", "MediaError",
           "MediaNotFound", "MediaForbidden", "MediaChanged", "MediaCorrupt", "MediaRangeError",
           "MediaNotWritable", "MediaTransactionError"]

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
    """A media contract error: `type` (`media/…`, the wire code), `status` and
    `detail`."""

    #: Alias de antes.
    tipo = _Alias("type")
    detalle = _Alias("detail")

    def __init__(self, type, status, detail):
        super().__init__("%s (%s): %s" % (type, status, detail))
        self.type, self.status, self.detail = type, status, detail


class MediaNotFound(MediaError, LookupError):
    """The collection or the item does not exist (`media/no-existe`)."""


class MediaForbidden(MediaError, PermissionError):
    """Not allowed to read it, or the collection is not declared (`media/sin-permiso`)."""


class MediaChanged(MediaError):
    """The pinned version can no longer be read whole: bytes of another
    version are never returned (`media/cambiado`)."""


class MediaCorrupt(MediaError, IOError):
    """The bytes do not match `size` or `digest`, or the stream was cut
    (`media/corrupto`)."""


class MediaRangeError(MediaError, ValueError):
    """A range that cannot be served (`media/rango`)."""


class MediaNotWritable(MediaError, PermissionError):
    """The collection is not a written one: its origin (`from`) fills it, not
    code (`media/no-escribible`)."""


class MediaTransactionError(MediaError, RuntimeError):
    """The transaction is not open: it expired, was closed, or belongs to
    another collection (`media/transaccion`)."""


#: Los nombres de antes: las MISMAS clases (`except ore.MediaNoExiste` caza un
#: `MediaNotFound`).
_ALIAS = {"MediaNoExiste": "MediaNotFound", "MediaSinPermiso": "MediaForbidden", "MediaCambiado": "MediaChanged",
          "MediaCorrupto": "MediaCorrupt", "MediaRango": "MediaRangeError", "MediaNoEscribible": "MediaNotWritable",
          "MediaTransaccion": "MediaTransactionError", "coleccion": "collection", "Coleccion": "Collection",
          "leer_varios": "read_many", "Transaccion": "Transaction"}


def __getattr__(nombre):
    nuevo = _ALIAS.get(nombre)
    if nuevo is None:
        raise AttributeError("module 'ore.medios' has no attribute %r" % (nombre,))
    _avisar("ore.medios.%s" % nombre, "ore.medios.%s" % nuevo)
    return globals()[nuevo]


_POR_TIPO = {
    "media/no-existe": MediaNotFound,
    "media/sin-permiso": MediaForbidden,
    "media/no-declarada": MediaForbidden,
    "media/cambiado": MediaChanged,
    "media/corrupto": MediaCorrupt,
    "media/rango": MediaRangeError,
    "media/sin-rangos": MediaRangeError,
    "media/permiso": MediaForbidden,
    "media/no-escribible": MediaNotWritable,
    "media/transaccion": MediaTransactionError,
    "media/digest-no-casa": MediaCorrupt,
}


def _error(status, cuerpo, que):
    cuerpo = cuerpo if isinstance(cuerpo, dict) else {}
    tipo = cuerpo.get("type") or {404: "media/no-existe", 403: "media/sin-permiso",
                                  412: "media/cambiado", 416: "media/rango"}.get(status, "media/origen")
    detalle = cuerpo.get("detail") or cuerpo.get("error") or "%s answered %s" % (que, status)
    return _POR_TIPO.get(tipo, MediaError)(tipo, status, detalle)


# ── la referencia ─────────────────────────────────────────────────────────────

@dataclasses.dataclass(frozen=True)
class MediaRef:
    """The value of a `Media<c>` (OOS v1alpha17 `01` §3): where an item is and
    what is known about it, without its bytes. Immutable; unknown fields are
    ignored."""
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
    #: La transacción de la colección en que entró (el listado, v1alpha17 `04` §1).
    transaction: str = None
    #: 0049 B9 · De qué ítem sale este fichero (`uri`, `digest`, `anchor`), si
    #: lo escribió `apply()`; y con qué se calculó (`key`, `fn`, `fn_version`, …).
    source: dict = None
    derivation: dict = None

    #: Alias de antes.
    de_json = _Alias("from_json")

    @classmethod
    def from_json(cls, d):
        """A `MediaRef` from its JSON (a dict); unknown fields are ignored."""
        campos = {f.name for f in dataclasses.fields(cls)}
        d = {k: v for k, v in (d or {}).items() if k in campos}
        if d.get("size") is not None:
            d["size"] = int(d["size"])
        return cls(**d)


# ── la colección ──────────────────────────────────────────────────────────────

@_kw({"nombre": "name"})
def collection(name):
    """`ore.collection("db.schema.name")` (or `db.name`): a media collection."""
    return Collection(name)


class Collection:
    """A media collection: `items()`, `stat()`, `apply()` and, if it is a
    written one, `transaction()`."""

    #: Alias de antes.
    nombre_corto = _Alias("short_name")
    aplicar = _Alias("apply")
    transaccion = _Alias("transaction")

    def __init__(self, name):
        from . import _corto, _partes
        #: El nombre en su forma corta (`base.nombre` en `default`): el que un
        #: transform declara, y el que se anota como leído.
        self.short_name = _corto(name, "collection(): the name")
        self.base, self.schema, self.nombre = _partes(self.short_name)
        self.ruta = "/media/%s/%s/%s" % (self.base, self.schema, self.nombre)
        #: La transacción que el listado leyó (B4·3): dentro de un transform,
        #: la que el servidor fijó al declararlo, aunque la colección cambie.
        self.as_of = None

    def __repr__(self):
        return "Collection(%s.%s.%s)" % (self.base, self.schema, self.nombre)

    def _pedir(self, op, consulta, que):
        from . import session, _rama_del_puesto, _lee
        # B4·3: leer una colección es leer, como `over()` y `sql()`: dentro de un
        # transform sólo sus `inputs` (PermissionError aquí, antes del 403 del
        # servidor), y fuera queda anotada en lo que la sesión leyó.
        _lee(self.short_name)
        q = urllib.parse.urlencode({k: v for k, v in consulta.items() if v is not None})
        # B3·6: con la rama del puesto, como el resto del SDK (`over`, `sql`):
        # una colección declarada en la rama se ve desde su puesto. Se pregunta
        # una vez por colección (cuesta una petición a la ficha del puesto).
        if not hasattr(self, "_rama"):
            self._rama = _rama_del_puesto()
        codigo, r = session.pedir("GET", "%s/%s%s" % (self.ruta, op, "?" + q if q else ""),
                                 seguir=False, plazo=90, cabeceras=self._rama)
        return codigo, r

    @_kw({"prefijo": "prefix", "estado": "state", "limite": "limit"})
    def items(self, prefix=None, state=None, limit=1000):
        """The listing of a transaction, lazy, by cursor: `Item`s without bytes.
        `prefix` filters by path, `state` by item state, `limit` is the page size."""
        cursor = None
        while True:
            codigo, r = self._pedir("items", {"prefix": prefix, "estado": state,
                                              "limit": limit, "cursor": cursor}, "items")
            if codigo != 200:
                raise _error(codigo, r, "items(%s)" % self)
            if r.get("as_of"):
                self.as_of = r["as_of"]
            for d in r.get("items") or []:
                yield Item(self, MediaRef.from_json(d))
            cursor = r.get("cursor")
            if not cursor:
                return

    def stat(self, path=None, digest=None, version=None):
        """One item, fresh, by `path`, `digest` or `version`: its `MediaRef` and
        whether it is the current version (`Item.current`)."""
        codigo, r = self._pedir("item", {"path": path, "digest": digest, "version": version}, "stat")
        if codigo != 200:
            raise _error(codigo, r, "stat(%s)" % (path or digest))
        it = Item(self, MediaRef.from_json(r))
        it.current = r.get("current")
        return it

    def _cabeceras(self):
        """La rama del puesto (B3·6), preguntada una vez por colección."""
        from . import _rama_del_puesto
        if not hasattr(self, "_rama"):
            self._rama = _rama_del_puesto()
        return self._rama

    @_kw({"salida": "output", "reintentar_errores": "retry_errors", "hilos": "threads",
          "guardar_cada_s": "save_every_s"})
    def apply(self, fn, version=None, params=None, output=None, retry_errors=False,
              threads=4, save_every_s=300):
        """**Incremental derivation** (0049 B5, D5): `fn(item)` on each item that
        needs it, and the result as an **anchored table** (v1alpha17 `03`) in
        `output` —inside a transform, its `output`—.

        **Files from files** (0049 B9): if `output` is a written collection
        (`ore.collection(…)` or the name of one), `fn(item)` returns (or yields)
        `ore.File(name, data, content_type=None, anchor=None)`s instead of rows,
        and each one is written to `<item path>/<name>` with where it comes from
        (`source`) and how (`derivation`). Same incremental rules, item by item:
        an item whose key did not change is skipped; one that changed replaces
        its files (and those it no longer gives are retired); an item that is
        gone takes its files with it; an item with no files, or that fails,
        leaves a mark so it is not recomputed. Returns `{items, new, recomputed,
        skipped, errors, removed, files_written, files_retired, written}`.

        `fn(item)` returns (or yields) rows: dicts with the payload columns and,
        if the row is a part of the item, `anchor` (`{"kind": "page", "page": 3}`,
        v1alpha17 `02`; `anchor_parent` for its parent). With no rows, the item
        is done with one row of `kind: item`. An exception is a result: a row
        with `_status.state: error`, and the others go on.

        What is there is not recomputed: each item's key (`_derivation.key`) is
        its identity —the `digest`, or `(collection, path, version)` of an
        unread virtual one—, `fn`, `version` (without it, the hash of `fn`'s
        code) and `params`. If it does not change its rows stay (with today's
        path: moving it does not recompute); if it changes they are redone;
        those of an item that is gone are removed. Errors are retried with
        `retry_errors=True`. `threads` items are computed at once.

        It saves every `save_every_s` seconds and at the end; with nothing to
        do, nothing is written. Returns the summary: `{items, new, recomputed,
        skipped, errors, removed, rows, written}`."""
        destino = _salida_de(output)
        if _es_escrita(destino):
            return _aplicar_ficheros(self, fn, version, params, destino, retry_errors, threads, save_every_s)
        return _aplicar(self, fn, version, params, output, retry_errors, threads, save_every_s)

    def derivations(self):
        """**The register** of a collection written by `apply()` (0049 B9): one
        entry per source item —`{source, derivation, state, files, error}`, with
        `state` `files`, `empty` or `error`—, lazily, by cursor."""
        cursor = None
        while True:
            codigo, r = self._pedir("derivations", {"cursor": cursor}, "derivations")
            if codigo != 200:
                raise _error(codigo, r, "derivations(%s)" % self)
            for d in r.get("derivations") or []:
                yield d
            cursor = r.get("cursor")
            if not cursor:
                return

    def transaction(self, ttl_s=3600):
        """**A transaction to write into this collection** (B4b·3). As a `with`:
        commit on exit, abort on an exception. By hand: `t.put(…)`, `t.commit()`.
        Inside a transform, only on its `output`."""
        from . import _transform
        if _transform is not None and self.short_name != _transform.output:
            raise PermissionError("`%s` is not the output of `%s` (`%s`): a transform only writes what it declares"
                                  % (self.short_name, _transform.nombre, _transform.output))
        return Transaction(self, ttl_s)

    def _donde(self, ref):
        """`content`: a dónde ir por los bytes de `ref`, fijado a su versión."""
        consulta = {"path": ref.path, "version": ref.version} if ref.path else {"digest": ref.digest}
        codigo, r = self._pedir("content", consulta, "content")
        if codigo not in (200, 307) or not (r or {}).get("url"):
            raise _error(codigo, r, "open(%s)" % ref.path)
        return r


# ── el ítem ───────────────────────────────────────────────────────────────────

class Item:
    """An item of a collection: its `ref` (a `MediaRef`) and how to read its
    bytes (`open`, `read_bytes`, `read_range`)."""

    #: Alias de antes.
    sha256_visto = _Alias("sha256_seen")
    coleccion = _Alias("collection")
    actual = _Alias("current")

    def __init__(self, col, ref):
        self.collection, self.ref = col, ref
        self.current = None
        #: El sha256 que una lectura entera calculó, si el ítem no lo traía.
        self.sha256_seen = None

    def __repr__(self):
        return "Item(%s@%s)" % (self.ref.path, self.ref.version)

    def open(self):
        """A read-only binary file pinned to the item's version: `read`, `seek`,
        `tell`. Use it with `with`: closing halfway does not download the rest."""
        return io.BufferedReader(_Lector(self), buffer_size=1 << 20)

    @_kw({"hilos": "threads"})
    def read_bytes(self, threads=8):
        """All the bytes, verified. A large item is read by ranges, `threads` at
        once."""
        hilos = threads
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
        """`length` bytes from `offset` (the contract's `read_range`)."""
        return _Acceso(self).rango(offset, offset + length - 1)


def _verificar(item, leidos, visto):
    ref = item.ref
    if ref.size is not None and leidos != ref.size:
        raise MediaCorrupt("media/corrupto", 502, "%s: %d bytes of %d" % (ref.path, leidos, ref.size))
    if ref.digest and ref.digest.startswith("sha256:") and ref.digest[7:].lower() != visto:
        raise MediaCorrupt("media/corrupto", 502, "%s: sha256 %s, and the item says %s"
                            % (ref.path, visto, ref.digest[7:]))
    if not ref.digest:
        item.sha256_seen = visto


# ── el acceso: dónde leer, y otra vez cuando caduca ───────────────────────────

# Los bytes tampoco siguen solos una redirección (el de `ore/__init__.py`).
from . import _SIN_SEGUIR as _ABRIDOR  # noqa: E402


# ── 0049 B5 · la derivación incremental ─────────────────────────────────────
#
# La tabla anclada ES el registro: `_derivation.key` dice con qué se calculó
# cada fila y `_status`, si salió. No hay otra tabla que mantener a la par.

#: Los campos de `MediaRef` que van en `_item` (sin `annotations`: un struct
#: abierto, que el lago no puede tipar si viene vacío).
_CAMPOS_ITEM = ("uri", "collection", "path", "version", "digest", "size",
                "content_type", "content_type_detected", "checksum")
#: Los campos de `Anchor` (v1alpha17 `02` §2): los que no son de su clase, nulos.
_CAMPOS_ANCLA = ("kind", "page", "bbox", "polygon", "space", "t_start", "t_end", "frame",
                 "char_start", "char_end", "text_of", "offset", "length")
_SISTEMA = ("_item", "_anchor", "_anchor_id", "_anchor_parent", "_derivation", "_status")


def _esquema_de_sistema():
    import pyarrow as pa
    S, F, I = pa.string(), pa.float64(), pa.int64()
    item = pa.struct([(k, I if k == "size" else S) for k in _CAMPOS_ITEM])
    punto = pa.struct([("x", F), ("y", F)])
    ancla = pa.struct([("kind", S), ("page", I), ("bbox", pa.struct([("x", F), ("y", F), ("w", F), ("h", F)])),
                       ("polygon", pa.list_(punto)),
                       ("space", pa.struct([("unit", S), ("width", F), ("height", F)])),
                       ("t_start", F), ("t_end", F), ("frame", I), ("char_start", I), ("char_end", I),
                       ("text_of", S), ("offset", I), ("length", I)])
    deriv = pa.struct([("key", S), ("fn", S), ("fn_version", S), ("model", S), ("model_rev", S),
                       ("params_hash", S), ("run", S), ("created", pa.timestamp("us", tz="UTC"))])
    estado = pa.struct([("state", S), ("error_type", S), ("error_message", S), ("attempts", I)])
    return [("_item", item), ("_anchor", ancla), ("_anchor_id", S), ("_anchor_parent", S),
            ("_derivation", deriv), ("_status", estado)]


#: A collection read in SQL `FROM` (0049 B7·1): one row per item, with the
#: columns of its listing (OOS v1alpha17 `04` §1; `COLUMNAS_DE_LISTADO` in ore-core).
COLUMNAS_DE_LA_RELACION = ("_item", "path", "version", "digest", "size", "content_type",
                           "content_type_detected", "checksum", "modified", "transaction")


def _relacion(col):
    """The collection as a relation for `sql()`: one row per item of its
    listing (no bytes are read). `_item` is the `MediaRef` as a struct —the same
    type as `_item` in an anchored table—, for a function to take."""
    return _relacion_de_refs(it.ref for it in col.items())


def _instante(v):
    """`modified` como instante (`DateTimeTz`), o nulo si no se entiende."""
    import datetime
    if not v:
        return None
    try:
        t = datetime.datetime.fromisoformat(str(v).replace("Z", "+00:00"))
    except ValueError:
        return None
    return t if t.tzinfo else t.replace(tzinfo=datetime.timezone.utc)


def _relacion_de_refs(refs):
    """`_relacion` of these `MediaRef`s: one row each (B7·3, one item alone),
    with the columns of the listing (v1alpha17 `04` §1)."""
    import pyarrow as pa
    tipo_item = dict(_esquema_de_sistema())["_item"]
    S = pa.string()
    filas = []
    for r in refs:
        filas.append({"_item": {c: getattr(r, c, None) for c in _CAMPOS_ITEM}, "path": r.path,
                      "version": r.version, "digest": r.digest, "size": r.size,
                      "content_type": r.content_type, "content_type_detected": r.content_type_detected,
                      "checksum": r.checksum, "modified": _instante(r.modified),
                      "transaction": getattr(r, "transaction", None)})
    esquema = pa.schema([("_item", tipo_item), ("path", S), ("version", S), ("digest", S),
                         ("size", pa.int64()), ("content_type", S), ("content_type_detected", S),
                         ("checksum", S), ("modified", pa.timestamp("us", tz="UTC")), ("transaction", S)])
    return pa.Table.from_pylist(filas, schema=esquema)


def _canonico(x):
    return json.dumps(x, sort_keys=True, separators=(",", ":"), ensure_ascii=False, default=str)


def _sha(*partes):
    return hashlib.sha256("\x1f".join("" if p is None else str(p) for p in partes).encode("utf-8")).hexdigest()


def _identidad(ref):
    """v1alpha17 `01` §3.1: el `digest`; sin él, el localizador fijado."""
    return ref.digest or "%s|%s|%s" % (ref.collection, ref.path, ref.version)


def _version_de(fn):
    """La versión de una función que no la dice: la de su código. Una de
    `get_function` trae la de su fichero entero (ORE 0056 V1)."""
    marcada = getattr(fn, "__ore_codigo__", None)
    if marcada:
        return marcada
    import inspect
    try:
        fuente = inspect.getsource(fn)
    except (OSError, TypeError):
        codigo = getattr(fn, "__code__", None)
        fuente = codigo.co_code.hex() if codigo is not None else repr(fn)
    return "codigo:" + hashlib.sha256(fuente.encode("utf-8")).hexdigest()[:12]


def _ancla_de(a):
    a = dict(a or {"kind": "item"})
    if not a.get("kind"):
        raise ValueError("apply(): an `anchor` without `kind` (v1alpha17 `02`): %r" % (a,))
    otros = set(a) - set(_CAMPOS_ANCLA)
    if otros:
        raise ValueError("apply(): `anchor` with fields that are not `Anchor`'s: %s" % ", ".join(sorted(otros)))
    return {k: a.get(k) for k in _CAMPOS_ANCLA}


def _identidades_de_filas(filas):
    """The identities of the items an anchored table has rows of (`_item`):
    `removed` counts items that are gone, not keys —a new `version` changes
    every key and removes no item—."""
    out = set()
    for f in filas:
        i = f.get("_item") or {}
        out.add(i.get("digest") or "%s|%s|%s" % (i.get("collection"), i.get("path"), i.get("version")))
    return out


def _aplicar(col, fn, version, params, salida, reintentar_errores, hilos, guardar_cada_s):
    import datetime
    import time
    import uuid
    import pyarrow as pa
    from . import _transform, _corto, _nombre_de
    import ore

    if salida is None:
        if _transform is None:
            raise ValueError("apply(): outside a transform, give the `output` (`db.schema.t`)")
        salida = _transform.output
    salida = _corto(_nombre_de(salida), "apply(): the `output`")
    nombre_fn = getattr(fn, "__name__", None) or type(fn).__name__
    fn_version = str(version) if version is not None else _version_de(fn)
    params_hash = None if params is None else hashlib.sha256(_canonico(params).encode("utf-8")).hexdigest()
    run = uuid.uuid4().hex
    sistema = _esquema_de_sistema()

    # Lo de hoy: un ítem por identidad (dos rutas con el mismo contenido son el
    # mismo ítem: se calcula una vez). Cuál de sus rutas, se decide abajo.
    rutas_de = {}
    for it in col.items():
        k = _sha(_identidad(it.ref), nombre_fn, fn_version, None, params_hash)
        rutas_de.setdefault(k, []).append(it)

    # Lo que ya está: las filas de la salida, por su clave.
    previas = {}
    try:
        filas_previas = ore.over(salida, "arrow").to_pylist()
    except LookupError:
        filas_previas = []
    for f in filas_previas:
        previas.setdefault(((f.get("_derivation") or {}).get("key")), []).append(f)
    rutas_previas = {((f.get("_item") or {}).get("collection"), (f.get("_item") or {}).get("path"))
                     for f in filas_previas}

    # La ruta de un ítem con varias: la que su fila ya dice, si sigue ahí (una
    # copia no lo mueve, y no se reescribe la tabla por el orden del listado,
    # B5·3); si no, la primera del listado.
    def ruta_de(k, its):
        dichas = {((f.get("_item") or {}).get("collection"), (f.get("_item") or {}).get("path"))
                  for f in previas.get(k, [])}
        return next((it for it in its if (it.ref.collection, it.ref.path) in dichas), its[0])
    hoy = {k: ruta_de(k, its) for k, its in rutas_de.items()}

    def estado(filas):
        return "error" if any((f.get("_status") or {}).get("state") == "error" for f in filas) else "ok"

    pendientes = [k for k in hoy if k not in previas
                  or (reintentar_errores and estado(previas[k]) == "error")]
    resumen = _Result({"items": len(hoy), "new": 0, "recomputed": 0, "skipped": len(hoy) - len(pendientes),
                       "errors": 0, "removed": len(_identidades_de_filas(filas_previas) - {_identidad(it.ref) for it in hoy.values()}), "rows": 0,
                       "written": False})
    hechas = {}   # clave → filas nuevas

    def item_json(it):
        return {c: getattr(it.ref, c, None) for c in _CAMPOS_ITEM}

    def calcular(k):
        it = hoy[k]
        ident = _identidad(it.ref)
        intentos = 1 + max([(f.get("_status") or {}).get("attempts") or 0 for f in previas.get(k, [])] or [0])
        deriv = {"key": k, "fn": nombre_fn, "fn_version": fn_version, "model": None, "model_rev": None,
                 "params_hash": params_hash, "run": run,
                 "created": datetime.datetime.now(datetime.timezone.utc)}
        try:
            salida_fn = fn(it)
            filas = list(salida_fn) if salida_fn is not None else []
            if isinstance(salida_fn, dict):
                filas = [salida_fn]
            out = []
            for f in filas:
                if not isinstance(f, dict):
                    raise TypeError("apply(): `%s` gave %s and not a dict per row" % (nombre_fn, type(f).__name__))
                f = dict(f)
                ancla = _ancla_de(_una_de(f, "anchor", "ancla"))
                padre = _una_de(f, "anchor_parent", "ancla_padre")
                malas = [c for c in f if c.startswith("_")]
                if malas:
                    raise ValueError("apply(): `%s` are system columns (v1alpha17 `03` §1)" % ", ".join(malas))
                out.append(dict(f, _item=item_json(it), _anchor=ancla,
                                _anchor_id=_sha(ident, _canonico(ancla), nombre_fn),
                                _anchor_parent=padre, _derivation=deriv,
                                _status={"state": "ok", "error_type": None, "error_message": None, "attempts": intentos}))
            if not out:
                ancla = _ancla_de(None)
                out.append({"_item": item_json(it), "_anchor": ancla, "_anchor_id": _sha(ident, _canonico(ancla), nombre_fn),
                            "_anchor_parent": None, "_derivation": deriv,
                            "_status": {"state": "ok", "error_type": None, "error_message": None, "attempts": intentos}})
            return k, out
        except Exception as e:  # noqa: BLE001 — un fallo de un ítem es su resultado
            ancla = _ancla_de(None)
            return k, [{"_item": item_json(it), "_anchor": ancla, "_anchor_id": _sha(ident, _canonico(ancla), nombre_fn),
                        "_anchor_parent": None, "_derivation": deriv,
                        "_status": {"state": "error", "error_type": type(e).__name__,
                                    "error_message": str(e)[:2000], "attempts": intentos}}]

    def montar():
        """La tabla entera: lo hecho ahora, lo que se queda (con su ruta de hoy) y,
        de lo pendiente aún sin hacer, lo que había (se rehará la próxima vez)."""
        filas = []
        for k, it in hoy.items():
            if k in hechas:
                filas.extend(hechas[k])
            elif k in previas:
                ref = item_json(it)
                filas.extend(dict(f, _item=ref) for f in previas[k])
        if not filas:
            return None
        cols = [c for c in dict.fromkeys(c for f in filas for c in f) if c not in _SISTEMA]
        # El tipo de cada columna de la carga, de lo que trae: pyarrow lo infiere.
        tipos = []
        for c in cols:
            try:
                tipos.append((c, pa.array([f.get(c) for f in filas]).type))
            except (pa.ArrowInvalid, pa.ArrowTypeError) as e:
                raise TypeError("apply(): column `%s` has no type: %s" % (c, e)) from None
        esquema = pa.schema(sistema + [(c, (pa.string() if pa.types.is_null(t) else t)) for c, t in tipos])
        return pa.Table.from_pylist(filas, schema=esquema)

    def guardar():
        t = montar()
        if t is None:
            return
        ore.write(salida, t, "overwrite", None, col.short_name)
        resumen["written"] = True
        resumen["rows"] = t.num_rows

    movido = any(k in previas and {((f.get("_item") or {}).get("collection"), (f.get("_item") or {}).get("path"))
                                   for f in previas[k]} != {(hoy[k].ref.collection, hoy[k].ref.path)}
                 for k in hoy)
    if not pendientes and not resumen["removed"] and not movido:
        resumen["rows"] = len(filas_previas)
        return resumen

    ultimo = time.time()
    with _cf.ThreadPoolExecutor(max(1, hilos)) as ex:
        for k, filas in (f.result() for f in _cf.as_completed([ex.submit(calcular, k) for k in pendientes])):
            hechas[k] = filas
            if estado(filas) == "error":
                resumen["errors"] += 1
            elif (hoy[k].ref.collection, hoy[k].ref.path) in rutas_previas:
                resumen["recomputed"] += 1
            else:
                resumen["new"] += 1
            if guardar_cada_s and time.time() - ultimo > guardar_cada_s:
                guardar()
                ultimo = time.time()
    guardar()
    return resumen


def _una_de(fila, en, es):
    """`fila[en]` o, si no, `fila[es]` (la clave de antes), sacada de la fila."""
    if en in fila and es in fila:
        raise ValueError("apply(): a row has both `%s` and its old name `%s`" % (en, es))
    if es in fila:
        _avisar("apply(): row key %r" % es, repr(en))
        return fila.pop(es)
    return fila.pop(en, None)


# ── 0049 B9 · ficheros que dan ficheros ─────────────────────────────────────

class File:
    """**A file that `apply()` writes** into a written collection (0049 B9):
    `name` is relative to the item it comes from (`p001.png` →
    `<item path>/p001.png`); `data` is bytes, a path or an open file, as in
    `put`; `content_type` the declared one (the bytes decide); `anchor`, what
    part of the item it is (v1alpha17 `02`: `{"kind": "page", "page": 1}`)."""

    def __init__(self, name, data, content_type=None, anchor=None):
        partes = name.split("/") if isinstance(name, str) else []
        if not partes or any(p in ("", ".", "..") for p in partes) or "\\" in name:
            raise ValueError("File(): `name` is a relative path without `.`, `..` or empty parts, not %r" % (name,))
        if anchor is not None:
            if not isinstance(anchor, dict) or not anchor.get("kind"):
                raise ValueError("File(): `anchor` is an Anchor (v1alpha17 `02`) with its `kind`: %r" % (anchor,))
            otros = set(anchor) - set(_CAMPOS_ANCLA)
            if otros:
                raise ValueError("File(): `anchor` with fields that are not `Anchor`'s: %s" % ", ".join(sorted(otros)))
        self.name, self.data, self.content_type, self.anchor = name, data, content_type, anchor

    def __repr__(self):
        return "File(%s)" % self.name


def _fichero_de_fila(fila):
    """**Una fila de la consulta de una colección derivada, como `File`** (0049
    B10·3): `name`, `data` y, si los dice, `content_type` y `anchor`. `data`
    son bytes (un `BLOB`), o un ítem (`Media`, el struct de `_item`) cuyos
    bytes se copian, fijados a su versión, con su tipo si la fila no dice otro.
    Un ancla de DuckDB trae todos los campos de su struct: los nulos no van."""
    nombre = fila.get("name")
    if not isinstance(nombre, str) or not nombre:
        raise ValueError("the query gave a file with no `name` (%r)" % (nombre,))
    datos, tipo = fila.get("data"), fila.get("content_type")
    if isinstance(datos, dict):
        ref = MediaRef.from_json(datos)
        datos = Item(collection(ref.collection), ref).read_bytes()
        tipo = tipo or ref.content_type
    elif isinstance(datos, (bytearray, memoryview)):
        datos = bytes(datos)
    elif not isinstance(datos, bytes):
        raise TypeError("`%s`: `data` is bytes (a BLOB) or an item (`Media`), not %s"
                        % (nombre, "null" if datos is None else type(datos).__name__))
    ancla = fila.get("anchor")
    if isinstance(ancla, dict):
        ancla = {k: v for k, v in ancla.items() if v is not None} or None
    return File(nombre, datos, tipo, ancla)


def _salida_de(output):
    """El nombre corto de la salida de `apply()`: la dada, o la del transform."""
    from . import _transform, _corto, _nombre_de
    if output is None:
        if _transform is None:
            raise ValueError("apply(): outside a transform, give the `output` (`db.schema.t`)")
        output = _transform.output
    return _corto(_nombre_de(output), "apply(): the `output`")


def _es_escrita(corto):
    """Si la salida es una `MediaCollection` (B9): la da `/documentos`."""
    from . import session, _ruta_de_vista
    codigo, _ = session.pedir("GET", _ruta_de_vista(corto, "MediaCollection"), plazo=60)
    return codigo == 200


def _identidad_servida(ref):
    """La identidad de un origen como la guarda el registro (`docs/media.md` §2):
    su `digest`; sin él, su `uri` fijada."""
    return ref.digest or ref.uri


def _ruta_de_uri(uri):
    """El camino de un ítem desde su `uri` (`ore://c/<camino>?v=…`)."""
    resto = (uri or "").split("://", 1)[-1]
    camino = resto.split("/", 1)[1] if "/" in resto else ""
    return urllib.parse.unquote(camino.split("?", 1)[0])


def _aplicar_ficheros(col, fn, version, params, salida, reintentar_errores, hilos, guardar_cada_s):
    import datetime
    import uuid

    nombre_fn = getattr(fn, "__name__", None) or type(fn).__name__
    fn_version = str(version) if version is not None else _version_de(fn)
    params_hash = None if params is None else hashlib.sha256(_canonico(params).encode("utf-8")).hexdigest()
    run = uuid.uuid4().hex
    destino = collection(salida)

    # El registro: una entrada por origen, por su identidad. Una escrita sin
    # ninguna transacción todavía no tiene registro (un ore-serve anterior a
    # B9 lo decía con un 404): vacío.
    registro = {}
    try:
        for d in destino.derivations():
            s = d.get("source") or {}
            registro[s.get("digest") or s.get("uri")] = d
    except MediaNotFound:
        registro = {}

    # Lo de hoy: un ítem por identidad (dos rutas con el mismo contenido son el
    # mismo ítem). La ruta, la que el registro ya dice si sigue ahí (una copia
    # no lo mueve); si no, la primera del listado.
    rutas_de = {}
    for it in col.items():
        rutas_de.setdefault(_identidad_servida(it.ref), []).append(it)

    def ruta_de(ident, its):
        dicha = _ruta_de_uri(((registro.get(ident) or {}).get("source") or {}).get("uri"))
        return next((it for it in its if it.ref.path == dicha), its[0])
    hoy = {i: ruta_de(i, its) for i, its in rutas_de.items()}
    clave = {i: _sha(_identidad(it.ref), nombre_fn, fn_version, None, params_hash) for i, it in hoy.items()}
    rutas_previas = {_ruta_de_uri((d.get("source") or {}).get("uri")) for d in registro.values()}

    def pendiente(i):
        d = registro.get(i)
        if d is None or (d.get("derivation") or {}).get("key") != clave[i]:
            return True
        return reintentar_errores and d.get("state") == "error"
    pendientes = [i for i in hoy if pendiente(i)]
    idos = [i for i in registro if i not in hoy]
    resumen = _Result({"items": len(hoy), "new": 0, "recomputed": 0, "skipped": len(hoy) - len(pendientes),
                       "errors": 0, "removed": len(idos), "files_written": 0, "files_retired": 0,
                       "written": False})
    if not pendientes and not idos:
        return resumen

    def calcular(i):
        """`fn` sobre un ítem: sus ficheros en memoria (o como rutas) —nada se
        sube hasta que la función termina, así que un fallo no deja ficheros
        sueltos—, o su error."""
        it = hoy[i]
        deriv = {"key": clave[i], "fn": nombre_fn, "fn_version": fn_version, "model": None, "model_rev": None,
                 "params_hash": params_hash, "run": run,
                 "created": datetime.datetime.now(datetime.timezone.utc).isoformat()}
        entrada = {"source": {"uri": it.ref.uri, "digest": it.ref.digest}, "derivation": deriv}
        try:
            dio = fn(it)
            ficheros = [dio] if isinstance(dio, File) else list(dio or [])
            vistos = set()
            for f in ficheros:
                if not isinstance(f, File):
                    raise TypeError("apply(): `%s` gave %s and not `ore.File` (the output is a collection)"
                                    % (nombre_fn, type(f).__name__))
                if f.name in vistos:
                    raise ValueError("apply(): `%s` gave two files named `%s` for `%s`" % (nombre_fn, f.name, it.ref.path))
                vistos.add(f.name)
            return i, entrada, ficheros, None
        except Exception as e:  # noqa: BLE001 — un fallo de un ítem es su resultado
            return i, entrada, [], {"type": type(e).__name__, "message": str(e)[:2000]}

    tx = None

    def abierta():
        nonlocal tx
        if tx is None:
            tx = destino.transaction()
        return tx

    def confirmar():
        nonlocal tx
        if tx is None:
            return
        t, tx = tx, None
        r = t.commit()
        resumen["written"] = resumen["written"] or not r.get("sin_cambios")
        resumen["files_retired"] += int(((r.get("derivations") or {}).get("files_retired")) or 0)

    if idos:
        abierta()._linaje["retire_sources"].extend(idos)
    try:
        _derivar_todo(pendientes, hilos, calcular, hoy, registro, rutas_previas, resumen, abierta,
                      confirmar, guardar_cada_s)
    except BaseException:
        # Lo confirmado se queda; lo de la transacción a medias, no.
        if tx is not None:
            try:
                tx.abort()
            except Exception:  # noqa: BLE001 — la de dentro es la que importa
                pass
        raise
    confirmar()
    return resumen


def _derivar_todo(pendientes, hilos, calcular, hoy, registro, rutas_previas, resumen, abierta, confirmar,
                  guardar_cada_s):
    """El bucle de `_aplicar_ficheros`: calcular por lotes (`hilos` a la vez),
    subir lo que dio cada ítem a la transacción abierta, apuntar su entrada y
    confirmar cada `guardar_cada_s` —entre lotes y entre ítems, nunca con una
    función a medias—."""
    import time
    ultimo = time.time()
    lote = max(1, hilos) * 2
    with _cf.ThreadPoolExecutor(max(1, hilos)) as ex:
        for a in range(0, len(pendientes), lote):
            for i, entrada, ficheros, error in ex.map(calcular, pendientes[a:a + lote]):
                it = hoy[i]
                t = abierta()
                if error is not None:
                    entrada.update(state="error", error=error)
                    resumen["errors"] += 1
                elif not ficheros:
                    entrada["state"] = "empty"
                else:
                    entrada["state"] = "files"
                    entrada["files"] = []
                    for f in ficheros:
                        camino = "%s/%s" % (it.ref.path, f.name)
                        t.put(camino, f.data, f.content_type)
                        entrada["files"].append({"path": camino, "anchor": f.anchor})
                        resumen["files_written"] += 1
                if error is None:
                    if i in registro or it.ref.path in rutas_previas:
                        resumen["recomputed"] += 1
                    else:
                        resumen["new"] += 1
                t._linaje["derivations"].append(entrada)
                if guardar_cada_s is not None and time.time() - ultimo >= guardar_cada_s:
                    confirmar()
                    ultimo = time.time()


class _Acceso:
    """La URL vigente de un ítem, fijada a su versión, y pedir otra si caduca."""

    def __init__(self, item):
        self.item = item
        self.url = None

    def vigente(self, otra=False):
        if self.url is None or otra:
            r = self.item.collection._donde(self.item.ref)
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
        raise MediaError("media/permiso", 401, "could not renew access to %s" % self.item.ref.path)

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
                raise MediaCorrupt("media/corrupto", 502, "%s: the stream was cut (%d bytes)"
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
                raise MediaRangeError("media/rango", 416, "the origin does not give the size: cannot seek from the end")
            nueva = self._size() + offset
        else:
            raise ValueError("whence")
        if nueva < 0:
            raise ValueError("seek before the start")
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
            raise ValueError("the file is closed")
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
            raise MediaCorrupt("media/corrupto", 502, "%s: the stream was cut at %d"
                               % (self.item.ref.path, self.pos))
        if not n:
            self._cerrar_flujo()
            if self._size() is not None and self.pos < self._size():
                raise MediaCorrupt("media/corrupto", 502, "%s: the stream was cut at %d of %d"
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

@_kw({"hilos": "threads"})
def read_many(items, threads=16):
    """The bytes of many items, `threads` at once (each request to a virtual
    collection's origin costs ~125 ms: one by one, 8 per second; with 16,
    about 120). Yields `(item, data, None)` or `(item, None, error)` as they
    finish: one's error does not stop the others. `items` may be lazy
    (`c.items()`)."""
    hilos = threads
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
    raise TypeError("put(): `data` is bytes, a path or a file, not %s" % type(datos).__name__)


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


class Transaction:
    """**An open transaction** on a written collection (B4b·3): `put` uploads
    items, `commit` leaves them written —and the pointer, with its
    provenance—, `abort` leaves nothing. Get one with
    `collection.transaction()`."""

    #: Alias de antes.
    coleccion = _Alias("collection")
    subidos = _Alias("uploaded")
    resultado = _Alias("result")
    cerrada = _Alias("closed")
    put_varios = _Alias("put_many")

    def __init__(self, col, ttl_s=3600):
        from . import session
        self.collection = col
        self.uploaded = []
        self.result = None
        self.closed = False
        codigo, r = session.pedir("POST", col.ruta + "/transactions", {"ttl_s": ttl_s},
                                  plazo=60, cabeceras=col._cabeceras())
        if codigo != 201:
            raise _error(codigo, r, "transaction(%s)" % col)
        self.id = r["transaction"]
        self._upload = r["upload"]
        #: 0049 B9: lo que el `commit` lleva además de lo subido: el linaje de
        #: lo que `apply()` derivó y lo que se retira.
        self._linaje = {"derivations": [], "retire_sources": [], "retire": []}

    def __repr__(self):
        return "Transaction(%s, %s%s)" % (self.collection, self.id, ", closed" if self.closed else "")

    def __enter__(self):
        return self

    def __exit__(self, tipo, valor, traza):
        if self.closed:
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
        if self.closed:
            raise MediaTransactionError("media/transaccion", 409, "%s: transaction %s is already closed" % (que, self.id))

    @_kw({"datos": "data", "tipo": "content_type"})
    def put(self, path, data, content_type=None):
        """Upload an item to `path` (relative to the collection). `data`: bytes,
        a path or a file. `content_type`, the declared one: it counts only if
        the bytes say nothing. Returns its `MediaRef` (the digest and the type
        the cell saw)."""
        datos, tipo = data, content_type
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
            ref = MediaRef.from_json(cuerpo)
            self.uploaded.append(ref)
            return ref
        raise MediaError("media/origen", 503, "put(%s): the upload was cut %d times: %s"
                         % (path, REINTENTOS + 1, ultimo))

    @_kw({"pares": "pairs", "hilos": "threads"})
    def put_many(self, pairs, threads=8):
        """Many `put`s at once: `pairs` yields `(path, data)` or
        `(path, data, content_type)`, and this yields `(path, ref, error)` as
        they finish —one's error is a value and does not stop the others, as in
        `read_many`—. At most `2 × threads` are in flight: `pairs` is not read
        whole up front."""
        pares, hilos = pairs, threads
        self._abierta("put_many")
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
        """Leave what was uploaded written —the pointer, with its provenance— and
        close it. If someone else committed at the same time, it commits again
        on top. Returns `{transaction, items, commit, metadata_location, …}`."""
        import time
        self._abierta("commit")
        espera = 0.5
        for intento in range(REINTENTOS + 1):
            codigo, r = self._cerrar("commit")
            if codigo == 200:
                self.closed, self.result = True, _en(r or {})
                return self.result
            carrera = codigo == 409 and not (r or {}).get("type")
            if not carrera or intento == REINTENTOS:
                raise _error(codigo, r, "commit(%s)" % self.id)
            time.sleep(espera)
            espera *= 2

    def delete(self, path):
        """Retire the item at `path` when this transaction commits (0049 B9). A
        path that is not a current item is `MediaNotFound` at commit."""
        self._abierta("delete(%s)" % path)
        self._linaje["retire"].append(path)

    def abort(self):
        """Leave nothing of what was uploaded, and close it."""
        if self.closed:
            return
        codigo, r = self._cerrar("abort")
        self.closed = True
        if codigo not in (204, 404):
            raise _error(codigo, r, "abort(%s)" % self.id)

    def _cerrar(self, op):
        from . import session
        cuerpo = {k: v for k, v in self._linaje.items() if v} if op == "commit" else {}
        return session.pedir("POST", "%s/transactions/%s/%s" % (self.collection.ruta, self.id, op), cuerpo,
                             plazo=300, cabeceras=self.collection._cabeceras())


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
