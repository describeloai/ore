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

`sql("select … from hr.espanoles")` (W3.3) pregunta a las copias por el nombre
de sus vistas: cada `paquete.vista` tras FROM/JOIN se resuelve igual que en
`over()`, se baja una vez por sesión y se registra en DuckDB como la vista
`paquete.vista`; devuelve un DataFrame. Medido en victor (2 CPU · 3 GB):
200 M de filas, `group by` con agregados en 1,9 s, `where` en 1 s.

Fuera del clúster (las pruebas de fuego) el almacén es un directorio:
`ORE_ALMACEN=dir:/ruta` lee `ore/v1/<clave>` de ahí.

**Todo es un dataset** (0031 §10): lo que `ore-serve` contesta por `datos` es o bien
`metadata_location` —el `metadata.json` vigente de una **tabla Iceberg** en el
bucket: se lee en sitio con DuckDB (`iceberg_scan` sobre la raíz y la versión, con el
token del pod como *bearer*; medido en `medida-w3-lago.py`: 10 M de filas, filtro con
poda en 0,5 s sin bajar nada)— o bien `clave`, el sobre `ORECOPY1` heredado, que se
baja una vez y se lee como Parquet. `over()` y `sql()` no distinguen.
"""
import io
import json
import os
import re
import urllib.request

MAGIA = b"ORECOPY1"

__all__ = ["over", "sql", "persona", "puesto", "tabla", "json_de"]


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

    def pedir(self, metodo, ruta, cuerpo=None, plazo=30):
        datos = None if cuerpo is None else json.dumps(cuerpo).encode("utf-8")
        req = urllib.request.Request(self.servidor + ruta, data=datos, method=metodo)
        req.add_header("accept", "application/json")
        if datos is not None:
            req.add_header("content-type", "application/json")
        for k, v in self._cabeceras.items():
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


def _resolver(vista):
    """Qué copia es `<paquete>.<vista>`, según ore-serve (en nombre de la persona)."""
    if not isinstance(vista, str) or vista.count(".") != 1:
        raise ValueError("se quiere `<paquete>.<vista>`, no %r" % (vista,))
    codigo, r = puesto.pedir("GET", "/puestos/%s/datos/%s" % (puesto.id, vista))
    if codigo == 409:
        raise RuntimeError("la copia de `%s` no está hecha: %s" % (vista, (r or {}).get("error", "")))
    if codigo == 404:
        raise LookupError("no hay ninguna `View` `%s` en el árbol" % vista)
    if codigo != 200:
        raise RuntimeError("ore-serve contestó %s por `%s`: %s" % (codigo, vista, (r or {}).get("error", r)))
    return r


def _fuente_de(vista):
    """De qué se lee la vista, como fragmento SQL de DuckDB: `iceberg_scan(...)` si es
    un dataset Iceberg (el puntero trae `metadata_location`), `read_parquet('…')` si
    es un sobre heredado (trae `clave`, y se baja una vez)."""
    r = _resolver(vista)
    if r.get("metadata_location"):
        return _iceberg(r["metadata_location"]), r
    f, r = _parquet_de(vista, r)
    r["_parquet"] = f
    return "read_parquet('%s')" % f.replace("'", "''").replace("\\", "/"), r


def _iceberg(metadata_location):
    """`iceberg_scan` sobre la raíz de la tabla y la versión del puntero. A DuckDB no se
    le da el fichero: con `allow_moved_paths` la raíz es lo que se le pasa (y con el
    fichero resolvía `…metadata.json/metadata/snap…`), y así no lista nada. En el
    bucket, la API XML de GCS por https con el token del pod como *bearer*."""
    raiz, fichero = metadata_location.rsplit("/metadata/", 1)
    version = fichero[: -len(".metadata.json")] if fichero.endswith(".metadata.json") else fichero
    con = _duckdb()
    # `iceberg` arrastra `avro` (y usa `json` e `icu`); con el autoinstalado apagado
    # hay que cargarlas por su nombre, en orden. `httpfs` sólo para el bucket.
    for e in LAGO:
        _cargar(con, e)
    if raiz.startswith("gs://"):
        _cargar(con, "httpfs")
        con.execute("create or replace secret ore_gcs (type http, bearer_token '%s')" % _token_de_google().replace("'", "''"))
        raiz = "https://storage.googleapis.com/" + raiz[5:]
    return "iceberg_scan('%s', version='%s', allow_moved_paths=true)" % (raiz.replace("'", "''").replace("\\", "/"), version.replace("'", "''"))


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


_VISTAS_EN_SQL = re.compile(r"(?i)\b(?:from|join)\s+([a-z_][a-z0-9_]*)\.([a-z_][a-z0-9_]*)\b")
_con = None


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
        _con.execute("set autoinstall_known_extensions = false")
        # Un instante es un instante: DuckDB enseña un TIMESTAMPTZ en la zona de la
        # sesión, y el contrato (0032 §1) lo quiere en UTC. Aquí la sesión ES UTC.
        _con.execute("set TimeZone = 'UTC'")
        if os.path.isdir(EXTENSIONES):
            _con.execute("set extension_directory = '%s'" % EXTENSIONES)
    return _con


def sql(texto, como="pandas"):
    """SQL (DuckDB) sobre las copias: cada `paquete.vista` tras FROM/JOIN se
    resuelve, se baja una vez y queda como vista `paquete.vista`. Devuelve lo
    mismo que `over()`: DataFrame con tipos de Arrow, `pyarrow.Table` o polars."""
    if not isinstance(texto, str) or not texto.strip():
        raise ValueError("sql() quiere una consulta")
    con = _duckdb()
    for esquema, nombre in sorted(set(_VISTAS_EN_SQL.findall(texto))):
        fuente, _ = _fuente_de("%s.%s" % (esquema, nombre))
        con.execute('create schema if not exists "%s"' % esquema)
        # Sin parámetros: un CREATE VIEW no se prepara. La fuente es nuestra (la
        # clave del artefacto o el puntero), ya escapada.
        con.execute('create or replace view "%s"."%s" as select * from %s' % (esquema, nombre, fuente))
    r = con.execute(texto)
    if r.description is None:
        return None
    return _como(_arrow(r), como)


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
