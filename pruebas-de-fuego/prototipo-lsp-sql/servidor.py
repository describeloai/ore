"""PROTOTIPO DESECHABLE · el servidor de lenguaje de SQL, como viviria en el agente.

No es el de verdad: es para ver, antes de escribir el de verdad, que el
enfoque medido en `medida-el-lsp-de-sql.py` aguanta un editor, un tecleo, un
arbol grande y un canal de verdad. Un proceso Python por stdio (el marco de LSP:
`Content-Length`), con DuckDB dentro, como el agente del puesto.

  catalogo      el indice del arbol (`ore assets --json`): lo que se lee desde
                SQL (Dataset y View) con sus columnas y tipos; las Table de otra
                fuente con su View inducida.
  completion    el contexto del cursor sobre texto a medio escribir (tokenizador
                de DuckDB + el indice), sin analizar.
  diagnosticos  DuckDB con una tabla VACIA por nombre, creada BAJO DEMANDA (la
                que la sentencia nombra); `explain` liga sin ejecutar, UNA
                sentencia cada vez (DuckDB ejecuta las que siguen a la primera);
                la posicion es el `^` de DuckDB. Se esperan `DEMORA` ms tras el
                ultimo cambio. «end of input» AL FINAL del texto no se dice (se
                esta escribiendo) salvo con `--mostrar-fin`; en otro sitio si.
                Una sentencia de mas de `TOPE_BINDER` no se liga.

Lo que los pasos encontraron y esta arreglado aqui: `duckdb.tokenize` da
posiciones en BYTES (una ñ delante cortaba los nombres); `explain` sobre varias
sentencias ejecutaba las demas; crear todas las tablas al arrancar eran 7.9 s
y 385 MB con 5000 datasets; tokenizar un texto enorme en cada tecla, 1.9 s.
  hover         el tipo de una columna, lo que se sabe de un dataset.

    python servidor.py --indice assets.json [--mostrar-fin] [--demora 300]

`ore/indice` (notificacion propia): recarga el indice sin cerrar nada.
"""
import json
import re
import sys
import threading
import time

DEMORA = 0.3
MOSTRAR_FIN = False
# Las tablas vacias, bajo demanda (solo las que la consulta nombra): con 5000
# datasets, crearlas todas al arrancar eran 7.9 s y 385 MB (P4). `--todo-al-arrancar`
# vuelve a lo de antes, para comparar.
PEREZOSO = True
VENTANA = 20000  # caracteres alrededor del cursor en una sentencia enorme
TOPE_BINDER = 100000  # una sentencia mas larga no se liga (P5: 360 KB eran 6 s con el candado)


def a_caracteres(texto):
    """`duckdb.tokenize` da posiciones en BYTES de UTF-8 (medido: con una ñ o un
    emoji delante, los nombres se cortaban mal). Byte -> caracter."""
    if len(texto) == len(texto.encode("utf-8")):
        return None
    m = []
    for i, ch in enumerate(texto):
        m.extend([i] * len(ch.encode("utf-8")))
    m.append(len(texto))
    return m


def tokenizar(texto):
    import duckdb
    try:
        ts = duckdb.tokenize(texto)
    except Exception:
        return []
    m = a_caracteres(texto)
    return [((m[pos] if m else pos), tipo) for pos, tipo in ts]


def arg(n, d=None):
    return sys.argv[sys.argv.index(n) + 1] if n in sys.argv else d


# ═════════════════════════════════════════════════════════════════════════════
# El catalogo
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


class Catalogo:
    def __init__(self, indice):
        items = list(indice["items"].values())
        self.legibles = {}
        self.ajenas = {}
        for i in items:
            n = ("%s.%s" % (i["paquete"], i["name"]))
            if i["kind"] in ("Dataset", "View"):
                self.legibles[n.lower()] = i
            elif i["kind"] == "Table":
                self.ajenas[n.lower()] = i
        self.paquetes = sorted({n.split(".")[0] for n in list(self.legibles) + list(self.ajenas)})
        self.con = None
        self.candado = threading.Lock()

    def cols(self, n):
        i = self.legibles.get(n)
        return [(c["name"], c.get("type")) for c in (i or {}).get("expone", [])]

    def montar(self):
        import duckdb
        con = duckdb.connect()
        for p in sorted({i["paquete"] for i in self.legibles.values()}):
            con.execute('create schema if not exists "%s"' % p.replace('"', '""'))
        self.con = con
        self.hechas = set()
        if not PEREZOSO:
            self.asegurar(list(self.legibles))

    def asegurar(self, nombres):
        """Las tablas vacias de estos nombres, si faltan (con el candado tomado)."""
        for n in nombres:
            if n in self.hechas or n not in self.legibles:
                continue
            i = self.legibles[n]
            cols = ", ".join('"%s" %s' % (c["name"].replace('"', '""'), tipo_duckdb(c.get("type"))) for c in i.get("expone", []))
            self.con.execute('create table "%s"."%s" (%s)' % (i["paquete"].replace('"', '""'), i["name"].replace('"', '""'), cols or "x VARCHAR"))
            self.hechas.add(n)


# ═════════════════════════════════════════════════════════════════════════════
# El contexto del cursor
# ═════════════════════════════════════════════════════════════════════════════
PALABRAS_DE_TABLA = {"from", "join", "into", "update", "table", "describe", "summarize", "pivot", "unpivot"}
NO_ALIAS = {"where", "join", "on", "group", "order", "limit", "left", "right", "inner", "outer", "full", "cross",
            "select", "using", "qualify", "having", "union", "window", "natural", "as"}


def tokens(texto):
    ts = tokenizar(texto)
    out = []
    for i, (pos, tipo) in enumerate(ts):
        fin = ts[i + 1][0] if i + 1 < len(ts) else len(texto)
        out.append((pos, texto[pos:fin].strip(), str(tipo).split(".")[-1].lower()))
    return [t for t in out if t[1]]


def alias_de(ts, cat):
    al = {}
    txt = [t[1] for t in ts]
    for i in range(len(txt) - 2):
        n = ("%s.%s" % (txt[i], txt[i + 2])).lower()
        if txt[i + 1] == "." and n in cat.legibles:
            al[txt[i + 2].lower()] = n
            j = i + 3
            if j < len(txt) and txt[j].lower() == "as":
                j += 1
            if j < len(txt) and re.match(r"^[A-Za-z_]\w*$", txt[j]) and txt[j].lower() not in NO_ALIAS:
                al[txt[j].lower()] = n
    return al


def en_cadena_o_comentario(antes):
    sin_com = re.sub(r"--[^\n]*\n", "\n", antes)
    return sin_com.count("'") % 2 == 1 or re.search(r"--[^\n]*$", antes) is not None


def sentencias(texto):
    """[(inicio, fin)] de cada sentencia: se parte por los `;` que el tokenizador
    ve (los de una cadena o un comentario no cuentan)."""
    ts = tokenizar(texto)
    cortes = [pos for pos, tipo in ts if str(tipo).endswith("operator") and texto[pos] == ";"]
    out, ini = [], 0
    for c in cortes:
        out.append((ini, c))
        ini = c + 1
    out.append((ini, len(texto)))
    return out


def la_del_cursor(texto, cursor):
    # primero la ventana (no se tokeniza un texto enorme en cada tecla), luego
    # la sentencia dentro de ella
    v0 = max(0, cursor - VENTANA // 2)
    v1 = min(len(texto), cursor + VENTANA // 2)
    for ini, fin in sentencias(texto[v0:v1]):
        if ini <= cursor - v0 <= fin:
            return v0 + ini, v0 + fin
    return v0, v1


def completar(texto, cursor, cat):
    # solo la sentencia del cursor: sus alias, su FROM; y rapido en un texto enorme
    ini, fin = la_del_cursor(texto, cursor)
    texto, cursor = texto[ini:fin], cursor - ini
    antes = texto[:cursor]
    if en_cadena_o_comentario(antes):
        return []
    m = re.search(r"[A-Za-z_]\w*$", antes)
    base = antes[: m.start()] if m else antes
    ts = [t for t in tokens(base) if t[2] != "comment"]
    al = alias_de([t for t in tokens(texto) if t[2] != "comment"], cat)
    if ts and ts[-1][1] == ".":
        q = ts[-2][1].lower() if len(ts) > 1 else ""
        if q in cat.paquetes:
            return [(n.split(".", 1)[1], 7, cat.legibles[n].get("kind", "")) for n in sorted(cat.legibles) if n.startswith(q + ".")]
        if q in al:
            return [(c, 5, t or "") for c, t in cat.cols(al[q])]
        return []
    ultima = next((t[1].lower() for t in reversed(ts) if t[2] == "keyword"), "")
    if ultima in PALABRAS_DE_TABLA:
        return [(n, 7, cat.legibles[n].get("kind", "")) for n in sorted(cat.legibles)] + [(p, 9, "paquete") for p in cat.paquetes]
    usados = sorted(set(al.values()))
    vistos, out = set(), []
    for n in usados:
        for c, t in cat.cols(n):
            if c not in vistos:
                vistos.add(c)
                out.append((c, 5, "%s · %s" % (t or "?", n)))
    return out + [(a, 6, al[a]) for a in sorted(al) if a not in al.values()]


# ═════════════════════════════════════════════════════════════════════════════
# Los diagnosticos
# ═════════════════════════════════════════════════════════════════════════════
PREFIJO = "explain "


def posicion(texto, mensaje):
    """(linea, columna) 0-based del error dentro de `texto`, o None. El `^` de
    DuckDB manda; sin el, «end of input» es el final."""
    ls = mensaje.split("\n")
    for k, l in enumerate(ls):
        g = re.match(r"^LINE (\d+): (.*)$", l)
        if g and k + 1 < len(ls) and "^" in ls[k + 1]:
            n = int(g.group(1))
            trozo = g.group(2)
            car = ls[k + 1].index("^") - len("LINE %d: " % n)
            linea = (PREFIJO + texto).split("\n")[n - 1] if n - 1 < len((PREFIJO + texto).split("\n")) else ""
            if trozo.startswith("..."):
                i = linea.find(trozo[3:].rstrip(".")[:20])
                col = i + car - 3 if i >= 0 else 0
            else:
                col = car
            if n == 1:
                col -= len(PREFIJO)
            return n - 1, max(0, col)
    if "at end of input" in mensaje:
        return texto.count("\n"), len(texto) - (texto.rfind("\n") + 1)
    return None


def nombres_de(texto, cat):
    ts = tokens(texto)
    return {("%s.%s" % (ts[i][1], ts[i + 2][1])).lower() for i in range(len(ts) - 2) if ts[i + 1][1] == "."}


def al_final(texto, linea, col):
    """¿Esta (linea, col) al final del texto (sin contar blancos)? Entonces se
    esta escribiendo: un «end of input» ahi no es un error todavia."""
    ls = texto.rstrip().split("\n")
    return linea >= len(ls) - 1 and col >= len(ls[-1]) if ls else True


def palabra_en(texto, linea, col):
    l = texto.split("\n")[linea] if linea < len(texto.split("\n")) else ""
    m = re.match(r"[\w.]+", l[col:])
    return col + (len(m.group(0)) if m else 1)


def diagnosticar(texto, cat):
    out = []
    # una Table de otra fuente: sql() no la lee
    ts = tokens(texto)
    for i in range(len(ts) - 2):
        n = ("%s.%s" % (ts[i][1], ts[i + 2][1])).lower()
        if ts[i + 1][1] == "." and n in cat.ajenas and n not in cat.legibles:
            ind = ((cat.ajenas[n].get("detalle") or {}).get("vistaInducida") or "").replace("view:", "")
            ini, fin = ts[i][0], ts[i + 2][0] + len(ts[i + 2][1])
            l0, c0 = texto[:ini].count("\n"), ini - (texto.rfind("\n", 0, ini) + 1)
            out.append({"range": {"start": {"line": l0, "character": c0}, "end": {"line": l0, "character": c0 + fin - ini}},
                        "severity": 1, "source": "ore",
                        "message": "`%s.%s` es una Table de otra fuente: sql() no la lee%s" % (
                            cat.ajenas[n]["paquete"], cat.ajenas[n]["name"], (", lee su View `%s`" % ind) if ind else "")})
    if not texto.strip():
        return out
    import difflib
    # una a una: `explain` cubre UNA sentencia, y DuckDB EJECUTA las que siguen
    for ini, fin in sentencias(texto):
        sentencia = texto[ini:fin]
        if not sentencia.strip():
            continue
        if len(sentencia) > TOPE_BINDER:
            l0 = texto[:ini].count("\n")
            out.append({"range": {"start": {"line": l0, "character": 0}, "end": {"line": l0, "character": 1}},
                        "severity": 3, "source": "ore",
                        "message": "sentencia de %d KB: no se comprueba mientras se escribe" % (len(sentencia) // 1024)})
            continue
        with cat.candado:
            try:
                cat.asegurar(nombres_de(sentencia, cat))
                cat.con.execute(PREFIJO + sentencia)
                continue
            except Exception as e:
                m = str(e)
        p = posicion(sentencia, m) or (0, 0)
        if "at end of input" in m and al_final(sentencia, *p) and not MOSTRAR_FIN:
            continue
        primera = m.split("\n")[0]
        sug = re.search(r"Candidate bindings: (.*)", m)
        if sug:
            primera += " · ¿" + sug.group(1).split(",")[0].strip() + "?"
        falta = re.search(r"Table with name (\S+) does not exist", m)
        if falta:
            # con las tablas bajo demanda DuckDB no sabe sugerir: se sugiere del indice
            parecidos = difflib.get_close_matches(falta.group(1).lower(), [n.split(".", 1)[1] for n in cat.legibles], n=1)
            if parecidos:
                primera = primera.split("!")[0] + "! · ¿" + parecidos[0] + "?"
        # la posicion, en el texto entero
        l0 = texto[:ini].count("\n")
        c0 = ini - (texto.rfind("\n", 0, ini) + 1)
        linea, col = l0 + p[0], (c0 + p[1]) if p[0] == 0 else p[1]
        out.append({"range": {"start": {"line": linea, "character": col},
                              "end": {"line": linea, "character": col + (palabra_en(sentencia, p[0], p[1]) - p[1])}},
                    "severity": 1, "source": "duckdb", "message": primera})
    return out


# ═════════════════════════════════════════════════════════════════════════════
# El hover
# ═════════════════════════════════════════════════════════════════════════════
def hover(texto, linea, col, cat):
    ls = texto.split("\n")
    l = ls[linea] if linea < len(ls) else ""
    a = col
    while a > 0 and re.match(r"[\w.]", l[a - 1]):
        a -= 1
    b = col
    while b < len(l) and re.match(r"[\w.]", l[b]):
        b += 1
    palabra = l[a:b].lower()
    if palabra in cat.legibles:
        i = cat.legibles[palabra]
        p = i.get("puntero") or {}
        return "**%s** · %s · dueño %s\n\n%d columnas · %s filas · %s\n\nconducto: %s" % (
            palabra, i["kind"], i.get("owner") or "?", len(i.get("expone", [])), p.get("filas", "?"), p.get("estado", "sin puntero"),
            ", ".join("%s %s" % kv for kv in ((i.get("acceso") or {}).get("conductos") or {}).items()) or "-")
    if "." in palabra:
        q, c = palabra.rsplit(".", 1)
        al = alias_de(tokens(texto), cat)
        if q in al:
            t = dict((x.lower(), y) for x, y in cat.cols(al[q])).get(c)
            if t is not None or c in [x.lower() for x, _ in cat.cols(al[q])]:
                return "`%s` · %s · de %s" % (c, t or "sin tipo", al[q])
        palabra = c
    al = alias_de(tokens(texto), cat)
    for n in sorted(set(al.values())):
        for x, t in cat.cols(n):
            if x.lower() == palabra:
                return "`%s` · %s · de %s" % (x, t or "sin tipo", n)
    return None


# ═════════════════════════════════════════════════════════════════════════════
# El protocolo
# ═════════════════════════════════════════════════════════════════════════════
class Servidor:
    def __init__(self, cat):
        self.cat = cat
        self.docs = {}
        self.relojes = {}
        self.salida = threading.Lock()
        self.vivo = True

    def mandar(self, m):
        b = json.dumps(m).encode("utf-8")
        with self.salida:
            sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(b) + b)
            sys.stdout.buffer.flush()

    def programar(self, uri):
        r = self.relojes.pop(uri, None)
        if r:
            r.cancel()
        t = threading.Timer(DEMORA, self.publicar, args=(uri, self.docs.get(uri, ("", 0))[1]))
        t.daemon = True
        self.relojes[uri] = t
        t.start()

    def publicar(self, uri, version):
        texto, v = self.docs.get(uri, ("", -1))
        if v != version:
            return  # ya hay uno mas nuevo en camino
        try:
            ds = diagnosticar(texto, self.cat)
        except Exception as e:
            ds = [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                   "severity": 2, "source": "ore", "message": "el servidor de SQL fallo: %s" % e}]
        if self.docs.get(uri, ("", -1))[1] == version:
            self.mandar({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                         "params": {"uri": uri, "version": version, "diagnostics": ds}})

    def atender(self, m):
        metodo, id_, p = m.get("method"), m.get("id"), m.get("params") or {}
        try:
            r = self._atender(metodo, p)
        except Exception as e:
            if id_ is not None:
                self.mandar({"jsonrpc": "2.0", "id": id_, "error": {"code": -32603, "message": str(e)}})
            return
        if id_ is not None and metodo is not None:
            self.mandar({"jsonrpc": "2.0", "id": id_, "result": r})

    def _atender(self, metodo, p):
        if metodo == "initialize":
            return {"capabilities": {"textDocumentSync": 1, "hoverProvider": True,
                                     "completionProvider": {"triggerCharacters": ["."]}},
                    "serverInfo": {"name": "ore-sql (prototipo)"}}
        if metodo == "textDocument/didOpen":
            d = p["textDocument"]
            self.docs[d["uri"]] = (d["text"], d.get("version", 1))
            self.programar(d["uri"])
        elif metodo == "textDocument/didChange":
            d = p["textDocument"]
            self.docs[d["uri"]] = (p["contentChanges"][-1]["text"], d.get("version", 0))
            self.programar(d["uri"])
        elif metodo == "textDocument/didClose":
            self.docs.pop(p["textDocument"]["uri"], None)
        elif metodo == "textDocument/completion":
            texto = self.docs.get(p["textDocument"]["uri"], ("", 0))[0]
            ls = texto.split("\n")
            pos = p["position"]
            cur = sum(len(x) + 1 for x in ls[: pos["line"]]) + pos["character"]
            return {"isIncomplete": False,
                    "items": [{"label": l, "kind": k, "detail": d} for l, k, d in completar(texto, cur, self.cat)]}
        elif metodo == "textDocument/hover":
            texto = self.docs.get(p["textDocument"]["uri"], ("", 0))[0]
            h = hover(texto, p["position"]["line"], p["position"]["character"], self.cat)
            return {"contents": {"kind": "markdown", "value": h}} if h else None
        elif metodo == "ore/indice":
            cat = Catalogo(json.load(open(p["fichero"], encoding="utf-8")))
            cat.montar()
            self.cat = cat
            for uri in list(self.docs):
                self.programar(uri)
        elif metodo == "shutdown":
            return None
        elif metodo == "exit":
            self.vivo = False
        return None

    def bucle(self):
        f = sys.stdin.buffer
        while self.vivo:
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
            if largo is None:
                continue
            try:
                m = json.loads(f.read(largo).decode("utf-8"))
            except Exception:
                continue
            self.atender(m)


def main():
    global DEMORA, MOSTRAR_FIN
    DEMORA = float(arg("--demora", "300")) / 1000
    MOSTRAR_FIN = "--mostrar-fin" in sys.argv
    global PEREZOSO
    PEREZOSO = "--todo-al-arrancar" not in sys.argv
    cat = Catalogo(json.load(open(arg("--indice"), encoding="utf-8")))
    cat.montar()
    Servidor(cat).bucle()


if __name__ == "__main__":
    main()
