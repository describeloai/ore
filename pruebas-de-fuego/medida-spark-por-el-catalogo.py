"""SPARK POR EL CATALOGO · medido antes de servir las Views por /v1.

La fase 2 de la regex (ver 8d18994): que `/v1` sirva las Views del arbol con
`loadView` —la spec REST de Iceberg— para que Spark (y Trino) las lean sin que
nadie mire el texto. DuckDB 1.5 no pide vistas (`medida-las-vistas-por-el-
catalogo.py`); Spark, segun la documentacion, si. Antes de construir nada se
mide con un Spark DE VERDAD (Docker: `apache/spark` 3.5 + `iceberg-spark-
runtime` 1.10 + `iceberg-aws-bundle`) contra el `/v1` de un `ore-serve` de
verdad —el mismo banco que `medida-el-catalogo-como-resolutor.py`: el S3 de
mentira, el arbol de prueba, PyIceberg escribiendo por `/v1`, un puesto
abierto—.

Entre Spark y ore-serve hay una LENTE (Python, stdlib): apunta cada peticion
con el paso que la provoco, cambia `127.0.0.1` por `host.docker.internal` en
lo que devuelve (Spark vive en un contenedor), y —solo en §3— sirve Views con
la forma de la spec (`LoadViewResult`), para ver que pide Spark antes de que
ore-serve las sirva de verdad.

  §1  ¿CONECTA?        config, namespaces, SHOW TABLES / SHOW VIEWS: que pide
                       Spark y que contesta /v1 hoy.
  §2  ¿QUE LEE HOY?    cada clase de nombre por `loadTable` (dataset escrito,
                       de otra persona, copia mantenida, sobre heredado, View
                       sobre dataset, View sobre Table, Table): filas o error.
                       Y los tipos del contrato (decimal, timestamptz).
  §3  ¿LEE VIEWS?      con la lente sirviendo `loadView`: sin `endpoints`, con
                       `endpoints`, con `view-endpoints-supported`; dialecto
                       spark, duckdb, los dos; el nombre en la SQL con y sin
                       catalogo. Filas contra lo que da la View en ORE.
  §4  ¿GOBIERNO?       un 403 de la spec en `loadTable` (el conducto, OOS4002):
                       que ensena Spark, leyendo la tabla y leyendo una View
                       que la lee. Y sin `vended-credentials`.
  §5  ¿CUANTO?         arrancar la sesion, la primera consulta, la segunda.
  §6  LO DE VERDAD     sin simular nada (la lente solo apunta y traduce la
                       direccion): lo que ore-serve sirve ya —loadView con el
                       dialecto spark, el 404 de una View como tabla, los
                       endpoints, listTables, lo de otra persona desde un
                       puesto, la copia mantenida—: filas contra ORE, una
                       cadena con comilla y barra, una View virtual, la
                       copia heredada y el 403 del conducto por una View.

    python pruebas-de-fuego/medida-spark-por-el-catalogo.py --jars DIR [--imagen apache/spark:3.5.6-python3] [--filas 20000]

`--jars DIR`: donde estan `iceberg-spark-runtime-3.5_2.12-1.10.0.jar` y
`iceberg-aws-bundle-1.10.0.jar` (Maven Central). Necesita Docker y lo mismo
que `medida-el-catalogo-como-resolutor.py` (target/release, pyiceberg...). No
toca el cluster ni la red de nadie: Spark habla con la lente y el S3 de
mentira, los dos en esta maquina.
"""
import importlib.util
import json
import os
import subprocess
import sys
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__)).replace("\\", "/")


def arg(nombre, defecto):
    return sys.argv[sys.argv.index(nombre) + 1] if nombre in sys.argv else defecto


JARS = arg("--jars", "")
IMAGEN = arg("--imagen", "apache/spark:3.5.6-python3")
FILAS = int(arg("--filas", "20000"))
if not JARS or not os.path.isdir(JARS):
    raise SystemExit("falta --jars DIR con iceberg-spark-runtime-3.5_2.12-1.10.0.jar e iceberg-aws-bundle-1.10.0.jar")

# El banco de la medida del catalogo, tal cual (lee --filas al importarse).
_argv = sys.argv
sys.argv = [sys.argv[0], "--filas", str(FILAS)] + (["--guardar", arg("--guardar", "")] if "--guardar" in sys.argv else [])
_sp = importlib.util.spec_from_file_location("resolutor", AQUI + "/medida-el-catalogo-como-resolutor.py")
R = importlib.util.module_from_spec(_sp)
_sp.loader.exec_module(R)
sys.argv = _argv
fila, escribir, arrancar = R.fila, R.escribir, R.arrancar

# ═════════════════════════════════════════════════════════════════════════════
# LA LENTE — delante del proxy del banco (que ya sabe forzar un 403).
# ═════════════════════════════════════════════════════════════════════════════
LENTE = r'''
import http.client, json, sys, threading, time, uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
ARRIBA = int(sys.argv[1]); REGISTRO = sys.argv[2]
MODO = {"paso": "", "vistas": "no", "config": "tal-cual", "calificar": "", "vista404": False, "vistas_def": {}}
UUIDS = {}
CERROJO = threading.Lock()
FUERA = "host.docker.internal"

def apunta(linea):
    with CERROJO:
        with open(REGISTRO, "a", encoding="utf-8") as f:
            f.write(json.dumps(linea) + "\n")

def arriba(metodo, ruta, cuerpo=None, cabeceras=None):
    c = http.client.HTTPConnection("127.0.0.1", ARRIBA, timeout=300)
    c.request(metodo, ruta, body=cuerpo, headers=cabeceras or {})
    r = c.getresponse(); b = r.read(); c.close()
    return r.status, r.getheaders(), b

def carga_vista(ns, v, d):
    """Un `LoadViewResult` de la spec con la SQL en los dialectos del modo."""
    ahora = int(time.time() * 1000)
    reps = []
    pref = (MODO["calificar"] + ".") if MODO["calificar"] else ""
    if MODO["vistas"] in ("duckdb", "ambos"):
        reps.append({"type": "sql", "sql": d["duckdb"], "dialect": "duckdb"})
    if MODO["vistas"] in ("spark", "ambos"):
        reps.append({"type": "sql", "sql": d["spark"].replace("{p}", pref), "dialect": "spark"})
    u = UUIDS.setdefault(ns + "." + v, str(uuid.uuid4()))
    meta = {"view-uuid": u, "format-version": 1, "location": "s3://copia/vistas/%s_%s" % (ns, v),
            "current-version-id": 1,
            "versions": [{"version-id": 1, "timestamp-ms": ahora, "schema-id": 0, "summary": {"operation": "create"},
                          "default-namespace": [ns], "representations": reps}],
            "version-log": [{"version-id": 1, "timestamp-ms": ahora}],
            "schemas": [{"type": "struct", "schema-id": 0, "fields": d["campos"]}],
            "properties": {}}
    return {"metadata-location": "s3://copia/vistas/%s_%s/v1.metadata.json" % (ns, v), "metadata": meta, "config": {}}

VISTA_ENDPOINTS = ["GET /v1/{prefix}/namespaces/{namespace}/views", "GET /v1/{prefix}/namespaces/{namespace}/views/{view}",
                   "HEAD /v1/{prefix}/namespaces/{namespace}/views/{view}"]
BASE_ENDPOINTS = ["GET /v1/{prefix}/namespaces", "GET /v1/{prefix}/namespaces/{namespace}", "HEAD /v1/{prefix}/namespaces/{namespace}",
                  "GET /v1/{prefix}/namespaces/{namespace}/tables", "GET /v1/{prefix}/namespaces/{namespace}/tables/{table}",
                  "HEAD /v1/{prefix}/namespaces/{namespace}/tables/{table}", "POST /v1/{prefix}/namespaces/{namespace}/tables",
                  "POST /v1/{prefix}/namespaces/{namespace}/tables/{table}", "POST /v1/{prefix}/transactions/commit"]

class P(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a): pass
    def _manda(self, codigo, b, t0, nota=""):
        self.send_response(codigo)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(b)
        h = self.headers
        apunta({"paso": MODO["paso"], "metodo": self.command, "ruta": self.path, "codigo": codigo,
                "ms": round((time.perf_counter() - t0) * 1000, 1), "nota": nota,
                "deleg": h.get("x-iceberg-access-delegation", ""), "puesto": h.get("x-ore-puesto", ""),
                "sujeto": h.get("x-ore-sujeto", ""), "ua": (h.get("user-agent", "") or "")[:40]})
    def _todo(self):
        t0 = time.perf_counter()
        n = int(self.headers.get("content-length", "0") or 0)
        cuerpo = self.rfile.read(n) if n else None
        if self.path == "/__modo":
            m = json.loads(cuerpo)
            proxy = m.pop("proxy", None)
            MODO.update(m)
            if proxy is not None:
                arriba("POST", "/__modo", json.dumps(proxy).encode(), {"content-type": "application/json"})
            self.send_response(200); self.send_header("content-length", "2"); self.end_headers(); self.wfile.write(b"{}")
            return
        seg = self.path.split("?")[0].strip("/").split("/")
        # las Views, cuando la lente las sirve
        if MODO["vistas"] != "no" and len(seg) >= 4 and seg[0] == "v1" and seg[1] == "namespaces" and seg[3] == "views":
            ns = seg[2]
            if len(seg) == 4:
                ids = [{"namespace": [ns], "name": k.split(".", 1)[1]} for k in MODO["vistas_def"] if k.split(".")[0] == ns]
                return self._manda(200, json.dumps({"identifiers": ids}).encode(), t0, "lente")
            clave = ns + "." + seg[4]
            d = MODO["vistas_def"].get(clave)
            if d is None:
                b = json.dumps({"error": {"code": 404, "type": "NoSuchViewException", "message": "no hay vista " + clave}}).encode()
                return self._manda(404, b, t0, "lente")
            return self._manda(200, json.dumps(carga_vista(ns, seg[4], d)).encode(), t0, "lente")
        hs = {k: v for k, v in self.headers.items() if k.lower() not in ("host", "connection", "content-length")}
        codigo, cab, b = arriba(self.command, self.path, cuerpo, hs)
        nota = ""
        if MODO["vista404"] and codigo == 400 and self.command in ("GET", "HEAD") and len(seg) == 5 and seg[3] == "tables"                 and (seg[2] + "." + seg[4]) in MODO["vistas_def"]:
            codigo = 404
            b = json.dumps({"error": {"code": 404, "type": "NoSuchTableException", "message": "no es una tabla: " + seg[4]}}).encode()
            nota = "400->404"
        if seg == ["v1", "config"] and codigo == 200 and MODO["config"] != "tal-cual":
            j = json.loads(b)
            if MODO["config"] == "endpoints":
                j["endpoints"] = BASE_ENDPOINTS + VISTA_ENDPOINTS
            elif MODO["config"] == "endpoints-sin-vistas":
                j["endpoints"] = BASE_ENDPOINTS
            elif MODO["config"] == "view-endpoints-supported":
                j.setdefault("overrides", {})["view-endpoints-supported"] = "true"
            b = json.dumps(j).encode(); nota = "config " + MODO["config"]
        if b"127.0.0.1" in b:
            b = b.replace(b"127.0.0.1", FUERA.encode()); nota = (nota + " fuera").strip()
        self._manda(codigo, b, t0, nota)
    do_GET = do_POST = do_HEAD = do_PUT = do_DELETE = _todo

s = ThreadingHTTPServer(("127.0.0.1", 0), P)
print("listo %d" % s.server_address[1], flush=True)
s.serve_forever()
'''

# ═════════════════════════════════════════════════════════════════════════════
# EL LADO DE SPARK — corre en el contenedor. Cada paso: el modo de la lente, la
# consulta, y una linea `PASO {json}` con filas o el error.
# ═════════════════════════════════════════════════════════════════════════════
SPARK_LADO = r'''
import json, sys, time, traceback, urllib.request
t_arranque = time.perf_counter()
from pyspark.sql import SparkSession
LENTE = sys.argv[1]; PASOS = json.load(open(sys.argv[2]))
def conf(b, nombre, extra):
    p = "spark.sql.catalog." + nombre
    b = b.config(p, "org.apache.iceberg.spark.SparkCatalog").config(p + ".type", "rest").config(p + ".uri", LENTE) \
         .config(p + ".io-impl", "org.apache.iceberg.aws.s3.S3FileIO").config(p + ".s3.path-style-access", "true") \
         .config(p + ".client.region", "us-east-1").config(p + ".cache-enabled", "false")
    for k, v in extra.items():
        b = b.config(p + "." + k, v)
    return b
b = SparkSession.builder.appName("medida").master("local[2]") \
    .config("spark.sql.extensions", "org.apache.iceberg.spark.extensions.IcebergSparkSessionExtensions") \
    .config("spark.ui.enabled", "false").config("spark.sql.session.timeZone", "UTC")
PUESTO = {"header.x-ore-sujeto": "agente:local", "header.x-ore-puesto": "puesto-ana-python"}
b = conf(b, "ore", dict(PUESTO, **{"header.X-Iceberg-Access-Delegation": "vended-credentials"}))
b = conf(b, "sincred", PUESTO)
for n in ("c_sin", "c_ep", "c_ves", "real"):
    b = conf(b, n, dict(PUESTO, **{"header.X-Iceberg-Access-Delegation": "vended-credentials"}))
spark = b.getOrCreate()
spark.sparkContext.setLogLevel("ERROR")
print("ARRANQUE %.1f" % (time.perf_counter() - t_arranque), flush=True)
def modo(m):
    r = urllib.request.Request(LENTE + "/__modo", data=json.dumps(m).encode(), method="POST",
                               headers={"content-type": "application/json"})
    urllib.request.urlopen(r).read()
for p in PASOS:
    # cada paso parte del modo de hoy: solo cambia lo que el paso dice
    modo(dict({"vistas": "no", "config": "tal-cual", "calificar": "", "vista404": False}, **p.get("modo", {}), paso=p["paso"]))
    t0 = time.perf_counter()
    try:
        df = spark.sql(p["sql"])
        filas = [list(r) for r in df.limit(20).collect()]
        out = {"ok": True, "filas": [[str(x) for x in f] for f in filas],
               "tipos": [[f.name, f.dataType.simpleString()] for f in df.schema.fields]}
    except Exception as e:
        m = str(e).strip().splitlines()
        clase = type(e).__name__
        java = ""
        for l in traceback.format_exc().splitlines() + m:
            if "Exception" in l and ("iceberg" in l or "spark" in l or "NoSuch" in l or "Forbidden" in l):
                java = l.strip(); break
        out = {"ok": False, "error": clase, "mensaje": " ".join(m)[:600], "java": java[:300]}
    out["paso"] = p["paso"]; out["ms"] = round((time.perf_counter() - t0) * 1000)
    print("PASO " + json.dumps(out), flush=True)
modo({"paso": "fin", "vistas": "no", "config": "tal-cual", "calificar": "", "proxy": {"forzar403": {}, "gcs": {}}})
'''

ES_ = "SELECT id, pais FROM {p}hr.ventas WHERE pais = 'ES'"


def vistas_def(esquema):
    """Las Views que la lente sirve: sus campos, con los ids y tipos de la tabla."""
    por_nombre = {f["name"]: f for f in esquema["fields"]}
    campos = lambda ns: [dict(por_nombre[n], id=i + 1) for i, n in enumerate(ns)]
    return {
        "hr.ventasES": {"spark": ES_, "duckdb": "select id, pais from hr.ventas where pais = 'ES'", "campos": campos(["id", "pais"])},
        # la SQL de DuckDB como la escribe `a_sql` (comillas dobles = identificador;
        # en Spark, comillas dobles = CADENA): ¿que hace Spark con ella?
        "hr.ventasQ": {"spark": ES_, "duckdb": "select \"id\", \"pais\" from \"hr\".\"ventas\" where (\"pais\" = 'ES')",
                       "campos": campos(["id", "pais"])},
        "hr.ventasV": {"spark": "SELECT id, pais, total, cuando FROM {p}hr.ventas", "duckdb": "select * from hr.ventas",
                       "campos": campos(["id", "pais", "total", "cuando"])},
    }


def pasos(vd):
    V = lambda **m: dict(m, vistas_def=vd)
    q = lambda paso, sql, **m: {"paso": paso, "sql": sql, "modo": V(**m)}
    s = []
    # §1
    s += [q("1.namespaces", "SHOW NAMESPACES IN ore"),
          q("1.tablas", "SHOW TABLES IN ore.hr"),
          q("1.vistas-hoy", "SHOW VIEWS IN ore.hr")]
    # §2
    for n in ("hr.ventas", "hr.clientes", "ventas.pedidos", "hr.ajena", "hr.ventas_es", "hr.espanoles",
              "hr.ventasV", "hr.empleados", "hr.empleados_t", "hr.nada"):
        s.append(q("2." + n, "SELECT count(*) AS n FROM ore." + n))
    s.append(q("2.tipos", "SELECT id, pais, total, cuando FROM ore.hr.ventas ORDER BY id LIMIT 2"))
    # §3
    # cada variante de /v1/config, en su catalogo (el config se lee al iniciarlo)
    es = lambda cat="c_ep": "SELECT count(*) AS n FROM %s.hr.ventasES" % cat
    W = dict(vistas="spark", vista404=True)
    s += [q("3.hoy·400", es("ore"), vistas="spark"),
          q("3.404·config-tal-cual", es("ore"), **W),
          q("3.404·endpoints-sin-vistas", es("c_sin"), config="endpoints-sin-vistas", **W),
          q("3.404·view-endpoints-supported", es("c_ves"), config="view-endpoints-supported", **W),
          q("3.404·endpoints", es("c_ep"), config="endpoints", **W),
          q("3.calificada-c_ep", es(), calificar="c_ep", **W),
          q("3.calificada-otro", es(), calificar="lago", **W),
          q("3.duckdb-solo", es(), vistas="duckdb", vista404=True),
          q("3.ambos", es(), vistas="ambos", vista404=True),
          q("3.comillas·duckdb-solo", "SELECT count(*) AS n FROM c_ep.hr.ventasQ", vistas="duckdb", vista404=True),
          q("3.comillas·duckdb-solo·filas", "SELECT * FROM c_ep.hr.ventasQ LIMIT 2", vistas="duckdb", vista404=True),
          q("3.comillas·ambos", "SELECT count(*) AS n FROM c_ep.hr.ventasQ", vistas="ambos", vista404=True),
          q("3.show-views", "SHOW VIEWS IN c_ep.hr", **W),
          q("3.show-tables", "SHOW TABLES IN c_ep.hr", **W),
          q("3.columnas", "SELECT * FROM c_ep.hr.ventasES ORDER BY id LIMIT 2", **W),
          q("3.tipos-vista", "SELECT * FROM c_ep.hr.ventasV ORDER BY id LIMIT 1", **W),
          q("3.join", "SELECT count(*) AS n FROM c_ep.hr.ventasES e JOIN c_ep.hr.clientes c ON e.id = c.id", **W)]
    # §4
    f403 = {"forzar403": {"hr.ventas": "OOS4002: materialization.payload no deja leer hr.ventas (medida)"}, "gcs": {}}
    s += [q("4.tabla-403", "SELECT count(*) AS n FROM ore.hr.ventas", proxy=f403),
          q("4.vista-sobre-403", es(), proxy=f403, **W),
          q("4.sin-vended-credentials", "SELECT count(*) AS n FROM sincred.hr.ventas", proxy={"forzar403": {}, "gcs": {}})]
    # §5
    s += [q("5.segunda", "SELECT count(*) AS n FROM ore.hr.ventas", proxy={"forzar403": {}, "gcs": {}}),
          q("5.tercera", "SELECT count(*) AS n FROM ore.hr.ventas")]
    # §6 · lo de verdad: la lente no sirve nada (catalogo `real`, iniciado aqui
    # con el config de ore-serve tal cual)
    s += [q("6.config·show-views", "SHOW VIEWS IN real.hr"),
          q("6.show-tables", "SHOW TABLES IN real.hr"),
          q("6.vista", "SELECT count(*) AS n FROM real.hr.ventasES"),
          q("6.columnas", "SELECT * FROM real.hr.ventasES ORDER BY id LIMIT 2"),
          q("6.cadena-rara", "SELECT count(*) AS n FROM real.hr.rarasES"),
          q("6.join", "SELECT count(*) AS n FROM real.hr.ventasES e JOIN real.hr.clientes c ON e.id = c.id"),
          q("6.virtual", "SELECT count(*) AS n FROM real.hr.empleados"),
          q("6.ajena", "SELECT count(*) AS n FROM real.hr.ajena"),
          q("6.mantenida", "SELECT count(*) AS n FROM real.hr.ventas_es"),
          q("6.heredada", "SELECT count(*) AS n FROM real.hr.espanoles"),
          q("6.vista-403", "SELECT count(*) AS n FROM real.hr.ventasES",
            proxy={"forzar403": {"hr.ventas": "OOS4002: materialization.payload no deja leer hr.ventas (medida)"}, "gcs": {}}),
          q("6.fin", "SELECT count(*) AS n FROM real.hr.ventasES", proxy={"forzar403": {}, "gcs": {}})]
    return s


def correr_spark(b, lente_puerto, plan):
    d = b.tmp + "/spark"
    os.makedirs(d, exist_ok=True)
    escribir(d + "/lado.py", SPARK_LADO)
    escribir(d + "/pasos.json", json.dumps(plan))
    jars = ",".join("/jars/" + f for f in sorted(os.listdir(JARS)) if f.endswith(".jar"))
    cmd = ["docker", "run", "--rm", "-v", "%s:/m" % d, "-v", "%s:/jars:ro" % JARS.replace("\\", "/"),
           "-e", "HOME=/tmp", "-e", "AWS_REGION=us-east-1", IMAGEN,
           "/opt/spark/bin/spark-submit", "--jars", jars, "--conf", "spark.driver.memory=1g",
           "/m/lado.py", "http://host.docker.internal:%d" % lente_puerto, "/m/pasos.json"]
    t0 = time.perf_counter()
    p = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace",
                       env=dict(os.environ, MSYS_NO_PATHCONV="1"))
    total = time.perf_counter() - t0
    escribir(b.tmp + "/spark.log", p.stdout + "\n--- stderr ---\n" + p.stderr)
    arranque = None
    res = {}
    for l in p.stdout.splitlines():
        if l.startswith("ARRANQUE "):
            arranque = float(l.split()[1])
        elif l.startswith("PASO "):
            j = json.loads(l[5:])
            res[j["paso"]] = j
    if not res:
        raise SystemExit("Spark no corrio ningun paso (codigo %d):\n%s" % (p.returncode, (p.stderr or p.stdout)[-2500:]))
    return res, arranque, total


def peticiones(lente_log):
    por = {}
    try:
        for l in open(lente_log, encoding="utf-8"):
            j = json.loads(l)
            por.setdefault(j["paso"], []).append(j)
    except FileNotFoundError:
        pass
    return por


def resumen(r):
    if r is None:
        return "—"
    if r["ok"]:
        f = r["filas"]
        return ("%s" % f[0][0]) if len(f) == 1 and len(f[0]) == 1 else json.dumps(f)[:120]
    return "✗ %s: %s" % (r["error"], (r["java"] or r["mensaje"])[:150])


def rutas(ps):
    return " · ".join("%s %s %d%s" % (x["metodo"], x["ruta"].split("?")[0].replace("/v1/namespaces", "…"), x["codigo"],
                                        (" [" + x["nota"] + "]") if x["nota"] else "") for x in ps)[:400]


def main():
    print("SPARK POR EL CATALOGO · %s · %d filas en hr.ventas" % (IMAGEN, FILAS))
    b = R.Banco()
    try:
        b.levantar()
        # una View con filtro, en el arbol: lo que ORE dice que es
        escribir(b.A + "/packages/hr/views/ventasES.yaml", """apiVersion: oos.dev/v1alpha12
kind: View
metadata: { name: ventasES, namespace: hr }
spec:
  owner: team:hr
  from: { dataset: hr.ventas }
  where: { pais: ES }
  fields: { id: id, pais: pais }
""")
        # y una con una cadena que Spark escapa distinto que DuckDB: comilla y barra
        escribir(b.A + "/packages/hr/views/rarasES.yaml", """apiVersion: oos.dev/v1alpha12
kind: View
metadata: { name: rarasES, namespace: hr }
spec:
  owner: team:hr
  from: { dataset: hr.ventas }
  where: { pais: ["O'B\\\\x", ES] }
  fields: { id: id, pais: pais }
""")
        a = subprocess.run([b.ore, "ask", ".", "--vista", "hr.ventasES", "--sql"], cwd=b.A, capture_output=True,
                           text=True, encoding="utf-8", errors="replace")
        fila("la View en ORE (`ore ask --sql`)", "codigo %d" % a.returncode, R.corto(a.stdout.strip() or a.stderr, 200))
        esperado = (FILAS + 3) // 4
        c, j = R.http("GET", b.directo + "/v1/namespaces/hr/tables/ventas", cabeceras=R.ANA)
        esquema = j["metadata"]["schemas"][-1]
        # la lente
        escribir(b.tmp + "/lente.py", LENTE)
        lente_log = b.tmp + "/lente.log"
        p, lp = arrancar([R.PY, b.tmp + "/lente.py", str(int(b.base.rsplit(":", 1)[1])), lente_log])
        b.procesos.append(p)
        print()
        res, arranque, total = correr_spark(b, lp, pasos(vistas_def(esquema)))
        por = peticiones(lente_log)

        def ver(titulo, prefijo, esperar=None):
            print()
            print(titulo)
            for k, r in res.items():
                if not k.startswith(prefijo):
                    continue
                marca = ""
                if esperar and r["ok"] and r["filas"] and len(r["filas"]) == 1:
                    marca = " ✓" if r["filas"][0][0] == str(esperar(k)) else " ≠ %s" % esperar(k)
                fila("  " + k, resumen(r) + marca, "%d ms" % r["ms"])
                if por.get(k):
                    print("       " + rutas(por[k]))

        ver("§1 · ¿CONECTA?  (Spark → lente → proxy → ore-serve /v1)", "1.")
        cuantos = {"hr.ventas": FILAS, "hr.clientes": (FILAS + 99) // 100, "ventas.pedidos": (FILAS + 99) // 100,
                   "hr.ajena": (FILAS + 99) // 100, "hr.ventas_es": FILAS}
        ver("§2 · ¿QUE LEE HOY?  count(*) por loadTable (✓ = las filas de la tabla)", "2.",
            lambda k: cuantos.get(k[2:], "?"))
        ver("§3 · ¿LEE VIEWS?  la lente sirve loadView; hr.ventasES = where pais=ES (✓ = %d, lo de ORE)" % esperado, "3.",
            lambda k: (FILAS + 99) // 100 if k == "3.join" else esperado)
        ver("§4 · ¿GOBIERNO?", "4.")
        reales = {"6.vista": esperado, "6.cadena-rara": esperado, "6.join": (FILAS + 99) // 100,
                  "6.ajena": (FILAS + 99) // 100, "6.mantenida": FILAS, "6.fin": esperado}
        ver("§6 · LO DE VERDAD  (ore-serve sirve; la lente solo apunta)", "6.", lambda k: reales.get(k, "?"))
        print()
        print("§5 · ¿CUANTO?")
        fila("  docker run entero", "%.1f s" % total, "contenedor + JVM + todos los pasos")
        fila("  la sesion de Spark", "%.1f s" % (arranque or -1), "SparkSession.getOrCreate()")
        for k in ("2.hr.ventas", "5.segunda", "5.tercera"):
            if k in res:
                fila("  " + k, "%d ms" % res[k]["ms"], "%d peticiones a /v1" % len(por.get(k, [])))
        cab = [x for xs in por.values() for x in xs if x["ruta"].startswith("/v1")]
        fila("  cabeceras que llegan", "puesto=%s · sujeto=%s" % (
            sorted({x["puesto"] for x in cab}), sorted({x["sujeto"] for x in cab})),
             "delegacion=%s · ua=%s" % (sorted({x["deleg"] for x in cab}), sorted({x["ua"] for x in cab})[:2]))
        json.dump({"res": res, "peticiones": por}, open(b.tmp + "/resultado.json", "w"), indent=1)
        print()
        print("  (todo: %s/resultado.json, spark.log, lente.log%s)" % (b.tmp, "" if R.GUARDAR else " — se borra al salir; --guardar DIR"))
    finally:
        b.cerrar()


if __name__ == "__main__":
    main()
