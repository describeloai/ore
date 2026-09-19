"""
`ore` · el SDK del puesto (0031 W3.1).

Lo que una celda importa. Hoy, una cosa: `over("<paquete>.<vista>")` devuelve la
copia de esa vista como DataFrame. El código nunca ve el bucket ni una
credencial: pregunta a `ore-serve` QUÉ copia es (con la identidad del puesto,
que la resuelve en nombre de la persona y con su potestad) y baja el artefacto
con la identidad del pod (Workload Identity). El sobre `ORECOPY1` se desenvuelve
aquí; la carga es Parquet.

Fuera del clúster (las pruebas de fuego) el almacén es un directorio:
`ORE_ALMACEN=dir:/ruta` lee `ore/v1/<clave>` de ahí.
"""
import io
import json
import os
import urllib.request

MAGIA = b"ORECOPY1"

__all__ = ["over", "puesto"]


class Puesto:
    """Lo que el agente sabe de sí: dónde está `ore-serve`, quién es, qué puesto es."""

    def __init__(self):
        self.servidor = os.environ.get("ORE_SERVE", "http://127.0.0.1:8080").rstrip("/")
        self.id = os.environ.get("PUESTO", "")
        self.bucket = os.environ.get("BUCKET", "")
        self.almacen = os.environ.get("ORE_ALMACEN", "gcs")
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


def over(vista, como="pandas"):
    """La copia de `<paquete>.<vista>` como DataFrame (`como="pandas"`) o como
    `pyarrow.Table` (`como="arrow"`)."""
    if not isinstance(vista, str) or vista.count(".") != 1:
        raise ValueError("over() quiere `<paquete>.<vista>`, no %r" % (vista,))
    codigo, r = puesto.pedir("GET", "/puestos/%s/datos/%s" % (puesto.id, vista))
    if codigo == 409:
        raise RuntimeError("la copia de `%s` no está hecha: %s" % (vista, (r or {}).get("error", "")))
    if codigo == 404:
        raise LookupError("no hay ninguna `View` `%s` en el árbol" % vista)
    if codigo != 200:
        raise RuntimeError("ore-serve contestó %s a over(%r): %s" % (codigo, vista, (r or {}).get("error", r)))
    import pyarrow.parquet as pq

    crudo = _bajar(r.get("bucket") or puesto.bucket, r["clave"])
    _, carga = _desenvolver(crudo)
    tabla = pq.read_table(io.BytesIO(carga))
    if como == "arrow":
        return tabla
    return tabla.to_pandas()
