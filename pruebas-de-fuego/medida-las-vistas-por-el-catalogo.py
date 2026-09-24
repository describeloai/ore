"""LAS VIEWS POR EL CATALOGO · ¿lee DuckDB una vista de Iceberg REST?

El arreglo de las Views (`ore ask --sql`, `datos_de_vista`) hace que una View
llegue al motor como SQL sobre sus datasets. Eso cambia el terreno de quitar la
regex de `sql()`: por ATTACH, DuckDB resuelve solo lo que el catalogo le da como
TABLA; una View no lo es. O el catalogo sirve tambien las Views —la spec REST de
Iceberg tiene `loadView`, con la SQL dentro y su dialecto—, o el SDK sigue
teniendo que saber que nombres hay en el texto, que es lo que hace la regex.

Se mide con un catalogo REST de mentira (Python, stdlib) que sirve UNA tabla —un
Parquet local, con su metadata de Iceberg escrita por PyIceberg— y UNA vista
cuya SQL la lee, apuntando cada peticion que llega:

  §1  ¿PIDE VISTAS?      con y sin `endpoints` en /v1/config: ¿llama DuckDB a
                         /views? ¿resuelve `cat.ns.vista`?
  §2  ¿QUE DIALECTO?     la representacion `sql` con dialecto `duckdb`, `spark`,
                         y sin el nuestro: ¿cual toma?
  §3  ¿Y NODE Y LA JVM?  lo mismo con los DuckDB de los otros dos SDK (si
                         estan a mano: --node DIR, --jdbc JAR, --java BIN)

    python pruebas-de-fuego/medida-las-vistas-por-el-catalogo.py [--node DIR] [--jdbc JAR --java BIN]

No toca el cluster ni ore-serve. Fuera de la maquina: extensions.duckdb.org si
faltan las extensiones.
"""
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.stdout.reconfigure(encoding="utf-8", errors="replace")

PETICIONES = []
MODO = {"endpoints": True, "dialecto": "duckdb"}
ESTADO = {}


def respuesta_vista():
    """Un `LoadViewResult` de la spec REST: la vista lee la tabla `hr.ventas`."""
    ahora = int(time.time() * 1000)
    rep = [{"type": "sql", "sql": "select id, pais from ore.hr.ventas where pais = 'ES'", "dialect": MODO["dialecto"]}]
    meta = {
        "view-uuid": ESTADO["uuid_vista"],
        "format-version": 1,
        "location": ESTADO["raiz"] + "/vistas/ventasES",
        "current-version-id": 1,
        "versions": [{"version-id": 1, "timestamp-ms": ahora, "schema-id": 0, "summary": {"operation": "create"},
                      "default-namespace": ["hr"], "representations": rep}],
        "version-log": [{"version-id": 1, "timestamp-ms": ahora}],
        "schemas": [{"type": "struct", "schema-id": 0, "fields": [
            {"id": 1, "name": "id", "required": False, "type": "long"},
            {"id": 2, "name": "pais", "required": False, "type": "string"}]}],
        "properties": {},
    }
    return {"metadata-location": ESTADO["raiz"] + "/vistas/ventasES/v1.metadata.json", "metadata": meta, "config": {}}


class Catalogo(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _json(self, codigo, cuerpo):
        b = json.dumps(cuerpo).encode()
        self.send_response(codigo)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_HEAD(self):
        PETICIONES.append("HEAD " + self.path)
        ruta = self.path.split("?")[0].rstrip("/")
        self.send_response(204 if ruta.endswith(("/tables/ventas", "/views/ventasES", "/namespaces/hr")) else 404)
        self.end_headers()

    def do_GET(self):
        PETICIONES.append("GET " + self.path)
        ruta = self.path.split("?")[0].rstrip("/")
        if ruta.endswith("/config"):
            c = {"defaults": {}, "overrides": {}}
            if MODO["endpoints"]:
                c["endpoints"] = [
                    "GET /v1/{prefix}/namespaces", "GET /v1/{prefix}/namespaces/{namespace}",
                    "HEAD /v1/{prefix}/namespaces/{namespace}",
                    "GET /v1/{prefix}/namespaces/{namespace}/tables",
                    "GET /v1/{prefix}/namespaces/{namespace}/tables/{table}",
                    "HEAD /v1/{prefix}/namespaces/{namespace}/tables/{table}",
                    "GET /v1/{prefix}/namespaces/{namespace}/views",
                    "GET /v1/{prefix}/namespaces/{namespace}/views/{view}",
                    "HEAD /v1/{prefix}/namespaces/{namespace}/views/{view}",
                ]
            return self._json(200, c)
        if ruta.endswith("/namespaces"):
            return self._json(200, {"namespaces": [["hr"]]})
        if ruta.endswith("/namespaces/hr"):
            return self._json(200, {"namespace": ["hr"], "properties": {}})
        if ruta.endswith("/namespaces/hr/tables"):
            return self._json(200, {"identifiers": [{"namespace": ["hr"], "name": "ventas"}]})
        if ruta.endswith("/namespaces/hr/views"):
            return self._json(200, {"identifiers": [{"namespace": ["hr"], "name": "ventasES"}]})
        if ruta.endswith("/namespaces/hr/tables/ventas"):
            return self._json(200, {"metadata-location": ESTADO["ml"], "metadata": ESTADO["metadata"], "config": {}})
        if ruta.endswith("/namespaces/hr/views/ventasES"):
            return self._json(200, respuesta_vista())
        return self._json(404, {"error": {"message": "no hay %s" % ruta, "type": "NoSuchTableException", "code": 404}})


def preparar():
    """La tabla: 8 filas escritas por PyIceberg en un catalogo SQL local; su
    metadata.json es lo que el catalogo de mentira sirve."""
    import importlib.util

    import pyarrow as pa

    # El mismo catalogo de arbol que usa el-puesto.sh para `hr.lago`: funciona con
    # las rutas de Windows, que el SqlCatalog de PyIceberg no.
    aqui = os.path.dirname(os.path.abspath(__file__))
    sp = importlib.util.spec_from_file_location("ice", os.path.join(aqui, "medida-w3-iceberg.py"))
    ice = importlib.util.module_from_spec(sp)
    sp.loader.exec_module(ice)
    d = tempfile.mkdtemp(prefix="ore-vistas-cat-").replace("\\", "/")
    cat = ice.catalogo_arbol()("prueba", d + "/arbol", warehouse=d + "/lago")
    cat.create_namespace("hr")
    t = pa.table({"id": pa.array(range(8), pa.int64()), "pais": pa.array(["ES", "PT", "FR", "DE"] * 2)})
    tb = cat.create_table("hr.ventas", t.schema)
    tb.append(t)
    ESTADO["raiz"] = d
    ESTADO["ml"] = tb.metadata_location
    ruta = tb.metadata_location
    for pre in ("file:///", "file://"):
        if ruta.startswith(pre):
            ruta = ruta[len(pre):]
    ESTADO["metadata"] = json.loads(open(ruta, encoding="utf-8").read())
    ESTADO["uuid_vista"] = str(uuid.uuid4())
    return d


def probar_python(base, etiqueta):
    import duckdb

    PETICIONES.clear()
    con = duckdb.connect()
    out = {}
    try:
        con.execute("install iceberg; load iceberg")
        con.execute("create or replace secret ore_ice (type iceberg, token 'x')")
        con.execute("attach '' as ore (type iceberg, endpoint '%s', secret ore_ice)" % base)
        out["tabla"] = con.execute("select count(*) from ore.hr.ventas").fetchone()[0]
    except Exception as e:
        out["tabla"] = "✗ " + str(e).splitlines()[0][:120]
    try:
        out["listado"] = [r[0] for r in con.execute(
            "select table_name from information_schema.tables where table_catalog = 'ore' order by 1").fetchall()]
    except Exception as e:
        out["listado"] = "✗ " + str(e).splitlines()[0][:100]
    try:
        r = con.execute("select * from ore.hr.ventasES order by id").fetchall()
        out["vista"] = "%d filas: %s" % (len(r), r[:4])
    except Exception as e:
        out["vista"] = "✗ " + str(e).splitlines()[0][:140]
    out["pidio_vistas"] = sorted({p for p in PETICIONES if "/views" in p})
    print("     %-26s tabla=%s" % (etiqueta, out["tabla"]))
    print("     %-26s listado: %s" % ("", out["listado"]))
    print("     %-26s vista:   %s" % ("", out["vista"]))
    print("     %-26s pidio /views: %s" % ("", out["pidio_vistas"] or "NUNCA"))
    return out


def main():
    import duckdb

    print("=== las Views por el catalogo  (%s · duckdb %s)" % (time.strftime("%Y-%m-%d"), duckdb.__version__))
    d = preparar()
    srv = ThreadingHTTPServer(("127.0.0.1", 0), Catalogo)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = "http://127.0.0.1:%d" % srv.server_address[1]
    try:
        print()
        print("  §1 ¿PIDE VISTAS? (Python)")
        MODO.update(endpoints=True, dialecto="duckdb")
        a = probar_python(base, "con endpoints")
        MODO.update(endpoints=False)
        probar_python(base, "sin endpoints")
        print()
        print("  §2 ¿QUE DIALECTO? (con endpoints)")
        MODO.update(endpoints=True)
        for dia in ("spark", "trino"):
            MODO["dialecto"] = dia
            probar_python(base, "dialecto %s" % dia)
        print()
        ok = isinstance(a.get("vista"), str) and not a["vista"].startswith("✗")
        if ok:
            print("  ⇒ DuckDB LEE una vista de Iceberg REST: el catalogo puede servir las Views con la")
            print("    SQL de `ore ask --sql`, y ATTACH lo resuelve todo sin analizar el texto.")
        else:
            print("  ⇒ DuckDB NO lee una vista de Iceberg REST (ver arriba si la pide o no): por ATTACH")
            print("    solo se resuelven TABLAS. Una View sigue necesitando que alguien sepa que el")
            print("    texto la nombra —la regex hoy, o ore-serve analizando el texto—.")
    finally:
        srv.shutdown()
    return 0


if __name__ == "__main__":
    sys.exit(main())
