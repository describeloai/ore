"""PROTOTIPO DESECHABLE · el banco del servidor de SQL: un editor de mentira por stdio.

  P2  CONFORMIDAD   los casos de `medida-el-lsp-de-sql.py` por el protocolo:
                    completion, diagnosticos (y su rango) y hover.
  P3  TECLEO        consultas escritas tecla a tecla (80 ms por tecla), con
                    completion en cada tecla (lo que hace Monaco). Dos editores:
                    HOY (didChange 200 ms despues, completion YA) y ARREGLADO
                    (el texto sale antes de pedir). ¿Cuantas completions se
                    calculan sobre texto viejo? ¿Cuanto parpadean los errores?
  P4  ESCALA        un indice de mentira de 1000 y 5000 datasets.
  P5  ROBUSTEZ      basura, un documento enorme, Unicode, rafagas, recargar el
                    indice con el fichero abierto, miles de peticiones.

    python banco.py --indice assets.json [--solo P2,P3] [--real]

`--real`: contra el modulo de verdad (`python -m ore.lsp_sql`, en
`puesto/python`) y no contra el prototipo; sus variantes (`--mostrar-fin`,
`--todo-al-arrancar`) no existen alli y se saltan.
"""
import json
import os
import re
import statistics
import subprocess
import sys
import tempfile
import threading
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, AQUI)
REAL = "--real" in sys.argv
PUESTO_PY = os.path.join(os.path.dirname(os.path.dirname(AQUI)), "puesto", "python")
if REAL:
    sys.path.insert(0, PUESTO_PY)
    from ore import lsp_sql as S  # noqa: E402
else:
    import servidor as S  # noqa: E402  (el oraculo: el mismo contexto, sobre el texto de verdad)


def arg(n, d=None):
    return sys.argv[sys.argv.index(n) + 1] if n in sys.argv else d


INDICE = arg("--indice")
SOLO = set((arg("--solo") or "P2,P3,P4,P5").split(","))
MEDIDO = {}


def fila(k, v="", nota=""):
    print("     %-46s %-20s %s" % (k, v, nota))


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


class Cliente:
    """Un editor de mentira: habla LSP por stdio con el servidor."""

    def __init__(self, indice, *extra):
        orden = ([sys.executable, "-m", "ore.lsp_sql", "--indice", indice] if REAL
                 else [sys.executable, os.path.join(AQUI, "servidor.py"), "--indice", indice, *extra])
        self.p = subprocess.Popen(orden, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                  cwd=PUESTO_PY if REAL else None)
        self.sig = 1
        self.resp = {}
        self.diags = []  # (t, uri, version, diagnostics)
        self.cv = threading.Condition()
        threading.Thread(target=self._leer, daemon=True).start()
        t0 = time.perf_counter()
        self.pedir("initialize", {"processId": None, "rootUri": "file:///trabajo", "capabilities": {}}, plazo=120)
        self.arranque_ms = (time.perf_counter() - t0) * 1000
        self.notificar("initialized", {})

    def _leer(self):
        f = self.p.stdout
        while True:
            largo = None
            while True:
                l = f.readline()
                if not l:
                    return
                l = l.strip()
                if not l:
                    break
                if l.lower().startswith(b"content-length:"):
                    largo = int(l.split(b":")[1])
            m = json.loads(f.read(largo).decode("utf-8"))
            with self.cv:
                if "id" in m and "method" not in m:
                    self.resp[m["id"]] = (time.perf_counter(), m)
                elif m.get("method") == "textDocument/publishDiagnostics":
                    pr = m["params"]
                    self.diags.append((time.perf_counter(), pr["uri"], pr.get("version"), pr["diagnostics"]))
                self.cv.notify_all()

    def crudo(self, b):
        self.p.stdin.write(b)
        self.p.stdin.flush()

    def _mandar(self, m):
        b = json.dumps(m).encode("utf-8")
        self.crudo(b"Content-Length: %d\r\n\r\n" % len(b) + b)

    def notificar(self, metodo, params):
        self._mandar({"jsonrpc": "2.0", "method": metodo, "params": params})

    def pedir(self, metodo, params, plazo=10):
        i = self.sig
        self.sig += 1
        t0 = time.perf_counter()
        self._mandar({"jsonrpc": "2.0", "id": i, "method": metodo, "params": params})
        with self.cv:
            self.cv.wait_for(lambda: i in self.resp, timeout=plazo)
            t, m = self.resp.pop(i, (None, None))
        return (m or {}).get("result"), ((t - t0) * 1000 if t else None), m

    def abrir(self, uri, texto, v=1):
        self.notificar("textDocument/didOpen", {"textDocument": {"uri": uri, "languageId": "sql", "version": v, "text": texto}})

    def cambiar(self, uri, texto, v):
        self.notificar("textDocument/didChange", {"textDocument": {"uri": uri, "version": v}, "contentChanges": [{"text": texto}]})

    def completion(self, uri, linea, col):
        r, t, _ = self.pedir("textDocument/completion", {"textDocument": {"uri": uri}, "position": {"line": linea, "character": col}})
        return [x["label"] for x in (r or {}).get("items", [])], t

    def esperar_diag(self, uri, version=None, plazo=5):
        with self.cv:
            self.cv.wait_for(lambda: any(d[1] == uri and (version is None or d[2] == version) for d in self.diags), timeout=plazo)
            ds = [d for d in self.diags if d[1] == uri and (version is None or d[2] == version)]
        return ds[-1] if ds else None

    def vivo(self):
        r, t, m = self.pedir("textDocument/hover", {"textDocument": {"uri": "file:///nada.sql"}, "position": {"line": 0, "character": 0}}, plazo=5)
        return m is not None

    def cerrar(self):
        try:
            self.pedir("shutdown", {}, plazo=3)
            self.notificar("exit", {})
            self.p.wait(5)
        except Exception:
            self.p.kill()


def lc(texto, cur):
    antes = texto[:cur]
    return antes.count("\n"), cur - (antes.rfind("\n") + 1)


# ═════════════════════════════════════════════════════════════════════════════
def p2():
    print()
    print("P2 · CONFORMIDAD  (por el protocolo, contra el arbol de victor)")
    c = Cliente(INDICE)
    fila("arrancar (montar el esquema + initialize)", "%.0f ms" % c.arranque_ms, "RSS %s MB" % (("%.0f" % rss(c.p.pid)) if rss(c.p.pid) else "?"))
    P = "standard_test"
    comp = [
        ("tras FROM", "select * from |", {P + ".products"}),
        ("tras el paquete", "select * from standard_test.|", {"products"}),
        ("la lista de SELECT", "select | from standard_test.products", {"max_price", "category"}),
        ("alias.", "select p.| from standard_test.products p", {"max_price"}),
        ("ON de un join", "select * from standard_test.products p join standard_test.product_ads a on a.|", {"ad_copy"}),
        ("FROM primero", "from standard_test.products select |", {"category"}),
        ("una linea mas abajo", "select *\nfrom standard_test.products p\nwhere p.|", {"max_price"}),
        ("en una cadena", "select * from standard_test.products where category = 'ab|", set()),
    ]
    bien, ts = 0, []
    for n, (nombre, t, esp) in enumerate(comp):
        uri = "file:///trabajo/c%d.sql" % n
        cur = t.index("|")
        texto = t.replace("|", "")
        c.abrir(uri, texto)
        labels, ms = c.completion(uri, *lc(texto, cur))
        ts.append(ms)
        ok = esp <= set(labels) if esp else not labels
        bien += ok
        fila("  completion: " + nombre, "✓" if ok else "✗", "%d items · %.1f ms" % (len(labels), ms))
    MEDIDO["P2 completion"] = "%d/%d · %.1f ms" % (bien, len(comp), med(ts))
    diag = [
        ("bien", "select id, summary_text from standard_test.ai_insights", None),
        ("columna mal", "select summary_txt from standard_test.ai_insights", (0, 7)),
        ("en la linea 3", "select id,\n  category\nfrom standard_test.products where catgory = 'a'", (2, 34)),
        ("tabla mal", "select * from standard_test.ai_insight", (0, 14)),
        ("tipo", "select sum(summary_text) from standard_test.ai_insights", (0, 7)),
        ("una Table de otra fuente", "select * from foreign_test.public_ShopifyStore", (0, 14)),
        ("a medio escribir (no se dice)", "select id from standard_test.products where ", None),
        ("CTAS", "create or replace table standard_test.x as select id from standard_test.products", None),
    ]
    bien, ts = 0, []
    for n, (nombre, texto, esp) in enumerate(diag):
        uri = "file:///trabajo/d%d.sql" % n
        t0 = time.perf_counter()
        c.abrir(uri, texto)
        d = c.esperar_diag(uri, 1)
        ts.append((d[0] - t0) * 1000 if d else 0)
        ds = d[3] if d else None
        if esp is None:
            ok = ds == []
            dice = "sin errores" if ok else (ds[0]["message"][:70] if ds else "no publico")
        else:
            ok = bool(ds) and (ds[0]["range"]["start"]["line"], ds[0]["range"]["start"]["character"]) == esp
            dice = ("L%d:C%d %s" % (ds[0]["range"]["start"]["line"] + 1, ds[0]["range"]["start"]["character"] + 1, ds[0]["message"][:60])) if ds else "no publico"
        bien += ok
        fila("  diagnostico: " + nombre, "✓" if ok else "✗", dice)
    MEDIDO["P2 diagnosticos"] = "%d/%d · %.0f ms (con %d ms de demora)" % (bien, len(diag), med(ts), S.DEMORA * 1000)
    hv = [("un dataset", "select * from standard_test.products", 20, "standard_test.products"),
          ("una columna", "select max_price from standard_test.products", 9, "max_price"),
          ("alias.columna", "select p.max_price from standard_test.products p", 12, "max_price")]
    for n, (nombre, texto, col, espera) in enumerate(hv):
        uri = "file:///trabajo/h%d.sql" % n
        c.abrir(uri, texto)
        r, ms, _ = c.pedir("textDocument/hover", {"textDocument": {"uri": uri}, "position": {"line": 0, "character": col}})
        v = ((r or {}).get("contents") or {}).get("value", "")
        fila("  hover: " + nombre, "✓" if espera in v else "✗", v.replace("\n", " ")[:90])
    c.cerrar()


# ═════════════════════════════════════════════════════════════════════════════
ESCRITURAS = [
    "select p.category, count(*) from standard_test.products p where p.max_price > 10 group by all",
    "select a.ad_copy, p.category\nfrom standard_test.product_ads a\njoin standard_test.products p on p.id = a.product_id",
    "select summary_txt from standard_test.ai_insights",
]


def p3():
    print()
    print("P3 · TECLEO  (80 ms por tecla, completion en cada tecla, como Monaco)")
    for mostrar_fin in ((False,) if REAL else (False, True)):
        for editor in ("HOY", "ARREGLADO"):
            c = Cliente(INDICE, *(["--mostrar-fin"] if mostrar_fin else []))
            viejas = total = 0
            lat, parpadeos, final_ok = [], 0, 0
            for n, objetivo in enumerate(ESCRITURAS):
                uri = "file:///trabajo/t%d.sql" % n
                c.abrir(uri, "")
                v, ultimo_envio, pendiente = 1, 0.0, None
                for k in range(1, len(objetivo) + 1):
                    texto = objetivo[:k]
                    ahora = time.perf_counter()
                    if editor == "ARREGLADO":
                        v += 1
                        c.cambiar(uri, texto, v)
                    else:
                        pendiente = texto
                    # Monaco pide completion al teclear una palabra o un punto
                    if re.match(r"[\w.]", objetivo[k - 1]):
                        labels, ms = c.completion(uri, *lc(texto, k))
                        lat.append(ms)
                        # el oraculo: lo que el servidor deberia dar CON ESE texto (el de
                        # verdad filtra por lo escrito y recorta: `completar_lista`)
                        oraculo = [x[0] for x in (S.completar_lista(texto, k, c_cat())[0] if hasattr(S, "completar_lista")
                                                  else S.completar(texto, k, c_cat()))]
                        total += 1
                        viejas += set(labels) != set(oraculo)
                    time.sleep(0.08)
                    # HOY: el texto sale cuando pasan 200 ms sin teclear (aqui: en las pausas)
                    if editor == "HOY" and pendiente and objetivo[k - 1] in " \n":
                        time.sleep(0.2)
                        v += 1
                        c.cambiar(uri, pendiente, v)
                        pendiente = None
                if pendiente:
                    time.sleep(0.2)
                    v += 1
                    c.cambiar(uri, pendiente, v)
                d = c.esperar_diag(uri, v, plazo=5)
                ds_de = [x for x in c.diags if x[1] == uri]
                parpadeos += sum(1 for i in range(1, len(ds_de)) if bool(ds_de[i][3]) != bool(ds_de[i - 1][3]))
                esperado = 1 if "summary_txt" in objetivo else 0
                final_ok += bool(d) and len(d[3]) == esperado
            fila("  %s · %s" % (editor, "fin de texto SE DICE" if mostrar_fin else "fin de texto no se dice"),
                 "%d/%d sobre texto viejo" % (viejas, total),
                 "completion %.1f ms p50 %.1f p95 · %d parpadeos · final bien %d/%d" % (med(lat), p95(lat), parpadeos, final_ok, len(ESCRITURAS)))
            MEDIDO["P3 %s %s" % (editor, "fin" if mostrar_fin else "sin fin")] = "%d/%d viejas · %d parpadeos" % (viejas, total, parpadeos)
            c.cerrar()


_CAT = {}


def c_cat():
    if "c" not in _CAT:
        _CAT["c"] = S.Catalogo(json.load(open(INDICE, encoding="utf-8")))
    return _CAT["c"]


# ═════════════════════════════════════════════════════════════════════════════
def indice_de_mentira(n, cols=30):
    items = {}
    for i in range(n):
        p = "paquete_%02d" % (i % 20)
        nombre = "dataset_%04d" % i
        tipos = ["Integer", "String", "Decimal", "DateTimeTz", "Boolean", "Date"]
        items["dataset:%s.%s" % (p, nombre)] = {
            "kind": "Dataset", "paquete": p, "name": nombre, "owner": "team:x", "acceso": {"conductos": {}},
            "expone": [{"name": "col_%02d" % j, "type": tipos[j % len(tipos)]} for j in range(cols)]}
    f = os.path.join(tempfile.gettempdir(), "indice-%d.json" % n)
    json.dump({"items": items}, open(f, "w", encoding="utf-8"))
    return f


def p4():
    print()
    print("P4 · ESCALA  (un indice de mentira: N datasets x 30 columnas, 20 paquetes)")
    for n, modo in (((19, ""), (1000, ""), (5000, "")) if REAL else
                    ((19, ""), (1000, "--todo-al-arrancar"), (1000, ""), (5000, "--todo-al-arrancar"), (5000, ""))):
        f = INDICE if n == 19 else indice_de_mentira(n)
        c = Cliente(f, *([modo] if modo else []))
        uri = "file:///trabajo/e.sql"
        q = "select a.col_01 from paquete_03.dataset_0003 a join paquete_04.dataset_0004 b on a.col_00 = b.col_00 where " \
            if n != 19 else "select p.category from standard_test.products p where "
        c.abrir(uri, q)
        _, t_from = c.completion(uri, 0, 0)
        c.cambiar(uri, "select * from ", 2)
        items, t_nombres = c.completion(uri, 0, 14)
        c.cambiar(uri, q, 3)
        cols, t_cols = c.completion(uri, 0, len(q))
        t0 = time.perf_counter()
        c.cambiar(uri, q + "x = 1", 4)
        d = c.esperar_diag(uri, 4, plazo=10)
        t_diag = (d[0] - t0) * 1000 - S.DEMORA * 1000 if d else -1
        fila("  %d datasets%s" % (n, " · todo al arrancar" if modo else " · bajo demanda"), "arranque %.0f ms" % c.arranque_ms,
             "RSS %s MB · tras FROM %d items %.0f ms · columnas %d %.0f ms · ligar %.0f ms" % (
                 ("%.0f" % rss(c.p.pid)) if rss(c.p.pid) else "?", len(items), t_nombres, len(cols), t_cols, t_diag))
        MEDIDO["P4 %d%s" % (n, " todo" if modo else "")] = "arranque %.0f ms · FROM %.0f ms · ligar %.0f ms" % (c.arranque_ms, t_nombres, t_diag)
        c.cerrar()


# ═════════════════════════════════════════════════════════════════════════════
def p5():
    print()
    print("P5 · ROBUSTEZ")
    c = Cliente(INDICE)
    uri = "file:///trabajo/r.sql"
    casos = [
        ("basura", "\x00\x01 select ;;; ''' \"\"\" from from from ((( "),
        ("Unicode", "select 'ñandú 🦙' as x, id from standard_test.products where category = 'café'"),
        ("un comentario sin fin", "select /* nunca se cierra from standard_test.products"),
        ("vacio", ""),
        ("muchas sentencias", "select 1; select 2; select id from standard_test.products;"),
    ]
    for k, (nombre, t) in enumerate(casos):
        c.abrir(uri + str(k), t)
        d = c.esperar_diag(uri + str(k), 1)
        labels, _ = c.completion(uri + str(k), 0, len(t))
        fila("  " + nombre, "✓ vivo" if c.vivo() else "✗ MUERTO", "diag: %s · completion %d" % (
            (d[3][0]["message"][:60] if d and d[3] else "ninguno") if d else "no publico", len(labels)))
    # varias sentencias: el error de la tercera, en su linea; y la que escribe NO se ejecuta
    t = "select 1;\nselect id from standard_test.products;\nselect catgory from standard_test.products"
    c.abrir(uri + "s", t)
    d = c.esperar_diag(uri + "s", 1)
    r0 = d[3][0]["range"]["start"] if d and d[3] else {}
    fila("  el error de la 3a sentencia", "✓" if (r0.get("line"), r0.get("character")) == (2, 7) else "✗",
         "L%s:C%s %s" % (r0.get("line", -1) + 1, r0.get("character", -1) + 1, d[3][0]["message"][:50] if d and d[3] else ""))
    c.abrir(uri + "w", "select 1; create table standard_test.zzz as select 1 as x")
    c.esperar_diag(uri + "w", 1)
    c.abrir(uri + "w2", "select x from standard_test.zzz")
    d = c.esperar_diag(uri + "w2", 1)
    fila("  una 2a sentencia que escribe", "✓ no se ejecuto" if d and d[3] else "✗ SE EJECUTO",
         d[3][0]["message"][:70] if d and d[3] else "select x from standard_test.zzz liga: la tabla existe")
    grande = "\n".join("select id, category from standard_test.products where id = %d union all" % i for i in range(5000)) + "\nselect 1, 'x'"
    t0 = time.perf_counter()
    c.abrir(uri + "g", grande)
    d = c.esperar_diag(uri + "g", 1, plazo=60)
    fila("  5000 lineas (%d KB)" % (len(grande) // 1024), "%.0f ms" % ((d[0] - t0) * 1000 if d else -1),
         "diag: %s · vivo %s" % (len(d[3]) if d else "no", c.vivo()))
    labels, t = c.completion(uri + "g", 2500, 20)
    fila("  completion en la linea 2500", "%.0f ms" % t, "%d items" % len(labels))
    # rafaga: 300 cambios seguidos, sin pausa
    c.abrir(uri + "b", "")
    antes = len([x for x in c.diags if x[1] == uri + "b"])
    for v in range(2, 302):
        c.cambiar(uri + "b", "select summary_txt from standard_test.ai_insights"[: (v % 49) + 1], v)
    c.cambiar(uri + "b", "select summary_txt from standard_test.ai_insights", 302)
    d = c.esperar_diag(uri + "b", 302, plazo=5)
    publicados = len([x for x in c.diags if x[1] == uri + "b"]) - antes
    fila("  rafaga de 301 cambios", "%d publicados" % publicados, "el ultimo: %s" % (d[3][0]["message"][:60] if d and d[3] else "?"))
    # recargar el indice con el fichero abierto: el dataset que faltaba aparece
    ind = json.load(open(INDICE, encoding="utf-8"))
    c.abrir(uri + "n", "select nueva from standard_test.recien_llegado")
    d1 = c.esperar_diag(uri + "n", 1)
    ind["items"]["dataset:standard_test.recien_llegado"] = {"kind": "Dataset", "paquete": "standard_test", "name": "recien_llegado",
                                                           "expone": [{"name": "nueva", "type": "String"}], "acceso": {}}
    f2 = os.path.join(tempfile.gettempdir(), "indice-recargado.json")
    json.dump(ind, open(f2, "w", encoding="utf-8"))
    n_antes = len(c.diags)
    t0 = time.perf_counter()
    c.notificar("ore/indice", {"fichero": f2})
    with c.cv:
        c.cv.wait_for(lambda: any(x[1] == uri + "n" for x in c.diags[n_antes:]), timeout=10)
    d2 = [x for x in c.diags[n_antes:] if x[1] == uri + "n"]
    fila("  recargar el indice (un dataset nuevo)", "%.0f ms" % ((d2[-1][0] - t0) * 1000 if d2 else -1),
         "antes: %s · despues: %s" % ((d1[3][0]["message"][:40] if d1 and d1[3] else "sin error"), ("sin error" if d2 and not d2[-1][3] else "sigue")))
    # un marco roto y un metodo que no existe
    c.crudo(b"Content-Length: 5\r\n\r\n{nop}")
    r, _, m = c.pedir("no/existe", {})
    fila("  un marco roto + un metodo que no existe", "✓ vivo" if c.vivo() else "✗ MUERTO", "")
    # 3000 peticiones: ¿crece la memoria?
    m0 = rss(c.p.pid)
    c.abrir(uri + "m", "select p. from standard_test.products p")
    for i in range(3000):
        c.completion(uri + "m", 0, 9)
    m1 = rss(c.p.pid)
    fila("  3000 completions", "RSS %s → %s MB" % (("%.0f" % m0) if m0 else "?", ("%.0f" % m1) if m1 else "?"), "vivo %s" % c.vivo())
    MEDIDO["P5 vivo al final"] = c.vivo()
    c.cerrar()


def main():
    print("%s · el servidor de SQL en el agente · %s" % ("EL MODULO DE VERDAD (ore.lsp_sql)" if REAL else "PROTOTIPO", INDICE))
    for nombre, f in (("P2", p2), ("P3", p3), ("P4", p4), ("P5", p5)):
        if nombre in SOLO:
            f()
    print()
    print("LO MEDIDO")
    for k, v in MEDIDO.items():
        fila("  " + k, str(v))


if __name__ == "__main__":
    main()
