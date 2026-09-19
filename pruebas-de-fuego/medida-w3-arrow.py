#!/usr/bin/env python3
"""
MEDIDA · W3.5 · Arrow en Node y en Java, antes de meterlo en las imágenes (20 de septiembre)

0032 §4 dice que la celda ve COLUMNAS y que Arrow JS y Arrow Java «se miden
antes». Aquí, sobre el MISMO Parquet de 23 tipos difíciles de
`medida-w3-leer.py` y el de 10 M de filas, los caminos columnares que hay:

  node · parquet-wasm + apache-arrow   el Parquet → IPC (Rust en wasm) → Table de Arrow JS
  node · DuckDB tipado por columnas    `@duckdb/node-api` getColumns(): valores tipados
                                       (DuckDBDecimalValue, DuckDBTimestampTZValue…), sin Arrow
  java · DuckDB JDBC → Arrow Java      `arrowExportStream` → VectorSchemaRoot (arrow-vector +
                                       memory-unsafe: --add-opens java.base/java.nio)

De cada uno: fidelidad campo a campo (la misma matriz), lo que pesa (npm, jars),
y a escala: cargar 10 M de filas y sumar una columna (columnar de verdad o no).

Uso:  python pruebas-de-fuego/medida-w3-arrow.py [--filas 10000000]
Nada de pago, nada en el clúster. npm y Maven Central se tocan una vez (%TEMP%).
"""
import datetime as dt
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ARROW_JAVA = "19.0.0"
JARS = [
    ("org/apache/arrow/arrow-vector", ARROW_JAVA), ("org/apache/arrow/arrow-format", ARROW_JAVA),
    ("org/apache/arrow/arrow-memory-core", ARROW_JAVA), ("org/apache/arrow/arrow-memory-unsafe", ARROW_JAVA), ("org/apache/arrow/arrow-c-data", ARROW_JAVA),
    ("com/google/flatbuffers/flatbuffers-java", "24.3.25"), ("com/fasterxml/jackson/core/jackson-core", "2.18.2"),
    ("com/fasterxml/jackson/core/jackson-annotations", "2.18.2"), ("com/fasterxml/jackson/core/jackson-databind", "2.18.2"),
    ("com/fasterxml/jackson/datatype/jackson-datatype-jsr310", "2.18.2"),
    ("org/slf4j/slf4j-api", "2.0.16"), ("commons-codec/commons-codec", "1.17.1"),
]

# Se reutiliza la tabla de tipos y la verdad de medida-w3-leer.py
espec = importlib.util.spec_from_file_location("leer", os.path.join(RAIZ, "pruebas-de-fuego", "medida-w3-leer.py"))
leer = importlib.util.module_from_spec(espec)
espec.loader.exec_module(leer)


def fila(k, v, nota=""):
    print("  %-30s %-40s %s" % (k, v, nota))


def sh(*args, cwd=None, env=None):
    exe = shutil.which(args[0]) or args[0]
    r = subprocess.run((exe,) + tuple(args[1:]), capture_output=True, cwd=cwd, env=env)
    return r.returncode, r.stdout.decode("utf-8", "replace"), r.stderr.decode("utf-8", "replace")


LECTOR_MJS = r'''
import { readFileSync } from "node:fs";
import { tableFromIPC } from "apache-arrow";
import { readParquet } from "parquet-wasm";
import { DuckDBInstance } from "@duckdb/node-api";
const [tipos, grande] = [process.env.TIPOS, process.env.GRANDE];
const salida = {};
const llanoArrow = (v) => {
  if (v === null || v === undefined) return null;
  if (typeof v === "bigint") return v.toString();
  if (typeof v === "number" && !Number.isFinite(v)) return String(v);
  if (v instanceof Date) return v.toISOString();
  if (v instanceof Uint8Array) return Array.from(v).map((b) => b.toString(16).padStart(2, "0")).join("");
  if (v instanceof Uint32Array) return "Uint32Array(" + v.length + ")";  // un decimal de Arrow JS: cuatro palabras, sin aritmética
  if (typeof v === "object" && typeof v.toArray === "function") return Array.from(v.toArray()).map(llanoArrow);
  if (typeof v === "object" && typeof v.toJSON === "function") return JSON.parse(JSON.stringify(v.toJSON(), (k, x) => typeof x === "bigint" ? x.toString() : x));
  return v;
};
// 1 · parquet-wasm + apache-arrow
let t = performance.now();
const tabla = tableFromIPC(readParquet(readFileSync(tipos)).intoIPCStream());
salida.pq_ms = Math.round(performance.now() - t);
salida.pq = {};
for (const f of tabla.schema.fields) {
  const col = tabla.getChild(f.name);
  const vals = []; for (let i = 0; i < tabla.numRows; i++) vals.push(llanoArrow(col.get(i)));
  salida.pq[f.name] = { tipo: String(f.type), valores: vals };
}
// 2 · DuckDB tipado por columnas
const inst = await DuckDBInstance.create(":memory:"); const con = await inst.connect();
const llanoDuck = (v) => {
  if (v === null || v === undefined) return null;
  if (typeof v === "bigint") return v.toString();
  if (typeof v === "number" && !Number.isFinite(v)) return String(v);
  if (typeof v === "object" && v.constructor && v.constructor.name.startsWith("DuckDB")) {
    if (v.constructor.name === "DuckDBListValue") return v.items.map(llanoDuck);
    if (v.constructor.name === "DuckDBStructValue") return Object.fromEntries(Object.entries(v.entries).map(([k, x]) => [k, llanoDuck(x)]));
    if (v.constructor.name === "DuckDBMapValue") return v.toString();
    if (v.constructor.name === "DuckDBBlobValue") return Array.from(v.bytes).map((b) => b.toString(16).padStart(2, "0")).join("");
    return v.toString();
  }
  return v;
};
t = performance.now();
const r = await con.runAndReadAll(`select * from read_parquet('${tipos.replaceAll("\\", "/")}')`);
const cols = r.getColumns(); const nombres = r.columnNames(); const tiposD = r.columnTypes();
salida.duck_ms = Math.round(performance.now() - t);
salida.duck = {};
nombres.forEach((n, i) => { salida.duck[n] = { tipo: String(tiposD[i]), valores: cols[i].map(llanoDuck) }; });
// 3 · a escala
if (grande) {
  t = performance.now();
  const g = tableFromIPC(readParquet(readFileSync(grande)).intoIPCStream());
  const carga = Math.round(performance.now() - t);
  t = performance.now();
  let s = 0; for (const x of g.getChild("importe").toArray()) s += x;
  salida.grande_pq = { carga_ms: carga, suma_ms: Math.round(performance.now() - t), filas: g.numRows, suma: s };
  t = performance.now();
  const rg = await con.runAndReadAll(`select * from read_parquet('${grande.replaceAll("\\", "/")}')`);
  const cg = rg.getColumns();
  const carga2 = Math.round(performance.now() - t);
  t = performance.now();
  let s2 = 0; for (const x of cg[2]) s2 += x;
  salida.grande_duck = { carga_ms: carga2, suma_ms: Math.round(performance.now() - t), filas: rg.rowCount, suma: s2 };
}
console.log(JSON.stringify(salida));
'''

LECTOR_JAVA = r'''
import java.sql.*;
import java.util.*;
import org.apache.arrow.memory.RootAllocator;
import org.apache.arrow.vector.*;
import org.apache.arrow.vector.complex.*;
import org.apache.arrow.vector.ipc.ArrowReader;
import org.apache.arrow.vector.types.pojo.Field;
public class LectorArrow {
    static Object llano(FieldVector v, int i) {
        if (v.isNull(i)) return null;
        Object o = v.getObject(i);
        if (v instanceof TimeStampMicroTZVector ts) { long us = ts.get(i); return java.time.Instant.ofEpochSecond(Math.floorDiv(us, 1_000_000L), Math.floorMod(us, 1_000_000L) * 1000).toString(); }
        if (v instanceof UInt8Vector u) return u.getObjectNoOverflow(i).toString();
        if (v instanceof TimeStampNanoVector) return o.toString();  // LocalDateTime con los 9 decimales
        if (v instanceof TimeMicroVector) return java.time.LocalTime.ofNanoOfDay(((Number) o).longValue() * 1000).toString();
        if (v instanceof DateDayVector d) return java.time.LocalDate.ofEpochDay(d.get(i)).toString();
        if (v instanceof DecimalVector d) return d.getObject(i).toPlainString();
        if (v instanceof VarBinaryVector b) { StringBuilder s = new StringBuilder(); for (byte x : b.get(i)) s.append(String.format("%02x", x)); return s.toString(); }
        if (v instanceof MapVector) { StringBuilder s = new StringBuilder("{"); boolean primero = true; for (Object e : (List<?>) o) { Map<?, ?> par = (Map<?, ?>) e; if (!primero) s.append(","); primero = false; s.append(par.get("key")).append(":").append(par.get("value")); } return s.append("}").toString(); }
        if (v instanceof StructVector st) { Map<String, Object> out = new LinkedHashMap<>(); for (FieldVector c : st.getChildrenFromFields()) out.put(c.getName(), llano(c, i)); return out; }
        if (o instanceof org.apache.arrow.vector.util.Text t) return t.toString();
        if (o instanceof Double d && (d.isNaN() || d.isInfinite())) return d.isNaN() ? "NaN" : (d > 0 ? "inf" : "-inf");
        return o;
    }
    public static void main(String[] a) throws Exception {
        Class.forName("org.duckdb.DuckDBDriver");
        Connection con = DriverManager.getConnection("jdbc:duckdb:");
        Map<String, Object> salida = new LinkedHashMap<>();
        try (RootAllocator alloc = new RootAllocator()) {
            long t = System.nanoTime();
            Statement s = con.createStatement();
            ResultSet rs = s.executeQuery("select * from read_parquet('" + System.getenv("TIPOS").replace("\\", "/") + "')");
            ArrowReader lector = (ArrowReader) ((org.duckdb.DuckDBResultSet) rs).arrowExportStream(alloc, 1024);
            Map<String, Object> cols = new LinkedHashMap<>();
            while (lector.loadNextBatch()) {
                VectorSchemaRoot raiz = lector.getVectorSchemaRoot();
                for (FieldVector v : raiz.getFieldVectors()) {
                    Map<String, Object> col = (Map<String, Object>) cols.computeIfAbsent(v.getName(), k -> { Map<String, Object> m = new LinkedHashMap<>(); m.put("tipo", v.getField().getType().toString()); m.put("valores", new ArrayList<Object>()); return m; });
                    List<Object> vs = (List<Object>) col.get("valores");
                    for (int i = 0; i < raiz.getRowCount(); i++) vs.add(llano(v, i));
                }
            }
            lector.close(); rs.close();
            salida.put("arrow_ms", (System.nanoTime() - t) / 1_000_000);
            salida.put("arrow", cols);
            if (System.getenv("GRANDE") != null) {
                t = System.nanoTime();
                ResultSet rg = s.executeQuery("select * from read_parquet('" + System.getenv("GRANDE").replace("\\", "/") + "')");
                ArrowReader lg = (ArrowReader) ((org.duckdb.DuckDBResultSet) rg).arrowExportStream(alloc, 65536);
                double suma = 0; long filas = 0; long sumaMs = 0;
                while (lg.loadNextBatch()) {
                    VectorSchemaRoot raiz = lg.getVectorSchemaRoot();
                    long t2 = System.nanoTime();
                    Float8Vector imp = (Float8Vector) raiz.getVector("importe");
                    for (int i = 0; i < raiz.getRowCount(); i++) suma += imp.get(i);
                    sumaMs += System.nanoTime() - t2;
                    filas += raiz.getRowCount();
                }
                lg.close(); rg.close();
                Map<String, Object> g = new LinkedHashMap<>();
                g.put("carga_ms", (System.nanoTime() - t) / 1_000_000 - sumaMs / 1_000_000); g.put("suma_ms", sumaMs / 1_000_000); g.put("filas", filas); g.put("suma", suma);
                salida.put("grande", g);
            }
        }
        System.out.println(ore.Json.escribir(salida));
    }
}
'''


def main():
    filas = 10_000_000
    if "--filas" in sys.argv:
        filas = int(sys.argv[sys.argv.index("--filas") + 1])
    print("MEDIDA · W3.5 · Arrow en Node y Java · %s" % dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%MZ"))
    import pyarrow.parquet as pq

    tmp = tempfile.mkdtemp(prefix="ore-arrow-")
    try:
        tipos = os.path.join(tmp, "tipos.parquet")
        pq.write_table(leer.tabla_de_tipos(), tipos)
        v = leer.verdad(leer.tabla_de_tipos())
        grande = os.path.join(tmp, "grande.parquet")
        pq.write_table(leer.tabla_grande(filas), grande)
        fila("los Parquet", "%d tipos · %d filas" % (len(v), filas), "%.0f MB" % (os.path.getsize(grande) / 1e6))
        env = dict(os.environ, TIPOS=tipos, GRANDE=grande, PYTHONIOENCODING="utf-8")
        resultados = {}

        # node
        nd = os.path.join(tempfile.gettempdir(), "ore-leer-node")
        os.makedirs(nd, exist_ok=True)
        t0 = time.time()
        c, out, err = sh("npm", "install", "--no-audit", "--no-fund", "--silent", "@duckdb/node-api", "parquet-wasm", "apache-arrow", cwd=nd)
        if c != 0:
            fila("node · npm install", "✗", err[-120:])
        else:
            peso = {}
            for p in ("parquet-wasm", "apache-arrow", "@duckdb/node-api", "@duckdb/node-bindings-win32-x64", "@duckdb/node-bindings-linux-x64"):
                d = os.path.join(nd, "node_modules", p)
                if os.path.isdir(d):
                    peso[p] = sum(os.path.getsize(os.path.join(r, f)) for r, _, fs in os.walk(d) for f in fs) / 1e6
            fila("node · lo que pesa", " · ".join("%s %.1f MB" % (k.split("/")[-1], m) for k, m in peso.items()), "npm install %d s" % (time.time() - t0))
            with open(os.path.join(nd, "lector-arrow.mjs"), "w", encoding="utf-8", newline="\n") as f:
                f.write(LECTOR_MJS)
            c, out, err = sh("node", "--no-warnings", "--max-old-space-size=6000", os.path.join(nd, "lector-arrow.mjs"), env=env, cwd=nd)
            if c != 0:
                fila("node", "✗", (err.strip().splitlines()[-1] if err.strip() else out)[:100])
            else:
                resultados["node"] = json.loads(out.strip().splitlines()[-1])

        # java
        lib = os.path.join(tempfile.gettempdir(), "ore-arrow-java")
        os.makedirs(lib, exist_ok=True)
        jars = []
        peso = 0
        for ruta, ver in JARS:
            nombre = "%s-%s.jar" % (ruta.rsplit("/", 1)[1], ver)
            f = os.path.join(lib, nombre)
            if not os.path.isfile(f):
                urllib.request.urlretrieve("https://repo1.maven.org/maven2/%s/%s/%s" % (ruta, ver, nombre), f)
            jars.append(f)
            peso += os.path.getsize(f)
        duck = os.environ.get("DUCKDB_JDBC_JAR") or os.path.join(tempfile.gettempdir(), "duckdb_jdbc-%s.jar" % leer.DUCKDB_JDBC)
        if not os.path.isfile(duck):
            urllib.request.urlretrieve("https://repo1.maven.org/maven2/org/duckdb/duckdb_jdbc/%s/duckdb_jdbc-%s.jar" % (leer.DUCKDB_JDBC, leer.DUCKDB_JDBC), duck)
        fila("java · lo que pesa", "arrow %s + deps: %.1f MB en %d jars" % (ARROW_JAVA, peso / 1e6, len(jars)), "duckdb_jdbc %.0f MB" % (os.path.getsize(duck) / 1e6))
        sep = ";" if os.name == "nt" else ":"
        cp = sep.join([leer.win(x) for x in jars] + [leer.win(duck)])
        clases = os.path.join(tmp, "clases")
        os.makedirs(clases)
        with open(os.path.join(tmp, "LectorArrow.java"), "w", encoding="utf-8", newline="\n") as f:
            f.write(LECTOR_JAVA)
        fuentes = [os.path.join(RAIZ, "puesto", "jvm", "ore", "Json.java"), os.path.join(tmp, "LectorArrow.java")]
        c, out, err = sh("javac", "-Xlint:-options", "-nowarn", "--release", "21", "-cp", cp, "-d", leer.win(clases), *[leer.win(x) for x in fuentes])
        if c != 0:
            fila("java · javac", "✗", " | ".join(l.strip() for l in err.strip().splitlines()[:6])[:400])
        else:
            c, out, err = sh("java", "--add-opens=java.base/java.nio=ALL-UNNAMED", "-Dstdout.encoding=UTF-8", "-cp", leer.win(clases) + sep + cp, "LectorArrow", env=env)
            if c != 0:
                fila("java", "✗", " | ".join(l.strip() for l in (err.strip() or out).splitlines()[:5])[:600])
            else:
                resultados["java"] = json.loads(out.strip().splitlines()[-1])

        # la matriz
        print()
        print("§1 · fidelidad por el camino columnar (✓ = igual que la verdad; si no, lo que llegó)")
        caminos = [("node·parquet-wasm+arrow", "node", "pq"), ("node·duckdb columnas", "node", "duck"), ("java·duckdb→arrow", "java", "arrow")]
        caminos = [c for c in caminos if c[1] in resultados]
        print("  %-13s %-24s %s" % ("columna", "verdad (arrow)", " ".join("%-28s" % c[0] for c in caminos)))
        deg = {c[0]: 0 for c in caminos}
        for col, vv in v.items():
            celdas = []
            for et, lng, modo in caminos:
                r = resultados[lng].get(modo, {}).get(col)
                if r is None:
                    celdas.append("— (no llega)"); deg[et] += 1; continue
                vals = [leer.canonico_desde_json(x) for x in r["valores"]]
                if vals == vv["valores"]:
                    celdas.append("✓ %s" % r.get("tipo", "")[:25])
                else:
                    dif = next((x for x, y in zip(vals, vv["valores"]) if x != y), "?")
                    if os.environ.get("DEPURAR"):
                        dif = "%s ⟂ %s" % (vals, vv["valores"])
                    celdas.append("≠ %s" % str(dif)[:(400 if os.environ.get("DEPURAR") else 26)]); deg[et] += 1
            print("  %-13s %-24s %s" % (col, ("%s · %s" % (vv["tipo"], str(vv["valores"][0])[:10]))[:24], " ".join(("%-28s" if not os.environ.get("DEPURAR") else "%s") % c for c in celdas)))
        print("  %-13s %-24s %s" % ("degradadas", "de %d" % len(v), " ".join("%-28s" % deg[c[0]] for c in caminos)))
        print()
        print("§2 · a escala (%d filas): cargar en columnas y sumar `importe`" % filas)
        n = resultados.get("node", {})
        for k, et in (("grande_pq", "node · parquet-wasm+arrow"), ("grande_duck", "node · duckdb columnas")):
            g = n.get(k)
            if g:
                fila(et, "cargar %d ms · sumar %d ms" % (g["carga_ms"], g["suma_ms"]), "%.1f M filas/s · suma %.1f" % (filas / max(g["carga_ms"], 1) / 1000, g["suma"]))
        g = resultados.get("java", {}).get("grande")
        if g:
            fila("java · duckdb→arrow", "cargar %d ms · sumar %d ms" % (g["carga_ms"], g["suma_ms"]), "%.1f M filas/s · suma %.1f" % (filas / max(g["carga_ms"], 1) / 1000, g["suma"]))
        fila("(referencia, filas-objeto)", "node 34 560 ms · java 15 734 ms · python 755 ms", "medida-w3-leer.py")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
