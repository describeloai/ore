"""LA LECTURA EN VIVO EN PYTHON (ADR 0053 F6·2), sin clúster.

`ore.sql()` contra un `ore-serve` de mentira que contesta como el de F6·1: el
reparto de la sentencia en `fuentes` (`federada`, `vistaFederada`), las lecturas
en Arrow por `/federation/read` y su final por `GET /federation/read/{id}`.

  1  una Table: se pide lo repartido (columnas, filtros, limit, orderBy con su
     `direccion`) y la sentencia corre tal cual — el servidor de mentira manda
     las 3 filas aunque se empujó `pais = 'ES'`, y DuckDB vuelve a filtrar (C)
  2  una vista viva sobre la Table, y una junta de las dos: una lectura por tabla
  3  un no del reparto (`OOS2044`) es `OriginReadError` con su código
  4  una lectura cortada: `TruncatedReadWarning`; con `strict=True`, error
  5  `ore.explain()`: imprime el texto y devuelve el plan
  6  (F7·1) `write()` de lo que se leyó en vivo se niega: es una copia

    PYTHONUTF8=1 python pruebas-de-fuego/la-lectura-en-vivo-en-python.py
"""
import http.server
import io
import json
import os
import sys
import threading
import warnings

import pyarrow as pa

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(RAIZ, "puesto", "python"))

PEDIDAS = []
CORTAR = {"si": False}

LECTURA = {"tabla": "pg.public.clientes", "fuente": "pg", "tipo": "postgres", "columnas": ["id", "pais"],
           "columnasDeLaTabla": ["id", "pais", "nota"],
           "empujados": [{"columna": "pais", "operador": "eq", "valor": "ES"}], "enElMotor": [],
           "orderBy": [{"columna": "id", "desc": True}], "limit": 10}


def arrow():
    t = pa.table({"id": pa.array([1, 2, 3], pa.int64()), "pais": pa.array(["ES", "PT", "ES"])})
    b = io.BytesIO()
    with pa.ipc.new_stream(b, t.schema) as w:
        w.write_table(t)
    return b.getvalue()


class H(http.server.BaseHTTPRequestHandler):
    def _json(self, codigo, d, cab=()):
        b = json.dumps(d).encode()
        self.send_response(codigo)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b)))
        for k, v in cab:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(b)

    def do_GET(self):
        PEDIDAS.append(("GET", self.path, None))
        if self.path.startswith("/federation/read/"):
            estado = "cortado" if CORTAR["si"] else "completo"
            return self._json(200, {"id": self.path.rsplit("/", 1)[1], "estado": estado, "motivo": "filas" if CORTAR["si"] else "", "filas": 3})
        self._json(404, {"error": "no"})

    def do_POST(self):
        n = int(self.headers.get("content-length", "0"))
        cuerpo = json.loads(self.rfile.read(n) or b"{}")
        PEDIDAS.append(("POST", self.path, cuerpo))
        if self.path == "/puestos/p1/sql":
            q = cuerpo["texto"]
            if "prohibida" in q:
                return self._json(422, {"error": "`pg.public.prohibida` declara `fullScan: forbidden`", "codigo": "OOS2044", "nombre": "pg.public.prohibida"})
            f = {"pg.public.clientes": {"federada": LECTURA}}
            if "pg.v_es" in q:
                # Nombra `nota`, que la sentencia no usa y no se pidió (lo que
                # rompió `bq_foreign` en vivo, F6·3).
                f["pg.v_es"] = {"vistaFederada": "SELECT id AS ident, pais, nota FROM pg.public.clientes"}
            return self._json(200, {"fuentes": f})
        if self.path == "/puestos/p1/explain":
            return self._json(200, {"plan": {"ok": True, "lecturas": [LECTURA]}, "texto": "pg.public.clientes\n  al origen    columnas  id, pais\n"})
        if self.path == "/federation/read":
            b = arrow()
            self.send_response(200)
            self.send_header("content-type", "application/vnd.apache.arrow.stream")
            self.send_header("ore-lectura", "fed-%d" % len(PEDIDAS))
            self.send_header("content-length", str(len(b)))
            self.end_headers()
            self.wfile.write(b)
            return
        self._json(404, {"error": "no"})

    def log_message(self, *a):
        pass


srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
threading.Thread(target=srv.serve_forever, daemon=True).start()
os.environ["ORE_SERVE"] = "http://127.0.0.1:%d" % srv.server_port
os.environ["PUESTO"] = "p1"
import ore  # noqa: E402

ore.session._cabeceras = {"x-ore-pod": "de-mentira"}
MAL = []


def mal(m):
    MAL.append(m)
    print("  ✗ " + m)


def bien(m):
    print("  ✓ " + m)


# 1
d = ore.sql("SELECT id FROM pg.public.clientes WHERE pais = 'ES' ORDER BY id DESC LIMIT 10")
leidas = [c for (m, r, c) in PEDIDAS if r == "/federation/read"]
esperado = {"tabla": "pg.public.clientes", "columnas": ["id", "pais"], "filtros": LECTURA["empujados"],
            "limit": 10, "orderBy": [{"columna": "id", "direccion": "desc"}]}
if leidas != [esperado]:
    mal("1 · lo pedido a /federation/read: %s" % leidas)
elif list(d["id"]) != [3, 1]:
    mal("1 · DuckDB no volvió a filtrar: %s" % list(d["id"]))
else:
    bien("1 · una Table: se pide lo repartido (orderBy con `direccion`) y DuckDB vuelve a filtrar → [3, 1]")

# 2
PEDIDAS.clear()
d = ore.sql("SELECT v.ident, c.pais FROM pg.v_es v JOIN pg.public.clientes c ON c.id = v.ident WHERE v.pais = 'ES' ORDER BY 1")
n = len([1 for (m, r, c) in PEDIDAS if r == "/federation/read"])
if n != 1 or list(d["ident"]) != [1, 3]:
    mal("2 · vista viva y junta: %d lecturas, %s" % (n, list(d["ident"])))
else:
    bien("2 · una vista viva sobre la Table, juntada con ella: UNA lectura, [1, 3]")

# 3
try:
    ore.sql("SELECT * FROM pg.public.prohibida")
    mal("3 · forbidden no falló")
except ore.OriginReadError as e:
    (bien if e.code == "OOS2044" and e.table == "pg.public.prohibida" else mal)("3 · un no del reparto → OriginReadError [%s] en %s" % (e.code, e.table))

# 4
CORTAR["si"] = True
with warnings.catch_warnings(record=True) as w:
    warnings.simplefilter("always")
    ore.sql("SELECT id FROM pg.public.clientes")
avisos = [x for x in w if issubclass(x.category, ore.TruncatedReadWarning)]
(bien if avisos and "incomplete" in str(avisos[0].message) else mal)("4 · cortada → TruncatedReadWarning: %s" % (avisos[0].message if avisos else w))
try:
    ore.sql("SELECT id FROM pg.public.clientes", strict=True)
    mal("4 · strict no falló")
except ore.OriginReadError as e:
    (bien if e.code == "cortado" else mal)("4 · strict=True → OriginReadError [%s]" % e.code)
CORTAR["si"] = False

# 5
buf = io.StringIO()
sys_stdout, sys.stdout = sys.stdout, buf
plan = ore.explain("SELECT id FROM pg.public.clientes WHERE pais = 'ES'")
sys.stdout = sys_stdout
(bien if "al origen" in buf.getvalue() and plan["lecturas"][0]["tabla"] == "pg.public.clientes" else mal)(
    "5 · ore.explain(): imprime el texto y devuelve el plan")

# 6 · F7·1: guardar en el lago lo leído en vivo es una copia, y no se hace aquí
for como in ("pandas", "arrow"):
    d = ore.sql("SELECT id FROM pg.public.clientes", format=como)
    try:
        ore.write("hr.copia_a_mano", d)
        mal("6 · write() de lo leído en vivo (%s) no se negó" % como)
    except PermissionError as e:
        (bien if "create or replace dataset" in str(e) and "pg.public.clientes" in str(e) else mal)(
            "6 · write() de lo leído en vivo (%s) → PermissionError que dice cómo copiar" % como)

srv.shutdown()
print()
print("✓ la lectura en vivo en Python (0053 F6·2 y F7·1)" if not MAL else "✗ la lectura en vivo en Python")
sys.exit(1 if MAL else 0)
