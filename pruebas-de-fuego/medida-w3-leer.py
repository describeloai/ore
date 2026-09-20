#!/usr/bin/env python3
"""
MEDIDA · W3.5 · el verbo LEER, de punta a punta en los tres lenguajes (19 de septiembre;
rehecha el 20 para 0032 T3: ahora mide EL CONTRATO)

`over("p.v")` y `sql()` existen en Python, TS y Java (W3.1–W3.4). La primera
pasada (19-09) enseñó que cada uno devolvía lo suyo —pandas, objetos JSON de
DuckDB, `List<Map>` de JDBC— y que la consola veía tres JSON distintos; de ahí
salió el contrato de tipos (0032). Desde T3 los tres SDK devuelven valores
tipados (Arrow / DuckDB tipado / Arrow Java) y un único JSON de consola
(`ore.tabla()`), y esta medida es la que lo comprueba:

  §1  LOS TIPOS      un Parquet con todos los tipos difíciles (enteros de 64
                     bits fuera del rango de un double, uint64, NaN/inf,
                     decimales de 18 y 38 dígitos, timestamps con y sin zona,
                     ns, date, time, lista, struct, map, binario, diccionario,
                     nulos en todo) leído por `over()`/`sql()` de los TRES SDK,
                     pasado por `ore.tabla()` —lo que la consola ve— y cotejado
                     CAMPO A CAMPO con la verdad (pyarrow). El criterio de T3:
                     ningún ≠ fuera de lo que la tabla de 0032 §1 dice
  §2  A ESCALA       10 M de filas (4 columnas): lo que cada lenguaje hace con
                     ellas según el contrato —Python las materializa (Arrow),
                     Node y Java hasta su límite y lo dicen, Java las recorre
                     por `arrow()`— y `sql()` con group by: ms y filas/s

Los SDK son los de verdad (`puesto/{python,node,jvm}`); `ore-serve` se
sustituye por un stub HTTP que contesta `GET /puestos/{id}/datos/{vista}`
—la resolución está probada en `el-puesto.sh`; aquí se mide el motor y los
tipos— y el almacén es un directorio (`ORE_ALMACEN=dir:`).

Uso:
  python pruebas-de-fuego/medida-w3-leer.py [--filas 10000000] [--sin-java] [--sin-node]

Hacen falta python con pyarrow/pandas/duckdb, `node` ≥ 22.13 con red (npm
install @duckdb/node-api, una vez), `javac`/`java` 21 y el jar de DuckDB JDBC
(se baja a %TEMP% una vez, o `DUCKDB_JDBC_JAR=`). Nada de pago, nada en el clúster.
"""
import datetime as dt
import decimal
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DUCKDB_JDBC = "1.5.5.1"


def fila(k, v, nota=""):
    print("  %-34s %-40s %s" % (k, v, nota))


def sh(*args, cwd=None, env=None, entrada=None):
    exe = shutil.which(args[0]) or args[0]
    r = subprocess.run((exe,) + tuple(args[1:]), capture_output=True, cwd=cwd, env=env, input=entrada)
    return r.returncode, r.stdout.decode("utf-8", "replace"), r.stderr.decode("utf-8", "replace")


def win(p):
    """Una ruta como la ve un programa nativo (en msys, `/c/x` → `C:/x`)."""
    return p.replace("\\", "/")


# ── §1 · el Parquet de los tipos difíciles, y su verdad ───────────────────
def tabla_de_tipos():
    import pyarrow as pa

    D = decimal.Decimal
    tz = dt.timezone.utc
    import zoneinfo
    mad = zoneinfo.ZoneInfo("Europe/Madrid")
    cols = {
        "i8": pa.array([127, None, -128], pa.int8()),
        "i32": pa.array([2147483647, None, -2147483648], pa.int32()),
        "i64": pa.array([9007199254740993, None, -9223372036854775808], pa.int64()),   # 2^53+1: no cabe en un double
        "u64": pa.array([18446744073709551615, None, 0], pa.uint64()),
        "f64": pa.array([1.5, None, float("nan")], pa.float64()),
        "f64_inf": pa.array([float("inf"), -0.0, 1e308], pa.float64()),
        "bool": pa.array([True, None, False], pa.bool_()),
        "texto": pa.array(["ñandú 🐍", None, ""], pa.string()),
        "texto_grande": pa.array(["a" * 5000, None, "b"], pa.large_string()),
        "diccionario": pa.array(["rojo", None, "rojo"], pa.string()).dictionary_encode(),
        "binario": pa.array([b"\x00\x01\xff", None, b""], pa.binary()),
        "fecha": pa.array([dt.date(2026, 9, 19), None, dt.date(1969, 12, 31)], pa.date32()),
        "ts_sin_zona": pa.array([dt.datetime(2026, 9, 19, 14, 30, 0, 123456), None, dt.datetime(1969, 12, 31, 23, 59, 59)], pa.timestamp("us")),
        "ts_utc": pa.array([dt.datetime(2026, 9, 19, 14, 30, 0, 123456, tz), None, dt.datetime(1969, 12, 31, 23, 59, 59, tzinfo=tz)], pa.timestamp("us", tz="UTC")),
        "ts_madrid": pa.array([dt.datetime(2026, 9, 19, 16, 30, 0, 123456, mad), None, dt.datetime(1970, 1, 1, 1, 59, 59, tzinfo=mad)], pa.timestamp("us", tz="Europe/Madrid")),
        "ts_ns": pa.array([1_726_756_200_123_456_789, None, 0], pa.timestamp("ns")),
        "hora": pa.array([dt.time(14, 30, 0, 123456), None, dt.time(0, 0)], pa.time64("us")),
        "dec_18_4": pa.array([D("12345.6789"), None, D("-0.0001")], pa.decimal128(18, 4)),
        "dec_38_10": pa.array([D("1234567890123456789012345678.1234567890"), None, D("0.0000000001")], pa.decimal128(38, 10)),
        "lista": pa.array([[1, 2, None], None, []], pa.list_(pa.int64())),
        "struct": pa.array([{"a": 1, "b": "x"}, None, {"a": None, "b": "y"}], pa.struct([("a", pa.int32()), ("b", pa.string())])),
        "mapa": pa.array([[("k1", 1), ("k2", 2)], None, []], pa.map_(pa.string(), pa.int32())),
        "todo_nulo": pa.array([None, None, None], pa.null()),
    }
    return pa.table(cols)


def verdad(tabla):
    """La verdad de cada columna: tipo Arrow y valores en una forma canónica (texto)."""
    v = {}
    for c in tabla.column_names:
        col = tabla.column(c)
        vals = [canonico(x) for x in col.to_pylist()]
        v[c] = {"tipo": str(col.type), "valores": vals}
    return v


def canonico(x):
    """Un valor, en texto, sin ambigüedad: lo que hay que ver en los tres lenguajes."""
    if x is None:
        return None
    if isinstance(x, bool):
        return "true" if x else "false"
    if isinstance(x, float):
        if x != x:
            return "NaN"
        if x in (float("inf"), float("-inf")):
            return "inf" if x > 0 else "-inf"
        # JSON no tiene −0 ni distingue 1.0 de 1: la forma se perdona, el valor no.
        if x == int(x) and abs(x) < 1e15:
            return str(int(x))
        return repr(x)
    if isinstance(x, int):
        return str(x)
    if isinstance(x, decimal.Decimal):
        return format(x, "f")
    if isinstance(x, bytes):
        import base64
        return base64.b64encode(x).decode("ascii")
    if isinstance(x, dt.datetime):
        # Un instante con zona es un instante: se compara en UTC, venga con el
        # desfase que venga (DuckDB lo enseña en la zona de la sesión).
        return x.astimezone(dt.timezone.utc).isoformat() if x.tzinfo else x.isoformat()
    if isinstance(x, (dt.date, dt.time)):
        return x.isoformat()
    if isinstance(x, list):
        # una lista de pares (map) o de valores
        if x and isinstance(x[0], tuple):
            return "{" + ",".join("%s:%s" % (canonico(a), canonico(b)) for a, b in x) + "}"
        return "[" + ",".join(str(canonico(e)) for e in x) + "]"
    if isinstance(x, dict):
        return "{" + ",".join("%s:%s" % (k, canonico(vv)) for k, vv in x.items()) + "}"
    return str(x)


def canonico_desde_json(x):
    """Lo que un SDK devolvió (ya pasado por JSON) → la misma forma canónica. Se
    perdona la FORMA (espacio por `T` en un instante, `\x00` por `0x00`, el
    `toString` de una lista o un mapa de Java) y no el VALOR: lo que aquí sale ≠
    es un dato distinto, no una manera distinta de escribirlo."""
    import re
    if isinstance(x, str):
        if x in ("Infinity", "-Infinity"):
            return "inf" if x[0] != "-" else "-inf"
        # java.time deja fuera los segundos cuando son cero: `00:00`, `1970-01-01T00:00`
        if re.fullmatch(r"\d\d:\d\d", x) or re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d", x):
            x = x + ":00"
        if re.match(r"^\d{4}-\d\d-\d\d \d\d:\d\d", x):
            x = x[:10] + "T" + x[11:]
        if re.match(r"^\d{4}-\d\d-\d\dT\d\d:\d\d(:\d\d(\.\d+)?)?[+-]\d\d$", x):
            x = x + ":00"
        if re.match(r"^\d{4}-\d\d-\d\dT.*Z$", x):
            x = x[:-1] + "+00:00"
        if re.match(r"^\d{4}-\d\d-\d\dT.*[+-]\d\d:\d\d$", x):
            try:
                return dt.datetime.fromisoformat(x).astimezone(dt.timezone.utc).isoformat()
            except ValueError:
                pass
        m = re.fullmatch(r"b'(.*)'", x)
        if m:
            x = m.group(1)
        if re.fullmatch(r"(\\x[0-9a-fA-F]{2})+", x):
            x = "".join(h.lower() for h in re.findall(r"\\x([0-9a-fA-F]{2})", x))
        if re.match(r"^\[.*\]$|^\{.*\}$", x) and (" " in x or "'" in x):
            x = x.replace(", ", ",").replace(": ", ":").replace("'", "").replace("=", ":").replace("null", "None")
        return x
    if x is None:
        return None
    if isinstance(x, bool):
        return "true" if x else "false"
    if isinstance(x, float):
        if x != x:
            return "NaN"
        if x in (float("inf"), float("-inf")):
            return "inf" if x > 0 else "-inf"
        if x == int(x) and abs(x) < 1e15:
            return str(int(x))
        return repr(x)
    if isinstance(x, int):
        return str(x)
    if isinstance(x, list):
        # Un map del contrato: `[{key, value}]` → la forma de la verdad.
        if x and all(isinstance(e, dict) and set(e) == {"key", "value"} for e in x):
            return "{" + ",".join("%s:%s" % (canonico_desde_json(e["key"]), canonico_desde_json(e["value"])) for e in x) + "}"
        return "[" + ",".join(str(canonico_desde_json(e)) for e in x) + "]"
    if isinstance(x, dict):
        return "{" + ",".join("%s:%s" % (k, canonico_desde_json(vv)) for k, vv in x.items()) + "}"
    return str(x)


# ── lo que corre en cada lenguaje: over() y sql() con el SDK de verdad ────
LECTOR_PY = r"""
import json, os, sys, time
sys.path.insert(0, os.environ["SDK_PY"])
import ore
ore.puesto._cabeceras = {"x-ore-sujeto": "agente:medida"}
def columnas(t):
    return {c["name"]: {"tipo": c["type"], "valores": [f[i] for f in t["filas"]]} for i, c in enumerate(t["columnas"])}
salida = {}
t = time.time(); df = ore.over("tipos.dificiles"); salida["over_ms"] = round((time.time() - t) * 1000)
salida["over"] = columnas(ore.tabla(df))
salida["nativo"] = {c: str(df[c].dtype) for c in df.columns}
t = time.time(); ds = ore.sql("select * from tipos.dificiles"); salida["sql_ms"] = round((time.time() - t) * 1000)
salida["sql"] = columnas(ore.tabla(ds))
if os.environ.get("GRANDE"):
    t = time.time(); g = ore.over("grande.filas"); salida["grande_over_ms"] = round((time.time() - t) * 1000); salida["grande_filas"] = len(g); salida["grande_como"] = "over(): DataFrame con ArrowDtype, entero"
    t = time.time(); s = float(g["importe"].sum()); salida["grande_suma_ms"] = round((time.time() - t) * 1000)
    t = time.time(); r = ore.sql("select pais, count(*) n, sum(importe) s from grande.filas group by 1 order by 2 desc"); salida["grande_sql_ms"] = round((time.time() - t) * 1000); salida["grande_grupos"] = len(r)
print(json.dumps(salida, default=str))
"""

LECTOR_MJS = r"""
import * as ore from "./ore/index.mjs";
ore.puesto._cabeceras = { "x-ore-sujeto": "agente:medida" };
const columnas = (t) => Object.fromEntries(t.columnas.map((c, i) => [c.name, { tipo: c.type, valores: t.filas.map((f) => f[i]) }]));
const salida = {};
let t = performance.now(); const filas = await ore.over("tipos.dificiles"); salida.over_ms = Math.round(performance.now() - t);
salida.over = columnas(ore.tabla(filas));
salida.nativo = Object.fromEntries(Object.keys(filas[0]).map((c) => { const v = filas.find((f) => f[c] !== null && f[c] !== undefined)?.[c]; return [c, v === undefined ? "null" : typeof v === "object" ? v.constructor.name : typeof v]; }));
t = performance.now(); const fs = await ore.sql("select * from tipos.dificiles"); salida.sql_ms = Math.round(performance.now() - t);
salida.sql = columnas(ore.tabla(fs));
if (process.env.GRANDE) {
  t = performance.now(); const g = await ore.over("grande.filas"); salida.grande_over_ms = Math.round(performance.now() - t); salida.grande_filas = g.length; salida.grande_como = `over(): ${g.length} filas de ${g.total} (truncada: ${g.truncada}, el límite por defecto)`;
  t = performance.now(); const c = await ore.over("grande.filas", { como: "columnas", limite: 10_000_000 }); salida.grande_columnas_ms = Math.round(performance.now() - t);
  t = performance.now(); let s = 0; for (const x of c.columnas[2]) s += x; salida.grande_suma_ms = Math.round(performance.now() - t); salida.grande_columnas_filas = c.columnas[0].length;
  t = performance.now(); const r = await ore.sql("select pais, count(*) n, sum(importe) s from grande.filas group by 1 order by 2 desc"); salida.grande_sql_ms = Math.round(performance.now() - t); salida.grande_grupos = r.length;
}
console.log(JSON.stringify(salida));
"""

LECTOR_JAVA = r"""
import java.util.*;
public class Lector {
    static Map<String, Object> columnas(Map<String, Object> t) {
        Map<String, Object> out = new LinkedHashMap<>();
        List<Map<String, Object>> cols = (List<Map<String, Object>>) t.get("columnas");
        List<List<Object>> filas = (List<List<Object>>) t.get("filas");
        for (int i = 0; i < cols.size(); i++) { List<Object> vs = new ArrayList<>(); for (List<Object> f : filas) vs.add(f.get(i)); Map<String, Object> c = new LinkedHashMap<>(); c.put("tipo", cols.get(i).get("type")); c.put("valores", vs); out.put((String) cols.get(i).get("name"), c); }
        return out;
    }
    public static void main(String[] a) throws Exception {
        java.lang.reflect.Field f = ore.Ore.Puesto.class.getDeclaredField("cabeceras"); f.setAccessible(true);
        f.set(ore.Ore.puesto, Map.of("x-ore-sujeto", "agente:medida"));
        Map<String, Object> salida = new LinkedHashMap<>();
        long t = System.nanoTime(); ore.Ore.Filas filas = ore.Ore.over("tipos.dificiles"); salida.put("over_ms", (System.nanoTime() - t) / 1_000_000);
        salida.put("over", columnas(ore.Ore.tabla(filas, 200)));
        Map<String, Object> nativo = new LinkedHashMap<>();
        for (String c : filas.tipos.keySet()) { String tipo = "null"; for (Map<String, Object> r : filas) { Object v = r.get(c); if (v != null) { tipo = v.getClass().getSimpleName(); break; } } nativo.put(c, tipo); }
        salida.put("nativo", nativo);
        t = System.nanoTime(); ore.Ore.Filas fs = ore.Ore.sql("select * from tipos.dificiles"); salida.put("sql_ms", (System.nanoTime() - t) / 1_000_000);
        salida.put("sql", columnas(ore.Ore.tabla(fs, 200)));
        if (System.getenv("GRANDE") != null) {
            t = System.nanoTime(); ore.Ore.Filas g = ore.Ore.over("grande.filas"); salida.put("grande_over_ms", (System.nanoTime() - t) / 1_000_000); salida.put("grande_filas", g.size()); salida.put("grande_como", "over(): " + g.size() + " filas de " + g.total + " (truncada: " + g.truncada + ", el límite por defecto)");
            t = System.nanoTime(); long n = 0; double s = 0;
            try (org.apache.arrow.vector.ipc.ArrowReader lector = ore.Ore.arrow("grande.filas")) {
                while (lector.loadNextBatch()) { var raiz = lector.getVectorSchemaRoot(); var imp = (org.apache.arrow.vector.Float8Vector) raiz.getVector("importe"); for (int i = 0; i < raiz.getRowCount(); i++) { s += imp.get(i); n++; } }
            }
            salida.put("grande_arrow_ms", (System.nanoTime() - t) / 1_000_000); salida.put("grande_arrow_filas", n);
            t = System.nanoTime(); ore.Ore.Filas r = ore.Ore.sql("select pais, count(*) n, sum(importe) s from grande.filas group by 1 order by 2 desc"); salida.put("grande_sql_ms", (System.nanoTime() - t) / 1_000_000); salida.put("grande_grupos", r.size());
        }
        System.out.println(ore.Json.escribir(salida));
    }
}
"""


# ── el stub de ore-serve: sólo `datos` ─────────────────────────────────────
class Stub(BaseHTTPRequestHandler):
    vistas = {}

    def do_GET(self):
        partes = self.path.split("/")
        if len(partes) == 5 and partes[1] == "puestos" and partes[3] == "datos" and partes[4] in self.vistas:
            cuerpo = json.dumps({"clave": self.vistas[partes[4]], "estado": "copiada", "bucket": "", "plan": "", "filas": ""}).encode()
            self.send_response(200)
        else:
            cuerpo = json.dumps({"error": "no hay ninguna `View` `%s` en el árbol" % partes[-1]}).encode()
            self.send_response(404)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(cuerpo)))
        self.end_headers()
        self.wfile.write(cuerpo)

    def log_message(self, *a):
        pass


def sobre(tabla, almacen, nombre):
    import pyarrow.parquet as pq

    b = io.BytesIO()
    pq.write_table(tabla, b)
    carga = b.getvalue()
    cab = json.dumps({"clave": "id", "conducto": nombre, "esquema": "x", "plan": "x", "testigo": "x"}).encode()
    art = b"ORECOPY1" + len(cab).to_bytes(4, "little") + cab + carga
    clave = "ore/v1/" + hashlib.sha256(art).hexdigest()
    os.makedirs(os.path.join(almacen, "ore", "v1"), exist_ok=True)
    with open(os.path.join(almacen, clave), "wb") as f:
        f.write(art)
    return clave, len(carga)


def jars_de_arrow():
    """Los jars de `puesto/jvm/jars.txt`, bajados una vez a %TEMP%/ore-arrow-java."""
    d = os.path.join(tempfile.gettempdir(), "ore-arrow-java")
    os.makedirs(d, exist_ok=True)
    out = []
    for linea in open(os.path.join(RAIZ, "puesto", "jvm", "jars.txt"), encoding="utf-8"):
        linea = linea.strip()
        if not linea or linea.startswith("#"):
            continue
        g, v = linea.split()
        n = "%s-%s.jar" % (g.rsplit("/", 1)[1], v)
        f = os.path.join(d, n)
        if not os.path.isfile(f):
            urllib.request.urlretrieve("https://repo1.maven.org/maven2/%s/%s/%s" % (g, v, n), f)
        out.append(f)
    return out


def tabla_grande(n):
    import duckdb

    con = duckdb.connect()
    return con.execute("select i as id, cast(i %% 1000 as integer) as cliente, (i * 7919) %% 100000 / 100.0 as importe, chr(cast(65 + i %% 26 as integer)) as pais from range(%d) t(i)" % n).to_arrow_table()


def main():
    filas = 10_000_000
    if "--filas" in sys.argv:
        filas = int(sys.argv[sys.argv.index("--filas") + 1])
    con_java = "--sin-java" not in sys.argv
    con_node = "--sin-node" not in sys.argv
    print("MEDIDA · W3.5 · leer · %s" % dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%MZ"))
    import pyarrow

    tmp = tempfile.mkdtemp(prefix="ore-leer-")
    almacen = os.path.join(tmp, "almacen")
    copias = os.path.join(tmp, "copias")
    try:
        # §1 el Parquet de los tipos
        tt = tabla_de_tipos()
        clave_t, bytes_t = sobre(tt, almacen, "tipos.dificiles")
        v = verdad(tt)
        fila("§1 · tipos.dificiles", "%d columnas · %d filas · %d bytes" % (tt.num_columns, tt.num_rows, bytes_t), "pyarrow %s" % pyarrow.__version__)
        t0 = time.time()
        tg = tabla_grande(filas)
        clave_g, bytes_g = sobre(tg, almacen, "grande.filas")
        fila("§2 · grande.filas", "%d filas · %.1f MB" % (filas, bytes_g / 1e6), "escrito en %d ms" % ((time.time() - t0) * 1000))
        del tg
        Stub.vistas = {"tipos.dificiles": clave_t, "grande.filas": clave_g}
        srv = HTTPServer(("127.0.0.1", 0), Stub)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        env = dict(os.environ, ORE_SERVE="http://127.0.0.1:%d" % srv.server_port, PUESTO="medida", ORE_ALMACEN="dir:" + win(almacen), ORE_COPIAS=win(copias), GRANDE="1", PYTHONIOENCODING="utf-8")
        resultados = {}

        # python
        e = dict(env, SDK_PY=win(os.path.join(RAIZ, "puesto", "python")))
        c, out, err = sh(sys.executable, "-c", LECTOR_PY, env=e)
        if c != 0:
            fila("python", "✗", err.strip().splitlines()[-1][:80] if err.strip() else out[:80])
        else:
            resultados["python"] = json.loads(out.strip().splitlines()[-1])

        # node
        if con_node:
            nd = os.path.join(tempfile.gettempdir(), "ore-leer-node")
            os.makedirs(nd, exist_ok=True)
            shutil.copy(os.path.join(RAIZ, "puesto", "node", "agente.mjs"), nd)
            shutil.copytree(os.path.join(RAIZ, "puesto", "node", "ore"), os.path.join(nd, "ore"), dirs_exist_ok=True)
            if not os.path.isdir(os.path.join(nd, "node_modules", "@duckdb", "node-api")):
                c, out, err = sh("npm", "install", "--no-audit", "--no-fund", "--silent", "@duckdb/node-api", cwd=nd)
                if c != 0:
                    fila("node", "✗ npm install", err[-100:])
            with open(os.path.join(nd, "lector.mjs"), "w", encoding="utf-8", newline="\n") as f:
                f.write(LECTOR_MJS)
            c, out, err = sh("node", "--no-warnings", os.path.join(nd, "lector.mjs"), env=env, cwd=nd)
            if c != 0:
                fila("node", "✗", (err.strip().splitlines()[-1] if err.strip() else out)[:90])
            else:
                resultados["node"] = json.loads(out.strip().splitlines()[-1])

        # java
        if con_java:
            jar = os.environ.get("DUCKDB_JDBC_JAR") or os.path.join(tempfile.gettempdir(), "duckdb_jdbc-%s.jar" % DUCKDB_JDBC)
            if not os.path.isfile(jar):
                urllib.request.urlretrieve("https://repo1.maven.org/maven2/org/duckdb/duckdb_jdbc/%s/duckdb_jdbc-%s.jar" % (DUCKDB_JDBC, DUCKDB_JDBC), jar)
            jars = [jar] + jars_de_arrow()
            clases = os.path.join(tmp, "clases")
            os.makedirs(clases)
            with open(os.path.join(tmp, "Lector.java"), "w", encoding="utf-8", newline="\n") as f:
                f.write(LECTOR_JAVA)
            fuentes = [os.path.join(RAIZ, "puesto", "jvm", "ore", x) for x in ("Json.java", "Ore.java", "Agente.java")] + [os.path.join(tmp, "Lector.java")]
            sep = ";" if os.name == "nt" else ":"
            cp = sep.join(win(x) for x in jars)
            c, out, err = sh("javac", "-Xlint:-options,-unchecked", "--release", "21", "-cp", cp, "-d", win(clases), *[win(x) for x in fuentes])
            if c != 0:
                fila("java", "✗ javac", err.strip().splitlines()[0][:90])
            else:
                c, out, err = sh("java", "--add-opens=java.base/java.nio=ALL-UNNAMED", "-Dstdout.encoding=UTF-8", "-Dfile.encoding=UTF-8", "-cp", win(clases) + sep + cp, "Lector", env=env)
                if c != 0:
                    fila("java", "✗", (err.strip().splitlines()[-1] if err.strip() else out)[:90])
                else:
                    resultados["java"] = json.loads(out.strip().splitlines()[-1])

        # ── la matriz: campo a campo ──────────────────────────────────────
        print()
        print("§1 · lo que la consola ve de cada tipo por `ore.tabla()` (✓ = igual que la verdad, con el tipo Arrow que el SDK declara; si no, lo que llegó)")
        lenguas = [("py·over", "python", "over"), ("py·sql", "python", "sql"), ("node·over", "node", "over"), ("node·sql", "node", "sql"), ("java·over", "java", "over"), ("java·sql", "java", "sql")]
        lenguas = [l for l in lenguas if l[1] in resultados]
        print("  %-13s %-26s %s" % ("columna", "verdad (arrow)", " ".join("%-22s" % l[0] for l in lenguas)))
        degradaciones = {l[0]: 0 for l in lenguas}
        for col, vv in v.items():
            celdas = []
            for et, lng, modo in lenguas:
                r = resultados[lng].get(modo, {}).get(col)
                if r is None:
                    celdas.append("— (no llega)")
                    degradaciones[et] += 1
                    continue
                vals = [canonico_desde_json(x) for x in r["valores"]]
                if vals == vv["valores"]:
                    celdas.append("✓ %s" % (r.get("tipo", ""))[:19])
                else:
                    dif = next((x for x, y in zip(vals, vv["valores"]) if x != y), "?")
                    celdas.append("≠ %s" % str(dif)[:20])
                    degradaciones[et] += 1
            print("  %-13s %-26s %s" % (col, ("%s · %s" % (vv["tipo"], str(vv["valores"][0])[:12]))[:26], " ".join("%-22s" % c for c in celdas)))
        print("  %-13s %-26s %s" % ("degradadas", "de %d" % len(v), " ".join("%-22s" % degradaciones[l[0]] for l in lenguas)))
        print()
        print("§1b · el tipo NATIVO que `over()` entrega en cada lenguaje (lo que una celda toca)")
        print("  %-13s %-34s %-30s %s" % ("columna", "python (pandas ArrowDtype)", "node (valor tipado)", "java"))
        for col in v:
            print("  %-13s %-34s %-30s %s" % (col, str(resultados.get("python", {}).get("nativo", {}).get(col, "—"))[:34], str(resultados.get("node", {}).get("nativo", {}).get(col, "—"))[:30], str(resultados.get("java", {}).get("nativo", {}).get(col, "—"))))
        print()
        print("§2 · a escala (%d filas, %.0f MB de Parquet)" % (filas, bytes_g / 1e6))
        for lng in ("python", "node", "java"):
            r = resultados.get(lng)
            if not r:
                continue
            ms_o, ms_s = r.get("grande_over_ms"), r.get("grande_sql_ms")
            fila("  %s · over()" % lng, "%s ms" % ms_o, "%s · %.1f M filas/s" % (r.get("grande_como"), r.get("grande_filas", 0) / max(ms_o, 1) / 1000) if ms_o else "")
            if r.get("grande_suma_ms") is not None and lng == "python":
                fila("  %s · sumar una columna" % lng, "%s ms" % r["grande_suma_ms"], "sobre el DataFrame")
            if r.get("grande_columnas_ms") is not None:
                fila("  %s · over(como: columnas)" % lng, "%s ms" % r["grande_columnas_ms"], "%s filas · sumar una columna %s ms" % (r.get("grande_columnas_filas"), r.get("grande_suma_ms")))
            if r.get("grande_arrow_ms") is not None:
                fila("  %s · arrow() recorrido" % lng, "%s ms" % r["grande_arrow_ms"], "%s filas sumadas · %.1f M filas/s" % (r.get("grande_arrow_filas"), r.get("grande_arrow_filas", 0) / max(r["grande_arrow_ms"], 1) / 1000))
            fila("  %s · sql() group by" % lng, "%s ms" % ms_s, "%s grupos" % r.get("grande_grupos"))
            fila("  %s · tipos: over/sql" % lng, "%s / %s ms" % (r.get("over_ms"), r.get("sql_ms")), "la primera bajada incluida")
        srv.shutdown()
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
        fila("limpieza", "el temporal fuera", "(el jar de DuckDB y node_modules quedan en %TEMP% para la próxima)")


if __name__ == "__main__":
    main()
