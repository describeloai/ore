"""EL LSP DE SQL · medido antes de construirlo (nuestro indice + el motor del puesto).

`medida-el-sql-del-arbol.py` §4 corrio los servidores de SQL que existen contra
nuestras semillas: ninguno ve el error de verdad (una columna mal escrita) porque
el esquema no esta en ninguna base que puedan abrir, y los dos LSP marcan como
error SQL valido de DuckDB. Lo que SI lo vio fue DuckDB con tablas VACIAS sacadas
del indice (§3 de alli). Esto mide lo que hace falta para construir el nuestro:

  §1  EL INDICE COMO CATALOGO   `ore assets --json` de un arbol real: nombres
                                que se leen desde SQL, columnas, tipos,
                                clasificacion; cuanto pesa y cuanto tarda.
  §2  EL CONTEXTO DEL CURSOR    autocompletar sobre texto A MEDIO ESCRIBIR (no
                                analiza): el tokenizador de DuckDB y el indice;
                                que acierta, que no, cuanto tarda.
  §3  EL BINDER                 DuckDB con una tabla vacia por dataset:
                                montarlo, cuanto ocupa, cuanto tarda en ligar
                                una consulta, que errores da y DONDE (la
                                linea y la columna para subrayar), y que no se
                                queje de lo que es valido.
  §4  LA UNIDAD `.sql`          `ore sql --arbol --json` (lee, escribe, OOS):
                                cuanto tarda por fichero.
  §5  EL CANAL                  la ida y vuelta editor -> ore-serve -> agente
                                -> ore-serve -> editor, con un ore-serve de
                                verdad (el banco de las medidas del catalogo).
  §6  EL EDITOR                 lo que la consola hace hoy con el servidor de
                                lenguaje (leido de su codigo) y lo que un SQL
                                cambia.

    python pruebas-de-fuego/medida-el-lsp-de-sql.py --arbol <dir> [--sin-canal]

`--arbol`: un clon del arbol (victor). No toca el cluster ni la red de nadie.
"""
import importlib.util
import json
import os
import re
import statistics
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__)).replace("\\", "/")
RAIZ = os.path.dirname(AQUI)
CONSOLA = "C:/rubix-platform"
EXE = ".exe" if os.name == "nt" else ""


def arg(n, d=None):
    return sys.argv[sys.argv.index(n) + 1] if n in sys.argv else d


ARBOL = arg("--arbol")
if not ARBOL or not os.path.isdir(ARBOL):
    raise SystemExit("falta --arbol <clon de un arbol>")
ORE = next(p for p in (RAIZ + "/target/release/ore" + EXE, RAIZ + "/target/debug/ore" + EXE) if os.path.isfile(p))
MEDIDO = {}


def fila(k, v="", nota=""):
    print("     %-44s %-22s %s" % (k, v, nota))


def ms(t0):
    return (time.perf_counter() - t0) * 1000


def med(xs):
    return statistics.median(xs) if xs else 0


def p95(xs):
    xs = sorted(xs)
    return xs[max(0, int(len(xs) * 0.95) - 1)] if xs else 0


# ═════════════════════════════════════════════════════════════════════════════
# §1 EL INDICE
# ═════════════════════════════════════════════════════════════════════════════
def s1():
    print()
    print("§1 · EL INDICE COMO CATALOGO  (`ore assets --json`, %s)" % os.path.basename(ARBOL.rstrip("/\\")))
    xs, salida = [], ""
    for _ in range(3):
        t0 = time.perf_counter()
        salida = subprocess.run([ORE, "assets", ".", "--json"], cwd=ARBOL, capture_output=True, text=True,
                                encoding="utf-8").stdout
        xs.append(ms(t0))
    j = json.loads(salida.strip().splitlines()[-1])
    items = list(j["items"].values())
    fila("ore assets --json", "%.0f ms" % med(xs), "%d KB (lo que la consola ya lee por GET /assets)" % (len(salida.encode()) // 1024))
    por = {}
    for i in items:
        por.setdefault(i["kind"], []).append(i)
    fila("por clase", ", ".join("%s %d" % (k, len(v)) for k, v in sorted(por.items())))
    legibles = [i for i in items if i["kind"] in ("Dataset", "View")]
    cols = [c for i in legibles for c in i.get("expone", [])]
    sin_tipo = [c for c in cols if c.get("type") in (None, "Opaque")]
    fila("lo que se lee desde SQL (Dataset + View)", "%d nombres" % len(legibles), "%d columnas; %d sin tipo u Opaque" % (len(cols), len(sin_tipo)))
    tablas = por.get("Table", [])
    fila("Table (de otra fuente: sql() da 409)", "%d" % len(tablas),
         "%d con su View inducida (a la que llevar)" % sum(1 for t in tablas if (t.get("detalle") or {}).get("vistaInducida")))
    clas = [i for i in legibles if (i.get("acceso") or {}).get("clasificacion")]
    fila("con clasificacion (para el hover)", "%d de %d" % (len(clas), len(legibles)),
         "y `acceso.conductos` en %d" % sum(1 for i in legibles if (i.get("acceso") or {}).get("conductos")))
    con_desc = sum(1 for i in legibles if i.get("description"))
    fila("con descripcion", "%d de %d" % (con_desc, len(legibles)))
    MEDIDO["§1 nombres/columnas"] = "%d/%d" % (len(legibles), len(cols))
    return legibles, tablas


# ═════════════════════════════════════════════════════════════════════════════
# §2 EL CONTEXTO DEL CURSOR
# ═════════════════════════════════════════════════════════════════════════════
PALABRAS_DE_TABLA = {"from", "join", "into", "update", "table", "describe", "summarize", "pivot", "unpivot"}


class Catalogo:
    def __init__(self, legibles):
        self.cols = {("%s.%s" % (i["paquete"], i["name"])).lower(): [c["name"] for c in i.get("expone", [])] for i in legibles}
        self.paquetes = sorted({n.split(".")[0] for n in self.cols})

    def nombres(self, paquete=None):
        return sorted(n for n in self.cols if paquete is None or n.startswith(paquete.lower() + "."))


def tokens(texto):
    import duckdb
    ts = duckdb.tokenize(texto)
    out = []
    for i, (pos, tipo) in enumerate(ts):
        fin = ts[i + 1][0] if i + 1 < len(ts) else len(texto)
        out.append((pos, texto[pos:fin].strip(), str(tipo).split(".")[-1].lower()))
    return out


def alias_de(ts, cat):
    """`<p>.<n> [as] <alias>` y `<p>.<n>` en todo el texto: alias -> nombre."""
    al = {}
    txt = [t[1] for t in ts]
    for i in range(len(txt) - 2):
        if txt[i + 1] == "." and ("%s.%s" % (txt[i], txt[i + 2])).lower() in cat.cols:
            n = ("%s.%s" % (txt[i], txt[i + 2])).lower()
            al[txt[i + 2].lower()] = n
            j = i + 3
            if j < len(txt) and txt[j].lower() == "as":
                j += 1
            if j < len(txt) and re.match(r"^[A-Za-z_]\w*$", txt[j]) and txt[j].lower() not in (
                    "where", "join", "on", "group", "order", "limit", "left", "right", "inner", "select", "using", "qualify", "having"):
                al[txt[j].lower()] = n
    return al


def completar(texto, cursor, cat):
    """Lo que se ofrece en `cursor`. Sin analizar: el texto esta a medio escribir."""
    antes = texto[:cursor]
    # Dentro de una cadena (sin cerrar: el tokenizador no la ve como cadena) o
    # de un comentario de linea: nada. Se cuentan las comillas, que es robusto.
    sin_com = re.sub(r"--[^\n]*\n", "\n", antes)
    if sin_com.count("'") % 2 == 1 or re.search(r"--[^\n]*$", antes):
        return "dentro de una cadena o un comentario", []
    # la palabra que se esta escribiendo no cuenta como contexto
    m = re.search(r"[A-Za-z_]\w*$", antes)
    base = antes[: m.start()] if m else antes
    ts = [t for t in tokens(base) if t[2] != "comment" and t[1]]
    todo = [t for t in tokens(texto) if t[2] != "comment" and t[1]]
    al = alias_de(todo, cat)
    if ts and ts[-1][1] == ".":
        q = ts[-2][1].lower() if len(ts) > 1 else ""
        if q in [p.lower() for p in cat.paquetes]:
            return "nombres de %s" % q, [n.split(".", 1)[1] for n in cat.nombres(q)]
        if q in al:
            return "columnas de %s" % al[q], cat.cols[al[q]]
        return "?", []
    ultima = next((t[1].lower() for t in reversed(ts) if t[2] == "keyword"), "")
    if ultima in PALABRAS_DE_TABLA:
        return "nombres", cat.nombres()
    usados = sorted(set(al.values()))
    return "columnas de %s" % ", ".join(usados), [c for n in usados for c in cat.cols[n]] + sorted(al)


def s2(cat):
    print()
    print("§2 · EL CONTEXTO DEL CURSOR  (tokenizador de DuckDB + indice; `|` es el cursor)")
    P = "standard_test"
    prod, ads = cat.cols.get(P + ".products", []), cat.cols.get(P + ".product_ads", [])
    casos = [
        ("tras FROM", "select * from |", {P + ".products"}),
        ("tras el paquete", "select * from standard_test.|", {"products", "product_ads"}),
        ("a medio nombre", "select * from standard_test.pro|", {"products"}),
        ("la lista de SELECT", "select | from standard_test.products", set(prod)),
        ("alias.", "select p.| from standard_test.products p", set(prod)),
        ("el ON de un join", "select * from standard_test.products p join standard_test.product_ads a on a.|", set(ads)),
        ("WHERE", "select * from standard_test.products where |", set(prod)),
        ("GROUP BY", "select count(*) from standard_test.products group by |", set(prod)),
        ("FROM primero", "from standard_test.products select |", set(prod)),
        ("un comentario delante", "-- from standard_test.user\nselect | from standard_test.products", set(prod)),
        ("subconsulta (ofrece tambien las de fuera)", "select * from standard_test.products where id in (select | from standard_test.product_ads)", set(ads) - set(prod)),
        ("una CTE", "with t as (select id, price from standard_test.product_price_history) select | from t", {"price"}),
        ("cadena sin cerrar", "select * from standard_test.products where category = 'ab|", set()),
    ]
    xs, bien = [], 0
    for nombre, t, esperado in casos:
        cur = t.index("|")
        texto = t.replace("|", "")
        t0 = time.perf_counter()
        try:
            ctx, ofrece = completar(texto, cur, cat)
        except Exception as e:
            ctx, ofrece = "✗ %s" % e, []
        xs.append(ms(t0))
        ok = esperado <= set(ofrece) if esperado else len(ofrece) == 0
        # lo que sobra: que ofrezca columnas de otra tabla donde no toca
        bien += ok
        fila("  " + nombre, "✓" if ok else "✗", "%s · %d ofrecidas" % (ctx[:60], len(ofrece)))
    fila("aciertos", "%d/%d" % (bien, len(casos)), "%.2f ms mediana · %.2f ms p95" % (med(xs), p95(xs)))
    MEDIDO["§2 aciertos"] = "%d/%d" % (bien, len(casos))


# ═════════════════════════════════════════════════════════════════════════════
# §3 EL BINDER
# ═════════════════════════════════════════════════════════════════════════════
def tipo_duckdb(t):
    t = t or ""
    if t.startswith("list<"):
        return tipo_duckdb(t[5:-1]) + "[]"
    m = re.match(r"^(Money|Quantity)<[^,]+,\s*(\d+)>$", t)
    if m:
        return "DECIMAL(38, %s)" % min(int(m.group(2)), 18)
    return {"Integer": "BIGINT", "Decimal": "DECIMAL(38, 18)", "Float": "DOUBLE", "Boolean": "BOOLEAN",
            "Date": "DATE", "Time": "TIME", "DateTime": "TIMESTAMP", "DateTimeTz": "TIMESTAMPTZ"}.get(t, "VARCHAR")


def rss_mb():
    try:
        import psutil
        return psutil.Process().memory_info().rss / 1e6
    except Exception:
        return None


def montar(legibles):
    import duckdb
    antes = rss_mb()
    t0 = time.perf_counter()
    con = duckdb.connect()
    for p in sorted({i["paquete"] for i in legibles}):
        con.execute('create schema "%s"' % p)
    for i in legibles:
        cols = ", ".join('"%s" %s' % (c["name"].replace('"', '""'), tipo_duckdb(c.get("type"))) for c in i.get("expone", []))
        con.execute('create table "%s"."%s" (%s)' % (i["paquete"], i["name"], cols or "x VARCHAR"))
    return con, ms(t0), (rss_mb() - antes) if antes else None


PREFIJO = "explain "


def donde(texto, e):
    """(linea, columna) 1-based del error: el `^` de DuckDB (su linea 1 lleva el
    `explain ` con el que se liga sin ejecutar), o el final si es «end of input»."""
    m = str(e)
    if "at end of input" in m:
        return texto.count("\n") + 1, len(texto) - (texto.rfind("\n") + 1) + 1, "fin"
    lineas = m.split("\n")
    for k, l in enumerate(lineas):
        g = re.match(r"^LINE (\d+): (.*)$", l)
        if g and k + 1 < len(lineas) and "^" in lineas[k + 1]:
            n = int(g.group(1))
            trozo = g.group(2)
            car = lineas[k + 1].index("^") - len("LINE %d: " % n)
            linea = (PREFIJO + texto).split("\n")[n - 1]
            if trozo.startswith("..."):
                # DuckDB recorta por la izquierda: se busca el trozo en la linea
                i = linea.find(trozo[3:].rstrip(".")[:20])
                col = i + car - 3 + 1 if i >= 0 else None
            else:
                col = car + 1
            if n == 1 and col is not None:
                col -= len(PREFIJO)
            return n, col, "caret"
    g = re.search(r'"([^"]+)"', m)
    if g and g.group(1) in texto:
        i = texto.index(g.group(1))
        return texto[:i].count("\n") + 1, i - (texto.rfind("\n", 0, i) + 1) + 1, "nombre"
    return None, None, "-"


def s3(legibles):
    print()
    print("§3 · EL BINDER  (DuckDB, una tabla VACIA por Dataset/View con los tipos del indice)")
    import duckdb
    fila("duckdb", duckdb.__version__)
    con, t_montar, mb = montar(legibles)
    fila("montar el esquema", "%.0f ms" % t_montar, "%d tablas · %s" % (len(legibles), ("+%.1f MB de RSS" % mb) if mb is not None else "RSS: sin psutil"))
    P = "standard_test"
    casos = [
        ("bien", "select id, summary_text from standard_test.ai_insights where user_id = 'x'", None),
        ("columna mal escrita", "select summary_txt from standard_test.ai_insights", ("summary_txt", 1, 8)),
        ("columna mal, linea 3", "select id,\n  category\nfrom standard_test.products where catgory = 'a'", ("catgory", 3, 35)),
        ("tabla mal escrita", "select * from standard_test.ai_insight", ("ai_insight", 1, 15)),
        ("alias.columna mal", "select a.idd from standard_test.ai_insights a", ("idd", 1, 8)),
        ("tipo: sum de texto", "select sum(summary_text) from standard_test.ai_insights", ("sum", 1, 8)),
        ("sintaxis", "select from where", ("where", 1, 13)),
        ("join bien", "select p.id, a.ad_copy from standard_test.products p join standard_test.product_ads a on a.product_id = p.id", None),
        ("group by all", "select category, count(*) from standard_test.products group by all", None),
        ("exclude", "select * exclude (image_url) from standard_test.products", None),
        ("qualify", "select id, row_number() over (partition by category order by id) rn from standard_test.products qualify rn = 1", None),
        ("FROM primero", "from standard_test.products select id", None),
        ("CTAS (la unidad)", "create or replace table standard_test.salida as select id from standard_test.products", None),
        ("insert by name", "insert into standard_test.prueba_de_pk by name select 'x' as id", None),
        ("a medio escribir", "select id from standard_test.products where ", ("where", 1, 45)),
        ("decimal * entero", "select max_price * 2 from standard_test.products", None),
    ]
    xs, bien, pos_bien, pos_n, falsos = [], 0, 0, 0, 0
    for nombre, q, esperado in casos:
        t0 = time.perf_counter()
        try:
            con.execute("explain " + q)
            e = None
        except Exception as ex:
            e = ex
        xs.append(ms(t0))
        if esperado is None:
            ok = e is None
            falsos += e is not None
            fila("  " + nombre, "✓ liga" if ok else "✗ FALSO", "" if ok else str(e).split("\n")[0][:80])
        else:
            ok = e is not None
            linea, col, como = donde(q, e) if e else (None, None, "-")
            pos_n += 1
            en_sitio = linea == esperado[1] and col is not None and abs(col - esperado[2]) <= 2
            pos_bien += en_sitio
            fila("  " + nombre, ("✓ error" if ok else "✗ no dice nada"),
                 "L%s:C%s (%s) %s · %s" % (linea, col, como, "✓" if en_sitio else "≠ L%d:C%d" % esperado[1:], str(e).split("\n")[0][:70] if e else ""))
        bien += ok
    fila("aciertos", "%d/%d" % (bien, len(casos)), "%d errores falsos en SQL valido · posicion bien %d/%d" % (falsos, pos_bien, pos_n))
    fila("ligar una consulta (explain)", "%.2f ms mediana" % med(xs), "%.2f ms p95" % p95(xs))
    MEDIDO["§3 aciertos"] = "%d/%d" % (bien, len(casos))
    MEDIDO["§3 falsos"] = falsos
    MEDIDO["§3 posicion"] = "%d/%d" % (pos_bien, pos_n)
    # la sugerencia: ¿dice DuckDB lo que se queria escribir?
    try:
        con.execute("explain select summary_txt from standard_test.ai_insights")
    except Exception as e:
        fila("la sugerencia de DuckDB", "", str(e).replace("\n", " ")[:120])
    con.close()


# ═════════════════════════════════════════════════════════════════════════════
# §4 LA UNIDAD
# ═════════════════════════════════════════════════════════════════════════════
def s4():
    print()
    print("§4 · LA UNIDAD `.sql`  (`ore sql <fichero> --arbol . --json`, un proceso por fichero)")
    d = tempfile.mkdtemp()
    casos = [("select", "select id from standard_test.products"),
             ("CTAS", "create or replace table standard_test.salida_lsp as select id, max_price from standard_test.products"),
             ("lee lo que no hay", "select * from standard_test.nada"),
             ("dos sentencias", "select 1; select 2")]
    for nombre, q in casos:
        f = os.path.join(d, "u.sql")
        open(f, "w", encoding="utf-8").write(q)
        xs, out = [], None
        for _ in range(3):
            t0 = time.perf_counter()
            out = subprocess.run([ORE, "sql", f, "--arbol", ".", "--json"], cwd=ARBOL, capture_output=True, text=True, encoding="utf-8")
            xs.append(ms(t0))
        dice = (out.stdout.strip().splitlines() or [out.stderr.strip()])[-1]
        fila("  " + nombre, "%.0f ms" % med(xs), "codigo %d · %s" % (out.returncode, dice[:90]))
    MEDIDO["§4 ms"] = round(med(xs))


# ═════════════════════════════════════════════════════════════════════════════
# §5 EL CANAL
# ═════════════════════════════════════════════════════════════════════════════
def s5():
    print()
    print("§5 · EL CANAL  (editor -> POST /lsp -> agente (SSE) -> POST /lsp/salida -> editor (SSE))")
    sys.argv[:] = [sys.argv[0], "--filas", "100"]
    sp = importlib.util.spec_from_file_location("resolutor", AQUI + "/medida-el-catalogo-como-resolutor.py")
    R = importlib.util.module_from_spec(sp)
    sp.loader.exec_module(R)
    b = R.Banco()
    try:
        b.levantar()
        base, P = b.directo, R.PUESTO
        recibidos = {}
        listo = threading.Event()

        def flujo(ruta, cab, cada):
            req = urllib.request.Request(base + ruta, headers=dict(cab, accept="text/event-stream"))
            with urllib.request.urlopen(req, timeout=120) as r:
                listo.set()
                ev = None
                for l in r:
                    l = l.decode("utf-8", "replace").rstrip("\r\n")
                    if l.startswith("event: "):
                        ev = l[7:]
                    elif l.startswith("data: ") and ev == "lsp":
                        cada(l[6:])

        def agente(dato):
            m = json.loads(dato)
            if "id" in m:
                respuesta = {"jsonrpc": "2.0", "id": m["id"], "result": {"items": [{"label": "products"}]}}
                R.http("POST", base + "/puestos/%s/lsp/salida" % P, {"mensajes": [json.dumps(respuesta)]}, R.AGENTE)

        def consola(dato):
            m = json.loads(dato)
            if "id" in m:
                recibidos[m["id"]] = time.perf_counter()

        threading.Thread(target=flujo, args=("/puestos/%s/lsp/agente" % P, R.AGENTE, agente), daemon=True).start()
        listo.wait(10)
        listo.clear()
        threading.Thread(target=flujo, args=("/puestos/%s/lsp/consola" % P, R.ANA, consola), daemon=True).start()
        listo.wait(10)
        time.sleep(0.5)
        xs = []
        for i in range(1, 41):
            m = {"jsonrpc": "2.0", "id": 1000 + i, "method": "textDocument/completion",
                 "params": {"textDocument": {"uri": "file:///trabajo/a.sql"}, "position": {"line": 0, "character": 14}}}
            t0 = time.perf_counter()
            R.http("POST", base + "/puestos/%s/lsp" % P, {"mensajes": [json.dumps(m)]}, R.ANA)
            for _ in range(400):
                if 1000 + i in recibidos:
                    break
                time.sleep(0.002)
            if 1000 + i in recibidos:
                xs.append((recibidos[1000 + i] - t0) * 1000)
        fila("ida y vuelta (sin servidor de lenguaje)", "%.0f ms mediana" % med(xs), "%.0f ms p95 · %d/40 llegaron" % (p95(xs), len(xs)))
        fila("  + el lote del editor (LOTE_MS)", "25 ms", "+ la espera antes de didChange: 200 ms (ArbolFileView)")
        MEDIDO["§5 canal ms"] = round(med(xs))
    finally:
        b.cerrar()


# ═════════════════════════════════════════════════════════════════════════════
# §6 EL EDITOR
# ═════════════════════════════════════════════════════════════════════════════
def s6():
    print()
    print("§6 · EL EDITOR  (la consola, leida: %s)" % CONSOLA)
    cliente = open(CONSOLA + "/components/code-workspace/servidor-de-lenguaje.ts", encoding="utf-8").read()
    vista = open(CONSOLA + "/components/code-workspace/ArbolFileView.tsx", encoding="utf-8").read()
    agente = open(RAIZ + "/puesto/python/agente.py", encoding="utf-8").read()
    hechos = [
        ("los lenguajes con servidor", "export type Lenguaje = 'python' | 'java';" in cliente, "`sql` no esta: un .sql no abre servidor"),
        ("quien abre el servidor", "file.language === 'python' ? 'python' : file.language === 'java' ? 'java' : null" in vista,
         "por el lenguaje del fichero; SQL iria al puesto de Python (DuckDB)"),
        ("UNO por puesto", "const VIVOS = new Map<string, ServidorDeLenguaje>();" in cliente and "VIVOS.get(puesto)" in cliente,
         "un .py y un .sql en el mismo puesto comparten cliente: hay que partirlo por lenguaje"),
        ("proveedores del lenguaje del cliente", "registerHoverProvider(this.lenguaje" in cliente,
         "hover y completion se registran para UN lenguaje de Monaco"),
        ("diagnosticos", "textDocument/publishDiagnostics" in cliente, "ya se pintan (dueño propio, conviven con los de `ore`)"),
        ("el disparador de completion", "triggerCharacters: ['.']" in cliente, "el punto: `paquete.` y `alias.`"),
        ("didChange tras 200 ms", "cambiar(file.id, texto), 200)" in vista, "lo que tarda en llegar el texto nuevo"),
        ("el agente: un proceso por LSP", "ORE_LSP" in agente and "pyright" in agente,
         "manda TODO al proceso de pyright: el SQL tiene que desviarse por uri/lenguaje"),
    ]
    for k, ok, nota in hechos:
        fila("  " + k, "✓" if ok else "✗ (cambió)", nota)


def main():
    print("EL LSP DE SQL · %s" % ARBOL)
    legibles, _ = s1()
    s2(Catalogo(legibles))
    s3(legibles)
    s4()
    if "--sin-canal" not in sys.argv:
        s5()
    s6()
    print()
    print("LO MEDIDO")
    for k, v in MEDIDO.items():
        fila("  " + k, str(v))


if __name__ == "__main__":
    main()
