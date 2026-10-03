"""
`ore` · the session SDK (0031 W3.1).

What a cell imports:

- `over("<db>.<schema>.<view>")` reads a dataset or view as a DataFrame with
  Arrow types (`format="arrow"` gives a `pyarrow.Table`, `format="polars"` a
  polars DataFrame); `sql("select …")` runs DuckDB SQL over the tree's names.
- `write(name, data, mode="overwrite"|"append"|"upsert", key=[…])` writes a
  dataset to the lake; `declare(document)` declares an ontology document.
- `@transform(inputs=[…], output="…")` scopes what a function reads and writes,
  and records provenance; `@function` marks a tree function with its contract.
- `create_database`, `create_schema`, `create_dataset`, `create_view`,
  `drop_view`, `create_collection`: what a SQL script creates.
- Media (`ore.medios`): `collection(name)` lists items (`items()`, `stat()`),
  `item.open()` reads bytes pinned to a version, `read_many()` reads many at
  once, `collection(name).transaction()` writes, `apply(fn, …)` derives an
  anchored table incrementally. `media_url`/`media_urls`/`media_columns` serve
  a `Media<c>` value.
- `person()` is who opened the session; `session` is the session object.

The old Spanish names (`crear_coleccion`, `coleccion`, `MediaNoExiste`, …) and
keyword arguments (`como=`, `modo=`, …) keep working as aliases.
"""
# `ore` · el SDK del puesto (0031 W3.1).
#
# Lo que una celda importa. `over("<paquete>.<vista>")` devuelve la copia de esa
# vista como DataFrame **con tipos de Arrow** (0032 T3: `pd.ArrowDtype`, así que un
# entero con nulos sigue siendo entero, un `Decimal` es exacto y un instante lleva
# su zona); `como="arrow"` da la `pyarrow.Table` y `como="polars"` un DataFrame de
# polars si está en la capa. El código nunca ve el bucket ni una credencial:
# pregunta a `ore-serve` QUÉ copia es (con la identidad del puesto, que la
# resuelve en nombre de la persona y con su potestad) y baja el artefacto con la
# identidad del pod (Workload Identity). El sobre `ORECOPY1` se desenvuelve aquí;
# la carga es Parquet.
#
# `persona()` (W3.4) dice quién abrió el puesto: la identidad con la que corre
# lo que haces aquí.
#
# `sql("select … from hr.espanoles")` (W3.3) pregunta a los datasets por su
# nombre: ore-serve dice qué nombres del árbol lee el texto (`POST /puestos/{id}/sql`,
# sin regex) y los resuelve igual que en `over()`; cada uno queda en DuckDB como
# la vista `paquete.nombre`; devuelve un DataFrame. Medido en victor (2 CPU · 3 GB):
# 200 M de filas, `group by` con agregados en 1,9 s, `where` en 1 s.
#
# Fuera del clúster (las pruebas de fuego) el almacén es un directorio:
# `ORE_ALMACEN=dir:/ruta` lee `ore/v1/<clave>` de ahí.
#
# **Todo es un dataset** (0031 §10): lo que `ore-serve` contesta por `datos` es o bien
# `metadata_location` —el `metadata.json` vigente de una **tabla Iceberg** en el
# bucket: se lee en sitio con DuckDB (`iceberg_scan` sobre la raíz y la versión, con el
# token del pod como *bearer*; medido en `medida-w3-lago.py`: 10 M de filas, filtro con
# poda en 0,5 s sin bajar nada)— o bien `clave`, el sobre `ORECOPY1` heredado, que se
# baja una vez y se lee como Parquet. `over()` y `sql()` no distinguen.
#
# **Escribir** (0031 §11, W3.6c): `write("p.t", datos)` deja un dataset —una tabla
# Iceberg en el lago, el `Dataset` escrito en el árbol, el puntero— desde un DataFrame de pandas
# o polars o una tabla de Arrow. El código no toca el bucket: la tabla va por IPC a
# `ore-store` (el escritor de Rust, el mismo de la copia), que la lleva al físico del
# contrato (0032: `ns` → `µs`, zona → UTC; `uint64` y `null` se niegan con el nombre
# de la columna) y escribe los ficheros **con la credencial que el catálogo prestó**
# —acotada al prefijo de esa tabla—; el commit va al catálogo REST de Iceberg de
# `ore-serve` (`/v1/…`), que valida, escribe el `metadata.json` y mueve el puntero.
# `modo="sobrescribir"` (por defecto), `"anexar"` o `"upsert"` (con `clave=[…]`, las
# columnas que identifican una fila: lo que había menos esas claves, más lo que
# llega, reescrito entero —copy-on-write— y la clave queda declarada en la tabla
# para la siguiente vez). Idempotente: la misma tabla al mismo nombre y modo otra
# vez no deja otro snapshot (la clave de operación).
# Un 409 (alguien escribió mientras tanto) se reintenta sobre lo que hay; un 5xx
# se MIRA antes de reintentar: si el commit entró, entró.
#
# **Declarar** (0031 §9, W3.7 ①): `declare(documento)` deja un documento de la
# ontología en el árbol desde la celda —una `View` sobre lo que acabas de escribir,
# una `Entity`, una `Interface`, un `Concept`, una `Table`— por la puerta de Forge
# (`PUT /documentos/{kind}/{ns}/{n}`: se compila antes de empujar, y se rechaza sólo
# lo que la escritura añade de malo). El commit lo firma **quien abrió el puesto**,
# y va a **su rama** si el puesto tiene una. `documento` es el YAML tal cual (str) o
# un dict `{kind, metadata, spec}`. Devuelve `{kind, nombre, fichero, commit, nueva}`;
# un 422 es `ValueError` con los diagnósticos.
#
# **Un transform** (0031 §9, W3.7 ③): `@transform(inputs=["p.a", "p.b"], output="p.c")`
# sobre una función. Dentro, `over()`/`sql()` de algo que no está en `inputs` y `write()`
# a algo que no es `output` son `PermissionError`: lo declarado es lo único que el
# código puede leer y escribir. Y lo que `write()` deja lleva su **procedencia** en el
# snapshot y en el puntero: `{inputs, transform, codigo?, puesto}` dentro de un
# transform, o `{leidas, puesto}` fuera (lo que la sesión leyó hasta ese momento). Es
# el linaje `salida ← código ← inputs`, escrito por quien lo produjo. `codigo` viene de
# `ORE_CODIGO` (`<ruta>@<commit>`), que `ore run` pone.
#
# **La media** (0049, `docs/media.md`; en `ore/medios.py`): `coleccion("b.s.c")` da
# sus ítems (`items()`, por cursor; `stat()`), y `item.open()` un fichero fijado a
# su versión, mantenida o virtual —la celda dice dónde están los bytes y el SDK los
# lee sin el token de ORE—. `leer_varios` baja muchos a la vez.
import functools
import io
import json
import os
import re
import urllib.request

MAGIA = b"ORECOPY1"

#: The version of this SDK's interface (S3, 2026-10-03): 2 is the English
#: names of S1. Code that ORE generates checks it before running
#: (`ore_core::sdk::API`), so a session on an older SDK says so.
API = 2

__all__ = ["API", "over", "sql", "write", "declare", "transform", "person", "session", "Session", "table", "to_json",
           "create_database", "create_schema", "create_dataset", "create_view", "drop_view", "create_collection",
           "media_url", "media_urls", "media_columns", "model", "Model", "function", "get_function",
           "collection", "Collection", "Item", "MediaRef", "read_many", "Transaction", "MediaError",
           "MediaNotFound", "MediaForbidden", "MediaChanged", "MediaCorrupt", "MediaRangeError",
           "MediaNotWritable", "MediaTransactionError"]


# ── Los nombres en inglés, y los de antes como alias (S1) ──────────────────
#
# El API público es inglés; cada nombre español de antes sigue importable y es
# EL MISMO objeto (por `__getattr__` del módulo, al final), cada argumento con
# nombre español sigue valiendo (`_kw`), y cada dict que se devuelve contesta
# también a sus claves de antes (`_Result`). Todo alias pasa por `_avisar`: el
# día que se quiera avisar (S5), se cambia `_AVISAR_ALIAS` y nada más.

_AVISAR_ALIAS = False


def _avisar(viejo, nuevo, nivel=3):
    """El aviso de un alias, si `_AVISAR_ALIAS` lo pide (hoy, callado)."""
    if _AVISAR_ALIAS:
        import warnings

        warnings.warn("`%s` is deprecated: use `%s`" % (viejo, nuevo), DeprecationWarning, stacklevel=nivel)


def _kw(mapa):
    """Decorador: los argumentos con nombre de antes (`{"como": "format"}`) valen
    en la función nueva. Dar el viejo y el nuevo a la vez es `TypeError`."""
    def decora(f):
        @functools.wraps(f)
        def envuelta(*a, **k):
            if k:
                for es, en in mapa.items():
                    if es in k:
                        if en in k:
                            raise TypeError("%s() got both `%s` and its old name `%s`" % (f.__name__, en, es))
                        _avisar("%s(%s=…)" % (f.__name__, es), "%s(%s=…)" % (f.__name__, en))
                        k[en] = k.pop(es)
            return f(*a, **k)
        envuelta.__ore_kw__ = dict(mapa)
        return envuelta
    return decora


class _Alias:
    """Un atributo o un método de antes en una clase: lee (y escribe) el nuevo."""

    def __init__(self, nuevo):
        self.nuevo = nuevo
        self.viejo = None

    def __set_name__(self, clase, nombre):
        self.viejo = "%s.%s" % (clase.__name__, nombre)

    def __get__(self, obj, clase=None):
        _avisar(self.viejo, self.nuevo)
        return getattr(clase if obj is None else obj, self.nuevo)

    def __set__(self, obj, valor):
        _avisar(self.viejo, self.nuevo)
        setattr(obj, self.nuevo, valor)


#: Las claves de antes de lo que el SDK devuelve → las de ahora. Una sola tabla:
#: el código que ore-serve genera lee `_hecho["creada"]`, `_escrito["filas"]`…
_ES_EN = {
    # write()
    "tabla": "table", "filas": "rows", "operacion": "operation", "repetida": "repeated", "modo": "mode",
    "anadidas": "added", "antes": "before",
    # declare()
    "nombre": "name", "fichero": "file", "nueva": "created",
    # create_*() / drop_view()
    "base": "database", "clase": "kind", "creada": "created", "creado": "created", "coleccion": "collection",
    "vista": "view", "estado": "status", "columnas": "columns", "copia": "copy",
    # Collection.apply()
    "nuevos": "new", "recalculados": "recomputed", "saltados": "skipped", "errores": "errors",
    "borrados": "removed", "escrito": "written",
    # Transaction.commit()
    "transaccion": "transaction", "cambios": "changes", "procedencia": "provenance",
    # media_url() / media_urls()
    "huella": "fingerprint", "tipo": "content_type", "disposicion": "disposition", "segundos": "seconds",
    "caduca_ms": "expires_ms", "camino": "path",
}


class _Result(dict):
    """Lo que el SDK devuelve: un dict con las claves en inglés que contesta
    también, por `[]`, `get` e `in`, a la de antes (`_ES_EN`). Al imprimirlo o
    recorrerlo, sólo las inglesas."""

    __slots__ = ()

    def __missing__(self, clave):
        en = _ES_EN.get(clave) if isinstance(clave, str) else None
        if en is not None and dict.__contains__(self, en):
            _avisar("[%r]" % clave, "[%r]" % en)
            return dict.__getitem__(self, en)
        raise KeyError(clave)

    def get(self, clave, defecto=None):
        try:
            return self[clave]
        except KeyError:
            return defecto

    def __contains__(self, clave):
        if dict.__contains__(self, clave):
            return True
        en = _ES_EN.get(clave) if isinstance(clave, str) else None
        return en is not None and dict.__contains__(self, en)


def _en(d):
    """Un dict del servidor (claves de antes) → `_Result` con las inglesas."""
    if not isinstance(d, dict):
        return d
    r = _Result()
    for k, v in d.items():
        en = _ES_EN.get(k, k) if isinstance(k, str) else k
        if en in r and en != k:
            continue  # la inglesa, si ya venía, manda
        dict.__setitem__(r, en, v)
    return r


class _SinRedirecciones(urllib.request.HTTPRedirectHandler):
    """0049 B3·5: una redirección de la celda (el `307` de `content`) no se sigue
    sola: urllib reenviaría el token de ORE a la URL de los bytes. Vuelve como
    respuesta, con su cuerpo."""

    def redirect_request(self, *a, **k):
        return None


_SIN_SEGUIR = urllib.request.build_opener(_SinRedirecciones())


class Session:
    """The session (a *puesto*): where `ore-serve` is, which session this is,
    and who opened it (`person`). `ore.session` is the one a cell runs in."""

    #: Alias de antes.
    persona = _Alias("person")

    def __init__(self):
        self.servidor = os.environ.get("ORE_SERVE", "http://127.0.0.1:8080").rstrip("/")
        self.id = os.environ.get("PUESTO", "")
        self.bucket = os.environ.get("BUCKET", "")
        self.almacen = os.environ.get("ORE_ALMACEN", "gcs")
        # Quién abrió el puesto: lo pone el agente al reclamarlo (de la ficha).
        self.person = ""
        # El token lo pone el agente (`agente.py`) y lo renueva; una celda no lo ve.
        self._cabeceras = {}
        # 0049 B2·3: **quién da la cabecera**, si el agente lo dice. Una celda corre
        # en el proceso del agente y puede durar más que el token (300 s): con el
        # proveedor, cada petición pide la cabecera vigente —el agente la renueva a
        # 60 s de caducar— en vez de usar la que se copió al empezar la celda.
        self._proveedor = None

    def pedir(self, metodo, ruta, cuerpo=None, plazo=30, cabeceras=None, seguir=True):
        datos = None if cuerpo is None else json.dumps(cuerpo).encode("utf-8")
        req = urllib.request.Request(self.servidor + ruta, data=datos, method=metodo)
        req.add_header("accept", "application/json")
        if datos is not None:
            req.add_header("content-type", "application/json")
        for k, v in (self._proveedor() if self._proveedor else self._cabeceras).items():
            req.add_header(k, v)
        # Desde qué puesto: el catálogo escribe en nombre de quien lo abrió.
        if self.id:
            req.add_header("x-ore-puesto", self.id)
        for k, v in (cabeceras or {}).items():
            req.add_header(k, v)
        abrir = urllib.request.urlopen if seguir else _SIN_SEGUIR.open
        try:
            with abrir(req, timeout=plazo) as r:
                texto = r.read().decode("utf-8")
                return r.status, (json.loads(texto) if texto.strip() else None)
        except urllib.error.HTTPError as e:
            texto = e.read().decode("utf-8", "replace")
            try:
                return e.code, json.loads(texto)
            except ValueError:
                return e.code, {"error": texto.strip()}


session = Session()


def _cabeza(texto):
    """`(kind, namespace, name)` de un documento YAML, sin analizador: la línea
    `kind:` y el `metadata:` (en línea `{ name: x, namespace: y }` o en bloque)."""
    m = re.search(r"^kind:\s*([A-Za-z]+)\s*$", texto, re.M)
    if not m:
        raise ValueError("declare(): the document has no `kind:`")
    kind = m.group(1)
    m = re.search(r"^metadata:[ \t]*(.*)$", texto, re.M)
    if not m:
        raise ValueError("declare(): the document has no `metadata:`")
    resto = m.group(1).strip()
    campos = {}
    if resto.startswith("{"):
        for par in resto.strip("{} ").split(","):
            if ":" in par:
                k, v = par.split(":", 1)
                campos[k.strip()] = v.strip().strip("\"'")
    else:
        for linea in texto[m.end():].splitlines():
            if not linea.startswith((" ", "\t")):
                break
            if ":" in linea:
                k, v = linea.strip().split(":", 1)
                campos[k.strip()] = v.strip().strip("\"'")
    if not campos.get("name"):
        raise ValueError("declare(): `metadata.name` is missing")
    return kind, campos.get("namespace", ""), campos["name"], campos.get("schema") or DEFAULT


@_kw({"documento": "document"})
def declare(document):
    """Declare an ontology document from the cell (a `View`, `Entity`,
    `Interface`, `Concept`, `Table`…): the YAML as a `str`, or a dict
    `{kind, metadata, spec}`. It is compiled before it is pushed, committed as
    the person who opened the session, on the session's branch.

    Returns `{kind, name, file, commit, created}`. A rejected document (422)
    raises `ValueError` with its diagnostics."""
    documento = document
    if isinstance(documento, str):
        kind, ns, nombre, schema = _cabeza(documento)
        cuerpo = {"yaml": documento}
    elif isinstance(documento, dict):
        kind = documento.get("kind")
        meta = documento.get("metadata") or {}
        ns, nombre = meta.get("namespace", ""), meta.get("name", "")
        schema = meta.get("schema") or DEFAULT
        if not kind or not nombre:
            raise ValueError("declare(): the document needs `kind` and `metadata.name`")
        cuerpo = documento
    else:
        raise ValueError("declare() takes the document's YAML or a dict, not %r" % (type(documento).__name__,))
    if not ns:
        raise ValueError("declare(): `metadata.namespace` is missing: a document lives in a database")
    # 0038: en su schema, `/documentos/{kind}/{base}/{schema}/{n}`; la de dos
    # tramos es `default`, y un documento de otro schema por ella es un 422.
    ruta = ("/documentos/%s/%s/%s" % (kind, ns, nombre) if schema == DEFAULT
            else "/documentos/%s/%s/%s/%s" % (kind, ns, schema, nombre))
    c, r = session.pedir("PUT", ruta, cuerpo, plazo=120)
    if c in (200, 201):
        s_ = r.get("schema", schema)
        return _Result({"kind": r.get("kind", kind),
                        "name": ".".join([r.get("namespace", ns)] + ([] if s_ == DEFAULT else [s_]) + [r.get("name", nombre)]),
                        "file": r.get("fichero", ""), "commit": r.get("commit", ""),
                        "created": bool(r.get("nueva", c == 201))})
    r = r or {}
    if r.get("diagnosticos"):
        raise ValueError("declare(%s.%s): %s" % (ns, nombre, "; ".join("%s: %s" % (d.get("codigo", "?"), d.get("mensaje", "")) for d in r["diagnosticos"])))
    raise RuntimeError("declare(%s.%s): %s (%s)" % (ns, nombre, r.get("error", "?"), c))


def person():
    """Who opened the session (`persona:…`): the identity everything you run
    here runs as (W3.4)."""
    if not session.person:
        raise RuntimeError("person(): the agent does not know yet who opened the session")
    return session.person


def _bajar(bucket, clave):
    """Los bytes del artefacto, por el almacén que toque."""
    if session.almacen.startswith("dir:"):
        with open(session.almacen[4:].rstrip("/") + "/" + clave, "rb") as f:
            return f.read()
    if session.almacen == "gcs":
        from google.cloud import storage  # noqa: WPS433 — sólo dentro del clúster

        return storage.Client().bucket(bucket).blob(clave).download_as_bytes()
    raise RuntimeError("ORE_ALMACEN=%r is not a store: use `gcs` or `dir:<path>`" % session.almacen)


def _desenvolver(crudo):
    if crudo[:8] != MAGIA:
        raise ValueError("the artifact is not an ORE copy (no `ORECOPY1`)")
    n = int.from_bytes(crudo[8:12], "little")
    cabecera = json.loads(crudo[12:12 + n])
    return cabecera, crudo[12 + n:]


# Lo que la sesión leyó (por nombre), y el transform activo si lo hay.
_leidas = []
_transform = None


class _Transform:
    def __init__(self, nombre, inputs, output):
        self.nombre, self.inputs, self.output = nombre, list(inputs), output


DEFAULT = "default"


def _corto(nombre, que="a tree name"):
    """`base.nombre` o `base.schema.nombre` (0038) → la forma corta, la clave del
    árbol: `base.nombre` en `default`, `base.schema.nombre` en otro schema.
    `ventas.pedidos` y `ventas.default.pedidos` son el mismo."""
    partes = nombre.split(".") if isinstance(nombre, str) else []
    if len(partes) not in (2, 3) or not all(partes):
        raise ValueError("%s is `<database>.<schema>.<name>` (or `<database>.<name>`, in `default`), not %r" % (que, nombre))
    if len(partes) == 3 and partes[1] == DEFAULT:
        return "%s.%s" % (partes[0], partes[2])
    return ".".join(partes)


def _partes(corto):
    """La forma corta → (base, schema, nombre)."""
    p = corto.split(".")
    return (p[0], DEFAULT, p[1]) if len(p) == 2 else (p[0], p[1], p[2])


def _v1_tabla(corto):
    """La ruta de `/v1` de una tabla, como Unity (0038 P4): la base es el
    `prefix`, el schema el namespace."""
    b, s_, n = _partes(corto)
    return "/v1/%s/namespaces/%s/tables/%s" % (b, s_, n)


def _nombre_de(x):
    """Un nombre del árbol, o una colección (`ore.collection(…)`) por su nombre."""
    return getattr(x, "short_name", x)


def transform(inputs, output):
    """`@transform(inputs=[…], output="db.schema.t")`: what is declared is the
    only thing the function may read (`over`, `sql`, a collection) and write
    (`write`, a collection's transaction); anything else is `PermissionError`.
    What it writes carries its provenance (`{inputs, transform, …}`). An input
    or the output may be a collection (0049 B4·3):
    `inputs=[ore.collection("legal.archive.contracts")]`; the server pins the
    transaction of each one to read when the transform is declared."""
    if isinstance(inputs, str):
        raise ValueError("transform(): `inputs` is a list of `<database>.<schema>.<name>`")
    inputs = [_corto(_nombre_de(i), "transform(): each input") for i in inputs]
    output = _corto(_nombre_de(output), "transform(): `output`")
    if output in inputs:
        raise ValueError("transform(): `%s` cannot be both an input and the output" % output)

    def decora(f):
        import functools

        @functools.wraps(f)
        def corre(*a, **kw):
            global _transform
            if _transform is not None:
                raise RuntimeError("transform(): `%s` is already running; a transform does not call another" % _transform.nombre)
            _transform = _Transform(getattr(f, "__name__", "transform"), inputs, output)
            # Y se le dice al servidor (W3.7 gobierno ⑤): mientras corre, él
            # resuelve sólo `inputs` y deja escribir sólo `output`. Si no
            # contesta (un ore-serve viejo), el SDK sigue acotando por su cuenta.
            session.pedir("POST", "/puestos/%s/transform" % session.id, {"nombre": _transform.nombre, "inputs": list(inputs), "output": output})
            try:
                return f(*a, **kw)
            finally:
                _transform = None
                session.pedir("DELETE", "/puestos/%s/transform" % session.id)
        corre.inputs, corre.output = list(inputs), output
        return corre
    return decora


def _lee(vista):
    """Anota una lectura, y dentro de un transform la acota a sus `inputs`."""
    vista = _corto(vista)
    # 0049 B5·2: su `output` también —un incremental lee lo que ya escribió—, y
    # leerse no es una entrada: no se anota.
    if _transform is not None and vista == _transform.output:
        return
    if _transform is not None and vista not in _transform.inputs:
        raise PermissionError("`%s` is not among the inputs of `%s` (%s): a transform only reads what it declares" % (vista, _transform.nombre, ", ".join(_transform.inputs)))
    if vista not in _leidas:
        _leidas.append(vista)


def _procedencia(nombre=None, anclada_a=None):
    """Lo que `write()` deja dicho de sí: de qué salió, qué código, desde qué puesto.
    Fuera de un transform es lo que la sesión leyó, **sin lo que se está escribiendo**
    (W3.7 gobierno ③: un dataset no sale de sí mismo)."""
    p = {"puesto": session.id}
    if _transform is not None:
        p["inputs"] = sorted(_transform.inputs)
        p["transform"] = _transform.nombre
    else:
        p["leidas"] = sorted(l for l in _leidas if l != nombre)
    if os.environ.get("ORE_CODIGO"):
        p["codigo"] = os.environ["ORE_CODIGO"]
    if anclada_a:
        p["anclada_a"] = anclada_a
    return p


def _resolver(vista):
    """Qué copia es `<paquete>.<vista>`, según ore-serve (en nombre de la persona)."""
    vista = _corto(vista)
    _lee(vista)
    codigo, r = session.pedir("GET", "/puestos/%s/datos/%s" % (session.id, vista))
    return _o_el_error(codigo, r, vista)


def _o_el_error(codigo, r, vista):
    """Lo que ore-serve contesto por un nombre, o el error de siempre: el mismo
    para `over()` (GET datos) que para `sql()` (POST sql)."""
    if codigo == 409:
        raise RuntimeError("the copy of `%s` is not made: %s" % (vista, (r or {}).get("error", "")))
    if codigo == 404:
        raise LookupError("there is no `View` or `Dataset` `%s` in the tree" % vista)
    if codigo == 403:
        # El conducto de la lectura (0031 W3.7 gobierno ②): lo que el dataset
        # lleva no cabe por `contextSurface.workspace`. Se dice tal cual.
        raise PermissionError((r or {}).get("error") or "ore-serve does not allow reading `%s` from a session" % vista)
    if codigo != 200:
        raise RuntimeError("ore-serve answered %s for `%s`: %s" % (codigo, vista, (r or {}).get("error", r)))
    return r


def _fuente_de(vista):
    """De qué se lee la vista, como fragmento SQL de DuckDB: `iceberg_scan(...)` si es
    un dataset Iceberg (el puntero trae `metadata_location`), `read_parquet('…')` si
    es un sobre heredado (trae `clave`, y se baja una vez)."""
    return _fuente_de_respuesta(vista, _resolver(vista))


# Donde se pone la vista de DuckDB de cada dataset que una View lee: aparte de
# los nombres del árbol, porque una View y su dataset pueden llamarse igual.
ESQUEMA_DE_DATASETS = "__ore_dataset"


def _fuente_de_respuesta(vista, r):
    """Lo que `datos` contestó, como fragmento SQL. Una View llega como su
    pregunta (`consulta`, SQL sobre `"__ore_dataset"."<p>.<n>"`) con sus datasets
    ya resueltos por el servidor: cada uno se pone como vista de DuckDB por el
    camino de siempre, y la View es la consulta encima. Medido: con `select *`
    sobre la raíz, una View con `where` y `fields` daba 20 000 filas y 4
    columnas donde dice 5 000 y 2 (`medida-la-vista-con-filtro.py`)."""
    if r.get("consulta"):
        con = _duckdb()
        # En `memory`, y nombradas con él: dentro de una vista de un catálogo
        # adjunto (una base, 0038) un schema sin cualificar se busca en ESE
        # catálogo, no en `memory` (medido).
        con.execute('create schema if not exists memory."%s"' % ESQUEMA_DE_DATASETS)
        for d, rd in (r.get("datasets") or {}).items():
            fuente, _ = _fuente_de_respuesta(d, rd)
            con.execute('create or replace view memory."%s"."%s" as select * from %s' % (ESQUEMA_DE_DATASETS, d.replace('"', '""'), fuente))
        return "(%s)" % r["consulta"].replace('"%s".' % ESQUEMA_DE_DATASETS, 'memory."%s".' % ESQUEMA_DE_DATASETS), r
    if r.get("metadata_location"):
        global _s3
        # La credencial de lectura que `datos` presta (W3.7 gobierno ②b): acotada
        # a este dataset; con ella se lee, y no con la identidad del pod.
        cred = r.get("credencial") or {}
        if r["metadata_location"].startswith("s3://") and not _s3:
            if cred.get("s3.access-key-id"):
                _s3 = cred
            else:
                c, l = session.pedir("GET", _v1_tabla(_corto(vista)), cabeceras=_DELEGAR)
                if c == 200 and (l or {}).get("config", {}).get("s3.access-key-id"):
                    _s3 = l["config"]
        return _iceberg(r["metadata_location"], cred.get("gcs.oauth2.token")), r
    f, r = _parquet_de(vista, r)
    r["_parquet"] = f
    return "read_parquet('%s')" % f.replace("'", "''").replace("\\", "/"), r


def _iceberg(metadata_location, prestada=None):
    """`iceberg_scan` sobre la raíz de la tabla y la versión del puntero. A DuckDB no se
    le da el fichero: con `allow_moved_paths` la raíz es lo que se le pasa (y con el
    fichero resolvía `…metadata.json/metadata/snap…`), y así no lista nada. En el
    bucket, la API XML de GCS por https con **la credencial prestada** para este
    dataset como *bearer* (un secreto por raíz, con `scope`: un `sql()` que junta dos
    datasets lleva dos); sin prestada, el token del pod (lo de antes de ②b)."""
    raiz, fichero = metadata_location.rsplit("/metadata/", 1)
    version = fichero[: -len(".metadata.json")] if fichero.endswith(".metadata.json") else fichero
    con = _duckdb()
    # `iceberg` arrastra `avro` (y usa `json` e `icu`); con el autoinstalado apagado
    # hay que cargarlas por su nombre, en orden. `httpfs` sólo para el bucket.
    for e in LAGO:
        _cargar(con, e)
    if raiz.startswith("gs://"):
        _cargar(con, "httpfs")
        raiz = "https://storage.googleapis.com/" + raiz[5:]
        if prestada:
            import hashlib
            con.execute("create or replace secret ore_gcs_%s (type http, bearer_token '%s', scope '%s')" % (hashlib.sha1(raiz.encode()).hexdigest()[:12], prestada.replace("'", "''"), raiz.replace("'", "''")))
        else:
            con.execute("create or replace secret ore_gcs (type http, bearer_token '%s')" % _token_de_google().replace("'", "''"))
    elif raiz.startswith("s3://") and _s3:
        # Un S3 (R2, o el de mentira de las pruebas): con la credencial que el
        # catálogo prestó al escribir, o la de la tabla que se pidió leer.
        _cargar(con, "httpfs")
        _secreto_s3(con, _s3)
    return "iceberg_scan('%s', version='%s', allow_moved_paths=true)" % (raiz.replace("'", "''").replace("\\", "/"), version.replace("'", "''"))


_s3 = None


def _secreto_s3(con, cfg):
    ep = cfg.get("s3.endpoint", "")
    ssl = "true" if ep.startswith("https://") else "false"
    ep = ep.replace("https://", "").replace("http://", "").rstrip("/")
    con.execute(
        "create or replace secret ore_s3 (type s3, key_id '%s', secret '%s', endpoint '%s', url_style 'path', use_ssl %s, region '%s')"
        % (cfg.get("s3.access-key-id", "").replace("'", "''"), cfg.get("s3.secret-access-key", "").replace("'", "''"), ep, ssl, cfg.get("s3.region", "auto"))
    )


EXTENSIONES = "/opt/ore/duckdb"
LAGO = ("json", "icu", "avro", "iceberg")


def _cargar(con, extension):
    """`LOAD` de una extensión. En la imagen están preinstaladas en `/opt/ore/duckdb`
    (el pod no tiene internet, y DuckDB tardaría 120 s en rendirse: medido); fuera,
    si falta, se instala una vez."""
    try:
        con.execute("load %s" % extension)
    except Exception:
        if os.path.isdir(EXTENSIONES):
            raise RuntimeError("the image lacks the DuckDB extension `%s`: it must be preinstalled in %s" % (extension, EXTENSIONES))
        con.execute("install %s" % extension)
        con.execute("load %s" % extension)


def _token_de_google():
    """El token de la identidad del pod (Workload Identity), para leer el bucket."""
    import google.auth
    import google.auth.transport.requests

    creds, _ = google.auth.default(scopes=["https://www.googleapis.com/auth/devstorage.read_only"])
    creds.refresh(google.auth.transport.requests.Request())
    return creds.token


def _parquet_de(vista, r=None):
    """La copia de la vista como fichero Parquet local, bajado UNA vez por sesión
    (la clave es el digest del artefacto: una clave nueva es otra copia)."""
    r = r or _resolver(vista)
    d = os.environ.get("ORE_COPIAS") or _copias_por_defecto()
    os.makedirs(d, exist_ok=True)
    f = os.path.join(d, r["clave"].replace("/", "_") + ".parquet")
    if not os.path.exists(f):
        crudo = _bajar(r.get("bucket") or session.bucket, r["clave"])
        _, carga = _desenvolver(crudo)
        tmp = f + ".parte"
        with open(tmp, "wb") as fh:
            fh.write(carga)
        os.replace(tmp, f)
    return f, r


def _copias_por_defecto():
    """`/trabajo/copias` en el puesto; fuera (las pruebas), un directorio temporal."""
    if os.path.isdir("/trabajo") and os.access("/trabajo", os.W_OK):
        return "/trabajo/copias"
    import tempfile

    return os.path.join(tempfile.gettempdir(), "ore-copias")


def _como(tabla, como):
    """Una `pyarrow.Table` en la forma pedida. `pandas` va con `ArrowDtype`: es la
    misma memoria de Arrow debajo, y nada se degrada (un `int64` con nulos no se
    vuelve `float64`, un `decimal128` no se vuelve `object` de `Decimal` sin tipo,
    un `timestamp[us, tz=UTC]` conserva la zona). Por eso aquí no hay `estricto`:
    no hay conversión con pérdida que pedir que falle."""
    if como == "arrow":
        return tabla
    if como == "pandas":
        import pandas as pd

        return tabla.to_pandas(types_mapper=pd.ArrowDtype)
    if como == "polars":
        import polars as pl

        return pl.from_arrow(tabla)
    raise ValueError("format=%r is not a format: use `pandas`, `arrow` or `polars`" % (como,))


# ── El modelo desde el código (ORE 0050 P4) ───────────────────────────────
#
# Una `Function` de `runtime: python` declara en `models` los modelos que su
# código puede llamar, y solo esos: `ore-serve` los resuelve al invocarla (la
# puerta y el id servido, como para `runtime: model`) y el arnés los deja aquí.
# Fuera de una función que los declare no hay ninguno, y la red del puesto
# tampoco llega a la puerta: la abre `ore-serve` al trabajo de esa función.
# La identidad es la del agente de la celda —la misma con la que la puerta
# atiende al Job de `runtime: model`—, y su token se renueva solo (0049 B2·3).

_MODELOS = None


def _modelos_de_la_funcion(modelos):
    """Lo que el arnés de una función declara que su código puede llamar."""
    global _MODELOS
    _MODELOS = dict(modelos or {})


class Model:
    """A model from the tree, served by the platform's model gateway. Get one
    with `ore.model(ref)` inside a function that declares it in `models`."""

    #: Alias de antes.
    referencia = _Alias("ref")
    servido = _Alias("served")
    pide = _Alias("ask")

    def __init__(self, ref, url, served):
        self.ref = ref
        self.url = url.rstrip("/")
        self.served = served

    def __repr__(self):
        return "Model(%r → %s)" % (self.ref, self.served)

    @_kw({"mensajes": "messages", "plazo": "timeout"})
    def chat(self, messages, timeout=120, **options):
        """`POST /chat/completions` with `messages` (the OpenAI shape); extra
        keyword `options` go in the request body. Returns the whole response,
        as JSON."""
        cuerpo = dict({"model": self.served, "messages": messages}, **options)
        req = urllib.request.Request(self.url + "/chat/completions", data=json.dumps(cuerpo).encode("utf-8"),
                                     method="POST")
        req.add_header("content-type", "application/json")
        for k, v in (session._proveedor() if session._proveedor else session._cabeceras).items():
            req.add_header(k, v)
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return json.loads(r.read().decode("utf-8"))
        except urllib.error.HTTPError as e:
            raise RuntimeError("the model gateway answered %s for `%s`: %s"
                               % (e.code, self.ref, e.read().decode("utf-8", "replace")[:300])) from None

    @_kw({"texto": "text"})
    def ask(self, text, **options):
        """One user message; returns the text of the answer (`temperature` 0
        unless given)."""
        options.setdefault("temperature", 0)
        r = self.chat([{"role": "user", "content": text}], **options)
        return r["choices"][0]["message"]["content"]


def function(f=None, *, over=None, reads=None, models=None, timeout=None):
    """`@function` or `@function(over=…, reads=[…], models=[…], timeout="…")`:
    marks the `def` as a tree function (ORE 0050). Its signature —annotated
    parameters and return type— is the contract: the `Function` document is
    derived from it by reading the file, without running it, so the
    arguments must be literals.

    In a session it is called like any other `def`, **with its contract**
    (G3, `ore.contrato`): each parameter arrives as the type it annotates
    (`"2026-10-02"` is a `date`, `12.5` a `Decimal`) and what it returns must
    be of the annotated type. The harness applies the same rule when it is
    invoked: what works here works invoked. What `over`/`reads`/`models` allow
    is enforced by the harness."""
    def marca(g):
        from .contrato import llamada

        envuelta = llamada(g)
        envuelta.__ore_function__ = {"over": over, "reads": list(reads or []), "models": list(models or []),
                                     "timeout": timeout}
        return envuelta
    return marca(f) if callable(f) else marca


_FUNCIONES = {}
#: The `spec` of each function `get_function()` read: SQL types its calls by it
#: (0049 B7·2).
_FUNCIONES_SPEC = {}


@_kw({"nombre": "name"})
def get_function(name):
    """The published function `name` (`<database>.<def>` or
    `<database>.<schema>.<def>`), to call from code like any `def` (ORE 0050 G3):

        echo = get_function("test_project.echo_types")
        echo(amount=Decimal("12.50"), day=date(2026, 10, 2))

    It runs **here**, in this process, with its contract: the code is its
    `entrypoint` in the tree this session sees. One with `over` (a call per
    row of a dataset) or with `models` is not called this way: a pipeline
    invokes it."""
    nombre = name
    partes = nombre.split(".")
    if len(partes) not in (2, 3) or not all(partes):
        raise ValueError("`get_function(%r)`: the name is `<database>.<def>` or `<database>.<schema>.<def>`" % nombre)
    if nombre in _FUNCIONES:
        return _FUNCIONES[nombre]
    from urllib.parse import quote

    from .contrato import llamada

    # In the session's branch: a function committed there and not yet in `main`
    # is read from there, document and code (without the header, `main`).
    rama = _rama_del_puesto()
    codigo, doc = session.pedir("GET", "/documentos/Function/" + "/".join(quote(p, safe="") for p in partes),
                                cabeceras=rama)
    if codigo == 404:
        raise LookupError("there is no published function `%s`" % nombre)
    if codigo != 200:
        raise RuntimeError("reading the function `%s`: %s %s" % (nombre, codigo, (doc or {}).get("error", "")))
    spec = doc.get("spec") or {}
    if spec.get("runtime") != "python":
        raise NotImplementedError("`%s` is `runtime: %s`: only code functions are called from code"
                                  % (nombre, spec.get("runtime")))
    if spec.get("over") or spec.get("models"):
        raise NotImplementedError("`%s` declares %s: it is invoked from a pipeline, which gives it its rows and its model"
                                  % (nombre, "`over`" if spec.get("over") else "`models`"))
    ruta, _, defn = str(spec.get("entrypoint", "")).rpartition(":")
    fichero = "packages/%s/%s" % (doc.get("paquete"), ruta)
    codigo, f = session.pedir("GET", "/arbol/" + "/".join(quote(p, safe="") for p in fichero.split("/")),
                              cabeceras=rama)
    if codigo != 200 or not isinstance(f, dict) or "texto" not in f:
        raise RuntimeError("reading the code of `%s` (%s): %s" % (nombre, fichero, codigo))
    modulo = {"__name__": "ore_funcion_" + "_".join(partes), "__file__": fichero}
    exec(compile(f["texto"], fichero, "exec"), modulo)
    if defn not in modulo or not callable(modulo[defn]):
        raise LookupError("`%s` does not define `%s`" % (fichero, defn))
    g = modulo[defn]
    g = g if getattr(g, "__ore_contrato__", False) else llamada(g)
    _FUNCIONES[nombre] = g
    _FUNCIONES_SPEC[nombre] = spec
    return g


@_kw({"referencia": "ref"})
def model(ref):
    """The model `ref` —as written in `models` (`extractor`, `ai.chat`) or by
    its full name (`sales.default.extractor`)—, if the running function
    declares it. Returns a `Model`."""
    referencia = ref
    clave = referencia[len("modelo/"):] if referencia.startswith("modelo/") else referencia
    if _MODELOS is None:
        raise PermissionError("`model(%r)`: a model is called from a function that declares it in `models` "
                              "(ORE 0050), not from a session" % referencia)
    m = _MODELOS.get(clave)
    if m is None:
        raise PermissionError("`%s` is not in this function's `models`: it declares %s"
                              % (referencia, sorted(_MODELOS) or "none"))
    return Model(clave, m["url"], m["model"])


@_kw({"vista": "view", "como": "format"})
def over(view, format="pandas"):
    """Read the dataset or view `<database>.<schema>.<name>`: a DataFrame with
    Arrow types (`format="pandas"`, the default), a `pyarrow.Table`
    (`format="arrow"`) or a polars DataFrame (`format="polars"`)."""
    vista, como = view, format
    fuente, r = _fuente_de(vista)
    if r.get("_parquet"):
        # El sobre, ya en local: pyarrow lo lee más deprisa que nadie (13 M filas/s).
        import pyarrow.parquet as pq

        return _como(_nunca_nulas(pq.read_table(r["_parquet"]), r, vista), como)
    return _como(_nunca_nulas(_arrow(_duckdb().sql("select * from %s" % fuente)), r, vista), como)


def _nunca_nulas(tabla, r, vista):
    """ORE 0051 P7 (OOS v1alpha22 `01` §7): las columnas que el árbol dice que
    nunca son nulas —lo que el origen garantiza o la consulta deriva; ore-serve
    lo manda en `nunca_nulas`— salen **no nulables** en el esquema de Arrow.
    DuckDB no mira la marca de Iceberg, así que sin esto todo saldría nulable.

    Antes de marcar se mira: una columna que trae un nulo NO se marca —un
    esquema que dice «nunca nula» sobre un nulo es justo la mentira que esto
    evita— y se avisa, porque entonces el árbol y los datos discrepan."""
    nunca = set((r or {}).get("nunca_nulas") or [])
    if not nunca:
        return tabla
    import warnings

    import pyarrow as pa

    campos, cambia = [], False
    for f, col in zip(tabla.schema, tabla.columns):
        if f.name in nunca and f.nullable:
            if col.null_count:
                warnings.warn("`%s.%s` is never null according to the tree but has %d nulls: left nullable"
                              % (vista, f.name, col.null_count), stacklevel=3)
            else:
                f, cambia = f.with_nullable(False), True
        campos.append(f)
    if not cambia:
        return tabla
    return pa.Table.from_arrays(tabla.columns, schema=pa.schema(campos, metadata=tabla.schema.metadata))


def _arrow(relacion):
    """Una relación de DuckDB → `pyarrow.Table` (el nombre del método cambió en 1.5)."""
    if hasattr(relacion, "to_arrow_table"):
        return relacion.to_arrow_table()
    return relacion.fetch_arrow_table()


_con = None

# ⛔ EL REPARTO DEL POD (medido en `medida-la-celda-que-no-cabe.py`).
#
# Sin esto, DuckDB se pone un `memory_limit` sacado de LA MAQUINA y no del pod
# —25 GiB medidos en una de 32 GB—, y en un contenedor de 4 GiB eso es pedirle
# al kernel que mate la sesión: SIGKILL, sin excepción, sin mensaje y sin
# informe. Con tope, lo que no quepa se derrama a disco y «no cabe» pasa a
# significar «tarda»: medido, 12 M de claves agrupadas con 200 MB son 12,2 s.
#
# En Python el reparto es distinto que en la JVM: aquí no hay heap aparte, así
# que la mitad del pod es para DuckDB y la otra mitad para lo que la celda
# tenga en memoria (pandas, polars, lo suyo).
_DUCKDB_POR_CIENTO = 50


def _tropo_mb():
    """Los MB que le tocan a DuckDB. `ORE_MEMORIA_MB` lo pone la plantilla del
    Job junto a `limits.memory`. Sin él —las pruebas, un portátil— se usa un
    suelo conservador: un tope equivocado da un error legible; ninguno da un
    proceso muerto."""
    try:
        pod = int(os.environ.get("ORE_MEMORIA_MB", "0") or 0)
    except ValueError:
        pod = 0
    return max(256, pod * _DUCKDB_POR_CIENTO // 100) if pod > 0 else 512


def _derrame():
    """Dónde derrama DuckDB lo que no le cabe.

    En el pod, el volumen de trabajo (`/trabajo`, un `emptyDir`), que es donde
    se puede escribir y muere con la sesión. Fuera del clúster, el temporal del
    sistema — ⛔ y NO el directorio actual, que en las pruebas es el
    repositorio: un motor derramando gigabytes dentro del árbol de fuentes es
    un susto que no hace falta darse."""
    import tempfile

    for base in ("/trabajo", tempfile.gettempdir()):
        if not os.path.isdir(base):
            continue
        d = os.path.join(base, ".duckdb-derrame")
        try:
            os.makedirs(d, exist_ok=True)
            return d
        except OSError:
            pass
    return tempfile.gettempdir()


def _duckdb():
    global _con
    if _con is None:
        import duckdb

        _con = duckdb.connect()
        hilos = os.environ.get("ORE_HILOS")
        if hilos:
            _con.execute("set threads to %d" % int(hilos))
        # Nunca salir a por una extensión: sin red, DuckDB se rinde a los 120 s
        # (medido en el clúster). Lo que la imagen trae está en /opt/ore/duckdb.
        # ⭐ Lo que le toca, y dónde derramar lo que no quepa (ver arriba).
        _con.execute("set memory_limit='%dMB'" % _tropo_mb())
        _con.execute("set temp_directory='%s'" % _derrame().replace("'", "''"))
        _con.execute("set autoinstall_known_extensions = false")
        # Un instante es un instante: DuckDB enseña un TIMESTAMPTZ en la zona de la
        # sesión, y el contrato (0032 §1) lo quiere en UTC. Aquí la sesión ES UTC.
        _con.execute("set TimeZone = 'UTC'")
        if os.path.isdir(EXTENSIONES):
            _con.execute("set extension_directory = '%s'" % EXTENSIONES)
    return _con


def _q(x):
    return '"%s"' % x.replace('"', '""')


def _registra(con, corto, fuente):
    """El nombre del árbol como vista de DuckDB, con sus tres niveles (0038): un
    catálogo por base (`attach ':memory:' as <base>`), un schema por schema. Lo
    de `default` lleva además su alias en `main`, que es donde DuckDB busca un
    nombre de DOS partes (medido): `ventas.pedidos` y `ventas.default.pedidos`
    leen lo mismo mientras las dos partes se admitan."""
    b, s_, n = _partes(corto)
    con.execute("attach if not exists ':memory:' as %s" % _q(b))
    con.execute("create schema if not exists %s.%s" % (_q(b), _q(s_)))
    # Sin parámetros: un CREATE VIEW no se prepara. La fuente es nuestra (la
    # clave del artefacto o el puntero), ya escapada.
    con.execute("create or replace view %s.%s.%s as select * from %s" % (_q(b), _q(s_), _q(n), fuente))
    if s_ == DEFAULT:
        con.execute("create or replace view %s.main.%s as select * from %s.%s.%s" % (_q(b), _q(n), _q(b), _q(s_), _q(n)))


@_kw({"texto": "query", "como": "format"})
def sql(query, format="pandas"):
    """DuckDB SQL over the tree's datasets and views. ore-serve says which tree
    names the query reads (a name in a comment or a string does not count) and
    resolves them like `over()`; each one is a DuckDB view under its own name.
    Returns what `over()` returns for `format` (pandas, arrow or polars), or
    `None` for a statement without a result."""
    texto, como = query, format
    if not isinstance(texto, str) or not texto.strip():
        raise ValueError("sql() needs a query")
    con = _duckdb()
    # Sin puesto no hay árbol, y sin un punto no hay `a.b`: el motor solo.
    codigo, r = (session.pedir("POST", "/puestos/%s/sql" % session.id, {"texto": texto})
                 if session.id and "." in texto else (200, {}))
    if codigo != 200:
        _o_el_error(codigo, r, (r or {}).get("nombre") or "?")
    for nombre, rd in sorted(((r or {}).get("fuentes") or {}).items()):
        # 0049 B7·1: a collection is read by its listing, through ore-medios
        # (what a transform did not declare is a 403 there, as in Python).
        if (rd or {}).get("collection"):
            from .medios import _relacion
            tabla = "__ore_coleccion_%d" % abs(hash(nombre))
            con.register(tabla, _relacion(collection(nombre)))
            _registra(con, nombre, _q(tabla))
            continue
        _lee(nombre)
        fuente, _ = _fuente_de_respuesta(nombre, rd)
        _registra(con, nombre, fuente)
    # 0049 B7·2: the tree functions it calls, registered under the names
    # ore-serve rewrote them to, with their contract.
    if (r or {}).get("functions"):
        from .sql_functions import register

        register(con, r["functions"], lambda n: (get_function(n), _FUNCIONES_SPEC[n]))
        texto = r["query"]
    r = con.execute(texto)
    if r.description is None:
        return None
    return _como(_arrow(r), como)


# ── Escribir (0031 §11) ────────────────────────────────────────────────────
_DELEGAR = {"x-iceberg-access-delegation": "vended-credentials"}
ICEBERG = {"int8": "long", "int16": "long", "int32": "long", "int64": "long", "uint8": "long", "uint16": "long", "uint32": "long",
           "halffloat": "double", "float": "double", "double": "double", "bool": "boolean", "string": "string", "large_string": "string",
           "string_view": "string", "date32[day]": "date", "date64[ms]": "date"}


def _tipo_iceberg(columna, t, ids=None, en_lista=False):
    """El tipo de Iceberg del esquema con el que la tabla se esboza (lo mismo que
    `ore-store` hace al escribir, 0032): lo que el contrato no tiene se niega aquí,
    con el nombre de la columna, antes de mandar nada.

    v1alpha17 (0049 B1): lo anidado —un struct, una lista, un vector (una lista de
    tamaño fijo, como la da numpy)— va con su forma y un id en cada hijo, que saca
    de `ids` (un contador compartido por el esquema). Dentro de una lista un real
    de 32 bits es `float`: es lo que un vector es."""
    import pyarrow as pa

    if ids is None:
        ids = iter(range(10**6, 10**7))
    if pa.types.is_struct(t):
        if t.num_fields == 0:
            raise ValueError("write(): column `%s` is a struct without fields" % columna)
        hijos = [(t.field(i), next(ids)) for i in range(t.num_fields)]
        return {"type": "struct", "fields": [
            {"id": i, "name": f.name, "type": _tipo_iceberg("%s.%s" % (columna, f.name), f.type, ids), "required": False}
            for f, i in hijos]}
    if pa.types.is_list(t) or pa.types.is_large_list(t) or pa.types.is_fixed_size_list(t):
        e = t.value_type
        if pa.types.is_list(e) or pa.types.is_large_list(e) or pa.types.is_fixed_size_list(e):
            raise ValueError("write(): column `%s` is a list of lists: write it as a list of structs" % columna)
        i = next(ids)
        return {"type": "list", "element-id": i, "element": _tipo_iceberg(columna + "[]", e, ids, en_lista=True),
                "element-required": False}
    if en_lista and str(t) in ("halffloat", "float"):
        return "float"
    s = str(t)
    if s in ICEBERG:
        return ICEBERG[s]
    if pa.types.is_decimal128(t):
        return "decimal(%d, %d)" % (t.precision, t.scale)
    if pa.types.is_time(t):
        return "time"
    if pa.types.is_timestamp(t):
        return "timestamptz" if t.tz else "timestamp"
    if pa.types.is_dictionary(t) and pa.types.is_string(t.value_type):
        return "string"
    if s == "uint64":
        raise ValueError("write(): column `%s` is uint64, which does not fit in int64 without lying (0032); convert it first" % columna)
    if s == "null":
        raise ValueError("write(): column `%s` has no type (null): give it one first (0032)" % columna)
    raise ValueError("write(): column `%s` is `%s`, which the type contract (0032) does not have" % (columna, s))


def _arrow_de(datos):
    """Lo que se escribe, como `pyarrow.Table`: pandas, polars, Table o RecordBatch."""
    import pyarrow as pa

    if isinstance(datos, pa.Table):
        return datos
    if isinstance(datos, pa.RecordBatch):
        return pa.Table.from_batches([datos])
    if type(datos).__module__.startswith("polars") and hasattr(datos, "to_arrow"):
        return datos.to_arrow()
    try:
        import pandas as pd

        if isinstance(datos, pd.Series):
            datos = datos.to_frame()
        if isinstance(datos, pd.DataFrame):
            return pa.Table.from_pandas(datos, preserve_index=False)
    except ImportError:
        pass
    raise TypeError("write() takes a pandas or polars DataFrame, or an Arrow Table, not %s" % type(datos).__name__)


def _ipc(t):
    import pyarrow as pa

    sink = pa.BufferOutputStream()
    with pa.ipc.new_stream(sink, t.schema) as w:
        w.write_table(t)
    return sink.getvalue().to_pybytes()


def _ore_store(config, ubicacion):
    """El escritor y su entorno: `ore-store-gcs` con el token prestado si la tabla
    vive en `gs://`, `ore-store-r2` con las claves prestadas si en `s3://`."""
    import shutil

    env = dict(os.environ)
    if ubicacion.startswith("gs://"):
        nombre = "ore-store-gcs"
        env["ORE_GCS_BUCKET"] = ubicacion[5:].split("/", 1)[0]
        env["ORE_GCS_TOKEN"] = config.get("gcs.oauth2.token", "")
        if not env["ORE_GCS_TOKEN"]:
            raise RuntimeError("write(): the catalog vended no credential for `%s`" % ubicacion)
    elif ubicacion.startswith("s3://"):
        nombre = "ore-store-r2"
        env["ORE_R2_BUCKET"] = ubicacion[5:].split("/", 1)[0]
        env["ORE_R2_S3_ENDPOINT"] = config.get("s3.endpoint", "")
        env["ORE_R2_ACCESS_KEY_ID"] = config.get("s3.access-key-id", "")
        env["ORE_R2_SECRET_ACCESS_KEY"] = config.get("s3.secret-access-key", "")
        env["ORE_R2_REGION"] = config.get("s3.region", "auto")
    else:
        raise RuntimeError("write(): the table lives in `%s`, which is not a lake this SDK can write" % ubicacion)
    binario = shutil.which(nombre, path=os.environ.get("ORE_STORE_DIR") or None) or shutil.which(nombre)
    if not binario:
        raise RuntimeError("write(): `%s` is not on the PATH (the session image has it; elsewhere, ORE_STORE_DIR)" % nombre)
    return binario, env


def _escribir_ficheros(binario, env, peticion, ipc):
    import subprocess

    p = subprocess.run([binario, "escribir"], input=json.dumps(peticion).encode("utf-8") + b"\n" + ipc, capture_output=True, env=env)
    if p.returncode != 0:
        err = p.stderr.decode("utf-8", "replace").strip()
        raise RuntimeError("write(): %s" % (err.replace("error: ", "", 1) or "the writer failed"))
    return json.loads(p.stdout.decode("utf-8"))


def _mensaje(r):
    e = (r or {}).get("error")
    if isinstance(e, dict):
        return e.get("message", str(e))
    return str(e or r)


def _por_posicion(tabla_arrow, nombre, posiciones):
    """`insert into p.t select …` (el SQL del árbol, la celda que escribe
    ore-serve): lo que se escribe va por NOMBRE de columna, y una columna del
    `select` que no lo tiene —una expresión sin alias, `0.5`, `sum(x)`— toma el
    de la columna de la tabla en su misma posición, como en SQL (decidido
    2026-09-24). `posiciones` las dice el analizador (desde 0)."""
    nombre = _corto(nombre)
    c, r = session.pedir("GET", _v1_tabla(nombre), cabeceras=_DELEGAR)
    primera = posiciones[0] + 1
    if c == 404:
        raise RuntimeError("insert into %s: the table does not exist yet, and column %d of the select has no name "
                           "to take: give it one (`… as name`)" % (nombre, primera))
    if c != 200:
        raise RuntimeError("insert into %s: ore-serve answered %s: %s" % (nombre, c, _mensaje(r)))
    md = r["metadata"]
    esquema = next((s for s in md.get("schemas", []) if s.get("schema-id") == md.get("current-schema-id")), None) or md.get("schema") or {}
    de_la_tabla = [f["name"] for f in esquema.get("fields", [])]
    columnas = list(tabla_arrow.column_names)
    for p in posiciones:
        if p >= len(de_la_tabla):
            raise RuntimeError("insert into %s: column %d of the select has no name and the table only has %d: "
                               "give it one (`… as name`)" % (nombre, p + 1, len(de_la_tabla)))
        columnas[p] = de_la_tabla[p]
    return tabla_arrow.rename_columns(columnas)


# ── Lo que crea un guion SQL (0039) ────────────────────────────────────────
# `create … database`, `create schema` y `create dataset … (cols)`: cada uno, el
# verbo que ya existía —el alta de una base, `createNamespace` y `createTable` de
# `/v1`—, en nombre de quien abrió el puesto y en su rama. Con `si_no_existe`,
# lo que ya está no es un error (`if not exists`).

@_kw({"nombre": "name", "clase": "kind", "origen": "origin", "incluye": "include", "si_no_existe": "if_not_exists"})
def create_database(name, kind="standard", origin=None, include=None, if_not_exists=False):
    """`create [standard|foreign] database name [from origin [include (…)]]`.
    Without `origin`, an empty standard database. With `if_not_exists`, one
    that already exists is not an error. Returns `{database, kind, created}`."""
    nombre, clase, origen, incluye, si_no_existe = name, kind, origin, include, if_not_exists
    cuerpo = {"name": nombre, "type": clase}
    if origen:
        cuerpo["source"] = origen
        cuerpo["only"] = list(incluye or [])
    c, r = session.pedir("POST", "/paquetes", cuerpo, plazo=600)
    if c == 409 and si_no_existe:
        return _Result({"database": nombre, "kind": clase, "created": False})
    if c not in (200, 201):
        raise RuntimeError("create database %s: %s" % (nombre, _mensaje(r)))
    return _Result({"database": nombre, "kind": (r or {}).get("type", clase), "created": True})


@_kw({"base": "database", "si_no_existe": "if_not_exists"})
def create_schema(database, schema, if_not_exists=False):
    """`create schema database.schema` (`createNamespace` of `/v1`). Returns
    `{schema, created}`."""
    base, si_no_existe = database, if_not_exists
    c, r = session.pedir("POST", "/v1/%s/namespaces" % base, {"namespace": [schema], "properties": {}}, plazo=120)
    if c == 409 and si_no_existe:
        return _Result({"schema": "%s.%s" % (base, schema), "created": False})
    if c != 200:
        raise RuntimeError("create schema %s.%s: %s" % (base, schema, _mensaje(r)))
    return _Result({"schema": "%s.%s" % (base, schema), "created": True})


@_kw({"nombre": "name", "columnas": "columns", "clave": "key", "si_no_existe": "if_not_exists"})
def create_dataset(name, columns, key=None, if_not_exists=False):
    """`create dataset db.schema.name (col type, …[, primary key (…)])`: an
    empty dataset with its schema (`createTable` of `/v1`). `columns` is
    `[(name, Iceberg type)]`; `key`, the `primary key` columns —the ones an
    `insert or replace` (upsert) uses—, declared on the dataset. Returns
    `{dataset, created}`."""
    nombre, columnas, clave, si_no_existe = name, columns, key, if_not_exists
    nombre = _corto(nombre, "create dataset: the name")
    base, ns, t = _partes(nombre)
    esquema = {"type": "struct", "schema-id": 0, "fields": [
        {"id": i + 1, "name": n, "type": ti, "required": False} for i, (n, ti) in enumerate(columnas)]}
    cuerpo = {"name": t, "schema": esquema}
    if clave:
        cuerpo["properties"] = {"ore.clave": ",".join(clave)}
    c, r = session.pedir("POST", "/v1/%s/namespaces/%s/tables" % (base, ns), cuerpo, plazo=120)
    if c == 409 and si_no_existe:
        return _Result({"dataset": nombre, "created": False})
    if c != 200:
        raise RuntimeError("create dataset %s: %s" % (nombre, _mensaje(r)))
    return _Result({"dataset": nombre, "created": True})


# ── La colección escrita (ADR 0049 B4b·3) ──────────────────────────────────
# `create media collection` en código: el documento `MediaCollection` de
# v1alpha19 SIN `from` —la llena el código, por transacciones—, escrito por
# `PUT /documentos/MediaCollection/…` en nombre de quien abrió el puesto y en su
# rama, como `crear_vista`. No lleva `derivedFrom`: al crearla no se ha leído
# nada; lo escribe la herramienta al confirmar lo que se escriba en ella.

#: Los medios de una colección (v1alpha16 `02`): uno, y sabe lo que guarda.
MEDIOS = ("document", "image", "audio", "video", "spreadsheet", "email")


def _yaml_de_coleccion(nombre, media, formatos, dueno, comentario, etiquetas, retencion):
    base, ns, n = _partes(nombre)
    q = json.dumps  # un escalar de YAML entre comillas: el de JSON vale
    lineas = ["apiVersion: oos.dev/v1alpha19", "kind: MediaCollection", "metadata:",
              "  name: %s" % n, "  namespace: %s" % base]
    if ns != DEFAULT:
        lineas.append("  schema: %s" % ns)
    if comentario:
        lineas.append("  description: %s" % q(comentario, ensure_ascii=False))
    if etiquetas:
        lineas.append("  labels: { %s }" % ", ".join("%s: %s" % (k, v) for k, v in etiquetas.items()))
    lineas += ["spec:"] + _owner(dueno) + ["  media: %s" % media,
                                         "  formats: [%s]" % ", ".join(formatos)]
    if retencion:
        lineas.append("  retention: %s" % retencion)
    return "\n".join(lineas) + "\n"


@_kw({"nombre": "name", "formatos": "formats", "dueno": "owner", "comentario": "comment",
      "etiquetas": "labels", "retencion": "retention", "si_no_existe": "if_not_exists"})
def create_collection(name, media, formats, owner=None, comment=None, labels=None,
                      retention=None, if_not_exists=False):
    """`create media collection db.schema.c (media, formats)`: an empty
    **written** collection, which code fills with
    `ore.collection(name).transaction()`.

    `media` is one of `MEDIOS` (`document`, `image`, `audio`, `video`,
    `spreadsheet`, `email`); `formats`, the extensions it accepts (the first is
    the primary one). `labels` (`{"gdpr.sensitivity": "high"}`) add to what is
    derived: they can raise, not lower. If it already exists it is an error, or
    `{created: False}` with `if_not_exists`. An OOS code comes back as
    `ValueError`. Returns `{collection, created}`.

    `owner` only to give it to someone else (`user:…`, `team:…`): without it,
    it belongs to whoever creates it, set by the server."""
    nombre, formatos, dueno, comentario, etiquetas, retencion, si_no_existe = (
        name, formats, owner, comment, labels, retention, if_not_exists)
    nombre = _corto(_nombre_de(nombre), "create media collection: the name")
    que = "create media collection %s" % nombre
    if media not in MEDIOS:
        raise ValueError("%s: `media` is one of %s, not %r" % (que, ", ".join(MEDIOS), media))
    if isinstance(formatos, str):
        formatos = [formatos]
    formatos = [f.lower().lstrip(".") for f in formatos or []]
    if not formatos or len(set(formatos)) != len(formatos) or \
            not all(re.match(r"^[a-z0-9][a-z0-9.+-]*$", f) for f in formatos):
        raise ValueError("%s: `formats` is a list of distinct extensions (`png`, `pdf`), not %r" % (que, formatos))
    ruta = _ruta_de_vista(nombre, "MediaCollection")
    c, _ = session.pedir("GET", ruta, plazo=60)
    if c == 200:
        if si_no_existe:
            return _Result({"collection": nombre, "created": False})
        raise RuntimeError("%s: a collection with that name already exists (`if not exists` leaves it as it is)" % que)
    _poner(que, ruta, _yaml_de_coleccion(nombre, media, formatos, dueno, comentario, etiquetas, retencion))
    return _Result({"collection": nombre, "created": True})


# ── La vista (ADR 0040 paso 5) ─────────────────────────────────────────────
# `create view` guarda la consulta tal como se escribió (`spec.sql`) y su
# contrato (`spec.columns`), que no escribe nadie: lo describe DuckDB aquí, SIN
# LEER UNA FILA (medido, `medida-create-view.py`: <1 ms con 0 filas o con 10 M),
# sobre tablas vacías con los tipos de lo que la consulta lee —las del índice
# del árbol, como el servidor de lenguaje—. Así no hace falta credencial ni
# puntero, y una vista puede leer también una Table (virtual). Lo escribe
# `PUT /documentos/View/…` en nombre de quien abrió el puesto y en su rama, y el
# compilador lo coteja: un código OOS vuelve como el error de la celda.

# DuckDB → OOS (0032). Un entero es un entero —`sum(bigint)` es HUGEINT en
# DuckDB y aquí `Integer`: si un día no cabe en 64 bits, la copia falla con la
# columna nombrada, como en Databricks (ANSI) o BigQuery—; `avg` y `/` son
# DOUBLE, `Float`. Lo que OOS no tiene (STRUCT, MAP, UNION) es `None`.
_ENTEROS = {"TINYINT", "SMALLINT", "INTEGER", "BIGINT", "HUGEINT", "UTINYINT", "USMALLINT", "UINTEGER",
            "UBIGINT", "UHUGEINT", "INT", "INT1", "INT2", "INT4", "INT8", "INT16", "INT32", "INT64", "INT128"}
_DE_DUCKDB = {"DECIMAL": "Decimal", "NUMERIC": "Decimal", "DOUBLE": "Float", "FLOAT": "Float", "REAL": "Float",
              "FLOAT4": "Float", "FLOAT8": "Float", "BOOLEAN": "Boolean", "VARCHAR": "String", "UUID": "String",
              "ENUM": "String", "DATE": "Date", "TIME": "Time", "TIME WITH TIME ZONE": "Time", "TIMETZ": "Time",
              "TIMESTAMP": "DateTime", "TIMESTAMP_S": "DateTime", "TIMESTAMP_MS": "DateTime",
              "TIMESTAMP_NS": "DateTime", "TIMESTAMP WITH TIME ZONE": "DateTimeTz", "TIMESTAMPTZ": "DateTimeTz",
              "BLOB": "Opaque", "INTERVAL": "Opaque", "BIT": "Opaque", "JSON": "Opaque"}


def _oos_de_duckdb(tipo):
    """El tipo de OOS de un tipo de DuckDB (lo que da `describe`), o `None`."""
    t = (tipo or "").strip().upper()
    if t.endswith("[]"):
        dentro = _oos_de_duckdb(t[:-2])
        # `list<T>` es de escalares (02-entity §3.3): un decimal dentro va sin
        # su precisión, como el `ARRAY<NUMERIC>` de BigQuery.
        if dentro and dentro.startswith("Decimal<"):
            dentro = "Decimal"
        return "list<%s>" % dentro if dentro and not dentro.startswith("list<") else None
    base = re.sub(r"\(.*\)$", "", t).strip()
    if base in _ENTEROS:
        return "Integer"
    # `DECIMAL(18,3)` lleva su precisión al contrato (0032 T5): `Decimal<18, 3>`,
    # no `Decimal` a secas, que la copia leería como (38, 18).
    m = re.match(r"^(?:DECIMAL|NUMERIC)\((\d+),\s*(\d+)\)$", t)
    if m and 1 <= int(m.group(1)) <= 38:
        return "Decimal<%s, %s>" % (m.group(1), m.group(2))
    return _DE_DUCKDB.get(base)


_NOMBRE_DE_COLUMNA = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


def _rama_del_puesto():
    c, ficha = session.pedir("GET", "/puestos/%s" % session.id) if session.id else (0, None)
    rama = (ficha or {}).get("rama") if c == 200 else None
    return {"x-ore-rama": rama} if rama else None


def _describir(sql, que):
    """`[(columna, tipo de DuckDB)]` de la consulta, sin leer una fila: DuckDB la
    describe sobre tablas vacías con los tipos del índice del árbol."""
    from ore import lsp_sql

    c, indice = session.pedir("GET", "/assets", cabeceras=_rama_del_puesto(), plazo=60)
    if c != 200:
        raise RuntimeError("%s: could not read the tree index (GET /assets → %s)" % (que, c))
    cat = lsp_sql.Catalogo(indice)
    try:
        # una vista también lee una Table (es virtual): sus columnas, igual
        for n, i in cat.ajenas.items():
            cat.con.execute("attach if not exists ':memory:' as %s" % lsp_sql._q(i["paquete"]))
            cat.con.execute("create schema if not exists %s.%s" % (lsp_sql._q(i["paquete"]), lsp_sql._q(i["schema"])))
            cat.legibles.setdefault(n, i)
        leidos = [n for _, _, n, _ in lsp_sql.nombres([t[1] for t in lsp_sql.tokens(sql)])]
        with cat.candado:
            cat.asegurar(leidos)
        try:
            return [(r[0], r[1]) for r in cat.con.execute("describe " + sql).fetchall()]
        except Exception as e:  # el binder de DuckDB: una columna o un nombre que no está
            raise ValueError("%s: the query cannot be described: %s" % (que, str(e).strip().splitlines()[0])) from None
    finally:
        cat.cerrar()


def _yaml_de_vista(nombre, sql, contrato, comentarios, comentario, dueno):
    base, ns, v = _partes(nombre)
    lineas = ["apiVersion: oos.dev/v1alpha14", "kind: View", "metadata:", "  name: %s" % v, "  namespace: %s" % base]
    if ns != DEFAULT:
        lineas.append("  schema: %s" % ns)
    if comentario:
        lineas.append("  description: %s" % json.dumps(comentario, ensure_ascii=False))
    lineas += ["spec:"] + _owner(dueno) + ["  dialect: duckdb", "  sql: |"]
    lineas += ["    " + l if l.strip() else "" for l in sql.replace("\r\n", "\n").strip("\n").split("\n")]
    lineas.append("  columns:")
    for c, t in contrato.items():
        d = comentarios.get(c)
        lineas.append("    %s: { type: %s%s }" % (c, json.dumps(t), ", description: %s" % json.dumps(d, ensure_ascii=False) if d else ""))
    return "\n".join(lineas) + "\n"


def _owner(dueno):
    """La línea `owner` de un `spec`, si se dice. Sin ella, el dueño lo pone el
    servidor (0052 · Ownership): quien lo crea —la persona que abrió el puesto—,
    o el que ya tenía si se reescribe. El SDK no inventa uno."""
    return ["  owner: %s" % dueno] if dueno else []


def _ruta_de_vista(nombre, kind="View"):
    base, ns, v = _partes(nombre)
    return ("/documentos/%s/%s/%s" % (kind, base, v) if ns == DEFAULT
            else "/documentos/%s/%s/%s/%s" % (kind, base, ns, v))


@_kw({"nombre": "name", "columnas": "columns", "comentario": "comment", "dueno": "owner",
      "o_reemplaza": "or_replace", "si_no_existe": "if_not_exists", "evolucion": "schema_evolution",
      "existe": "exists", "anterior": "previous_columns", "materializada": "materialized"})
def create_view(name, sql, columns=None, comment=None, owner=None, or_replace=False,
                if_not_exists=False, schema_evolution=False, exists=None, previous_columns=None,
                materialized=False):
    """`create [or replace] view [if not exists] db.schema.v [(col [comment '…'], …)]
    [comment '…'] [with schema evolution] as <sql>` (ADR 0040 step 5).

    The contract (column types) is described by DuckDB without reading a row.
    `columns`, `[(name, comment)]`, renames the select's columns by position.
    `exists` and `previous_columns` (the contract it had, `{column: type}`) are
    given by `ore-serve` when it writes the cell. Replacing a view may ADD
    columns; removing one or changing its type breaks its readers, so it needs
    `schema_evolution` (`with schema evolution`). Returns
    `{view, status: created|replaced|already exists, columns}`.

    `materialized` (`create materialized view`, ADR 0040 step 7): after the
    view, its copy —the dataset `<view>_copia`, `from: { view }`—; whoever
    reads the view reads its copy. Then it also returns `copy`."""
    nombre, columnas, comentario, dueno = name, columns, comment, owner
    o_reemplaza, si_no_existe, evolucion = or_replace, if_not_exists, schema_evolution
    existe, anterior, materializada = exists, previous_columns, materialized
    nombre = _corto(nombre, "create view: the name")
    que = "create view %s" % nombre
    if existe and si_no_existe:
        return _Result({"view": nombre, "status": "already exists", "columns": anterior})
    if existe and not o_reemplaza:
        raise RuntimeError("%s: a view with that name already exists (`create or replace view` replaces it)" % que)
    descritas = _describir(sql, que)
    if columnas:
        if len(columnas) != len(descritas):
            raise ValueError("%s: the list names %d columns and the query gives %d" % (que, len(columnas), len(descritas)))
        descritas = [(c[0], t) for c, (_, t) in zip(columnas, descritas)]
    comentarios = {c[0]: c[1] for c in (columnas or []) if len(c) > 1 and c[1]}
    contrato, vistos = {}, set()
    for c, t in descritas:
        if not _NOMBRE_DE_COLUMNA.match(c):
            raise ValueError("%s: column `%s` has no name: give it one (`… as name`), or name them in the "
                             "list: `create view %s (a, b, …) as …`" % (que, c, nombre))
        if c.lower() in vistos:
            raise ValueError("%s: `%s` appears twice: each column of the contract needs its own name (`… as other`)" % (que, c))
        vistos.add(c.lower())
        oos = _oos_de_duckdb(t)
        if oos is None:
            raise ValueError("%s: `%s` is %s, and OOS has no such type: take its fields out (`s.field as x`) or "
                             "turn it into text (`to_json(s) as x`)" % (que, c, t))
        contrato[c] = oos
    estado = "replaced" if existe else "created"
    if existe and anterior:
        quitadas = [c for c in anterior if c not in contrato]
        cambiadas = ["%s (%s → %s)" % (c, anterior[c], contrato[c]) for c in anterior if c in contrato and contrato[c] != anterior[c]]
        if (quitadas or cambiadas) and not evolucion:
            rompe = (["removes " + ", ".join(quitadas)] if quitadas else []) +                     (["changes " + ", ".join(cambiadas)] if cambiadas else [])
            raise ValueError("%s: replacing it breaks its contract —%s— and its readers stop finding what they read. "
                             "If that is what you want: `create or replace view … with schema evolution as …`"
                             % (que, "; ".join(rompe)))
        nuevas = [c for c in contrato if c not in anterior]
        if nuevas:
            print("%s · adds %s to the contract" % (nombre, ", ".join(nuevas)))
    texto = _yaml_de_vista(nombre, sql, contrato, comentarios, comentario, dueno)
    _poner(que, _ruta_de_vista(nombre), texto)
    hecho = _Result({"view": nombre, "status": estado, "columns": contrato})
    if materializada:
        copia = nombre + "_copia"
        _poner(que, _ruta_de_vista(copia, "Dataset"), _yaml_de_copia(copia, nombre, dueno))
        hecho["copy"] = copia
    return hecho


def _poner(que, ruta, texto):
    """`PUT /documentos/…` con el YAML tal cual; un código OOS es el error."""
    c, r = session.pedir("PUT", ruta, {"yaml": texto}, plazo=120)
    if c not in (200, 201):
        r = r or {}
        if r.get("diagnosticos"):
            raise ValueError("%s: %s" % (que, "; ".join("%s: %s" % (d.get("codigo", "?"), d.get("mensaje", "")) for d in r["diagnosticos"])))
        raise RuntimeError("%s: %s (%s)" % (que, r.get("error", "?"), c))


def _yaml_de_copia(copia, vista, dueno):
    """El dataset que copia entera la vista `vista` (ADR 0040 decisión D)."""
    base, ns, n = _partes(copia)
    lineas = ["apiVersion: oos.dev/v1alpha14", "kind: Dataset", "metadata:", "  name: %s" % n, "  namespace: %s" % base]
    if ns != DEFAULT:
        lineas.append("  schema: %s" % ns)
    lineas += ["spec:"] + _owner(dueno) + ["  from: { view: %s }" % vista]
    return "\n".join(lineas) + "\n"


@_kw({"nombre": "name", "si_existe": "if_exists"})
def drop_view(name, if_exists=False):
    """`drop view [if exists] db.schema.v`: removes it from the tree on the
    session's branch. If something reads it, the tree would get worse and it
    is not removed: the OOS code says why. Returns
    `{view, status: dropped|not found}`."""
    nombre, si_existe = name, if_exists
    nombre = _corto(nombre, "drop view: the name")
    c, r = session.pedir("DELETE", _ruta_de_vista(nombre), plazo=120)
    if c in (200, 204):
        return _Result({"view": nombre, "status": "dropped"})
    if c == 404 and si_existe:
        return _Result({"view": nombre, "status": "not found"})
    r = r or {}
    if r.get("diagnosticos"):
        raise ValueError("drop view %s: %s" % (nombre, "; ".join("%s: %s" % (d.get("codigo", "?"), d.get("mensaje", "")) for d in r["diagnosticos"])))
    raise RuntimeError("drop view %s: %s (%s)" % (nombre, r.get("error", "?"), c))


_ARROW_DE_ICEBERG = {"long": "int64", "int": "int32", "string": "string", "boolean": "bool", "double": "float64",
                     "float": "float32", "date": "date32", "binary": "binary"}


def _arrow_de_iceberg(tipo):
    import pyarrow as pa

    if tipo in _ARROW_DE_ICEBERG:
        return pa.type_for_alias(_ARROW_DE_ICEBERG[tipo])
    if tipo == "timestamp":
        return pa.timestamp("us")
    if tipo == "timestamptz":
        return pa.timestamp("us", tz="UTC")
    if tipo == "time":
        return pa.time64("us")
    m = re.match(r"decimal\((\d+),\s*(\d+)\)$", tipo or "")
    if m:
        return pa.decimal128(int(m.group(1)), int(m.group(2)))
    return None


def _como_la_tabla(tabla_arrow, nombre):
    """`insert into b.s.d …` (el guion SQL, 0039): cada valor, al tipo de su
    columna en el dataset, como hace SQL al insertar —`current_timestamp` es
    TIMESTAMPTZ y la columna puede ser TIMESTAMP; `1` es INTEGER y la columna
    BIGINT—. Lo que no se convierte sin perder se deja, y `write()` dice por qué
    no cabe. Si el dataset aún no existe, sus tipos son los de lo que llega."""
    import pyarrow as pa

    nombre = _corto(nombre)
    c, r = session.pedir("GET", _v1_tabla(nombre), cabeceras=_DELEGAR)
    if c != 200:
        return tabla_arrow
    md = r["metadata"]
    esquema = next((s for s in md.get("schemas", []) if s.get("schema-id") == md.get("current-schema-id")), None) or md.get("schema") or {}
    tipos = {f["name"]: f["type"] for f in esquema.get("fields", []) if isinstance(f.get("type"), str)}
    columnas = []
    for campo, col in zip(tabla_arrow.schema, tabla_arrow.columns):
        destino = _arrow_de_iceberg(tipos.get(campo.name))
        if destino is not None and campo.type != destino:
            try:
                col = col.cast(destino)
            except (pa.ArrowInvalid, pa.ArrowNotImplementedError):
                pass
        columnas.append(col)
    return pa.table(columnas, names=tabla_arrow.column_names)


#: Los modos de `write()`: los de ahora y los de antes → la palabra que viaja.
_MODOS = {"overwrite": "sobrescribir", "append": "anexar", "upsert": "upsert",
          "sobrescribir": "sobrescribir", "anexar": "anexar"}
_MODOS_ES = {"sobrescribir": "overwrite", "anexar": "append"}


def _resultado_de_escritura(escrito):
    """El resultado de una sentencia que escribe (el guion SQL, 0039), como en
    Databricks: una tabla de una fila. `num_inserted_rows` son las filas que
    llegaron —no el total del dataset—; en un upsert, las que ya estaban por su
    clave son `num_updated_rows` (antes + llegan − después). La misma escritura
    otra vez no deja nada nuevo: ceros."""
    import pyarrow as pa

    llegan = 0 if escrito.get("repeated") else int(escrito.get("added") or 0)
    if escrito.get("mode") == "upsert":
        actualizadas = 0 if escrito.get("repeated") else max(0, int(escrito.get("before") or 0) + llegan - int(escrito.get("rows") or 0))
        return pa.table({"num_affected_rows": pa.array([llegan], pa.int64()),
                         "num_updated_rows": pa.array([actualizadas], pa.int64()),
                         "num_inserted_rows": pa.array([llegan - actualizadas], pa.int64())})
    return pa.table({"num_affected_rows": pa.array([llegan], pa.int64()),
                     "num_inserted_rows": pa.array([llegan], pa.int64())})


def _sql_per_item(output, coll, query, name):
    """**A query over a collection, computed item by item** (0049 B7·3): what
    `create or replace dataset … as select … from <collection>` runs. It is
    `collection(coll).apply()`: for each item still to compute, `query` runs
    with the collection holding that item alone, and its rows —with their
    `anchor`, if they have one— are that item's. Same registry by key as in
    Python: what did not change is neither computed nor written. The version
    is the query and the document of each function it calls."""
    import hashlib
    import threading

    import duckdb

    from .medios import _canonico, _relacion_de_refs
    from .sql_functions import register

    codigo, r = session.pedir("POST", "/puestos/%s/sql" % session.id, {"texto": query})
    if codigo != 200:
        _o_el_error(codigo, r, (r or {}).get("nombre") or "?")
    r = r or {}
    calls = r.get("functions") or []
    texto = r.get("query") or query
    colecciones = [n for n, rd in (r.get("fuentes") or {}).items() if (rd or {}).get("collection")]
    funciones = {c["name"]: get_function(c["name"]) for c in calls}
    huella = hashlib.sha256(_canonico({
        "query": " ".join(query.split()),
        "functions": {n: _FUNCIONES_SPEC.get(n) for n in sorted(funciones)},
    }).encode("utf-8")).hexdigest()[:16]
    hilo = threading.local()

    def conexion():
        # One per thread: `apply()` computes items in parallel, and each item is
        # the collection of ITS connection.
        if not hasattr(hilo, "con"):
            con = duckdb.connect()
            con.execute("set TimeZone = 'UTC'")
            con.execute("set autoinstall_known_extensions = false")
            con.execute("set memory_limit='%dMB'" % max(256, _tropo_mb() // 4))
            register(con, calls, lambda n: (funciones[n], _FUNCIONES_SPEC[n]))
            hilo.con = con
        return hilo.con

    def per_item(item):
        con = conexion()
        con.register("__ore_item", _relacion_de_refs([item.ref]))
        for n in colecciones:
            _registra(con, n, _q("__ore_item"))
        return _arrow(con.execute(texto)).to_pylist()

    per_item.__name__ = name
    return collection(coll).apply(per_item, version="sql:" + huella, output=output)


def _resultado_de_aplicar(hecho):
    """The result of a statement that writes an anchored dataset (0049 B7·3):
    one row with what `apply()` did."""
    import pyarrow as pa

    claves = ("items", "new", "recomputed", "skipped", "errors", "removed", "rows")
    return pa.table({k: pa.array([int(hecho.get(k) or 0)], pa.int64()) for k in claves})


def _resultado_de_crear(objeto, creado):
    """El resultado de una sentencia que crea (el guion SQL, 0039): qué, y si se
    creó o ya estaba (`if not exists`). `creado` es un booleano, o el estado ya
    dicho (`replaced`, `dropped`…: la vista, ADR 0040 paso 5)."""
    import pyarrow as pa

    estado = creado if isinstance(creado, str) else ("created" if creado else "already exists")
    return pa.table({"object": [objeto], "status": [estado]})


@_kw({"nombre": "name", "datos": "data", "modo": "mode", "clave": "key", "anclada_a": "anchored_to"})
def write(name, data, mode="overwrite", key=None, anchored_to=None):
    """Write `data` (a pandas or polars DataFrame, or an Arrow Table) as the
    lake dataset `<database>.<schema>.<name>`. The code never touches the
    bucket: the table goes to `ore-store` with a credential the catalog vends,
    and the commit goes through ore-serve's Iceberg REST catalog.

    `mode`: `"overwrite"` (default), `"append"` or `"upsert"` (with
    `key=[…]`, the columns that identify a row; the key is then declared on
    the table). The old values `"sobrescribir"` and `"anexar"` still work.
    Idempotent: the same table to the same name and mode again leaves no new
    snapshot. Returns `{table, rows, snapshot, metadata_location, operation,
    repeated, mode, added, before}`.

    `anchored_to="db.schema.collection"` (0049 B5·1): an **anchored table**
    on that collection (`_item`, `_anchor`, `_anchor_id`, `_anchor_parent`,
    `_derivation`, `_status`); it merges by `_anchor_id`, so no `upsert`.
    `Collection.apply()` writes it."""
    import hashlib

    nombre, datos, clave, anclada_a = name, data, key, anchored_to
    if mode not in _MODOS:
        raise ValueError("mode=%r: use `overwrite`, `append` or `upsert`" % (mode,))
    if mode in _MODOS_ES:
        _avisar("write(mode=%r)" % mode, "write(mode=%r)" % _MODOS_ES[mode])
    # Lo que viaja (la petición a ore-store y la semilla de la clave de
    # operación) es la palabra de siempre: el protocolo no cambia.
    modo = _MODOS[mode]
    mode = _MODOS_ES.get(mode, mode)
    nombre = _corto(nombre, "write(): the name")
    if anclada_a is not None:
        anclada_a = _corto(_nombre_de(anclada_a), "write(): `anchored_to`")
        if modo == "upsert":
            raise ValueError("write(): an anchored table merges by `_anchor_id`, not by upsert")
    if clave is not None and (isinstance(clave, str) or not all(isinstance(c, str) for c in clave)):
        raise ValueError("key=%r: a list of column names" % (clave,))
    if clave is not None and modo != "upsert":
        raise ValueError("`key` goes with mode=\"upsert\"")
    clave_upsert = list(clave) if clave else None
    if _transform is not None and nombre != _transform.output:
        raise PermissionError("`%s` is not the output of `%s` (%s): a transform only writes what it declares" % (nombre, _transform.nombre, _transform.output))
    base, ns, t = _partes(nombre)  # el namespace de /v1 es el schema (0038 P4)
    tabla_arrow = _arrow_de(datos)
    if tabla_arrow.num_rows == 0:
        raise ValueError("write(): the table has no rows")
    # Los ids como los reparte Iceberg al crear: primero las columnas, luego los
    # hijos de cada una (v1alpha17, 0049 B1).
    n = len(tabla_arrow.schema)
    ids = iter(range(n + 1, 10**7))
    esquema = {"type": "struct", "schema-id": 0, "fields": [
        {"id": i + 1, "name": f.name, "type": _tipo_iceberg(f.name, f.type, ids), "required": False} for i, f in enumerate(tabla_arrow.schema)]}
    ipc = _ipc(tabla_arrow)
    # La clave de operación la calcula el escritor DEL CONTENIDO (los valores,
    # no los bytes del IPC, que llevan relleno y cambian entre dos lecturas de
    # lo mismo), con esta semilla: la misma tabla al mismo nombre y modo es la
    # misma escritura, y el catálogo no la repite.
    semilla = "%s|%s" % (nombre, modo) + ("|" + ",".join(clave_upsert) if clave_upsert else "")
    clave = None
    dataset = "catalogo/%s/%s/%s" % (base, ns, t)  # una etiqueta: la ubicación la da el catálogo

    def cargar():
        c, r = session.pedir("GET", _v1_tabla(nombre), cabeceras=_DELEGAR)
        if c == 200:
            # Prestado sólo para leer (lo de otra persona, un mantenido): el
            # porqué, antes de escribir un fichero con una credencial que no escribe.
            if (r.get("config") or {}).get("ore.solo-lectura"):
                raise RuntimeError("write(%s): %s" % (nombre, r["config"]["ore.solo-lectura"]))
            return r["metadata-location"], None, r.get("config", {}), r["metadata"]["location"]
        if c == 404:
            c, r = session.pedir("POST", "/v1/%s/namespaces/%s/tables" % (base, ns), {"name": t, "stage-create": True, "schema": esquema, "properties": {}}, cabeceras=_DELEGAR)
            if c != 200:
                raise RuntimeError("write(%s): %s" % (nombre, _mensaje(r)))
            return None, r["metadata"], r.get("config", {}), r["metadata"]["location"]
        raise RuntimeError("write(%s): ore-serve answered %s: %s" % (nombre, c, _mensaje(r)))

    global _s3
    for intento in range(4):
        base, esbozo, config, ubicacion = cargar()
        if config.get("s3.access-key-id"):
            _s3 = config
        binario, env = _ore_store(config, ubicacion)
        peticion = {"dataset": dataset, "modo": modo, "operacion": "contenido", "semilla": semilla, "procedencia": _procedencia(nombre, anclada_a)}
        if clave_upsert:
            peticion["clave"] = clave_upsert
        if base:
            peticion["base"] = base
        else:
            peticion["esbozo"] = esbozo
        escrito = _escribir_ficheros(binario, env, peticion, ipc)
        clave = escrito.get("operacion") or clave
        c, r = session.pedir("POST", _v1_tabla(nombre),
                            {"identifier": {"namespace": [ns], "name": t}, "requirements": escrito["requirements"], "updates": escrito["updates"]}, plazo=120)
        if c == 200:
            snap = ((r or {}).get("metadata") or {}).get("current-snapshot-id")
            # la misma operación ya estaba: el catálogo contesta con lo que hay
            # (el mismo puntero) y no deja nada
            repetida = base is not None and (r or {}).get("metadata-location") == base
            return _Result({"table": nombre, "rows": escrito["filas"], "snapshot": str(snap or ""),
                            "metadata_location": (r or {}).get("metadata-location", ""),
                            "operation": clave, "repeated": repetida, "mode": mode,
                            "added": escrito.get("anadidas", 0), "before": escrito.get("antes", 0)})
        if c == 409:
            # alguien escribió mientras tanto (o la tabla nació): otra vez sobre lo que hay
            continue
        if c >= 500:
            # el commit pudo entrar: se MIRA antes de reintentar
            c2, r2 = session.pedir("GET", _v1_tabla(nombre))
            if c2 == 200:
                md = r2["metadata"]
                vigente = [s for s in md.get("snapshots", []) if s.get("snapshot-id") == md.get("current-snapshot-id")]
                if vigente and vigente[0].get("summary", {}).get("ore.operacion") == clave:
                    return _Result({"table": nombre, "rows": escrito["filas"], "snapshot": str(md.get("current-snapshot-id")),
                                    "metadata_location": r2["metadata-location"], "operation": clave, "repeated": False,
                                    "mode": mode, "added": escrito.get("anadidas", 0), "before": escrito.get("antes", 0)})
            raise RuntimeError("write(%s): the catalog answered %s and the commit is not there: %s" % (nombre, c, _mensaje(r)))
        raise RuntimeError("write(%s): %s" % (nombre, _mensaje(r)))
    raise RuntimeError("write(%s): four times someone else wrote first; try again" % nombre)


# ── El JSON de la consola (0032 §1) ───────────────────────────────────────
# ── 0046 E9·4 · Los ficheros de una colección ──────────────────────────────
#
# Una propiedad `Media<c>` de una Entity guarda **la huella** de un ítem de la
# colección `c`: un texto. Servirlo es pedirle a ore-serve una URL firmada, que
# vive unos minutos y se abre sin credencial (el lago, o el origen si la
# colección es virtual, dan los bytes, a rangos). Quién puede, lo decide
# ore-serve; lo que se sirve queda en la actividad de la organización.
#
#   ore.media_de("legal.registro")                → {"documento": "legal.archivo.contratos"}
#   ore.media("legal.archivo.contratos", huella)  → {"url": …, "tipo": "application/pdf", …}
#   ore.medias("legal.archivo.contratos", huellas) → {huella: {…}, …}  (de cien en cien)

def _coleccion(coleccion):
    """`base.schema.nombre` (o `base.nombre`) → la ruta de la colección."""
    b, s_, n = _partes(_corto(coleccion, "media_url(): the collection"))
    return "/colecciones/%s/%s/%s/items" % (b, s_, n)


def _servido(codigo, r, que):
    if codigo == 200:
        return r
    if codigo == 404:
        raise LookupError("%s: %s" % (que, (r or {}).get("error", "not found")))
    if codigo == 403:
        raise PermissionError("%s: %s" % (que, (r or {}).get("error", "not allowed to serve it")))
    raise RuntimeError("ore-serve answered %s for %s: %s" % (codigo, que, (r or {}).get("error", r)))


@_kw({"vista": "view"})
def media_columns(view):
    """Which collection each `Media<c>` column of `view` points to (what the
    backing Entity declares): `{column: "db.schema.collection"}`."""
    return dict((_resolver(view) or {}).get("media") or {})


@_kw({"coleccion": "collection", "huella": "fingerprint"})
def media_url(collection, fingerprint, ttl=None):
    """The item of `collection` with this `fingerprint`, served:
    `{url, content_type, disposition, seconds, expires_ms, path, …}`. The `url`
    opens without a credential for `ttl` seconds (300 by default; 30 to 3600):
    do not store or share it."""
    coleccion, huella = collection, fingerprint
    ruta = _coleccion(coleccion)
    if ttl is None:
        from urllib.parse import quote
        codigo, r = session.pedir("GET", "%s/%s" % (ruta, quote(huella, safe="")))
        return _en(_servido(codigo, r, "media_url(%s, %s)" % (coleccion, huella)))
    item = media_urls(coleccion, [huella], ttl=ttl).get(huella)
    if item is None:
        raise LookupError("media_url(%s, %s): no item has that fingerprint" % (coleccion, huella))
    return item


@_kw({"coleccion": "collection", "huellas": "fingerprints"})
def media_urls(collection, fingerprints, ttl=None):
    """The items of several fingerprints —a list, a gallery— in batches of a
    hundred: `{fingerprint: {url, content_type, …}}`. Missing ones are left out."""
    coleccion, huellas = collection, fingerprints
    ruta = _coleccion(coleccion) + "/resolver"
    huellas = list(dict.fromkeys(h for h in huellas if h))
    out = {}
    for i in range(0, len(huellas), 100):
        cuerpo = {"huellas": huellas[i:i + 100]}
        if ttl is not None:
            cuerpo["ttl"] = str(int(ttl))
        codigo, r = session.pedir("POST", ruta, cuerpo)
        r = _servido(codigo, r, "media_urls(%s)" % coleccion)
        for it in r.get("items", []):
            it.setdefault("segundos", r.get("segundos"))
            it.setdefault("caduca_ms", r.get("caduca_ms"))
            out[it["huella"]] = _en(it)
    return out


@_kw({"valor": "value", "limite": "limit"})
def table(value, limit=200):
    """A DataFrame (pandas or polars), a Series or an Arrow Table → the console's
    `tabla` output (0032 §1): `columnas` (each column's Arrow type), `filas`
    (the first `limit` rows, as contract JSON), `total` and `limite`. Its keys
    are the console's wire format, shared with the Node and Java agents, so
    they stay as they are. Anything else → `None`."""
    valor, limite = value, limit
    import pyarrow as pa

    t = None
    if isinstance(valor, pa.Table):
        t = valor
    elif isinstance(valor, pa.RecordBatch):
        t = pa.Table.from_batches([valor])
    else:
        try:
            import pandas as pd
            if isinstance(valor, pd.Series):
                valor = valor.to_frame()
            if isinstance(valor, pd.DataFrame):
                # Con `ArrowDtype` (lo que `over()` da) es la misma memoria; con
                # tipos de numpy se convierte, y `preserve_index=False` porque
                # el índice no es una columna de la copia.
                t = pa.Table.from_pandas(valor, preserve_index=False)
        except ImportError:
            pass
        if t is None and type(valor).__module__.startswith("polars") and hasattr(valor, "to_arrow"):
            t = valor.to_arrow()
    if t is None:
        return None
    total = t.num_rows
    cabeza = t.slice(0, limite)
    columnas = [{"name": f.name, "type": str(f.type)} for f in cabeza.schema]
    por_columna = [[_a_json(v, f.type) for v in cabeza.column(i).to_pylist()] for i, f in enumerate(cabeza.schema)]  # noqa: E501
    filas = [list(f) for f in zip(*por_columna)] if por_columna else []
    return {"columnas": columnas, "filas": filas, "total": total, "limite": limite}


ENTERO_EXACTO = 2 ** 53


def _iso(v):
    """ISO 8601 con `T`, segundos siempre y la fracción sólo si no es cero, sin
    ceros de más: `12:00:00.5`, no `12:00:00.500000` — lo mismo que Node y Java."""
    s = v.isoformat()
    if "." in s:
        s = s.rstrip("0").rstrip(".")
    return s


@_kw({"tipo": "oos_type"})
def to_json(v, oos_type=None):
    """An Arrow value (already in Python) → the contract's JSON (0032 §1):
    integer → number if |x| ≤ 2⁵³, else string · decimal → always a string
    (except scale 0, which is an integer) · float → number, with
    `NaN`/`Infinity`/`-Infinity` as strings · date `YYYY-MM-DD` · time
    `HH:MM:SS[.ffffff]` · naive datetime in ISO with `T` · instant in UTC with
    `Z` · bytes in base64 · list → array · struct → object · map →
    `[{key, value}]`. `oos_type` is the Arrow type of the column, if known.
    Nothing is silently degraded: what does not fit a JSON number goes as a
    string."""
    return _a_json(v, oos_type)


def _a_json(v, tipo=None):
    import datetime as dt
    import decimal

    if v is None:
        return None
    if isinstance(v, bool):
        return v
    if isinstance(v, int):
        return v if -ENTERO_EXACTO <= v <= ENTERO_EXACTO else str(v)
    if isinstance(v, float):
        if v != v:
            return "NaN"
        if v in (float("inf"), float("-inf")):
            return "Infinity" if v > 0 else "-Infinity"
        return v
    if isinstance(v, str):
        return v
    if isinstance(v, decimal.Decimal):
        # Un decimal de escala 0 (un HUGEINT de DuckDB, `sum(1)`, `count`) es un
        # entero y va como los enteros; con decimales, cadena siempre.
        if v == v.to_integral_value() and (tipo is None or getattr(tipo, "scale", 0) == 0) and -ENTERO_EXACTO <= v <= ENTERO_EXACTO:
            return int(v)
        return format(v, "f")
    if isinstance(v, dt.datetime):
        if v.tzinfo is not None:
            return _iso(v.astimezone(dt.timezone.utc).replace(tzinfo=None)) + "Z"
        return _iso(v)
    if isinstance(v, dt.date):
        return v.isoformat()
    if isinstance(v, dt.time):
        return _iso(v)
    if isinstance(v, (bytes, bytearray)):
        import base64
        return base64.b64encode(bytes(v)).decode("ascii")
    if isinstance(v, list):
        # Un map de Arrow llega como lista de pares (tuplas).
        if v and isinstance(v[0], tuple) and len(v[0]) == 2:
            return [{"key": _a_json(k), "value": _a_json(x)} for k, x in v]
        return [_a_json(x) for x in v]
    if isinstance(v, dict):
        return {str(k): _a_json(x) for k, x in v.items()}
    if hasattr(v, "item"):
        try:
            return _a_json(v.item())
        except (ValueError, AttributeError):
            pass
    return str(v)


# 0049 B3·5: la media en código (al final: `medios` usa `session` y los nombres).
from .medios import (collection, Collection, Item, MediaRef, read_many, Transaction, MediaError,  # noqa: E402
                     MediaNotFound, MediaForbidden, MediaChanged, MediaCorrupt, MediaRangeError,
                     MediaNotWritable, MediaTransactionError)

#: Los nombres de antes → los de ahora: `ore.crear_coleccion is ore.create_collection`.
_ALIAS = {
    "persona": "person", "puesto": "session", "Puesto": "Session",
    "funcion": "get_function", "modelo": "model", "Modelo": "Model",
    "crear_base": "create_database", "crear_schema": "create_schema", "crear_dataset": "create_dataset",
    "crear_coleccion": "create_collection", "crear_vista": "create_view", "borrar_vista": "drop_view",
    "media_de": "media_columns", "media": "media_url", "medias": "media_urls",
    "tabla": "table", "json_de": "to_json",
    "coleccion": "collection", "Coleccion": "Collection", "leer_varios": "read_many",
    "Transaccion": "Transaction",
    "MediaNoExiste": "MediaNotFound", "MediaSinPermiso": "MediaForbidden", "MediaCambiado": "MediaChanged",
    "MediaCorrupto": "MediaCorrupt", "MediaRango": "MediaRangeError", "MediaNoEscribible": "MediaNotWritable",
    "MediaTransaccion": "MediaTransactionError",
}


def __getattr__(nombre):
    """Un nombre de antes (`ore.crear_coleccion`, `from ore import puesto`): el
    mismo objeto que el de ahora."""
    nuevo = _ALIAS.get(nombre)
    if nuevo is None:
        raise AttributeError("module 'ore' has no attribute %r" % (nombre,))
    _avisar("ore.%s" % nombre, "ore.%s" % nuevo)
    return globals()[nuevo]


def __dir__():
    return sorted(set(globals()) | set(_ALIAS))
