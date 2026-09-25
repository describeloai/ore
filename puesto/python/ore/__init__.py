"""
`ore` · el SDK del puesto (0031 W3.1).

Lo que una celda importa. `over("<paquete>.<vista>")` devuelve la copia de esa
vista como DataFrame **con tipos de Arrow** (0032 T3: `pd.ArrowDtype`, así que un
entero con nulos sigue siendo entero, un `Decimal` es exacto y un instante lleva
su zona); `como="arrow"` da la `pyarrow.Table` y `como="polars"` un DataFrame de
polars si está en la capa. El código nunca ve el bucket ni una credencial:
pregunta a `ore-serve` QUÉ copia es (con la identidad del puesto, que la
resuelve en nombre de la persona y con su potestad) y baja el artefacto con la
identidad del pod (Workload Identity). El sobre `ORECOPY1` se desenvuelve aquí;
la carga es Parquet.

`persona()` (W3.4) dice quién abrió el puesto: la identidad con la que corre
lo que haces aquí.

`sql("select … from hr.espanoles")` (W3.3) pregunta a los datasets por su
nombre: ore-serve dice qué nombres del árbol lee el texto (`POST /puestos/{id}/sql`,
sin regex) y los resuelve igual que en `over()`; cada uno queda en DuckDB como
la vista `paquete.nombre`; devuelve un DataFrame. Medido en victor (2 CPU · 3 GB):
200 M de filas, `group by` con agregados en 1,9 s, `where` en 1 s.

Fuera del clúster (las pruebas de fuego) el almacén es un directorio:
`ORE_ALMACEN=dir:/ruta` lee `ore/v1/<clave>` de ahí.

**Todo es un dataset** (0031 §10): lo que `ore-serve` contesta por `datos` es o bien
`metadata_location` —el `metadata.json` vigente de una **tabla Iceberg** en el
bucket: se lee en sitio con DuckDB (`iceberg_scan` sobre la raíz y la versión, con el
token del pod como *bearer*; medido en `medida-w3-lago.py`: 10 M de filas, filtro con
poda en 0,5 s sin bajar nada)— o bien `clave`, el sobre `ORECOPY1` heredado, que se
baja una vez y se lee como Parquet. `over()` y `sql()` no distinguen.

**Escribir** (0031 §11, W3.6c): `write("p.t", datos)` deja un dataset —una tabla
Iceberg en el lago, el `Dataset` escrito en el árbol, el puntero— desde un DataFrame de pandas
o polars o una tabla de Arrow. El código no toca el bucket: la tabla va por IPC a
`ore-store` (el escritor de Rust, el mismo de la copia), que la lleva al físico del
contrato (0032: `ns` → `µs`, zona → UTC; `uint64` y `null` se niegan con el nombre
de la columna) y escribe los ficheros **con la credencial que el catálogo prestó**
—acotada al prefijo de esa tabla—; el commit va al catálogo REST de Iceberg de
`ore-serve` (`/v1/…`), que valida, escribe el `metadata.json` y mueve el puntero.
`modo="sobrescribir"` (por defecto), `"anexar"` o `"upsert"` (con `clave=[…]`, las
columnas que identifican una fila: lo que había menos esas claves, más lo que
llega, reescrito entero —copy-on-write— y la clave queda declarada en la tabla
para la siguiente vez). Idempotente: la misma tabla al mismo nombre y modo otra
vez no deja otro snapshot (la clave de operación).
Un 409 (alguien escribió mientras tanto) se reintenta sobre lo que hay; un 5xx
se MIRA antes de reintentar: si el commit entró, entró.

**Declarar** (0031 §9, W3.7 ①): `declare(documento)` deja un documento de la
ontología en el árbol desde la celda —una `View` sobre lo que acabas de escribir,
una `Entity`, una `Interface`, un `Concept`, una `Table`— por la puerta de Forge
(`PUT /documentos/{kind}/{ns}/{n}`: se compila antes de empujar, y se rechaza sólo
lo que la escritura añade de malo). El commit lo firma **quien abrió el puesto**,
y va a **su rama** si el puesto tiene una. `documento` es el YAML tal cual (str) o
un dict `{kind, metadata, spec}`. Devuelve `{kind, nombre, fichero, commit, nueva}`;
un 422 es `ValueError` con los diagnósticos.

**Un transform** (0031 §9, W3.7 ③): `@transform(inputs=["p.a", "p.b"], output="p.c")`
sobre una función. Dentro, `over()`/`sql()` de algo que no está en `inputs` y `write()`
a algo que no es `output` son `PermissionError`: lo declarado es lo único que el
código puede leer y escribir. Y lo que `write()` deja lleva su **procedencia** en el
snapshot y en el puntero: `{inputs, transform, codigo?, puesto}` dentro de un
transform, o `{leidas, puesto}` fuera (lo que la sesión leyó hasta ese momento). Es
el linaje `salida ← código ← inputs`, escrito por quien lo produjo. `codigo` viene de
`ORE_CODIGO` (`<ruta>@<commit>`), que `ore run` pone.
"""
import io
import json
import os
import re
import urllib.request

MAGIA = b"ORECOPY1"

__all__ = ["over", "sql", "write", "declare", "transform", "persona", "puesto", "tabla", "json_de"]


class Puesto:
    """Lo que el agente sabe de sí: dónde está `ore-serve`, quién es, qué puesto es."""

    def __init__(self):
        self.servidor = os.environ.get("ORE_SERVE", "http://127.0.0.1:8080").rstrip("/")
        self.id = os.environ.get("PUESTO", "")
        self.bucket = os.environ.get("BUCKET", "")
        self.almacen = os.environ.get("ORE_ALMACEN", "gcs")
        # Quién abrió el puesto: lo pone el agente al reclamarlo (de la ficha).
        self.persona = ""
        # El token lo pone el agente (`agente.py`) y lo renueva; una celda no lo ve.
        self._cabeceras = {}

    def pedir(self, metodo, ruta, cuerpo=None, plazo=30, cabeceras=None):
        datos = None if cuerpo is None else json.dumps(cuerpo).encode("utf-8")
        req = urllib.request.Request(self.servidor + ruta, data=datos, method=metodo)
        req.add_header("accept", "application/json")
        if datos is not None:
            req.add_header("content-type", "application/json")
        for k, v in self._cabeceras.items():
            req.add_header(k, v)
        # Desde qué puesto: el catálogo escribe en nombre de quien lo abrió.
        if self.id:
            req.add_header("x-ore-puesto", self.id)
        for k, v in (cabeceras or {}).items():
            req.add_header(k, v)
        try:
            with urllib.request.urlopen(req, timeout=plazo) as r:
                texto = r.read().decode("utf-8")
                return r.status, (json.loads(texto) if texto.strip() else None)
        except urllib.error.HTTPError as e:
            texto = e.read().decode("utf-8", "replace")
            try:
                return e.code, json.loads(texto)
            except ValueError:
                return e.code, {"error": texto.strip()}


puesto = Puesto()


def _cabeza(texto):
    """`(kind, namespace, name)` de un documento YAML, sin analizador: la línea
    `kind:` y el `metadata:` (en línea `{ name: x, namespace: y }` o en bloque)."""
    m = re.search(r"^kind:\s*([A-Za-z]+)\s*$", texto, re.M)
    if not m:
        raise ValueError("declare(): el documento no dice `kind:`")
    kind = m.group(1)
    m = re.search(r"^metadata:[ \t]*(.*)$", texto, re.M)
    if not m:
        raise ValueError("declare(): el documento no tiene `metadata:`")
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
        raise ValueError("declare(): `metadata.name` no está")
    return kind, campos.get("namespace", ""), campos["name"], campos.get("schema") or DEFAULT


def declare(documento):
    """Declara un documento de la ontología desde la celda (ver arriba): el YAML
    (str) o un dict `{kind, metadata, spec}`. Lo firma quien abrió el puesto, en
    su rama. Devuelve `{kind, nombre, fichero, commit, nueva}`."""
    if isinstance(documento, str):
        kind, ns, nombre, schema = _cabeza(documento)
        cuerpo = {"yaml": documento}
    elif isinstance(documento, dict):
        kind = documento.get("kind")
        meta = documento.get("metadata") or {}
        ns, nombre = meta.get("namespace", ""), meta.get("name", "")
        schema = meta.get("schema") or DEFAULT
        if not kind or not nombre:
            raise ValueError("declare(): el documento quiere `kind` y `metadata.name`")
        cuerpo = documento
    else:
        raise ValueError("declare() quiere el YAML del documento o un dict, no %r" % (type(documento).__name__,))
    if not ns:
        raise ValueError("declare(): `metadata.namespace` no está: un documento vive en un paquete")
    # 0038: en su schema, `/documentos/{kind}/{base}/{schema}/{n}`; la de dos
    # tramos es `default`, y un documento de otro schema por ella es un 422.
    ruta = ("/documentos/%s/%s/%s" % (kind, ns, nombre) if schema == DEFAULT
            else "/documentos/%s/%s/%s/%s" % (kind, ns, schema, nombre))
    c, r = puesto.pedir("PUT", ruta, cuerpo, plazo=120)
    if c in (200, 201):
        s_ = r.get("schema", schema)
        return {"kind": r.get("kind", kind),
                "nombre": ".".join([r.get("namespace", ns)] + ([] if s_ == DEFAULT else [s_]) + [r.get("name", nombre)]),
                "fichero": r.get("fichero", ""), "commit": r.get("commit", ""), "nueva": bool(r.get("nueva", c == 201))}
    r = r or {}
    if r.get("diagnosticos"):
        raise ValueError("declare(%s.%s): %s" % (ns, nombre, "; ".join("%s: %s" % (d.get("codigo", "?"), d.get("mensaje", "")) for d in r["diagnosticos"])))
    raise RuntimeError("declare(%s.%s): %s (%s)" % (ns, nombre, r.get("error", "?"), c))


def persona():
    """Quién abrió el puesto (`persona:…`): la identidad con la que corre lo que
    haces aquí (W3.4). Lo sabe el agente desde que reclama el puesto."""
    if not puesto.persona:
        raise RuntimeError("persona(): el agente aún no sabe quién abrió el puesto")
    return puesto.persona


def _bajar(bucket, clave):
    """Los bytes del artefacto, por el almacén que toque."""
    if puesto.almacen.startswith("dir:"):
        with open(puesto.almacen[4:].rstrip("/") + "/" + clave, "rb") as f:
            return f.read()
    if puesto.almacen == "gcs":
        from google.cloud import storage  # noqa: WPS433 — sólo dentro del clúster

        return storage.Client().bucket(bucket).blob(clave).download_as_bytes()
    raise RuntimeError("ORE_ALMACEN=%r no es un almacén: vale `gcs` o `dir:<ruta>`" % puesto.almacen)


def _desenvolver(crudo):
    if crudo[:8] != MAGIA:
        raise ValueError("el artefacto no es una copia de ORE (sin `ORECOPY1`)")
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


def _corto(nombre, que="un nombre del árbol"):
    """`base.nombre` o `base.schema.nombre` (0038) → la forma corta, la clave del
    árbol: `base.nombre` en `default`, `base.schema.nombre` en otro schema.
    `ventas.pedidos` y `ventas.default.pedidos` son el mismo."""
    partes = nombre.split(".") if isinstance(nombre, str) else []
    if len(partes) not in (2, 3) or not all(partes):
        raise ValueError("%s es `<base>.<schema>.<nombre>` (o `<base>.<nombre>`, en `default`), no %r" % (que, nombre))
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


def transform(inputs, output):
    """`@transform(inputs=[…], output="p.t")`: lo declarado es lo único que la
    función puede leer (`over`, `sql`) y escribir (`write`); lo demás es
    `PermissionError`. Lo escrito lleva `procedencia: {inputs, transform, …}`."""
    if isinstance(inputs, str):
        raise ValueError("transform(): `inputs` es una lista de `<base>.<schema>.<nombre>`")
    inputs = [_corto(i, "transform(): cada input") for i in inputs]
    output = _corto(output, "transform(): `output`")
    if output in inputs:
        raise ValueError("transform(): `%s` no puede ser input y output a la vez" % output)

    def decora(f):
        import functools

        @functools.wraps(f)
        def corre(*a, **kw):
            global _transform
            if _transform is not None:
                raise RuntimeError("transform(): `%s` ya está corriendo; un transform no llama a otro" % _transform.nombre)
            _transform = _Transform(getattr(f, "__name__", "transform"), inputs, output)
            # Y se le dice al servidor (W3.7 gobierno ⑤): mientras corre, él
            # resuelve sólo `inputs` y deja escribir sólo `output`. Si no
            # contesta (un ore-serve viejo), el SDK sigue acotando por su cuenta.
            puesto.pedir("POST", "/puestos/%s/transform" % puesto.id, {"nombre": _transform.nombre, "inputs": list(inputs), "output": output})
            try:
                return f(*a, **kw)
            finally:
                _transform = None
                puesto.pedir("DELETE", "/puestos/%s/transform" % puesto.id)
        corre.inputs, corre.output = list(inputs), output
        return corre
    return decora


def _lee(vista):
    """Anota una lectura, y dentro de un transform la acota a sus `inputs`."""
    vista = _corto(vista)
    if _transform is not None and vista not in _transform.inputs:
        raise PermissionError("`%s` no está en los inputs de `%s` (%s): un transform sólo lee lo que declara" % (vista, _transform.nombre, ", ".join(_transform.inputs)))
    if vista not in _leidas:
        _leidas.append(vista)


def _procedencia(nombre=None):
    """Lo que `write()` deja dicho de sí: de qué salió, qué código, desde qué puesto.
    Fuera de un transform es lo que la sesión leyó, **sin lo que se está escribiendo**
    (W3.7 gobierno ③: un dataset no sale de sí mismo)."""
    p = {"puesto": puesto.id}
    if _transform is not None:
        p["inputs"] = sorted(_transform.inputs)
        p["transform"] = _transform.nombre
    else:
        p["leidas"] = sorted(l for l in _leidas if l != nombre)
    if os.environ.get("ORE_CODIGO"):
        p["codigo"] = os.environ["ORE_CODIGO"]
    return p


def _resolver(vista):
    """Qué copia es `<paquete>.<vista>`, según ore-serve (en nombre de la persona)."""
    vista = _corto(vista)
    _lee(vista)
    codigo, r = puesto.pedir("GET", "/puestos/%s/datos/%s" % (puesto.id, vista))
    return _o_el_error(codigo, r, vista)


def _o_el_error(codigo, r, vista):
    """Lo que ore-serve contesto por un nombre, o el error de siempre: el mismo
    para `over()` (GET datos) que para `sql()` (POST sql)."""
    if codigo == 409:
        raise RuntimeError("la copia de `%s` no está hecha: %s" % (vista, (r or {}).get("error", "")))
    if codigo == 404:
        raise LookupError("no hay ninguna `View` ni `Dataset` `%s` en el árbol" % vista)
    if codigo == 403:
        # El conducto de la lectura (0031 W3.7 gobierno ②): lo que el dataset
        # lleva no cabe por `contextSurface.workspace`. Se dice tal cual.
        raise PermissionError((r or {}).get("error") or "ore-serve no deja leer `%s` desde un puesto" % vista)
    if codigo != 200:
        raise RuntimeError("ore-serve contestó %s por `%s`: %s" % (codigo, vista, (r or {}).get("error", r)))
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
                c, l = puesto.pedir("GET", _v1_tabla(_corto(vista)), cabeceras=_DELEGAR)
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
            raise RuntimeError("la imagen no trae la extensión `%s` de DuckDB: hay que preinstalarla en %s" % (extension, EXTENSIONES))
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
        crudo = _bajar(r.get("bucket") or puesto.bucket, r["clave"])
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
    raise ValueError("como=%r no es una forma: vale `pandas`, `arrow` o `polars`" % (como,))


def over(vista, como="pandas"):
    """La copia de `<paquete>.<vista>`: DataFrame con tipos de Arrow
    (`como="pandas"`, por defecto), `pyarrow.Table` (`como="arrow"`) o DataFrame
    de polars (`como="polars"`)."""
    fuente, r = _fuente_de(vista)
    if r.get("_parquet"):
        # El sobre, ya en local: pyarrow lo lee más deprisa que nadie (13 M filas/s).
        import pyarrow.parquet as pq

        return _como(pq.read_table(r["_parquet"]), como)
    return _como(_arrow(_duckdb().sql("select * from %s" % fuente)), como)


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


def sql(texto, como="pandas"):
    """SQL (DuckDB) sobre los datasets del árbol. El texto entero va a ore-serve
    (`POST /puestos/{id}/sql`), que dice qué nombres del árbol lee —con el
    tokenizador y el árbol como filtro: un nombre en un comentario o en una
    cadena no cuenta, `from a, b` cuenta los dos, un esquema de la sesión es del
    motor— y los resuelve como `over()`, en una ida y vuelta; cada uno queda como
    vista `paquete.nombre`. Hasta aquí era una regex que fallaba 5 de 13 casos
    (`medida-el-sql-del-arbol.py`). Devuelve lo mismo que `over()`: DataFrame con
    tipos de Arrow, `pyarrow.Table` o polars."""
    if not isinstance(texto, str) or not texto.strip():
        raise ValueError("sql() quiere una consulta")
    con = _duckdb()
    # Sin puesto no hay árbol, y sin un punto no hay `a.b`: el motor solo.
    codigo, r = (puesto.pedir("POST", "/puestos/%s/sql" % puesto.id, {"texto": texto})
                 if puesto.id and "." in texto else (200, {}))
    if codigo != 200:
        _o_el_error(codigo, r, (r or {}).get("nombre") or "?")
    for nombre, rd in sorted(((r or {}).get("fuentes") or {}).items()):
        _lee(nombre)
        fuente, _ = _fuente_de_respuesta(nombre, rd)
        _registra(con, nombre, fuente)
    r = con.execute(texto)
    if r.description is None:
        return None
    return _como(_arrow(r), como)


# ── Escribir (0031 §11) ────────────────────────────────────────────────────
_DELEGAR = {"x-iceberg-access-delegation": "vended-credentials"}
ICEBERG = {"int8": "long", "int16": "long", "int32": "long", "int64": "long", "uint8": "long", "uint16": "long", "uint32": "long",
           "halffloat": "double", "float": "double", "double": "double", "bool": "boolean", "string": "string", "large_string": "string",
           "string_view": "string", "date32[day]": "date", "date64[ms]": "date"}


def _tipo_iceberg(columna, t):
    """El tipo de Iceberg del esquema con el que la tabla se esboza (lo mismo que
    `ore-store` hace al escribir, 0032): lo que el contrato no tiene se niega aquí,
    con el nombre de la columna, antes de mandar nada."""
    import pyarrow as pa

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
        raise ValueError("write(): la columna `%s` es uint64, que no cabe en int64 sin mentir (0032); conviértela antes" % columna)
    if s == "null":
        raise ValueError("write(): la columna `%s` no tiene tipo (null): dale uno antes (0032)" % columna)
    raise ValueError("write(): la columna `%s` es `%s`, que el contrato de tipos (0032) no tiene" % (columna, s))


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
    raise TypeError("write() quiere un DataFrame de pandas o polars, o una Table de Arrow, no %s" % type(datos).__name__)


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
            raise RuntimeError("write(): el catálogo no prestó credencial para `%s`" % ubicacion)
    elif ubicacion.startswith("s3://"):
        nombre = "ore-store-r2"
        env["ORE_R2_BUCKET"] = ubicacion[5:].split("/", 1)[0]
        env["ORE_R2_S3_ENDPOINT"] = config.get("s3.endpoint", "")
        env["ORE_R2_ACCESS_KEY_ID"] = config.get("s3.access-key-id", "")
        env["ORE_R2_SECRET_ACCESS_KEY"] = config.get("s3.secret-access-key", "")
        env["ORE_R2_REGION"] = config.get("s3.region", "auto")
    else:
        raise RuntimeError("write(): la tabla vive en `%s`, que no es un lago que este SDK sepa escribir" % ubicacion)
    binario = shutil.which(nombre, path=os.environ.get("ORE_STORE_DIR") or None) or shutil.which(nombre)
    if not binario:
        raise RuntimeError("write(): no está `%s` en el PATH (la imagen del puesto lo lleva; fuera, ORE_STORE_DIR)" % nombre)
    return binario, env


def _escribir_ficheros(binario, env, peticion, ipc):
    import subprocess

    p = subprocess.run([binario, "escribir"], input=json.dumps(peticion).encode("utf-8") + b"\n" + ipc, capture_output=True, env=env)
    if p.returncode != 0:
        err = p.stderr.decode("utf-8", "replace").strip()
        raise RuntimeError("write(): %s" % (err.replace("error: ", "", 1) or "el escritor falló"))
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
    c, r = puesto.pedir("GET", _v1_tabla(nombre), cabeceras=_DELEGAR)
    primera = posiciones[0] + 1
    if c == 404:
        raise RuntimeError("insert into %s: la tabla no existe todavía, y la columna %d del select no tiene nombre "
                           "del que tomarlo: dale uno (`… as nombre`)" % (nombre, primera))
    if c != 200:
        raise RuntimeError("insert into %s: ore-serve contestó %s: %s" % (nombre, c, _mensaje(r)))
    md = r["metadata"]
    esquema = next((s for s in md.get("schemas", []) if s.get("schema-id") == md.get("current-schema-id")), None) or md.get("schema") or {}
    de_la_tabla = [f["name"] for f in esquema.get("fields", [])]
    columnas = list(tabla_arrow.column_names)
    for p in posiciones:
        if p >= len(de_la_tabla):
            raise RuntimeError("insert into %s: la columna %d del select no tiene nombre y la tabla sólo tiene %d: "
                               "dale uno (`… as nombre`)" % (nombre, p + 1, len(de_la_tabla)))
        columnas[p] = de_la_tabla[p]
    return tabla_arrow.rename_columns(columnas)


def write(nombre, datos, modo="sobrescribir", clave=None):
    """Escribe `datos` como el dataset `<paquete>.<tabla>` del lago (ver arriba).
    Devuelve `{tabla, filas, snapshot, metadata_location, operacion, repetida}`."""
    import hashlib

    nombre = _corto(nombre, "write(): el nombre")
    if modo not in ("sobrescribir", "anexar", "upsert"):
        raise ValueError("modo=%r: vale `sobrescribir`, `anexar` o `upsert`" % (modo,))
    if clave is not None and (isinstance(clave, str) or not all(isinstance(c, str) for c in clave)):
        raise ValueError("clave=%r: una lista de nombres de columna" % (clave,))
    if clave is not None and modo != "upsert":
        raise ValueError("`clave` es de modo=\"upsert\"")
    clave_upsert = list(clave) if clave else None
    if _transform is not None and nombre != _transform.output:
        raise PermissionError("`%s` no es el output de `%s` (%s): un transform sólo escribe lo que declara" % (nombre, _transform.nombre, _transform.output))
    base, ns, t = _partes(nombre)  # el namespace de /v1 es el schema (0038 P4)
    tabla_arrow = _arrow_de(datos)
    if tabla_arrow.num_rows == 0:
        raise ValueError("write(): la tabla no tiene filas")
    esquema = {"type": "struct", "schema-id": 0, "fields": [
        {"id": i + 1, "name": f.name, "type": _tipo_iceberg(f.name, f.type), "required": False} for i, f in enumerate(tabla_arrow.schema)]}
    ipc = _ipc(tabla_arrow)
    # La clave de operación la calcula el escritor DEL CONTENIDO (los valores,
    # no los bytes del IPC, que llevan relleno y cambian entre dos lecturas de
    # lo mismo), con esta semilla: la misma tabla al mismo nombre y modo es la
    # misma escritura, y el catálogo no la repite.
    semilla = "%s|%s" % (nombre, modo) + ("|" + ",".join(clave_upsert) if clave_upsert else "")
    clave = None
    dataset = "catalogo/%s/%s/%s" % (base, ns, t)  # una etiqueta: la ubicación la da el catálogo

    def cargar():
        c, r = puesto.pedir("GET", _v1_tabla(nombre), cabeceras=_DELEGAR)
        if c == 200:
            # Prestado sólo para leer (lo de otra persona, un mantenido): el
            # porqué, antes de escribir un fichero con una credencial que no escribe.
            if (r.get("config") or {}).get("ore.solo-lectura"):
                raise RuntimeError("write(%s): %s" % (nombre, r["config"]["ore.solo-lectura"]))
            return r["metadata-location"], None, r.get("config", {}), r["metadata"]["location"]
        if c == 404:
            c, r = puesto.pedir("POST", "/v1/%s/namespaces/%s/tables" % (base, ns), {"name": t, "stage-create": True, "schema": esquema, "properties": {}}, cabeceras=_DELEGAR)
            if c != 200:
                raise RuntimeError("write(%s): %s" % (nombre, _mensaje(r)))
            return None, r["metadata"], r.get("config", {}), r["metadata"]["location"]
        raise RuntimeError("write(%s): ore-serve contestó %s: %s" % (nombre, c, _mensaje(r)))

    global _s3
    for intento in range(4):
        base, esbozo, config, ubicacion = cargar()
        if config.get("s3.access-key-id"):
            _s3 = config
        binario, env = _ore_store(config, ubicacion)
        peticion = {"dataset": dataset, "modo": modo, "operacion": "contenido", "semilla": semilla, "procedencia": _procedencia(nombre)}
        if clave_upsert:
            peticion["clave"] = clave_upsert
        if base:
            peticion["base"] = base
        else:
            peticion["esbozo"] = esbozo
        escrito = _escribir_ficheros(binario, env, peticion, ipc)
        clave = escrito.get("operacion") or clave
        c, r = puesto.pedir("POST", _v1_tabla(nombre),
                            {"identifier": {"namespace": [ns], "name": t}, "requirements": escrito["requirements"], "updates": escrito["updates"]}, plazo=120)
        if c == 200:
            snap = ((r or {}).get("metadata") or {}).get("current-snapshot-id")
            # la misma operación ya estaba: el catálogo contesta con lo que hay
            # (el mismo puntero) y no deja nada
            repetida = base is not None and (r or {}).get("metadata-location") == base
            return {"tabla": nombre, "filas": escrito["filas"], "snapshot": str(snap or ""), "metadata_location": (r or {}).get("metadata-location", ""),
                    "operacion": clave, "repetida": repetida}
        if c == 409:
            # alguien escribió mientras tanto (o la tabla nació): otra vez sobre lo que hay
            continue
        if c >= 500:
            # el commit pudo entrar: se MIRA antes de reintentar
            c2, r2 = puesto.pedir("GET", _v1_tabla(nombre))
            if c2 == 200:
                md = r2["metadata"]
                vigente = [s for s in md.get("snapshots", []) if s.get("snapshot-id") == md.get("current-snapshot-id")]
                if vigente and vigente[0].get("summary", {}).get("ore.operacion") == clave:
                    return {"tabla": nombre, "filas": escrito["filas"], "snapshot": str(md.get("current-snapshot-id")), "metadata_location": r2["metadata-location"], "operacion": clave, "repetida": False}
            raise RuntimeError("write(%s): el catálogo contestó %s y el commit no está: %s" % (nombre, c, _mensaje(r)))
        raise RuntimeError("write(%s): %s" % (nombre, _mensaje(r)))
    raise RuntimeError("write(%s): cuatro veces alguien escribió antes; vuelve a intentarlo" % nombre)


# ── El JSON de la consola (0032 §1) ───────────────────────────────────────
def tabla(valor, limite=200):
    """Un DataFrame (pandas o polars), una Series o una Table de Arrow → la salida
    `tabla` del contrato (0032 §1, columna «JSON de la consola»): `columnas` con el
    tipo de Arrow por nombre, y las primeras filas en el JSON de la tabla. Es el
    MISMO JSON que emiten los agentes de Node y de Java: la consola no distingue.
    `limite` es cuántas filas van en `filas`; `total` dice cuántas hay."""
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
    por_columna = [[json_de(v, f.type) for v in cabeza.column(i).to_pylist()] for i, f in enumerate(cabeza.schema)]  # noqa: E501
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


def json_de(v, tipo=None):
    """Un valor de Arrow (ya en Python) → el JSON del contrato (0032 §1):
    entero → número si |x| ≤ 2⁵³, si no cadena · decimal → cadena siempre
    (salvo el de escala 0, que es un entero y va como tal) ·
    float → número, y `NaN`/`Infinity`/`-Infinity` como cadena · fecha `YYYY-MM-DD`
    · hora `HH:MM:SS[.ffffff]` · fecha-hora sin zona en ISO con `T` · instante en
    UTC con `Z` · bytes en base64 · lista → array · struct → objeto · map →
    `[{key, value}]`. Nunca se degrada en silencio: lo que no cabe en un número
    de JSON va como cadena, no como un número parecido."""
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
            return [{"key": json_de(k), "value": json_de(x)} for k, x in v]
        return [json_de(x) for x in v]
    if isinstance(v, dict):
        return {str(k): json_de(x) for k, x in v.items()}
    if hasattr(v, "item"):
        try:
            return json_de(v.item())
        except (ValueError, AttributeError):
            pass
    return str(v)
