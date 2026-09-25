"""EL EDITOR DE SQL, A FONDO · P4 escala, P5 robustez, P6 canal, con TODO de verdad.

El cliente de la consola (`servidor-de-lenguaje.ts`, por `el-editor-sql-
conductor.mjs`), un ore-serve de verdad (el banco de `medida-el-catalogo-como-
resolutor.py`) y el agente de verdad (`puesto/python/agente.py`, con
`ore.lsp_sql` en su proceso y un pyright de mentira al lado).

  P4  ESCALA     el arbol con 1000 y 5000 datasets mas: `/assets` en frio y en
                 caliente, lo que cuesta el PRIMER mensaje de SQL (el indice y
                 el catalogo), completion tras FROM con miles de nombres por el
                 canal, un diagnostico, la memoria del agente, y si pyright se
                 queda esperando mientras tanto.
  P5  ROBUSTEZ   mensajes rotos por el canal; el arbol cambia con el fichero
                 abierto; una celda pesada (Python puro y DuckDB) mientras se
                 teclea; un .sql de 360 KB; el agente se reinicia; el puesto se
                 cierra.
  P6  CANAL      tecleando SQL solo y SQL con Python a la vez: latencias, y
                 cuantas peticiones HTTP por segundo salen del editor.

    python pruebas-de-fuego/el-editor-sql-a-fondo.py --arboles <dir con arbol-1000/ y arbol-5000/> [--consola C:/rubix-platform] [--solo P4,P5,P6]

`--arboles`: los generados por un arbol sintetico de N datasets escritos de 30
columnas en 20 paquetes (`paquete_XX/datasets/dataset_NNNN.yaml`).
"""
import importlib.util
import json
import os
import shutil
import statistics
import subprocess
import sys
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__)).replace("\\", "/")
RAIZ = os.path.dirname(AQUI)


def arg(n, d=None):
    return sys.argv[sys.argv.index(n) + 1] if n in sys.argv else d


CONSOLA = arg("--consola", "C:/rubix-platform")
ARBOLES = arg("--arboles")
SOLO = set((arg("--solo") or "P4,P5,P6").split(","))
sys.argv[:] = [sys.argv[0], "--filas", "2000"]
_sp = importlib.util.spec_from_file_location("resolutor", AQUI + "/medida-el-catalogo-como-resolutor.py")
R = importlib.util.module_from_spec(_sp)
_sp.loader.exec_module(R)
_se = importlib.util.spec_from_file_location("editor", AQUI + "/el-editor-sql.py")
E = importlib.util.module_from_spec(_se)
sys.argv[:] = [sys.argv[0]]
_se.loader.exec_module(E)
MEDIDO = {}


def fila(k, v="", nota=""):
    print("     %-50s %-22s %s" % (k, v, nota))


def med(xs):
    return statistics.median(xs) if xs else 0


def p95(xs):
    xs = sorted(xs)
    return xs[max(0, int(len(xs) * 0.95) - 1)] if xs else 0


def rss(pid):
    try:
        import psutil
        return psutil.Process(pid).memory_info().rss / 1e6
    except Exception:
        return None


class Agente:
    def __init__(self, b):
        self.b, self.p, self.log = b, None, None

    def arrancar(self):
        b = self.b
        self.log = open(b.tmp + "/agente.log", "a", encoding="utf-8")
        env = dict(os.environ, ORE_SERVE=b.directo, PUESTO=R.PUESTO, ORE_SUJETO="agente:local",
                   ORE_ALMACEN="dir:" + b.tmp, TTL="900", ORE_MEMORIA_MB="1024",
                   ORE_LSP="%s %s" % (os.path.basename(sys.executable), (b.tmp + "/pyright-de-mentira.py").replace("\\", "/")),
                   TRABAJO_DIR=b.tmp, PYTHONUNBUFFERED="1", PYTHONIOENCODING="utf-8")
        self.p = subprocess.Popen([sys.executable, RAIZ + "/puesto/python/agente.py"], env=env, stdout=self.log, stderr=subprocess.STDOUT)
        time.sleep(2.5)

    def parar(self):
        if self.p:
            self.p.terminate()
            try:
                self.p.wait(10)
            except Exception:
                self.p.kill()
        if self.log:
            self.log.close()
        self.p = None

    def rss(self):
        return rss(self.p.pid) if self.p else None


class Conductor:
    def __init__(self, b):
        self.p = subprocess.Popen(["node", "--experimental-strip-types", "--no-warnings", AQUI + "/el-editor-sql-conductor.mjs",
                                   CONSOLA, b.directo, R.PUESTO, "persona:ana"],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8")

    def __call__(self, orden, **k):
        self.p.stdin.write(json.dumps(dict(k, orden=orden)) + "\n")
        self.p.stdin.flush()
        while True:
            l = self.p.stdout.readline()
            if not l:
                raise SystemExit("el conductor murio: %s" % self.p.stderr.read()[-2000:])
            if l.startswith("R "):
                r = json.loads(l[2:])
                if not r.get("ok"):
                    print("       (orden %s: %s)" % (orden, r.get("error", "")[:300]))
                return r

    def cerrar(self):
        try:
            self.p.stdin.write('{"orden":"salir"}\n')
            self.p.stdin.flush()
            self.p.wait(10)
        except Exception:
            self.p.kill()


def assets(b, veces=1):
    xs, n = [], 0
    for _ in range(veces):
        t0 = time.perf_counter()
        c, j = R.http("GET", b.directo + "/assets", cabeceras=R.AGENTE)
        xs.append((time.perf_counter() - t0) * 1000)
        n = len((j or {}).get("items", {}))
    return xs, n


# ═════════════════════════════════════════════════════════════════════════════
def p4(b, ag):
    print()
    print("P4 · ESCALA  (el árbol del banco + N datasets de 30 columnas; ore-serve sobre un DIRECTORIO: sin caché de /assets)")
    for n in (0, 1000, 5000):
        if n:
            src = os.path.join(ARBOLES, "arbol-%d" % n, "packages")
            for pk in os.listdir(src):
                shutil.copytree(os.path.join(src, pk), os.path.join(b.A, "packages", pk), dirs_exist_ok=True)
        frio, items = assets(b, 1)
        caliente, _ = assets(b, 2)
        ag.parar()
        ag.arrancar()
        base_rss = ag.rss()
        c = Conductor(b)
        c("abrir", lenguaje="python", ruta="transforms/x.py", texto="x = 1\n")
        # el PRIMER mensaje de SQL (initialize → el índice → el catálogo) y, A LA VEZ, una completion de Python
        j = c("juntas", ordenes=[{"orden": "abrir", "lenguaje": "sql", "ruta": "consultas/p4.sql", "texto": "select * from "},
                                 {"orden": "completion", "lenguaje": "python", "ruta": "transforms/x.py", "texto": "x = 1\n", "cur": 1}])
        primero, py_mientras = j["rs"][0]["fin_ms"], j["rs"][1]["ms"]
        tras_from = c("completion", lenguaje="sql", ruta="consultas/p4.sql", texto="select * from ")
        q = "select d.col_01 from paquete_03.dataset_0003 d where d." if n else "select v.pais from hr.ventas v where v."
        alias = c("completion", lenguaje="sql", ruta="consultas/p4.sql", texto=q)
        c("cambiar", lenguaje="sql", ruta="consultas/p4.sql", texto=(q + "col_99 = 1") if n else (q + "totl > 1"))
        d = c("marcas", ruta="consultas/p4.sql", esperar_ms=15000, no_vacias=True)
        despues = ag.rss()
        fila("  +%d datasets (%d items en /assets)" % (n, items), "/assets %.0f ms frío" % frio[0],
             "%.0f ms en caliente · %s" % (med(caliente), "SIN caché: cada petición recompila"))
        fila("    el primer mensaje de SQL (initialize + índice)", "%.0f ms" % primero,
             "y una completion de Python lanzada A LA VEZ: %.0f ms" % py_mientras)
        fila("    completion tras FROM", "%.0f ms" % tras_from["ms"], "%d nombres · %d KB al editor" % (tras_from["n"], tras_from["bytes"] // 1024))
        fila("    completion tras `alias.`", "%.0f ms" % alias["ms"], "%d columnas" % alias["n"])
        fila("    diagnóstico (cambiar → marca)", "%s ms" % (("%.0f" % d["ms"]) if d["ms"] else "—"), (d["xs"][0][2] if d["xs"] else "sin marca")[:60])
        fila("    el agente", "RSS %s → %s MB" % (("%.0f" % base_rss) if base_rss else "?", ("%.0f" % despues) if despues else "?"), "")
        MEDIDO["P4 +%d" % n] = "primer mensaje %.0f ms · python a la vez %.0f ms · FROM %.0f ms/%d · diag %s ms" % (
            primero, py_mientras, tras_from["ms"], tras_from["n"], ("%.0f" % d["ms"]) if d["ms"] else "—")
        c.cerrar()
    # fuera lo sintético: P5 y P6 sobre el árbol del banco
    for pk in os.listdir(b.A + "/packages"):
        if pk.startswith("paquete_"):
            R.borrar(os.path.join(b.A, "packages", pk))
    ag.parar()
    ag.arrancar()


# ═════════════════════════════════════════════════════════════════════════════
def p5(b, ag):
    print()
    print("P5 · ROBUSTEZ")
    c = Conductor(b)
    uri = "consultas/p5.sql"
    c("abrir", lenguaje="sql", ruta=uri, texto="select * from ")
    base = c("completion", lenguaje="sql", ruta=uri, texto="select * from ")
    fila("  punto de partida: completion tras FROM", "%d nombres" % base["n"], "%.0f ms" % base["ms"])
    # 1 · mensajes rotos por el canal
    rotos = ["no es json", '{"jsonrpc":"2.0","id":"sql:x"}', '{"jsonrpc":"2.0","id":"sql:y","method":"textDocument/completion","params":{}}',
             '{"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":"file:///trabajo/consultas/p5.sql","version":99}}}',
             '{"jsonrpc":"2.0","id":"sql:z","method":"textDocument/hover","params":{"textDocument":{"uri":"file:///trabajo/no-abierto.sql"},"position":{"line":9,"character":9}}}',
             '{"jsonrpc":"2.0","id":"sql:w","method":"no/existe","params":null}']
    cod, _ = R.http("POST", b.directo + "/puestos/%s/lsp" % R.PUESTO, {"mensajes": rotos}, R.ANA)
    time.sleep(1)
    r = c("completion", lenguaje="sql", ruta=uri, texto="select * from ")
    fila("  6 mensajes rotos por el canal (%d)" % cod, "✓ sigue" if r["n"] == base["n"] else "✗ %d" % r["n"], "completion después: %.0f ms" % r["ms"])
    MEDIDO["P5 rotos"] = r["n"] == base["n"]
    # 2 · el árbol cambia con el fichero abierto
    R.escribir(b.A + "/packages/hr/datasets/nuevo_p5.yaml",
               "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: nuevo_p5, namespace: hr }\nspec:\n  owner: team:hr\n  changes: { mode: append }\n  columns:\n    x: { type: Integer }\n")
    t0 = time.time()
    visto = None
    r = c("completion", lenguaje="sql", ruta=uri, texto="select * from ")
    ya = "hr.default.nuevo_p5" in r["labels"] or r["n"] > base["n"]
    otra = 0
    while not ya and time.time() - t0 < 45:
        time.sleep(3)
        otra += 1
        c("abrir", lenguaje="sql", ruta="consultas/otra%d.sql" % otra, texto="select 1")
        r = c("completion", lenguaje="sql", ruta=uri, texto="select * from ")
        ya = r["n"] > base["n"]
    visto = time.time() - t0 if ya else None
    fila("  un dataset nuevo en el árbol, con el .sql abierto", ("%.0f s" % visto) if visto is not None else "✗ no llegó",
         "hasta que se ofrece (se refresca al abrir otro fichero si el índice tiene > 30 s)")
    MEDIDO["P5 dataset nuevo"] = ("%.0f s" % visto) if visto is not None else "no"
    # 3 · una celda pesada mientras se teclea
    for nombre, cel in (("Python puro (tiene el GIL)", "sum(i*i for i in range(60_000_000))"),
                        ("DuckDB (suelta el GIL)", "import duckdb; duckdb.sql('select sum(i) from range(4000000000) t(i)').fetchall()")):
        cod, j = R.http("POST", b.directo + "/puestos/%s/ejecutar" % R.PUESTO, {"texto": cel, "lenguaje": "python"}, R.ANA)
        n = (j or {}).get("celda")
        time.sleep(0.5)
        lat = []
        for _ in range(8):
            lat.append(c("completion", lenguaje="sql", ruta=uri, texto="select v. from hr.ventas v", cur=9)["ms"])
            time.sleep(0.2)
        estado = None
        for _ in range(120):
            cc, jj = R.http("GET", b.directo + "/puestos/%s/celdas/%s" % (R.PUESTO, n), cabeceras=R.ANA)
            estado = (jj or {}).get("estado")
            if estado == "hecha":
                break
            time.sleep(0.5)
        fila("  completion durante una celda de " + nombre, "%.0f ms p50" % med(lat), "%.0f ms máx · la celda: %s" % (max(lat), estado))
        MEDIDO["P5 celda " + nombre.split(" ")[0]] = "%.0f ms p50 · %.0f máx" % (med(lat), max(lat))
    # 4 · un .sql de 360 KB
    grande = "\n".join("select id, pais from hr.ventas where id = %d union all" % i for i in range(7000)) + "\nselect 1, 'x'"
    t0 = time.perf_counter()
    c("abrir", lenguaje="sql", ruta="consultas/grande.sql", texto=grande)
    d = c("marcas", ruta="consultas/grande.sql", esperar_ms=30000)
    r = c("completion", lenguaje="sql", ruta="consultas/grande.sql", texto=grande, cur=len(grande) // 2)
    fila("  un .sql de %d KB por el canal" % (len(grande) // 1024), "marca a los %s ms" % (("%.0f" % d["ms"]) if d["ms"] else "—"),
         "%s · completion a mitad %.0f ms (%d)" % ((d["xs"][0][2] if d["xs"] else "sin marca")[:50], r["ms"], r["n"]))
    MEDIDO["P5 grande"] = "marca %s ms · completion %.0f ms" % (("%.0f" % d["ms"]) if d["ms"] else "—", r["ms"])
    # 5 · el agente se reinicia con el .sql abierto: se pide con EL MISMO texto
    #     que ya salió (no sale ningún didChange que lo tape): o el servidor dice
    #     que no lo tiene y el cliente lo reabre, o se queda vacío.
    q = "select v. from hr.ventas v"
    c("completion", lenguaje="sql", ruta=uri, texto=q, cur=9)
    ag.parar()
    ag.arrancar()
    r1 = c("completion", lenguaje="sql", ruta=uri, texto=q, cur=9)
    h = c("hover", lenguaje="sql", ruta=uri, texto=q, cur=17)
    fila("  el agente se reinicia con el .sql abierto", ("✓ %d columnas" % r1["n"]) if r1["n"] >= 4 else "✗ %d" % r1["n"],
         "con el mismo texto, sin reabrir · %.0f ms · hover %s" % (r1["ms"], "✓" if h.get("valor") else "✗"))
    MEDIDO["P5 agente reiniciado"] = "%d columnas sin reabrir (%.0f ms) · hover %s" % (r1["n"], r1["ms"], bool(h.get("valor")))
    # 6 · seis relevos del agente, cada uno con un editor NUEVO que abre Python y
    #     SQL a la vez: ¿llega todo? (medido antes: 1 de cada 6 perdía los
    #     mensajes de Python —el flujo del agente muerto se los llevaba—)
    c.cerrar()
    bien, tiempos = 0, []
    for _ in range(6):
        ag.parar()
        ag.arrancar()
        c2 = Conductor(b)
        c2("abrir", lenguaje="python", ruta="transforms/x.py", texto="x = 1\n")
        j = c2("juntas", ordenes=[{"orden": "abrir", "lenguaje": "sql", "ruta": "consultas/r.sql", "texto": "select * from "},
                                  {"orden": "completion", "lenguaje": "python", "ruta": "transforms/x.py", "texto": "x = 1\n", "cur": 1}])
        py = j["rs"][1]
        tiempos.append(py["ms"])
        bien += py.get("n") == 1 and py["ms"] < 5000
        c2.cerrar()
    fila("  seis relevos del agente, Python y SQL a la vez", "%d/6" % bien, "completion de Python: %s ms" % [round(x) for x in tiempos])
    MEDIDO["P5 relevos"] = "%d/6" % bien
    c = Conductor(b)
    c("abrir", lenguaje="sql", ruta=uri, texto="select * from ")
    # 7 · el puesto se cierra
    cod, _ = R.http("DELETE", b.directo + "/puestos/%s" % R.PUESTO, None, R.ANA)
    time.sleep(1)
    r = c("completion", lenguaje="sql", ruta=uri, texto="select * from ")
    r2 = c("completion", lenguaje="sql", ruta=uri, texto="select * from ")
    fila("  el puesto se cierra (DELETE %d)" % cod, "completion %.0f ms" % r["ms"], "%d items · la siguiente %.0f ms" % (r["n"], r2["ms"]))
    MEDIDO["P5 puesto cerrado"] = "%.0f ms, luego %.0f ms" % (r["ms"], r2["ms"])
    c.cerrar()


# ═════════════════════════════════════════════════════════════════════════════
def p6(b, ag):
    print()
    print("P6 · EL CANAL  (80 ms por tecla; el texto 200 ms después; completion en cada tecla)")
    c = Conductor(b)
    q = "select v.pais, count(*) as n, sum(v.total) from hr.ventas v join hr.clientes c on c.id = v.id where v.pais = 'ES' group by all"
    py = "import ore\ndf = ore.over('hr.ventas')\nprint(df.shape, df.columns)\n"
    c("abrir", lenguaje="sql", ruta="consultas/p6.sql", texto="")
    c("abrir", lenguaje="python", ruta="transforms/p6.py", texto="")
    for nombre, tareas in (("SQL solo", [{"lenguaje": "sql", "ruta": "consultas/p6.sql", "objetivo": q}]),
                           ("SQL y Python a la vez", [{"lenguaje": "sql", "ruta": "consultas/p6.sql", "objetivo": q},
                                                      {"lenguaje": "python", "ruta": "transforms/p6.py", "objetivo": py * 2}])):
        c("cambiar", lenguaje="sql", ruta="consultas/p6.sql", texto="")
        c("cambiar", lenguaje="python", ruta="transforms/p6.py", texto="")
        time.sleep(0.5)
        a = c("cuenta")
        r = c("a_la_vez", tareas=tareas)
        z = c("cuenta")
        s = r["rs"][0]
        posts = z["posts"] - a["posts"]
        seg = r["ms"] / 1000
        fila("  " + nombre, "SQL %.0f ms p50" % med(s["lat"]), "%.0f p95 · %d completions · %d vacías" % (p95(s["lat"]), len(s["lat"]), s["vacias"]))
        if len(r["rs"]) > 1:
            fila("    Python a la vez", "%.0f ms p50" % med(r["rs"][1]["lat"]), "%.0f p95" % p95(r["rs"][1]["lat"]))
        fila("    peticiones del editor a ore-serve", "%.1f / s" % (posts / seg), "%d en %.1f s · %d eventos de vuelta · códigos %s" % (
            posts, seg, z["eventos"] - a["eventos"], {k: z["codigos"].get(k, 0) - a["codigos"].get(k, 0) for k in z["codigos"]}))
        MEDIDO["P6 " + nombre] = "SQL %.0f/%.0f ms · %.1f pet/s" % (med(s["lat"]), p95(s["lat"]), posts / seg)
    c.cerrar()


def main():
    print("EL EDITOR DE SQL, A FONDO · cliente de la consola + ore-serve + agente, de verdad")
    b = R.Banco()
    ag = Agente(b)
    try:
        b.levantar()
        R.escribir(b.tmp + "/pyright-de-mentira.py", E.PYRIGHT_DE_MENTIRA)
        ag.arrancar()
        if "P4" in SOLO:
            p4(b, ag)
        if "P6" in SOLO:
            p6(b, ag)
        if "P5" in SOLO:
            p5(b, ag)  # el ultimo: cierra el puesto
        print()
        print("LO MEDIDO")
        for k, v in MEDIDO.items():
            fila("  " + k, str(v))
    finally:
        ag.parar()
        b.cerrar()


if __name__ == "__main__":
    main()
