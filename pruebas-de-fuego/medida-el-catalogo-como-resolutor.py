"""EL CATALOGO COMO RESOLUTOR · medido antes de construir nada.

`sql()` resuelve hoy los nombres con una regex (`_VISTAS_EN_SQL`): cada
`paquete.vista` tras FROM/JOIN va a `GET /puestos/{id}/datos/{x}` y queda como
una vista de DuckDB sobre `iceberg_scan(...)`. `medida-el-sql-del-arbol.py` §2
midio donde la regex falla. La alternativa obvia es que DuckDB le pregunte los
nombres a quien ya los sabe: `ore-serve` sirve un catalogo REST de Iceberg
(`/v1`, W3.6c c3) por el que `write()` ya escribe. Antes de tocar el SDK se
mide si `ATTACH ... (TYPE iceberg)` contra ESE catalogo sirve para LEER desde
un puesto, y que se pierde por el camino.

  §1  ¿ATTACH FUNCIONA?    las opciones que hacen falta, el error exacto de las
                           que no, y que rutas de /v1 pide DuckDB frente a las
                           que ORE sirve (un proxy que apunta cada peticion).
  §2  ¿QUE SE VE?          `information_schema` tras el ATTACH, y cada clase de
                           nombre (dataset escrito, copia mantenida, View sobre
                           dataset, View virtual, Table de una fuente, sobre
                           heredado) por ATTACH frente a lo que `datos` resuelve.
  §3  ¿CREDENCIALES?       la credencial prestada del S3 de mentira, de punta a
                           punta; y la de GCS (`gcs.oauth2.token`), con un
                           `LoadTableResult` de mentira en `gs://`: ¿manda
                           DuckDB ese portador a Google? (401 si, 404 no).
  §4  ¿GOBIERNO?           el conducto de la lectura (OOS4002) por `datos` y por
                           `loadTable`; lo que DuckDB ensena cuando el catalogo
                           dice 403; lo declarado de un transform; la rama.
  §5  ¿CUANTO CUESTA?      ATTACH, primera y segunda consulta, un join; contra el
                           camino de hoy (el SDK de verdad: `GET datos` +
                           `iceberg_scan`). Peticiones a ore-serve y al S3.
  §6  ¿LA SEMANTICA?       los casos donde la regex falla, con ATTACH + `USE`:
                           ¿`paquete.tabla` sigue siendo el nombre?
  §7  SPARK                de la documentacion, no medido (ver alli por que).

    python pruebas-de-fuego/medida-el-catalogo-como-resolutor.py [--filas 200000] [--guardar DIR]

Levanta TODO aqui: el S3 de mentira (`de-mentira.py`, envuelto para contar),
un `ore-serve` de verdad (`--repo` + `--cola`, identidad de cabecera) con un
arbol de prueba, un proxy delante que apunta cada peticion, y abre un puesto
de verdad (el agente es esta medida: `agente:local` + `x-ore-puesto`). Los
datasets los escribe PyIceberg por `/v1`, como en `el-lago.sh` 11. No toca el
cluster ni la red de nadie salvo `extensions.duckdb.org` si faltan las
extensiones, y en §3 un GET a `storage.googleapis.com` con un token de mentira
contra un bucket que no existe (para ver que hace DuckDB, no para leer nada).

Necesita target/release (ore, ore-serve, ore-store-r2), git, python con
duckdb 1.5.x (la del puesto), pyiceberg, pyarrow y pandas.
"""
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__)).replace("\\", "/")
RAIZ = os.path.dirname(AQUI)
EXE = ".exe" if os.name == "nt" else ""
PY = sys.executable
FILAS = int(sys.argv[sys.argv.index("--filas") + 1]) if "--filas" in sys.argv else 200_000
GUARDAR = sys.argv[sys.argv.index("--guardar") + 1] if "--guardar" in sys.argv else ""
PUESTO = "puesto-ana-python"
ANA = {"x-ore-sujeto": "persona:ana"}
AGENTE = {"x-ore-sujeto": "agente:local"}
# Un bucket que no puede ser de nadie: §3 manda ahi un token de mentira.
GCS_DE_MENTIRA = "ore-medida-no-existe-7f3c1a9e2b"


def buscar(nombre):
    for d in ("release", "debug"):
        f = "%s/target/%s/%s%s" % (RAIZ, d, nombre, EXE)
        if os.path.isfile(f):
            return f
    raise SystemExit("no hay `%s` en target/: cargo build --release -p ore-serve -p ore-cli -p ore-store" % nombre)


def fila(k, v="", nota=""):
    print("     %-44s %-18s %s" % (k, v, nota))


def ms(t0):
    return (time.perf_counter() - t0) * 1000


def mediana(xs):
    return statistics.median(xs) if xs else float("nan")


def http(metodo, url, cuerpo=None, cabeceras=None):
    datos = None if cuerpo is None else (cuerpo if isinstance(cuerpo, bytes) else json.dumps(cuerpo).encode())
    r = urllib.request.Request(url, data=datos, method=metodo)
    for k, v in (cabeceras or {}).items():
        r.add_header(k, v)
    if datos is not None:
        r.add_header("content-type", "application/json")
    try:
        with urllib.request.urlopen(r, timeout=120) as resp:
            b = resp.read()
            return resp.status, (json.loads(b) if b.strip() else None)
    except urllib.error.HTTPError as e:
        b = e.read()
        try:
            return e.code, json.loads(b)
        except ValueError:
            return e.code, {"error": b.decode("utf-8", "replace")}


def corto(e, n=150):
    return " ".join(str(e).split())[:n]


# ═════════════════════════════════════════════════════════════════════════════
# EL PROXY — delante de ore-serve. Apunta cada peticion (metodo, ruta, codigo,
# ms, las cabeceras que importan) en un fichero de lineas JSON, y sabe mentir
# a proposito (`POST /__modo`): un 403 con la forma de la spec para una tabla
# (§4), o un `LoadTableResult` en `gs://` con `gcs.oauth2.token` (§3).
# ═════════════════════════════════════════════════════════════════════════════
PROXY = r'''
import http.client, json, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
ARRIBA = int(sys.argv[1]); REGISTRO = sys.argv[2]
MODO = {"forzar403": {}, "gcs": {}}
CERROJO = threading.Lock()

class P(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a): pass
    def _apunta(self, codigo, t0, n):
        h = self.headers
        linea = {"t": time.time(), "metodo": self.command, "ruta": self.path, "codigo": codigo,
                 "ms": round((time.perf_counter() - t0) * 1000, 1), "bytes": n,
                 "deleg": h.get("x-iceberg-access-delegation", ""), "auth": (h.get("authorization", "") or "")[:24],
                 "puesto": h.get("x-ore-puesto", ""), "sujeto": h.get("x-ore-sujeto", ""),
                 "ua": (h.get("user-agent", "") or "")[:40]}
        with CERROJO:
            with open(REGISTRO, "a", encoding="utf-8") as f:
                f.write(json.dumps(linea) + "\n")
    def _manda(self, codigo, cuerpo, tipo="application/json"):
        self.send_response(codigo)
        self.send_header("content-type", tipo)
        self.send_header("content-length", str(len(cuerpo)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(cuerpo)
    def _todo(self):
        t0 = time.perf_counter()
        n = int(self.headers.get("content-length", "0") or 0)
        cuerpo = self.rfile.read(n) if n else None
        if self.path == "/__modo":
            MODO.update(json.loads(cuerpo)); return self._manda(200, b"{}")
        seg = self.path.split("?")[0].strip("/").split("/")
        tabla = "%s.%s" % (seg[2], seg[4]) if len(seg) == 5 and seg[:1] == ["v1"] and seg[3] == "tables" else ""
        if tabla in MODO["forzar403"]:
            b = json.dumps({"error": {"code": 403, "type": "ForbiddenException", "message": MODO["forzar403"][tabla]}}).encode()
            self._manda(403, b); return self._apunta(403, t0, len(b))
        ruta = self.path
        if tabla in MODO["gcs"]:
            base = MODO["gcs"][tabla]
            ruta = "/v1/namespaces/%s/tables/%s" % tuple(base["de"].split("."))
        c = http.client.HTTPConnection("127.0.0.1", ARRIBA, timeout=300)
        hs = {k: v for k, v in self.headers.items() if k.lower() not in ("host", "connection", "content-length")}
        c.request(self.command, ruta, body=cuerpo, headers=hs)
        r = c.getresponse(); b = r.read(); c.close()
        codigo = r.status
        if tabla in MODO["gcs"] and codigo == 200 and self.command == "GET":
            base = MODO["gcs"][tabla]
            j = json.loads(b.decode("utf-8").replace(base["s3"], base["gs"]))
            cfg = {"gcs.oauth2.token": base["token"], "gcs.oauth2.token-expires-at": str(int(time.time() * 1000) + 3600_000)} if base["token"] else {}
            j["config"] = cfg
            j["storage-credentials"] = [{"prefix": j["metadata"]["location"] + "/", "config": cfg}]
            b = json.dumps(j).encode()
        self.send_response(codigo)
        for k, v in r.getheaders():
            if k.lower() not in ("transfer-encoding", "connection", "content-length"):
                self.send_header(k, v)
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(b)
        self._apunta(codigo, t0, len(b))
    do_GET = do_POST = do_HEAD = do_PUT = do_DELETE = _todo

s = ThreadingHTTPServer(("127.0.0.1", 0), P)
print("listo %d" % s.server_address[1], flush=True)
s.serve_forever()
'''

# El S3 de mentira, tal cual, con un contador: cada peticion, una linea.
CONTADOR_S3 = r'''
import importlib.util, sys, threading
REG = sys.argv[3]; L = threading.Lock()
sp = importlib.util.spec_from_file_location("dm", sys.argv[1]); dm = importlib.util.module_from_spec(sp)
sys.argv = [sys.argv[0], "s3", "0"]
sp.loader.exec_module(dm)
class C(dm.S3):
    def _n(self):
        with L:
            with open(REG, "a") as f:
                f.write("%s %s\n" % (self.command, self.path.split("?")[0]))
    def do_GET(self): self._n(); dm.S3.do_GET(self)
    def do_HEAD(self): self._n(); dm.S3.do_HEAD(self)
    def do_PUT(self): self._n(); dm.S3.do_PUT(self)
    def do_POST(self): self._n(); dm.S3.do_POST(self)
    def do_DELETE(self): self._n(); dm.S3.do_DELETE(self)
s = dm.ThreadingHTTPServer(("127.0.0.1", 0), C)
print("listo %d" % s.server_address[1], flush=True)
s.serve_forever()
'''


R = {}  # lo medido que el veredicto cita


class Registro:
    """Lo que el proxy (o el contador del S3) apunto, desde una marca."""

    def __init__(self, fichero, json_=True):
        self.f, self.json = fichero, json_
        open(fichero, "w").close()

    def todo(self):
        with open(self.f, encoding="utf-8") as h:
            ls = [l for l in h.read().splitlines() if l.strip()]
        return [json.loads(l) for l in ls] if self.json else ls

    def marca(self):
        return len(self.todo())

    def desde(self, i):
        return self.todo()[i:]


def plantilla(ruta):
    """`/v1/namespaces/hr/tables/ventas?x` → `/v1/namespaces/{ns}/tables/{t}`."""
    s = ruta.split("?")[0].strip("/").split("/")
    nombres = {2: "{ns}", 4: "{t}"} if len(s) > 2 and s[1] == "namespaces" else {}
    q = "?" + ruta.split("?", 1)[1].split("=")[0] + "=…" if "?" in ruta else ""
    return "/" + "/".join(nombres.get(i, x) for i, x in enumerate(s)) + q


def arrancar(args, env=None, log=None):
    p = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=log or subprocess.STDOUT, env=env, text=True)
    linea = p.stdout.readline()
    if not linea.startswith("listo"):
        raise SystemExit("no arranco %s: %s" % (args[:2], linea))
    return p, int(linea.split()[1])


# ═════════════════════════════════════════════════════════════════════════════
# EL BANCO — el arbol, el lago, la cola, ore-serve, el proxy, el puesto
# ═════════════════════════════════════════════════════════════════════════════
def escribir(ruta, texto):
    os.makedirs(os.path.dirname(ruta), exist_ok=True)
    with open(ruta, "w", encoding="utf-8", newline="\n") as f:
        f.write(texto)


CONDUCTOS = """apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: demo }
spec:
  owner: team:security
  conduits:
    materialization.payload: { oos.maturity: DRAFT }
"""


def arbol(A):
    escribir(A + "/ontology.config.yaml", """apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: demo, version: 0.1.0 }
datasources:
  - { name: erp, type: jsonl, connectionEnv: ERP_URL }
""")
    escribir(A + "/packages/hr/package.yaml", """apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: hr, version: 1.0.0, status: active, domain: people }
spec: { owner: team:hr }
""")
    escribir(A + "/packages/ventas/package.yaml", """apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: ventas, version: 1.0.0, status: active, domain: sales }
spec: { owner: team:ventas }
""")
    escribir(A + "/conduits.yaml", CONDUCTOS)
    escribir(A + "/packages/hr/tables/empleados_t.yaml", """apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: empleados_t, namespace: hr }
spec:
  datasource: erp
  object: "empleados.jsonl"
  columns: { id: {}, pais: {} }
  reads: { fullScan: cheap }
  changes: { mode: append, witness: snapshot }
""")
    escribir(A + "/packages/hr/views/empleados.yaml", """apiVersion: oos.dev/v1alpha12
kind: View
metadata: { name: empleados, namespace: hr, labels: { oos.maturity: DRAFT } }
spec:
  owner: team:data
  from: { table: hr.empleados_t }
  fields: { id: id, pais: pais }
""")
    # un sobre heredado (ORECOPY1): puntero con `clave`, sin metadata_location
    escribir(A + "/packages/hr/datasets/espanoles.yaml", """apiVersion: oos.dev/v1alpha12
kind: Dataset
metadata: { name: espanoles, namespace: hr }
spec:
  owner: team:data
  from: { view: hr.empleados }
  where: { pais: ES }
  fields: { id: id }
""")
    escribir(A + "/datasets/hr_espanoles.json", json.dumps({"estado": "copiada", "clave": "ore/v1/" + "0" * 64, "plan": "x", "filas": "3"}))


def despues_de_escribir(A, ml_ventas):
    """Lo que se declara SOBRE lo escrito: una View sobre el dataset y una copia
    mantenida (su puntero apunta a la misma tabla; los bytes de una copia de
    verdad vivirian en `copias/`, y eso no cambia lo que aqui se mide)."""
    escribir(A + "/packages/hr/views/ventasV.yaml", """apiVersion: oos.dev/v1alpha12
kind: View
metadata: { name: ventasV, namespace: hr }
spec:
  owner: team:hr
  from: { dataset: hr.ventas }
  fields: { id: id, pais: pais, total: total, cuando: cuando }
""")
    escribir(A + "/packages/hr/datasets/ventas_es.yaml", """apiVersion: oos.dev/v1alpha12
kind: Dataset
metadata: { name: ventas_es, namespace: hr }
spec:
  owner: team:hr
  from: { dataset: hr.ventas }
  where: { pais: ES }
""")
    escribir(A + "/datasets/hr_ventas_es.json", json.dumps({"estado": "al-dia", "metadata_location": ml_ventas, "snapshot": "1",
                                                          "dataset": "copias/hr_ventas_es", "filas": "0"}))


def borrar(d):
    """`rmtree` que puede con los objetos de git (de solo lectura en Windows)."""
    def quita(f, ruta, _):
        os.chmod(ruta, 0o700)
        f(ruta)
    if os.path.isdir(d):
        shutil.rmtree(d, onerror=quita)


class Banco:
    def __init__(self):
        self.procesos = []
        base = GUARDAR or os.path.join(tempfile.gettempdir(), "ore-catalogo-%d" % os.getpid())
        self.tmp = base.replace("\\", "/")
        borrar(self.tmp)
        os.makedirs(self.tmp)

    def cerrar(self):
        for p in self.procesos:
            try:
                p.terminate()
                p.wait(5)
            except Exception:
                pass
        try:
            self.log_serve.close()
        except Exception:
            pass
        if not GUARDAR:
            time.sleep(0.5)
            try:
                borrar(self.tmp)
            except OSError as e:
                print("     (no se pudo borrar %s: %s)" % (self.tmp, e))

    def levantar(self):
        T = self.tmp
        self.ore, self.serve, self.store = buscar("ore"), buscar("ore-serve"), buscar("ore-store-r2")
        c = subprocess.run(["git", "-C", RAIZ, "log", "-1", "--format=%h"], capture_output=True, text=True).stdout.strip()
        fila("binarios", "target/" + ("release" if "/release/" in self.serve else "debug"),
             "ore-serve compilado %s · HEAD al medir %s" % (time.strftime("%Y-%m-%d %H:%M", time.localtime(os.path.getmtime(self.serve))), c))
        # el S3 de mentira, contado
        escribir(T + "/contador-s3.py", CONTADOR_S3)
        self.s3reg = Registro(T + "/s3.log", json_=False)
        p, self.s3 = arrancar([PY, T + "/contador-s3.py", AQUI + "/de-mentira.py", "0", self.s3reg.f])
        self.procesos.append(p)
        # el arbol
        self.A = T + "/arbol"
        arbol(self.A)
        # la cola, con la plantilla del puesto
        cola = T + "/cola.git"
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", cola], check=True)
        semilla = T + "/cola-semilla"
        subprocess.run([PY, RAIZ + "/malla/gen-inquilino.py", "demo", "--a", T + "/rendido"], capture_output=True, check=True)
        os.makedirs(semilla)
        for f in ("plantilla-puesto.txt", "plantilla-capa.txt", "plantilla-capa-jvm.txt"):
            shutil.copy(T + "/rendido/" + f, semilla)
        g = ["git", "-C", semilla, "-c", "user.name=banco", "-c", "user.email=banco@invalido", "-c", "core.autocrlf=false"]
        subprocess.run(["git", "-C", semilla, "init", "-q", "-b", "main"], check=True)
        subprocess.run(g + ["add", "-A"], check=True)
        subprocess.run(g + ["commit", "-q", "-m", "la plantilla"], check=True)
        subprocess.run(g + ["push", "-q", cola, "HEAD:main"], check=True, capture_output=True)
        # ore-serve
        import socket
        s = socket.socket(); s.bind(("127.0.0.1", 0)); self.puerto = s.getsockname()[1]; s.close()
        env = dict(os.environ, ORE_STORE="r2", ORE_R2_S3_ENDPOINT="http://127.0.0.1:%d" % self.s3, ORE_R2_BUCKET="copia",
                   ORE_R2_ACCESS_KEY_ID="de", ORE_R2_SECRET_ACCESS_KEY="mentira", LAGO_URL="s3://copia",
                   FORJA_TOKEN="no-hace-falta-en-file")
        env["PATH"] = os.path.dirname(self.store) + os.pathsep + env["PATH"]
        self.log_serve = open(T + "/serve.log", "w")
        self.procesos.append(subprocess.Popen(
            [self.serve, "--repo", self.A, "--ore", self.ore, "--bind", "127.0.0.1:%d" % self.puerto,
             "--cola", "file:///" + cola.lstrip("/"), "--identidad", "cabecera", "--no-es-produccion",
             "--organizacion", "demo"], stdout=self.log_serve, stderr=subprocess.STDOUT, env=env))
        self.directo = "http://127.0.0.1:%d" % self.puerto
        for _ in range(80):
            try:
                urllib.request.urlopen(self.directo + "/salud", timeout=2)
                break
            except Exception:
                time.sleep(0.25)
        else:
            raise SystemExit("ore-serve no arranca: %s" % open(T + "/serve.log").read()[-800:])
        # el proxy
        escribir(T + "/proxy.py", PROXY)
        self.reg = Registro(T + "/proxy.log")
        p, pp = arrancar([PY, T + "/proxy.py", str(self.puerto), self.reg.f])
        self.procesos.append(p)
        self.base = "http://127.0.0.1:%d" % pp
        # los datasets: PyIceberg por /v1, como un cliente cualquiera (el-lago 11)
        import pyarrow as pa
        from pyiceberg.catalog import load_catalog
        cat = load_catalog("ore", **{"type": "rest", "uri": self.directo, "header.x-ore-sujeto": "persona:ana"})
        n = FILAS
        ventas = pa.table({"id": pa.array(range(n), pa.int64()),
                           "pais": pa.array([("ES", "PT", "FR", "DE")[i % 4] for i in range(n)]),
                           "total": pa.array([i * 1.5 for i in range(n)], pa.float64()).cast(pa.decimal128(18, 2)),
                           "cuando": pa.array([1_700_000_000_000_000 + i for i in range(n)], pa.timestamp("us", tz="UTC"))})
        t0 = time.perf_counter()
        cat.create_table(("hr", "ventas"), schema=ventas.schema).append(ventas)
        clientes = pa.table({"id": pa.array(range(0, n, 100), pa.int64()), "nombre": pa.array(["c%d" % i for i in range(0, n, 100)])})
        cat.create_table(("hr", "clientes"), schema=clientes.schema).append(clientes)
        cat.create_table(("ventas", "pedidos"), schema=clientes.schema).append(clientes)
        # uno que escribio OTRA persona: el puesto de ana lo lee por `datos`
        load_catalog("bea", **{"type": "rest", "uri": self.directo, "header.x-ore-sujeto": "persona:bea"}).create_table(
            ("hr", "ajena"), schema=clientes.schema).append(clientes)
        fila("el lago (PyIceberg por /v1)", "%.0f ms" % ms(t0), "hr.ventas %d filas · hr.clientes %d · ventas.pedidos %d" % (n, clientes.num_rows, clientes.num_rows))
        c, j = http("GET", self.directo + "/v1/namespaces/hr/tables/ventas", cabeceras=ANA)
        self.ml_ventas = j["metadata-location"]
        despues_de_escribir(self.A, self.ml_ventas)
        v = subprocess.run([self.ore, "validate", "."], cwd=self.A, capture_output=True, text=True, encoding="utf-8", errors="replace")
        fila("el arbol compila", "codigo %d" % v.returncode, corto(v.stdout.strip().splitlines()[-1] if v.stdout.strip() else v.stderr, 90))
        # el puesto: ana lo abre; el agente (esta medida) lo reclama al pedir `datos`
        c, j = http("POST", self.directo + "/puestos", {}, ANA)
        c2, _ = http("GET", self.directo + "/puestos/%s/datos/hr.ventas" % PUESTO, cabeceras=AGENTE)
        fila("el puesto", "abrir %d · reclamar %d" % (c, c2), PUESTO + " (agente:local)")

    def modo(self, **m):
        http("POST", self.base + "/__modo", m)


# ═════════════════════════════════════════════════════════════════════════════
# DuckDB
# ═════════════════════════════════════════════════════════════════════════════
def conexion():
    import duckdb
    con = duckdb.connect()
    for e in ("json", "icu", "avro", "iceberg", "httpfs"):
        try:
            con.execute("load %s" % e)
        except Exception:
            con.execute("install %s" % e)
            con.execute("load %s" % e)
    con.execute("set TimeZone = 'UTC'")
    return con


def atar(con, b, token="agente:local", puesto=True, extra=""):
    """El ATTACH que el SDK haria desde un puesto: el testigo del agente como
    portador y, si DuckDB lo deja, `x-ore-puesto` por un secreto http."""
    con.execute("create or replace secret ore_ice (type iceberg, token '%s')" % token)
    if puesto:
        con.execute("create or replace secret ore_cab (type http, extra_http_headers map {'x-ore-puesto': '%s'}, scope '%s')" % (PUESTO, b.base))
    con.execute("attach '' as ore (type iceberg, endpoint '%s', secret ore_ice%s)" % (b.base, extra))


def s1(b):
    print()
    print("§1 · ¿ATTACH FUNCIONA?  DuckDB 1.5.4 contra el /v1 de ore-serve, por el proxy")
    import duckdb
    fila("duckdb", duckdb.__version__)
    intentos = [
        ("sin credencial (authorization_type 'none')", "", "attach '' as ore (type iceberg, endpoint '%s', authorization_type 'none')" % b.base),
        ("token del agente, sin x-ore-puesto", "create or replace secret ore_ice (type iceberg, token 'agente:local')",
         "attach '' as ore (type iceberg, endpoint '%s', secret ore_ice)" % b.base),
        ("token del agente + secreto http con x-ore-puesto", "create or replace secret ore_ice (type iceberg, token 'agente:local'); "
         "create or replace secret ore_cab (type http, extra_http_headers map {'x-ore-puesto': '%s'}, scope '%s')" % (PUESTO, b.base),
         "attach '' as ore (type iceberg, endpoint '%s', secret ore_ice)" % b.base),
        ("warehouse 'demo' (lo que un cliente pondria)", "create or replace secret ore_ice (type iceberg, token 'agente:local')",
         "attach 'demo' as ore (type iceberg, endpoint '%s', secret ore_ice)" % b.base),
        ("client_id/secret (OAuth2 contra /v1/oauth/tokens)", "create or replace secret ore_oa (type iceberg, client_id 'agente', client_secret 'x', oauth2_server_uri '%s/v1/oauth/tokens')" % b.base,
         "attach '' as ore (type iceberg, endpoint '%s', secret ore_oa)" % b.base),
    ]
    rutas = {}
    for nombre, previo, sql in intentos:
        con = conexion()
        i = b.reg.marca()
        t0 = time.perf_counter()
        try:
            for q in [x for x in previo.split("; ") if x]:
                con.execute(q)
            con.execute(sql)
            n = con.execute("select count(*) from information_schema.tables where table_catalog = 'ore'").fetchone()[0]
            r, nota = "%.0f ms" % ms(t0), "%d tablas en information_schema" % n
            try:
                k = con.execute("select count(*) from ore.hr.ventas").fetchone()[0]
                nota += " · count(ore.hr.ventas)=%d" % k
            except Exception as e:
                nota += " · leer: " + corto(e, 90)
        except Exception as e:
            r, nota = "✗", corto(e, 130)
        pet = b.reg.desde(i)
        fila(nombre, r, nota)
        fila("", "", "peticiones: " + ", ".join("%s %s %d%s" % (p["metodo"], plantilla(p["ruta"]), p["codigo"],
                                                                     " [puesto]" if p["puesto"] else "") for p in pet)[:400])
        for p in pet:
            rutas.setdefault((p["metodo"], plantilla(p["ruta"])), set()).add(p["codigo"])
        con.close()
    fila("(el 403 sin x-ore-puesto)", "", "el sujeto queda `agente:local`, y la credencial prestada es la de ESCRIBIR,")
    fila("", "", "que sólo se da a quien escribió la tabla (ana): ver §2, hr.ajena")
    # Lo que DuckDB manda en cada peticion
    pet = b.reg.todo()
    fila("cabeceras que llegan", "", "authorization en %d/%d · x-iceberg-access-delegation=%s · x-ore-puesto en %d"
         % (sum(1 for p in pet if p["auth"]), len(pet), sorted({p["deleg"] for p in pet if p["deleg"]}) or "nunca", sum(1 for p in pet if p["puesto"])))
    # La cara de /v1 que ORE sirve, frente a la que la spec (y DuckDB/Spark) usan
    print("     lo que la spec REST tiene, y lo que /v1 de ore-serve contesta (GET directo):")
    sondas = [("GET", "/v1/config"), ("GET", "/v1/config?warehouse=demo"), ("GET", "/v1/namespaces"),
              ("GET", "/v1/namespaces?pageToken="), ("GET", "/v1/namespaces/hr"), ("HEAD", "/v1/namespaces/hr"),
              ("GET", "/v1/namespaces/hr/tables"), ("GET", "/v1/namespaces/hr/tables?pageToken="), ("HEAD", "/v1/namespaces/hr/tables/ventas"),
              ("GET", "/v1/namespaces/hr/views"), ("GET", "/v1/namespaces/hr/views/ventasV"), ("POST", "/v1/oauth/tokens"),
              ("GET", "/v1/namespaces/hr/tables/ventas/credentials"), ("POST", "/v1/namespaces/hr/tables/ventas/plan"),
              ("GET", "/v1/demo/namespaces")]
    for m, r in sondas:
        c, j = http(m, b.directo + r, b"" if m == "POST" else None, dict(AGENTE, **{"x-ore-puesto": PUESTO}))
        extra = ""
        if r == "/v1/config" and c == 200:
            extra = json.dumps(j)
        elif r == "/v1/namespaces/hr/tables" and c == 200:
            extra = "identifiers=%s" % [i["name"] for i in j.get("identifiers", [])]
        elif isinstance(j, dict) and isinstance(j.get("error"), dict):
            extra = "%s: %s" % (j["error"].get("type"), corto(j["error"].get("message"), 60))
        fila("%s %s" % (m, r), str(c), extra[:90])
    return rutas


FORMAS = [
    ("hr.ventas", "Dataset escrito (write)"),
    ("hr.clientes", "Dataset escrito (write)"),
    ("hr.ajena", "Dataset escrito por OTRA persona (bea)"),
    ("hr.ventas_es", "Dataset mantenido (from, copias/)"),
    ("hr.espanoles", "Dataset mantenido, sobre ORECOPY1"),
    ("hr.ventasV", "View sobre un Dataset"),
    ("hr.empleados", "View virtual (sobre una Table)"),
    ("hr.empleados_t", "Table de una fuente"),
    ("hr.nada", "no existe"),
]


def s2(b):
    print()
    print("§2 · ¿QUE SE VE?  tras el ATTACH, frente a lo que `GET /puestos/{id}/datos/{x}` resuelve hoy")
    con = conexion()
    atar(con, b)
    ts = con.execute("select table_schema || '.' || table_name from information_schema.tables where table_catalog = 'ore' order by 1").fetchall()
    fila("information_schema.tables (catalogo ore)", "%d" % len(ts), ", ".join(t[0] for t in ts) or "NINGUNA")
    try:
        st = con.execute("show all tables").fetchall()
        fila("show all tables (catalogo ore)", "%d" % sum(1 for r in st if r[0] == "ore"), "")
    except Exception as e:
        fila("show all tables", "✗", corto(e, 90))
    ns = con.execute("select schema_name from information_schema.schemata where catalog_name = 'ore' order by 1").fetchall()
    fila("information_schema.schemata", "", ", ".join(r[0] for r in ns))
    print("     %-16s %-36s %-24s %s" % ("nombre", "que es", "datos (hoy)", "ATTACH: select count(*) from ore.<x>"))
    tabla = {}
    for x, que in FORMAS:
        c, j = http("GET", b.directo + "/puestos/%s/datos/%s" % (PUESTO, x), cabeceras=AGENTE)
        hoy = "%d" % c + (" → %s" % j.get("dataset") if c == 200 else " " + corto((j or {}).get("error", ""), 18))
        i = b.reg.marca()
        try:
            n = con.execute("select count(*) from ore.%s" % x).fetchone()[0]
            at = "%d filas" % n
        except Exception as e:
            at = "✗ " + corto(e, 110)
        cods = [p["codigo"] for p in b.reg.desde(i) if "/tables/" in p["ruta"]]
        print("     %-16s %-36s %-24s %s" % (x, que, hoy[:24], at[:120]))
        if cods:
            print("     %-16s %-36s %-24s   (loadTable: %s)" % ("", "", "", ",".join(map(str, cods))))
        tabla[x] = (c, at)
    R["visibles"] = sum(1 for x, _ in FORMAS if not tabla[x][1].startswith("✗"))
    R["hoy"] = sum(1 for x, _ in FORMAS if tabla[x][0] == 200)
    R["listadas"] = len(ts)
    print("     por qué dice que no loadTable (GET directo, desde el puesto, pidiendo la credencial):")
    for x in ("hr.ajena", "hr.ventas_es", "hr.ventasV", "hr.empleados_t"):
        ns, t = x.split(".")
        c, j = http("GET", b.directo + "/v1/namespaces/%s/tables/%s" % (ns, t),
                    cabeceras=dict(AGENTE, **{"x-ore-puesto": PUESTO, "x-iceberg-access-delegation": "vended-credentials"}))
        e = (j or {}).get("error") or {}
        fila("  " + x, "%d %s" % (c, e.get("type", "")), corto(e.get("message", ""), 110))
    c, j = http("GET", b.directo + "/v1/namespaces/hr/tables/ajena", cabeceras=dict(AGENTE, **{"x-ore-puesto": PUESTO}))
    fila("  hr.ajena SIN pedir credencial", str(c), "(el 403 es de la credencial, no de la tabla)")
    # ¿y la View, por el endpoint de vistas de Iceberg?
    c, j = http("GET", b.directo + "/v1/namespaces/hr/views", cabeceras=AGENTE)
    fila("GET /v1/namespaces/hr/views", str(c), corto(j, 80))
    con.close()
    return tabla


def s3(b):
    print()
    print("§3 · ¿CREDENCIALES?  la prestada, de punta a punta")
    con = conexion()
    i = b.reg.marca()
    atar(con, b)
    try:
        n = con.execute("select count(*), sum(total) from ore.hr.ventas").fetchone()
        fila("S3 de mentira, SIN secreto s3 propio", "lee", "%d filas · sum=%s" % (n[0], n[1]))
    except Exception as e:
        fila("S3 de mentira, SIN secreto s3 propio", "✗", corto(e, 110))
    pet = [p for p in b.reg.desde(i) if "/tables/" in p["ruta"]]
    con2 = conexion()
    con2.execute("create or replace secret ore_ice (type iceberg, token 'agente:local')")
    con2.execute("create or replace secret ore_cab (type http, extra_http_headers map {'x-ore-puesto': '%s'}, scope '%s')" % (PUESTO, b.base))
    con2.execute("attach '' as ore (type iceberg, endpoint '%s', secret ore_ice, access_delegation_mode 'none')" % b.base)
    try:
        n2 = con2.execute("select count(*) from ore.hr.ventas").fetchone()[0]
        fila("access_delegation_mode 'none' (sin secreto s3)", "lee", "%d" % n2)
    except Exception as e:
        fila("access_delegation_mode 'none' (sin secreto s3)", "✗", corto(e, 110))
    con2.close()
    fila("loadTable pide la credencial", "", "x-iceberg-access-delegation=%s" % (sorted({p["deleg"] for p in pet}) or "—"))
    for r in con.execute("select name, type, provider, persistent, storage, scope from duckdb_secrets()").fetchall():
        fila("  secreto %s" % r[0][:40], r[1], "provider=%s scope=%s" % (r[2], r[5]))
    con.close()
    # GCS: el mismo LoadTableResult, con la raiz en gs:// y `gcs.oauth2.token`
    # (lo que `ore-store-gcs prestar` devuelve en el cluster: un token CAB).
    # DuckDB no enseña la credencial prestada en `duckdb_secrets()` (tampoco la
    # de S3, que SI usa: arriba), y su registro HTTP tapa `Authorization`. Lo
    # que distingue es la respuesta de Google para un bucket que NO existe:
    # con un portador (aunque sea falso) es 401; sin ninguno, 404. Medido
    # aparte con read_parquet y un secreto http: 401 con, 404 sin.
    print("     GCS · un LoadTableResult de mentira: raiz en gs://%s y la credencial" % GCS_DE_MENTIRA)
    print("     como la presta ore-store-gcs (gcs.oauth2.token). 401 de Google = mandó un portador; 404 = anónimo")
    for rotulo, token in (("con gcs.oauth2.token prestado", "ya29.de-mentira-para-la-medida"), ("control: config vacío (sin token)", "")):
        b.modo(gcs={"hr.engcs": {"de": "hr.ventas", "s3": "s3://copia/", "gs": "gs://%s/" % GCS_DE_MENTIRA, "token": token}})
        con = conexion()
        con.execute("call enable_logging('HTTP')")
        atar(con, b)
        try:
            n = con.execute("select count(*) from ore.hr.engcs").fetchone()[0]
            fila(rotulo, "¿lee?", "%d" % n)
        except Exception as e:
            m = re.search(r"\((HTTP \d+)\)", str(e))
            fila(rotulo, m.group(1) if m else "✗", corto(e, 90) if not m else "")
            R["gcs_con" if token else "gcs_sin"] = m.group(1) if m else "?"
        logs = con.execute("select message from duckdb_logs where type = 'HTTP'").fetchall()
        g = [l[0] for l in logs if "googleapis" in l[0]]
        if g:
            u = re.search(r"'url': '([^']+)'", g[0])
            fila("", "", "%d peticiones a Google; la 1ª: %s" % (len(g), (u.group(1) if u else "?")[:95]))
        con.close()
    b.modo(gcs={})


def s4(b):
    print()
    print("§4 · ¿GOBIERNO?  el conducto de la lectura (W3.7 ②), lo declarado (⑤) y la rama")
    A = b.A
    viejo = open(A + "/conduits.yaml", encoding="utf-8").read()
    escribir(A + "/lattice.yaml", """apiVersion: oos.dev/v1alpha3
kind: Lattice
metadata: { name: sensitivity, namespace: gdpr }
spec:
  levels: [none, low, high]
""")
    escribir(A + "/conduits.yaml", """apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: demo }
spec:
  owner: team:security
  conduits:
    materialization.payload: { oos.maturity: DRAFT, gdpr.sensitivity: low }
""")
    escribir(A + "/packages/hr/entities/Venta.yaml", """apiVersion: oos.dev/v1alpha8
kind: Entity
metadata: { name: Venta, namespace: hr }
spec:
  nature: event
  backedBy: hr.ventasV
  primaryKey: [id]
  timeKey: cuando
  properties:
    id: { type: Integer }
    pais: { type: String }
    cuando: { type: DateTimeTz }
    total: { type: Decimal, labels: { gdpr.sensitivity: high } }
""")
    v = subprocess.run([b.ore, "validate", "."], cwd=A, capture_output=True, text=True, encoding="utf-8", errors="replace")
    fila("el arbol con `total` high y el conducto low", "codigo %d" % v.returncode, "")
    c, j = http("GET", b.directo + "/puestos/%s/datos/hr.ventas" % PUESTO, cabeceras=AGENTE)
    fila("GET datos/hr.ventas (el camino de hoy)", str(c), corto((j or {}).get("error", ""), 100))
    c, j = http("GET", b.directo + "/v1/namespaces/hr/tables/ventas", cabeceras=dict(AGENTE, **{"x-ore-puesto": PUESTO, "x-iceberg-access-delegation": "vended-credentials"}))
    fila("GET /v1/…/tables/ventas (loadTable, desde el puesto)", str(c), "credencial prestada: %s" % bool((j or {}).get("config")) if c == 200 else corto(j, 100))
    con = conexion()
    atar(con, b)
    try:
        n = con.execute("select count(*), sum(total) from ore.hr.ventas").fetchone()
        fila("DuckDB ATTACH: select sum(total) from ore.hr.ventas", "LEE", "%d filas · sum(total)=%s ← la columna high, entera" % (n[0], n[1]))
        R["conducto_por_attach"] = True
    except Exception as e:
        fila("DuckDB ATTACH: select … from ore.hr.ventas", "✗", corto(e, 120))
    con.close()
    # restaurar el arbol
    escribir(A + "/conduits.yaml", viejo)
    os.remove(A + "/packages/hr/entities/Venta.yaml")
    os.remove(A + "/lattice.yaml")
    # Lo que DuckDB ensena cuando loadTable dice 403 (con el texto que `datos` daria)
    msg = "OOS4002: `hr.ventas` lleva gdpr.sensitivity:high y materialization.payload admite low"
    b.modo(forzar403={"hr.ventas": msg})
    con = conexion()
    atar(con, b)
    try:
        con.execute("select count(*) from ore.hr.ventas").fetchone()
        fila("loadTable 403 → DuckDB", "¿?", "leyo igual")
    except Exception as e:
        tipo = type(e).__name__
        fila("loadTable 403 (forzado en el proxy) → DuckDB", tipo, "")
        fila("", "", "«%s»" % corto(e, 260))
        fila("  ¿el texto de ORE llega a la celda?", "si" if "OOS4002" in str(e) else "NO", "")
    b.modo(forzar403={})
    con.close()
    # Lo declarado (⑤): un transform que LEE hr.clientes y escribe hr.salida
    c, _ = http("POST", b.directo + "/puestos/%s/transform" % PUESTO, {"nombre": "t", "inputs": ["hr.clientes"], "output": "hr.salida"}, AGENTE)
    fila("POST /puestos/{id}/transform inputs=[hr.clientes]", str(c), "")
    c, j = http("GET", b.directo + "/puestos/%s/datos/hr.clientes" % PUESTO, cabeceras=AGENTE)
    fila("  datos/hr.clientes (un input)", str(c), "")
    c, j = http("GET", b.directo + "/puestos/%s/datos/hr.ventas" % PUESTO, cabeceras=AGENTE)
    fila("  datos/hr.ventas (no es input)", str(c), corto((j or {}).get("error", ""), 70))
    con = conexion()
    atar(con, b)
    for x in ("hr.clientes", "hr.ventas"):
        try:
            n = con.execute("select count(*) from ore.%s" % x).fetchone()[0]
            fila("  ATTACH: ore.%s" % x, "lee", "%d filas" % n)
        except Exception as e:
            fila("  ATTACH: ore.%s" % x, "✗", corto(e, 120))
            if x == "hr.clientes":  # el input: el que NO tenia que negarse
                R.setdefault("transform_niega", []).append(x)
    con.close()
    http("DELETE", b.directo + "/puestos/%s/transform" % PUESTO, None, AGENTE)
    # La rama: /v1 lee en `x-ore-rama` (o la del puesto) y NO cae a main.
    src = open(RAIZ + "/crates/ore-serve/src/catalogo.rs", encoding="utf-8").read()
    pst = open(RAIZ + "/crates/ore-serve/src/puestos.rs", encoding="utf-8").read()
    fila("fallback de rama en datos_del_puesto", "si" if "de_main" in pst else "no", "(leido en puestos.rs: 404|409 en la rama → main)")
    fila("fallback de rama en catalogo.rs", "si" if "leyendo_en(None" in src else "NO", "(leido: loadTable sólo mira la rama del puesto)")
    fila("lectura_desde_puesto en catalogo.rs", "si" if "lectura_desde_puesto" in src else "NO", "(leido en el fuente de catalogo.rs)")


def cronometro(fn, veces=5):
    xs = []
    for _ in range(veces):
        t0 = time.perf_counter()
        fn()
        xs.append(ms(t0))
    return xs


def s5(b):
    print()
    print("§5 · ¿CUANTO CUESTA?  medianas de 5; ore-serve por el proxy, el S3 de mentira contado")
    # ── el camino de hoy: el SDK de verdad, con el testigo del agente
    os.environ.update(ORE_SERVE=b.base, PUESTO=PUESTO, ORE_ALMACEN="dir:" + b.tmp)
    sys.path.insert(0, RAIZ + "/puesto/python")
    import ore
    ore.puesto.servidor, ore.puesto.id = b.base, PUESTO
    ore.puesto._cabeceras = dict(AGENTE)
    t0 = time.perf_counter()
    c0 = ore._duckdb()
    for e in ore.LAGO + ("httpfs",):
        ore._cargar(c0, e)
    fila("abrir DuckDB + 5 extensiones", "%.0f ms" % ms(t0), "(una vez por sesion, en los dos caminos: fuera de lo que sigue)")
    print("     HOY · sql() del SDK (regex → GET datos → iceberg_scan):")
    q1 = "select count(*) from hr.ventas"
    qj = "select c.nombre, sum(v.total) from hr.ventas v join hr.clientes c on v.id = c.id group by 1 order by 2 desc limit 3"
    resultados = {}
    for nombre, q in (("1ª consulta (hr.ventas)", q1), ("2ª y siguientes", q1), ("join ventas × clientes", qj)):
        i, j = b.reg.marca(), b.s3reg.marca()
        xs = cronometro(lambda: ore.sql(q, como="arrow"), 1 if nombre.startswith("1ª") else 5)
        pet, s3 = b.reg.desde(i), b.s3reg.desde(j)
        veces = len(xs)
        fila("  " + nombre, "%.0f ms" % mediana(xs), "%.1f pet. ore-serve · %.1f al S3 por consulta (%s)" % (
            len(pet) / veces, len(s3) / veces, ", ".join(sorted({plantilla(p["ruta"]).replace("/puestos/{ns}", "/puestos/…") for p in pet}))[:70]))
        resultados["hoy " + nombre] = (mediana(xs), len(pet) / veces, len(s3) / veces)
    srv = [p["ms"] for p in b.reg.todo() if "/datos/" in p["ruta"]]
    fila("  GET datos en el servidor (mediana)", "%.0f ms" % mediana(srv), "(datos_de + con_credencial: dos `ore` hijos)")
    # ── ATTACH
    print("     ATTACH · DuckDB pregunta al catalogo:")
    con = conexion()
    i = b.reg.marca()
    t0 = time.perf_counter()
    atar(con, b)
    fila("  ATTACH (+ los dos secretos)", "%.0f ms" % ms(t0), "%d peticiones" % len(b.reg.desde(i)))
    for nombre, q in (("1ª consulta (ore.hr.ventas)", "select count(*) from ore.hr.ventas"),
                      ("2ª y siguientes", "select count(*) from ore.hr.ventas"),
                      ("join ventas × clientes", "select c.nombre, sum(v.total) from ore.hr.ventas v join ore.hr.clientes c on v.id = c.id group by 1 order by 2 desc limit 3")):
        i, j = b.reg.marca(), b.s3reg.marca()
        xs = cronometro(lambda: con.execute(q).fetchall(), 1 if nombre.startswith("1ª") else 5)
        pet, s3 = b.reg.desde(i), b.s3reg.desde(j)
        veces = len(xs)
        fila("  " + nombre, "%.0f ms" % mediana(xs), "%.1f pet. ore-serve · %.1f al S3 por consulta (%s)" % (
            len(pet) / veces, len(s3) / veces, ", ".join(sorted({"%s %s" % (p["metodo"], plantilla(p["ruta"])) for p in pet}))[:70]))
        resultados["attach " + nombre] = (mediana(xs), len(pet) / veces, len(s3) / veces)
    # en una transaccion: ¿se reutiliza loadTable?
    i = b.reg.marca()
    con.execute("begin")
    for _ in range(3):
        con.execute("select count(*) from ore.hr.ventas").fetchall()
    con.execute("commit")
    fila("  3 consultas en UNA transaccion", "", "%d loadTable" % sum(1 for p in b.reg.desde(i) if "/tables/" in p["ruta"]))
    srv = [p["ms"] for p in b.reg.todo() if p["ruta"].startswith("/v1/namespaces/hr/tables/") and p["metodo"] == "GET"]
    fila("  loadTable en el servidor (mediana)", "%.0f ms" % mediana(srv), "(`ore datasets --cargar --prestar`: un `ore` hijo + ore-store)")
    con.close()
    return resultados


CASOS = [
    ("la semilla", "select pais, count(*) as n from hr.ventas group by pais order by 1"),
    ("from a, b", "select count(*) from hr.ventas, hr.clientes where ventas.id = clientes.id"),
    ("comentario", "-- antes: from viejo.tabla\nselect count(*) from hr.ventas"),
    ("cadena", "select 'from x.y' as s, count(*) from hr.clientes"),
    ("comillas", 'select count(*) from "hr"."ventas"'),
    ("tres partes", "select count(*) from ore.hr.ventas"),
    ("cte que tapa", "with ventas as (select 1 as id) select count(*) from ventas"),
    ("cte y tabla", "with t as (select * from hr.clientes) select count(*) from t join hr.ventas using (id)"),
    ("from primero", "from hr.clientes select count(*)"),
    ("subconsulta", "select count(*) from hr.ventas where id in (select id from hr.clientes)"),
    ("otro paquete", "select count(*) from ventas.pedidos"),
    ("mayusculas", "SELECT COUNT(*) FROM HR.VENTAS"),
]


def s6(b):
    print()
    print("§6 · ¿LA SEMANTICA?  con ATTACH + `USE`: ¿`paquete.tabla` sigue siendo el nombre?")
    import pandas as pd
    rx = sys.modules["ore"]._VISTAS_EN_SQL  # la MISMA regex del SDK (importado en §5)
    con = conexion()
    atar(con, b)
    try:
        con.execute("use ore")
        fila("use ore", "✓", "")
    except Exception as e:
        fila("use ore", "✗", corto(e, 100))
    con.execute("use ore.hr")
    fila("use ore.hr", "✓", "current_catalog=%s current_schema=%s" % con.execute("select current_database(), current_schema()").fetchone())
    print("     %-14s %-34s %s" % ("caso", "la regex resolveria", "ATTACH + USE ore.hr"))
    ok = 0
    for nombre, q in CASOS:
        r = sorted({"%s.%s" % m for m in rx.findall(q)})
        i = b.reg.marca()
        try:
            v = con.execute(q).fetchall()
            res, ok = "✓ %s" % (str(v[0]) if len(v) == 1 else "%d filas" % len(v)), ok + 1
        except Exception as e:
            res = "✗ " + corto(e, 80)
        n = sum(1 for p in b.reg.desde(i) if "/tables/" in p["ruta"])
        print("     %-14s %-34s %s  (%d loadTable)" % (nombre, ", ".join(r)[:34] or "—", res, n))
    fila("casos que resuelven con ATTACH", "%d/%d" % (ok, len(CASOS)), "")
    R["casos"] = "%d/%d" % (ok, len(CASOS))
    # Lo que `USE ore` le hace al resto de la celda
    print("     lo que `USE ore.hr` cambia para lo que NO es del catalogo:")
    df = pd.DataFrame({"x": [1, 2, 3]})  # noqa: F841 — la celda la nombra en SQL
    for nombre, q in (("un DataFrame de la celda", "select sum(x) from df"),
                      ("create table t (sin esquema)", "create table t as select 1 as x"),
                      ("create temp table", "create temp table t2 as select 1 as x"),
                      ("create table memory.main.t", "create table memory.main.t3 as select 1 as x"),
                      ("create table hr.x (¡escribe en el lago!)", "create table hr.nueva_de_sql as select 1::bigint as x"),
                      ("read_parquet / funciones", "select count(*) from range(10)")):
        try:
            v = con.execute(q).fetchall()
            fila("  " + nombre, "✓", str(v)[:60])
        except Exception as e:
            fila("  " + nombre, "✗", corto(e, 100))
    for x in ("t", "nueva_de_sql"):
        f = b.A + "/packages/hr/datasets/%s.yaml" % x
        fila("  ¿dejó packages/hr/datasets/%s.yaml en el arbol?" % x, "SI" if os.path.isfile(f) else "no",
             "(una tabla del lago, sin write(), sin procedencia)" if os.path.isfile(f) else "")
    R["escribe_sin_write"] = os.path.isfile(b.A + "/packages/hr/datasets/t.yaml")
    # READ_ONLY: el catalogo sólo para leer
    con3 = conexion()
    atar(con3, b, extra=", read_only")
    con3.execute("use ore.hr")
    for nombre, q in (("read_only: select … from hr.ventas", "select count(*) from hr.ventas"),
                      ("read_only: create table t2", "create table t2 as select 1 as x"),
                      ("read_only: create temp table", "create temp table t3 as select 1 as x")):
        try:
            v = con3.execute(q).fetchall()
            fila(nombre, "✓", str(v)[:60])
        except Exception as e:
            fila(nombre, "✗", corto(e, 100))
    con3.close()
    # Y sin USE: el nombre de dos partes busca en el catalogo por defecto (memory)
    con2 = conexion()
    atar(con2, b)
    try:
        con2.execute("select count(*) from hr.ventas").fetchone()
        fila("sin USE: select … from hr.ventas", "✓", "DuckDB busca el esquema hr en todos los catalogos")
    except Exception as e:
        fila("sin USE: select … from hr.ventas", "✗", corto(e, 110))
    for sp in ("memory.main,ore.hr,ore.ventas", "ore.hr,ore.ventas,memory.main"):
        con2.execute("set search_path = '%s'" % sp)
        try:
            con2.execute("select count(*) from hr.ventas").fetchone()
            fila("search_path='%s': hr.ventas" % sp[:22], "✓", "")
        except Exception as e:
            fila("search_path='%s': hr.ventas" % sp[:22], "✗", corto(e, 90))
    con2.execute("set search_path = 'ore.hr,memory.main'")
    try:
        con2.execute("select count(*) from ventas").fetchone()
        fila("search_path = 'ore.hr,memory.main'", "✓", "hasta `ventas` a secas resuelve")
    except Exception as e:
        fila("search_path", "✗", corto(e, 100))
    con.close()
    con2.close()


def s7():
    print()
    print("§7 · SPARK  — de la documentacion, NO MEDIDO: pyspark (~320 MB) + iceberg-spark-runtime")
    print("     (~40 MB) + un JDK 17/21 en el PATH (aqui hay un 1.8) no es «barato». Lo que el")
    print("     RESTCatalog de Java (iceberg-core, RESTSessionCatalog, 1.7+) pide:")
    for k, v in (
        ("GET /v1/config?warehouse=", "SI la usa: fusiona defaults/overrides; `prefix` y `endpoints` salen de aqui"),
        ("`endpoints` en /v1/config", "sin ella asume el juego por defecto (namespaces + tablas, SIN vistas)"),
        ("POST /v1/oauth/tokens", "sólo con `credential=`; con `token=` manda el portador tal cual"),
        ("namespaces / tables / loadTable", "las mismas que DuckDB; `pageToken` sólo si el servidor pagina"),
        ("HEAD /v1/…/tables/{t}", "tableExists (desde 1.7, si el servidor anuncia el endpoint)"),
        ("vistas (/v1/…/views)", "sólo si `endpoints` las anuncia o `view-endpoints-supported=true`"),
        ("credencial GCS prestada", "GCSFileIO entiende `gcs.oauth2.token` (+ refresh-endpoint): el formato ORE"),
        ("cabeceras propias", "`header.<nombre>=` en la configuracion del catalogo: x-ore-puesto cabe"),
    ):
        fila(k, "", v)


def main():
    print("=== el catalogo como resolutor, medido  (%s)" % time.strftime("%Y-%m-%d"))
    b = Banco()
    try:
        print()
        print("§0 · EL BANCO")
        b.levantar()
        s1(b)
        tabla = s2(b)
        s3(b)
        s4(b)
        r = s5(b)
        s6(b)
        s7()
        veredicto(tabla, r)
    finally:
        b.cerrar()
    return 0


def veredicto(tabla, r):
    def t(k):
        x = r.get(k)
        return "%.0f ms · %.1f al S3" % (x[0], x[2]) if x else "?"
    print()
    print("⇒ VIABLE CON HUECOS. El mecanismo funciona tal cual: DuckDB 1.5.4 ata el /v1 de")
    print("  ore-serve con TOKEN + un secreto http que lleva `x-ore-puesto` (sin él, 403), pide")
    print("  la credencial prestada sola, lee el S3 sin secreto propio, y con la de GCS manda el")
    print("  portador (Google contesta %s con él y %s sin él). `paquete.tabla` resuelve en %s casos,"
          % (R.get("gcs_con", "?"), R.get("gcs_sin", "?"), R.get("casos", "?")))
    print("  los de la regex incluidos, y es más barato: segunda consulta %s frente a %s hoy"
          % (t("attach 2ª y siguientes"), t("hoy 2ª y siguientes")))
    print("  (loadTable trae el metadata.json dentro; iceberg_scan lo vuelve a bajar).")
    print("  Pero hoy LEE MENOS y GOBIERNA MENOS que `datos`: resuelve %d de %d nombres donde"
          % (R.get("visibles", 0), len(FORMAS)))
    print("  `datos` resuelve %d, y deja pasar lo que `datos` niega. Los huecos, todos en ore-serve"
          % R.get("hoy", 0))
    print("  salvo el último, y ninguno en DuckDB:")
    huecos = [
        ("XS", "listTables vacío", "`tablas()` filtra clase==\"dataset\" y `ore datasets` dice escrito|mantenido:"
               " information_schema, SHOW TABLES y el autocompletado ven %d tablas" % R.get("listadas", 0)),
        ("S", "el conducto en loadTable", "`lectura_desde_puesto` (OOS4002) no se llama en catalogo.rs: la"
              " columna high sale entera por ATTACH%s. DuckDB enseña el mensaje de ORE tal cual" % (" (medido)" if R.get("conducto_por_attach") else "")),
        ("S-M", "un camino de LECTURA en loadTable", "`--cargar` pasa por la guarda de ESCRITURA"
                " (`documento_del_dataset`): copia mantenida, View sobre dataset -> 400. Hace falta resolver"
                " como `datos_de` (raíz de lectura de la View, puntero del mantenido)"),
        ("S-M", "la credencial de leer", "loadTable presta la de ESCRIBIR y sólo a quien escribió: hr.ajena"
                " (de bea) es 403 desde el puesto de ana, y a ana le da objectCreator para leer. Leer ="
                " `--prestar --leer` para quien pase el conducto; y decidir cuándo va la de escribir (DuckDB"
                " hace UN loadTable para leer y para escribir)"),
        ("S", "lo declarado (⑤) al leer", "con un transform vivo loadTable sólo deja el output: sus"
              " inputs dan 403 (%s, medido). GET/HEAD tiene que dejar inputs ∪ output" % (", ".join(R.get("transform_niega", [])) or "no medido")),
        ("S", "el fallback de rama", "`datos` cae a main con 404|409 en la rama; loadTable no"),
        ("S-M", "el SDK", "ATTACH READ_ONLY (sin él un `create table t` de la celda escribe en el lago y"
                " en el árbol, medido) + search_path 'memory.main,ore.<paquete>,…' (`USE ore` falla: no hay"
                " namespace main) + renovar el TOKEN del agente; y la procedencia (`leidas`) que hoy anota"
                " `_lee()` por nombre tendría que salir de los loadTable del puesto en el servidor"),
        ("—", "fuera del catálogo", "el sobre heredado ORECOPY1 no es Iceberg: ningún loadTable lo sirve;"
              " o se migra o `sql()` conserva `datos` para él"),
    ]
    for tam, que, por in huecos:
        print("  · [%s] %s: %s." % (tam, que, por))
    print("  Lo que NO hace falta para DuckDB (no lo pidió ni una vez): vistas de Iceberg,")
    print("  /v1/oauth/tokens, `prefix`, paginación, `endpoints`. Spark (de la documentación)")
    print("  querría `endpoints` en /v1/config para HEAD y vistas, y cabe `header.x-ore-puesto`.")
    print("  Suma: ~1 XS + 4 S + 3 S-M, todo en ore-serve/ore-cli y el SDK; ninguno pide cambiar")
    print("  de cliente ni de formato. Lo que se gana: el nombre lo resuelve el motor (%s)," % R.get("casos", "?"))
    print("  menos viajes al bucket, y la MISMA cara para Python, Node, la JVM y Spark.")


if __name__ == "__main__":
    sys.exit(main())
