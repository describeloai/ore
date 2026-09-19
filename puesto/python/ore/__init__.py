"""
`ore` · el SDK del puesto (0031 W3.1).

Lo que una celda importa. Hoy, una cosa: `over("<paquete>.<vista>")` devuelve la
copia de esa vista como DataFrame. El código nunca ve el bucket ni una
credencial: pregunta a `ore-serve` QUÉ copia es (con la identidad del puesto,
que la resuelve en nombre de la persona y con su potestad) y baja el artefacto
con la identidad del pod (Workload Identity). El sobre `ORECOPY1` se desenvuelve
aquí; la carga es Parquet.

`persona()` (W3.4) dice quién abrió el puesto: la identidad con la que corre
lo que haces aquí.

`sql("select … from hr.espanoles")` (W3.3) pregunta a las copias por el nombre
de sus vistas: cada `paquete.vista` tras FROM/JOIN se resuelve igual que en
`over()`, se baja una vez por sesión y se registra en DuckDB como la vista
`paquete.vista`; devuelve un DataFrame. Medido en victor (2 CPU · 3 GB):
200 M de filas, `group by` con agregados en 1,9 s, `where` en 1 s.

Fuera del clúster (las pruebas de fuego) el almacén es un directorio:
`ORE_ALMACEN=dir:/ruta` lee `ore/v1/<clave>` de ahí.
"""
import io
import json
import os
import re
import urllib.request

MAGIA = b"ORECOPY1"

__all__ = ["over", "sql", "persona", "puesto"]


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


def _parquet_de(vista):
    """La copia de la vista como fichero Parquet local, bajado UNA vez por sesión
    (la clave es el digest del artefacto: una clave nueva es otra copia)."""
    r = _resolver(vista)
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


def over(vista, como="pandas"):
    """La copia de `<paquete>.<vista>` como DataFrame (`como="pandas"`) o como
    `pyarrow.Table` (`como="arrow"`)."""
    import pyarrow.parquet as pq

    f, _ = _parquet_de(vista)
    tabla = pq.read_table(f)
    if como == "arrow":
        return tabla
    return tabla.to_pandas()


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
    return _con


def sql(texto, como="pandas"):
    """SQL (DuckDB) sobre las copias: cada `paquete.vista` tras FROM/JOIN se
    resuelve, se baja una vez y queda como vista `paquete.vista`. Devuelve un
    DataFrame (`como="pandas"`) o una `pyarrow.Table` (`como="arrow"`)."""
    if not isinstance(texto, str) or not texto.strip():
        raise ValueError("sql() quiere una consulta")
    con = _duckdb()
    for esquema, nombre in sorted(set(_VISTAS_EN_SQL.findall(texto))):
        f, _ = _parquet_de("%s.%s" % (esquema, nombre))
        con.execute('create schema if not exists "%s"' % esquema)
        # Sin parámetros: un CREATE VIEW no se prepara. La ruta es nuestra (la
        # clave del artefacto), sin comillas dentro; se escapa igual.
        con.execute('create or replace view "%s"."%s" as select * from read_parquet(\'%s\')' % (esquema, nombre, f.replace("'", "''").replace("\\", "/")))
    r = con.execute(texto)
    if r.description is None:
        return None
    tabla = r.fetch_arrow_table()
    if como == "arrow":
        return tabla
    return tabla.to_pandas()
