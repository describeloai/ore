"""EL CATALOGO EN LOS TRES MOTORES · medido antes de quitar la regex de los tres SDK.

`medida-el-catalogo-como-resolutor.py` midio que DuckDB (Python) puede resolver
`paquete.tabla` preguntando al `/v1` de ore-serve en vez de a la regex. El SDK
tiene tres lenguajes y cada uno lleva SU DuckDB: Python (`duckdb`), Node
(`@duckdb/node-api` 1.5.5-r.5, lo de la imagen) y la JVM (DuckDB JDBC
1.5.5.1). Antes de quitar la regex de los tres se mide que los tres hacen lo
mismo con el MISMO SQL, porque si alguno no puede, el SDK de ese lenguaje
necesita otro camino.

  §8a  EL MISMO GUION, TRES MOTORES   un corredor por motor (python / node /
                                      java) que ejecuta la misma lista de
                                      frases y dice, por frase, tiempos,
                                      primera fila o el error EXACTO:
       ①  extensiones + TOKEN + secreto http con `x-ore-puesto` + ATTACH READ_ONLY
       ②  un dataset escrito, y un join de dos
       ④  `search_path` para que `paquete.tabla` resuelva; READ_ONLY niega
          escribir en el lago; lo de la sesion sigue yendo a memoria
       ⑤  attach, 1ª y 2ª consulta, join (mediana de 5) y peticiones a /v1
  §8b  EL CONDUCTO (OOS4002)          el arbol con una columna `high` y el
                                      conducto `low`: lo que ve CADA motor
       y la §4 de la medida hermana, otra vez, con los binarios de hoy.

    python pruebas-de-fuego/medida-el-catalogo-en-los-tres.py [--filas 200000] [--node DIR] [--jdbc JAR] [--java BIN]

  --node DIR   un directorio con node_modules/@duckdb/node-api (sin el, se
               instala 1.5.5-r.5 en un temporal con npm)
  --jdbc JAR   duckdb_jdbc-1.5.5.1.jar (sin el, ORE_JARS o $TMP/ore-jars/duckdb_jdbc.jar,
               como `el-puesto.sh` 9; y si no esta, se baja de Maven Central)
  --java BIN   el `java` de un JDK 21 (el lanzador de un fichero fuente); sin el, el del PATH

El banco (S3 de mentira contado, ore-serve de verdad con cola, proxy, puesto
reclamado, datasets por PyIceberg) es el de la medida hermana, importado tal
cual. No toca el cluster. Fuera de la maquina: extensions.duckdb.org (las
extensiones de 1.5.5 si faltan) y, si faltan, npm y Maven Central.
"""
import importlib.util
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__)).replace("\\", "/")
sp = importlib.util.spec_from_file_location("resolutor", AQUI + "/medida-el-catalogo-como-resolutor.py")
m = importlib.util.module_from_spec(sp)
sp.loader.exec_module(m)
fila, corto, PUESTO = m.fila, m.corto, m.PUESTO


def arg(nombre, defecto=""):
    return sys.argv[sys.argv.index(nombre) + 1] if nombre in sys.argv else defecto


# ═════════════════════════════════════════════════════════════════════════════
# LOS CORREDORES — uno por motor, el mismo contrato: por stdin, una frase por
# linea `id<TAB>veces<TAB>sql`; por stdout, una linea JSON por frase:
# {id, ms:[...], fila:[...] | null, filas, error, t0, t1} (t0/t1 en segundos de
# epoca, para casar con lo que el proxy apunto). Un error no para el guion.
# ═════════════════════════════════════════════════════════════════════════════
CORREDOR_PY = r'''
import json, sys, time
import duckdb
con = duckdb.connect()
for linea in sys.stdin.read().splitlines():
    if not linea.strip():
        continue
    id_, veces, q = linea.split("\t", 2)
    r = {"id": id_, "ms": [], "fila": None, "filas": 0, "error": None, "t0": time.time()}
    for _ in range(int(veces)):
        t = time.perf_counter()
        try:
            c = con.execute(q)
            filas = c.fetchall() if c.description else []
            r["ms"].append((time.perf_counter() - t) * 1000)
            r["filas"] = len(filas)
            r["fila"] = [str(x) for x in filas[0]] if filas else None
        except Exception as e:
            r["error"] = "%s: %s" % (type(e).__name__, e)
            break
    r["t1"] = time.time()
    print(json.dumps(r), flush=True)
'''

CORREDOR_NODE = r'''
import { DuckDBInstance, version } from '@duckdb/node-api';
import { readFileSync } from 'node:fs';
const inst = await DuckDBInstance.create(':memory:');
const con = await inst.connect();
for (const linea of readFileSync(0, 'utf8').split(/\r?\n/)) {
  if (!linea.trim()) continue;
  const [id, veces, ...resto] = linea.split('\t');
  const q = resto.join('\t');
  const r = { id, ms: [], fila: null, filas: 0, error: null, t0: Date.now() / 1000, motor: version() };
  for (let i = 0; i < Number(veces); i++) {
    const t = performance.now();
    try {
      const res = await con.runAndReadAll(q);
      const filas = res.getRows();
      r.ms.push(performance.now() - t);
      r.filas = filas.length;
      r.fila = filas.length ? filas[0].map(v => String(v)) : null;
    } catch (e) { r.error = `${e.name}: ${e.message}`; break; }
  }
  r.t1 = Date.now() / 1000;
  console.log(JSON.stringify(r));
}
'''

CORREDOR_JAVA = r'''
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.sql.*;
import java.util.*;

public class Corredor {
    static String j(String s) {
        if (s == null) return "null";
        StringBuilder b = new StringBuilder("\"");
        for (char c : s.toCharArray()) {
            if (c == '"' || c == '\\') b.append('\\').append(c);
            else if (c < 0x20) b.append(String.format("\\u%04x", (int) c));
            else b.append(c);
        }
        return b.append('"').toString();
    }
    public static void main(String[] a) throws Exception {
        PrintStream out = new PrintStream(new FileOutputStream(FileDescriptor.out), true, StandardCharsets.UTF_8);
        Connection con = DriverManager.getConnection("jdbc:duckdb:");
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        String linea;
        while ((linea = in.readLine()) != null) {
            if (linea.isBlank()) continue;
            String[] p = linea.split("\t", 3);
            List<Double> ms = new ArrayList<>();
            String fila = "null", error = null;
            int filas = 0;
            double t0 = System.currentTimeMillis() / 1000.0;
            for (int i = 0; i < Integer.parseInt(p[1]); i++) {
                long t = System.nanoTime();
                try (Statement st = con.createStatement()) {
                    boolean hay = st.execute(p[2]);
                    filas = 0; fila = "null";
                    if (hay) try (ResultSet rs = st.getResultSet()) {
                        int n = rs.getMetaData().getColumnCount();
                        while (rs.next()) {
                            if (filas == 0) {
                                StringJoiner sj = new StringJoiner(",", "[", "]");
                                for (int c = 1; c <= n; c++) sj.add(j(String.valueOf(rs.getObject(c))));
                                fila = sj.toString();
                            }
                            filas++;
                        }
                    }
                    ms.add((System.nanoTime() - t) / 1e6);
                } catch (SQLException e) {
                    error = e.getClass().getSimpleName() + ": " + e.getMessage();
                    break;
                }
            }
            StringJoiner sm = new StringJoiner(",", "[", "]");
            for (double x : ms) sm.add(String.format(Locale.ROOT, "%.2f", x));
            out.println("{\"id\":" + j(p[0]) + ",\"ms\":" + sm + ",\"fila\":" + fila + ",\"filas\":" + filas
                + ",\"error\":" + j(error) + ",\"t0\":" + t0 + ",\"t1\":" + (System.currentTimeMillis() / 1000.0) + "}");
        }
    }
}
'''

EXTENSIONES = ("json", "icu", "avro", "iceberg", "httpfs")


def guion(base, lectura=True):
    """La misma lista de frases para los tres. `lectura=False` es la de §8b."""
    f = [("ext_" + e, 1, "install %s; load %s" % (e, e)) for e in EXTENSIONES]
    f += [("version", 1, "select version()"),
          ("tz", 1, "set TimeZone = 'UTC'"),
          ("secreto_ice", 1, "create or replace secret ore_ice (type iceberg, token 'agente:local')"),
          ("secreto_cab", 1, "create or replace secret ore_cab (type http, extra_http_headers map {'x-ore-puesto': '%s'}, scope '%s')" % (PUESTO, base)),
          ("attach", 1, "attach '' as ore (type iceberg, endpoint '%s', secret ore_ice, read_only)" % base),
          ("search_path", 1, "set search_path = 'memory.main,ore.hr,ore.ventas'")]
    if not lectura:
        return f + [("conducto", 1, "select count(*), sum(total) from hr.ventas")]
    return f + [
        ("q1", 1, "select count(*), sum(total) from hr.ventas"),
        ("q2", 5, "select count(*), sum(total) from hr.ventas"),
        ("join", 5, "select c.nombre, sum(v.total) from hr.ventas v join hr.clientes c on v.id = c.id group by 1 order by 2 desc limit 3"),
        ("tres_partes", 1, "select count(*) from ore.hr.clientes"),
        ("otro_paquete", 1, "select count(*) from ventas.pedidos"),
        ("ro_lago", 1, "create table ore.hr.por_motor as select 1::bigint as x"),
        ("ro_dos_partes", 1, "create table hr.por_motor as select 1::bigint as x"),
        ("memoria", 1, "create table t as select 42 as x"),
        ("memoria_lee", 1, "select x, current_database() from t"),
        ("listar", 1, "select count(*) from information_schema.tables where table_catalog = 'ore'"),
    ]


def correr(motor, orden, frases, env=None):
    entrada = "\n".join("%s\t%d\t%s" % (i, v, q.replace("\n", " ")) for i, v, q in frases) + "\n"
    t0 = time.perf_counter()
    p = subprocess.run(orden, input=entrada.encode("utf-8"), capture_output=True, env=env, timeout=900)
    total = (time.perf_counter() - t0) * 1000
    res = {}
    for l in p.stdout.decode("utf-8", "replace").splitlines():
        if l.startswith("{"):
            r = json.loads(l)
            res[r["id"]] = r
    if not res:
        raise SystemExit("el corredor de %s no dijo nada (codigo %d): %s" % (motor, p.returncode, p.stderr.decode("utf-8", "replace")[-600:]))
    return res, total


def peticiones(b, r):
    """Las peticiones a /v1 que el proxy apunto mientras la frase corria."""
    return [x for x in b.reg.todo() if r["t0"] - 0.05 <= x["t"] <= r["t1"] + 0.05 and x["ruta"].startswith("/v1")]


def mediana(xs):
    return statistics.median(xs) if xs else float("nan")


def preparar(tmp):
    """Los tres corredores, y lo que cada uno necesita."""
    motores = {}
    escribir = m.escribir
    escribir(tmp + "/corredor.py", CORREDOR_PY)
    motores["python"] = [sys.executable, tmp + "/corredor.py"]
    # Node: @duckdb/node-api de la imagen
    nodo = arg("--node")
    if not nodo:
        nodo = tmp + "/node"
        os.makedirs(nodo, exist_ok=True)
        escribir(nodo + "/package.json", '{"name": "corredor", "type": "module", "private": true}\n')
        subprocess.run("npm install --no-audit --no-fund --silent @duckdb/node-api@1.5.5-r.5", cwd=nodo, shell=True, check=True, capture_output=True)
    escribir(nodo + "/corredor.mjs", CORREDOR_NODE)
    motores["node"] = ["node", nodo + "/corredor.mjs"]
    # JVM: DuckDB JDBC 1.5.5.1 con el lanzador de un fichero fuente de Java 21
    jar = arg("--jdbc") or os.path.join(os.environ.get("ORE_JARS") or os.path.join(tempfile.gettempdir(), "ore-jars"), "duckdb_jdbc.jar")
    if not os.path.isfile(jar):
        os.makedirs(os.path.dirname(jar), exist_ok=True)
        import urllib.request
        urllib.request.urlretrieve("https://repo1.maven.org/maven2/org/duckdb/duckdb_jdbc/1.5.5.1/duckdb_jdbc-1.5.5.1.jar", jar)
    escribir(tmp + "/Corredor.java", CORREDOR_JAVA)
    java = arg("--java") or shutil.which("java") or "java"
    motores["jvm"] = [java, "-cp", jar, tmp + "/Corredor.java"]
    return motores


def s8(b):
    print()
    print("§8 · NODE Y LA JVM  el mismo guion en los tres motores del puesto, contra el mismo /v1")
    motores = preparar(b.tmp)
    res = {}
    for motor, orden in motores.items():
        res[motor], total = correr(motor, orden, guion(b.base))
        fila("  %s" % motor, "%.0f ms en total" % total, "DuckDB %s" % (res[motor].get("version", {}).get("fila") or ["?"])[0])
    print()
    print("§8a · ① ② ④ · lo que dice cada frase (✓ primera fila, ✗ el error)")
    ids = [i for i, _, _ in guion(b.base) if not i.startswith("ext_") and i not in ("version", "tz")]
    exts = [i for i, _, _ in guion(b.base) if i.startswith("ext_")]
    for motor in motores:
        malas = [i for i in exts if res[motor].get(i, {}).get("error")]
        fila("  %s · extensiones" % motor, "✓" if not malas else "✗", ("; ".join("%s: %s" % (i, corto(res[motor][i]["error"], 80)) for i in malas)))
    for i in ids:
        for motor in motores:
            r = res[motor].get(i, {})
            if r.get("error"):
                v, n = "✗", corto(r["error"], 130)
            else:
                v, n = "✓", (str(r.get("fila")) if r.get("fila") is not None else "")[:90]
            fila("  %-12s %s" % (i, motor), v, n)
    print()
    print("§8a · ⑤ · tiempos (mediana de 5 salvo attach y 1ª) y peticiones a /v1 por ejecucion")
    print("     (attach: GET /v1/config; 1ª: la lista de tablas del esquema + loadTable; luego, loadTable)")
    print("     %-8s %-14s %-14s %-14s %-14s %s" % ("motor", "attach", "1ª consulta", "2ª (×5)", "join (×5)", "/v1 en attach · 1ª · 2ª · join"))
    tabla = {}
    for motor in motores:
        rr = res[motor]
        cel, pet = [], []
        for i in ("attach", "q1", "q2", "join"):
            r = rr.get(i, {})
            cel.append("%.0f ms" % mediana(r.get("ms", [])) if r.get("ms") else "✗")
            veces = max(1, len(r.get("ms", [])))
            pet.append("%.1f" % (len(peticiones(b, r)) / veces) if r else "?")
        tabla[motor] = cel
        print("     %-8s %-14s %-14s %-14s %-14s %s" % (motor, cel[0], cel[1], cel[2], cel[3], " · ".join(pet)))
    # §8b · el conducto: una columna high y el conducto low
    print()
    print("§8b · ③ · el conducto de la lectura (OOS4002): `hr.ventas.total` high, materialization.payload low")
    A = b.A
    viejo = open(A + "/conduits.yaml", encoding="utf-8").read()
    m.escribir(A + "/lattice.yaml", "apiVersion: oos.dev/v1alpha3\nkind: Lattice\nmetadata: { name: sensitivity, namespace: gdpr }\nspec:\n  levels: [none, low, high]\n")
    m.escribir(A + "/conduits.yaml", "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: { name: demo }\nspec:\n  owner: team:security\n  conduits:\n    materialization.payload: { oos.maturity: DRAFT, gdpr.sensitivity: low }\n")
    m.escribir(A + "/packages/hr/entities/Venta.yaml", """apiVersion: oos.dev/v1alpha8
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
    c, j = m.http("GET", b.directo + "/puestos/%s/datos/hr.ventas" % PUESTO, cabeceras=m.AGENTE)
    fila("  GET datos/hr.ventas (la referencia)", str(c), corto((j or {}).get("error", ""), 100))
    msgs = {}
    for motor, orden in motores.items():
        rr, _ = correr(motor, orden, guion(b.base, lectura=False))
        r = rr.get("conducto", {})
        cods = [x["codigo"] for x in peticiones(b, r) if "/tables/" in x["ruta"]] if r else []
        if r.get("error"):
            fila("  %s" % motor, "✗ (loadTable %s)" % ",".join(map(str, cods)), "")
            fila("", "", "«%s»" % corto(r["error"], 330))
            msgs[motor] = r["error"]
        else:
            fila("  %s" % motor, "LEE", "%s ← el conducto NO se aplico" % r.get("fila"))
    comun = "OOS4002: `hr.ventas.total`"
    fila("  ¿el texto de ORE llega igual a los tres?", "si" if msgs and all(comun in v for v in msgs.values()) and len(msgs) == len(motores) else "NO", "")
    for motor, v in msgs.items():
        fila("  prefijo que pone %s" % motor, "", corto(v.split("HTTP Error")[0] if "HTTP Error" in v else v[:60], 60))
    m.escribir(A + "/conduits.yaml", viejo)
    os.remove(A + "/packages/hr/entities/Venta.yaml")
    os.remove(A + "/lattice.yaml")
    return res, tabla, msgs


def main():
    print("=== el catalogo en los tres motores, medido  (%s)" % time.strftime("%Y-%m-%d"))
    b = m.Banco()
    try:
        print()
        print("§0 · EL BANCO  (el de medida-el-catalogo-como-resolutor.py)")
        b.levantar()
        # la §4 de la medida hermana, con los binarios de hoy
        m.s4(b)
        res, tabla, msgs = s8(b)
        veredicto(res, tabla, msgs, b)
    finally:
        b.cerrar()
    return 0


def veredicto(res, tabla, msgs, b):
    ok = {mo: all(not res[mo].get(i, {}).get("error") for i in ("attach", "q1", "join", "search_path")) for mo in res}
    ro = {mo: bool(res[mo].get("ro_lago", {}).get("error")) and not res[mo].get("memoria", {}).get("error") for mo in res}
    print()
    print("⇒ %s. El mismo SQL —extensiones, TOKEN, secreto http con x-ore-puesto, ATTACH"
          % ("LOS TRES IGUAL" if all(ok.values()) and all(ro.values()) else "NO SON IGUALES"))
    print("  READ_ONLY y search_path 'memory.main,ore.<paquete>,…'— hace lo mismo en %s:" % ", ".join(mo for mo in res if ok[mo]))
    print("  `paquete.tabla` resuelve, el join tambien, READ_ONLY niega el lago y lo de la sesion")
    print("  va a memoria (%s). El 403 del conducto llega con el texto de ORE a %d de %d motores."
          % (", ".join("%s %s" % (mo, "si" if ro[mo] else "NO") for mo in res), len(msgs), len(res)))
    print("  Lo que cambia por SDK es sólo la forma de ejecutar frases (execute / runAndReadAll /")
    print("  Statement) y la clase del error; ni opciones, ni secretos, ni el orden de las frases.")


if __name__ == "__main__":
    sys.exit(main())
