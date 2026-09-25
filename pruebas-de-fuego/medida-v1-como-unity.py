"""/v1 COMO UNITY (0038 P4) · lo que hacen los clientes de verdad con el `prefix`.

Decidido en 0038: en el catálogo REST de Iceberg la BASE es el `prefix` (el
`warehouse` que el cliente pide) y el SCHEMA el namespace, de un nivel — como
el Iceberg REST de Unity Catalog, donde Spark nombra `ventas.espana.pedidos`
igual que el SQL. Antes de tocar ore-serve, lo que hay que saber de los
clientes: ¿mandan `warehouse` a `/v1/config`, respetan el `prefix` que vuelve
en `overrides`, y con qué rutas piden después?

Un servidor de mentira que apunta cada petición y contesta lo mínimo de la
spec: `config` con `overrides.prefix`, una base con los schemas `default` y
`espana`, y la tabla `espana.pedidos` que no existe (404, lo que diga el
cliente es lo que se mide).

  §1 PyIceberg   load_catalog(uri, warehouse=ventas): list_namespaces,
                 list_tables(espana), load_table(espana.pedidos)
  §2 DuckDB      ATTACH 'ventas' AS v (TYPE iceberg, ENDPOINT …): SHOW ALL
                 TABLES, select * from v.espana.pedidos
  §3 sin prefix  un cliente que no manda warehouse: ¿qué rutas pide? (lo de hoy)

    python pruebas-de-fuego/medida-v1-como-unity.py
"""
import json
import sys
import threading
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
PEDIDAS = []


class Mentira(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _json(self, codigo, obj):
        b = json.dumps(obj).encode()
        self.send_response(codigo)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def _todo(self):
        u = urllib.parse.urlsplit(self.path)
        largo = int(self.headers.get("content-length") or 0)
        if largo:
            self.rfile.read(largo)
        PEDIDAS.append("%s %s%s" % (self.command, u.path, ("?" + u.query) if u.query else ""))
        q = urllib.parse.parse_qs(u.query)
        seg = [urllib.parse.unquote(s) for s in u.path.strip("/").split("/")]
        if seg[:2] == ["v1", "config"]:
            wh = (q.get("warehouse") or [None])[0]
            return self._json(200, {"defaults": {}, "overrides": {"prefix": wh} if wh else {}})
        if seg[0] == "v1":
            resto = seg[1:]
            if resto and resto[0] not in ("namespaces", "transactions", "config"):
                resto = resto[1:]  # el prefix
            if resto == ["namespaces"]:
                return self._json(200, {"namespaces": [["default"], ["espana"]]})
            if len(resto) == 2 and resto[0] == "namespaces":
                return self._json(200, {"namespace": [resto[1]], "properties": {}})
            if len(resto) == 3 and resto[2] == "tables":
                return self._json(200, {"identifiers": [{"namespace": [resto[1]], "name": "pedidos"}]})
            if len(resto) == 3 and resto[2] == "views":
                return self._json(200, {"identifiers": []})
        return self._json(404, {"error": {"message": "de mentira: no hay", "type": "NoSuchTableException", "code": 404}})

    do_GET = do_POST = do_HEAD = do_DELETE = _todo


def servidor():
    s = ThreadingHTTPServer(("127.0.0.1", 0), Mentira)
    threading.Thread(target=s.serve_forever, daemon=True).start()
    return s, "http://127.0.0.1:%d" % s.server_address[1]


def ver(titulo, f):
    PEDIDAS.clear()
    try:
        r = f()
        dice = "→ %s" % (r,)
    except Exception as e:
        dice = "→ %s: %s" % (type(e).__name__, " ".join(str(e).split())[:200])
    print("\n  " + titulo)
    print("    " + dice)
    for p in PEDIDAS:
        print("    · " + p)


s, url = servidor()

print("§1 PyIceberg")
try:
    from pyiceberg.catalog import load_catalog

    cat = load_catalog("m", **{"type": "rest", "uri": url, "warehouse": "ventas"})
    ver("list_namespaces()", lambda: cat.list_namespaces())
    ver("list_tables('espana')", lambda: cat.list_tables("espana"))
    ver("load_table('espana.pedidos')", lambda: cat.load_table("espana.pedidos"))
    cat2 = load_catalog("m2", **{"type": "rest", "uri": url})
    print("\n§3 sin warehouse (PyIceberg)")
    ver("list_tables('ventas')", lambda: cat2.list_tables("ventas"))
except ImportError:
    print("  (sin pyiceberg)")

print("\n§2 DuckDB")
try:
    import duckdb

    con = duckdb.connect()
    con.execute("install iceberg; load iceberg")
    ver("ATTACH 'ventas' AS v (TYPE iceberg, ENDPOINT …, AUTHORIZATION_TYPE none)",
        lambda: con.execute("attach 'ventas' as v (type iceberg, endpoint '%s', authorization_type 'none')" % url).fetchall())
    ver("show all tables", lambda: con.execute("show all tables").fetchall())
    ver("select * from v.espana.pedidos", lambda: con.execute("select * from v.espana.pedidos").fetchall())
    ver("select * from v.pedidos (dos partes)", lambda: con.execute("select * from v.pedidos").fetchall())
except ImportError:
    print("  (sin duckdb)")
s.shutdown()
