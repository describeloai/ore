"""EL SERVIDOR DE LENGUAJE DE SQL, dentro del agente del puesto.

Ningun servidor de SQL que existe ve el error de verdad de una consulta del
arbol —una columna mal escrita— porque el esquema no esta en ninguna base que
puedan abrir (`medida-el-sql-del-arbol.py` §4). Este si: el esquema es el
INDICE del arbol (`GET /assets`) y el comprobador es el DuckDB que ya vive en el
puesto, con una tabla VACIA por dataset (`medida-el-lsp-de-sql.py`; el prototipo
de seis pasos en `pruebas-de-fuego/prototipo-lsp-sql/`).

Corre DENTRO del proceso del agente, no como otro proceso: no hay nada que
arrancar ni que pagar (40 MB medidos frente a los 210 de un pyright), y abrir
un `.sql` no enciende el servidor de Python. La correa del agente le da lo que
es de SQL (`es_sql`) y le pasa una funcion para mandar lo que contesta.

  completion    el contexto del cursor sobre texto a medio escribir: el
                tokenizador de DuckDB y el indice, sin analizar. Tras FROM los
                nombres enteros (`base.schema.nombre`, 0038), tras `base.` sus
                schemas, tras `base.schema.` sus nombres, tras `alias.` sus
                columnas, y en lo demas las columnas de lo que la sentencia lee.
  diagnosticos  `explain` en DuckDB, UNA sentencia cada vez, con las tablas
                vacias de lo que nombra (creadas bajo demanda). La posicion es
                el `^` de DuckDB. `DEMORA` tras el ultimo cambio. Una Table de
                otra fuente se dice, con la View que la lee; y un nombre de dos
                partes, con el aviso `ORE-SQL-2P` (0038: se lee en `default`).
  hover         el tipo de una columna; de un dataset, dueño, filas, estado y
                conducto.

Lo que el prototipo encontro, y aqui esta hecho: `duckdb.tokenize` da
posiciones en BYTES (con una ñ delante se cortaban los nombres); `explain` de
varias sentencias EJECUTA las que siguen a la primera; crear todas las tablas
al arrancar eran 7.4 s y 385 MB con 5000 datasets; tokenizar un texto enorme en
cada tecla, 1.9 s; ligar una sentencia de 360 KB, 6 s. Y «end of input» al
final del texto no es un error: se esta escribiendo.

Lo que `el-editor-sql-a-fondo.py` (P4-P6, todo de verdad) encontro, y aqui esta
hecho: cargar el indice DENTRO del hilo de la correa bloqueaba el canal entero
(una completion de Python, 8 s: se agoto) → el servidor trabaja en SU hilo y el
indice se refresca en segundo plano; tras FROM con 5000 nombres, 898 KB por
tecla → filtro y tope; un agente reiniciado no sabia de los ficheros abiertos →
`DESCONOCIDO` y el cliente los reabre.

Para probarlo sin agente, por stdio (el marco de LSP):

    python -m ore.lsp_sql --indice assets.json
"""
import difflib
import json
import re
import sys
import threading
import time

DEMORA = 0.3
VENTANA = 20000
TOPE_BINDER = 100000
# Lo que dura el indice antes de volver a pedirlo (en segundo plano).
FRESCO_S = 30
# Cuantos nombres se ofrecen como mucho: con 5000 datasets, tras FROM eran 898 KB
# por tecla (medido, `el-editor-sql-a-fondo.py` P4). Se filtra por lo que se
# esta escribiendo y, si no cabe, la lista es INCOMPLETA: el editor vuelve a pedir.
TOPE_ITEMS = 200
# El error con el que se dice «ese documento no lo tengo» (un agente que se
# reinicio): el cliente lo reabre con su texto y repite (LSP RequestFailed).
DESCONOCIDO = -32803
PREFIJO = "explain "
PALABRAS_DE_TABLA = {"from", "join", "into", "update", "table", "describe", "summarize", "pivot", "unpivot"}
NO_ALIAS = {"where", "join", "on", "group", "order", "limit", "left", "right", "inner", "outer", "full", "cross",
            "select", "using", "qualify", "having", "union", "window", "natural", "as"}


def es_sql(m):
    """¿Es de SQL este mensaje del editor? El id `sql:n` que pone su cliente, una
    uri `.sql`, o el `initialize` que lo dice."""
    i = m.get("id")
    if isinstance(i, str) and i.startswith("sql:"):
        return True
    p = m.get("params") or {}
    if str((p.get("textDocument") or {}).get("uri", "")).endswith(".sql"):
        return True
    return m.get("method") == "initialize" and (p.get("initializationOptions") or {}).get("lenguaje") == "sql"


# ═════════════════════════════════════════════════════════════════════════════
# El catalogo: el indice, y las tablas vacias
# ═════════════════════════════════════════════════════════════════════════════
def tipo_duckdb(t):
    """El fisico de un tipo de OOS (0032 §1) en DuckDB."""
    t = t or ""
    if t.startswith("list<"):
        return tipo_duckdb(t[5:-1]) + "[]"
    m = re.match(r"^(Money|Quantity)<[^,]+,\s*(\d+)>$", t)
    if m:
        return "DECIMAL(38, %s)" % min(int(m.group(2)), 18)
    # `Decimal<p, s>` (0032 T4): la precisión y la escala del tipo, no las de
    # por defecto. Antes caía en VARCHAR y DuckDB lo trataba como texto.
    m = re.match(r"^Decimal<\s*(\d+)\s*,\s*(\d+)\s*>$", t)
    if m:
        return "DECIMAL(%s, %s)" % (m.group(1), m.group(2))
    return {"Integer": "BIGINT", "Decimal": "DECIMAL(38, 18)", "Float": "DOUBLE", "Boolean": "BOOLEAN",
            "Date": "DATE", "Time": "TIME", "DateTime": "TIMESTAMP", "DateTimeTz": "TIMESTAMPTZ"}.get(t, "VARCHAR")


def _q(n):
    return '"%s"' % str(n).replace('"', '""')


# 0038: el nombre de lo que se lee es `base.schema.nombre`; dos partes es
# `default`, y se avisa (el mismo codigo que ore-serve y `ore sql`).
DEFAULT = "default"
DOS_PARTES = "ORE-SQL-2P"


def corto(b, s_, n):
    """La forma corta, en minusculas: la clave del indice (`b.n` en `default`)."""
    return ("%s.%s" % (b, n) if s_.lower() == DEFAULT else "%s.%s.%s" % (b, s_, n)).lower()


def nombres(txt):
    """[(i, j, clave, dos_partes)] de cada `a.b` y `a.b.c` de una lista de trozos
    (`j`, el indice del ultimo), que no sea parte de uno mas largo. Las comillas
    de un identificador no cuentan."""
    t = [x.strip('"') if x != "." else x for x in txt]
    punto = lambda k: 0 <= k < len(t) and t[k] == "."
    out, i = [], 0
    while i + 2 < len(t):
        if punto(i + 1) and not punto(i) and not punto(i + 2) and not punto(i - 1):
            if punto(i + 3) and i + 4 < len(t) and not punto(i + 4) and not punto(i + 5):
                out.append((i, i + 4, corto(t[i], t[i + 2], t[i + 4]), False))
                i += 5
                continue
            if not punto(i + 3):
                out.append((i, i + 2, corto(t[i], DEFAULT, t[i + 2]), True))
                i += 3
                continue
        i += 1
    return out


def clave_de(palabra):
    """`a.b` o `a.b.c` escrito → su clave."""
    p = palabra.split(".")
    if len(p) == 3:
        return corto(*p)
    if len(p) == 2:
        return corto(p[0], DEFAULT, p[1])
    return palabra.lower()


class Catalogo:
    """Lo que se lee desde SQL (Dataset y View) y las Table de otra fuente."""

    def __init__(self, indice):
        self.legibles, self.ajenas = {}, {}
        self.schemas = {}
        for i in (indice.get("items") or {}).values():
            s_ = i.get("schema") or DEFAULT
            n = corto(i.get("paquete"), s_, i.get("name"))
            i = dict(i, schema=s_, completo="%s.%s.%s" % (i.get("paquete"), s_, i.get("name")))
            if i.get("kind") in ("Dataset", "View"):
                self.legibles[n] = i
            elif i.get("kind") == "Table":
                self.ajenas[n] = i
            else:
                continue
            self.schemas.setdefault(str(i.get("paquete")).lower(), set()).add(s_)
        self.paquetes = sorted(self.schemas)
        self.candado = threading.Lock()
        self.hechas = set()
        import duckdb
        # Una conexion PROPIA: nada que ver con la de las celdas (`ore._duckdb()`).
        # Un catalogo por base y un schema por schema (0038), como en `sql()`.
        self.con = duckdb.connect()
        for i in self.legibles.values():
            self.con.execute("attach if not exists ':memory:' as %s" % _q(i["paquete"]))
            self.con.execute("create schema if not exists %s.%s" % (_q(i["paquete"]), _q(i["schema"])))

    def cols(self, n):
        return [(c.get("name"), c.get("type")) for c in (self.legibles.get(n) or {}).get("expone", [])]

    def asegurar(self, nombres):
        """Las tablas vacias de estos nombres, si faltan. Con el candado tomado."""
        for n in nombres:
            if n in self.hechas or n not in self.legibles:
                continue
            i = self.legibles[n]
            cols = ", ".join("%s %s" % (_q(c["name"]), tipo_duckdb(c.get("type"))) for c in i.get("expone", []))
            b, s_, t = _q(i["paquete"]), _q(i["schema"]), _q(i["name"])
            self.con.execute("create table %s.%s.%s (%s)" % (b, s_, t, cols or "x VARCHAR"))
            # lo de `default`, tambien en `main`: donde DuckDB busca dos partes
            if i["schema"] == DEFAULT:
                self.con.execute("create or replace view %s.main.%s as select * from %s.%s.%s" % (b, t, b, s_, t))
            self.hechas.add(n)

    def cerrar(self):
        # ⛔ Con el candado: el refresco cierra el catálogo viejo desde SU hilo,
        #   y cerrar la conexión mientras otro hilo hace un `explain` con ella
        #   es memoria liberada en DuckDB —un «Segmentation fault» del agente
        #   entero (visto en el-puesto 3d)—. Se espera a que termine.
        with self.candado:
            try:
                self.con.close()
            except Exception:
                pass


# ═════════════════════════════════════════════════════════════════════════════
# El texto: tokens, sentencias
# ═════════════════════════════════════════════════════════════════════════════
# ⛔ `duckdb.tokenize` es del módulo: usa la conexión por defecto, que no es de
#   varios hilos a la vez. Y aquí los hay: cada documento abierto se diagnostica
#   en el hilo de su temporizador. Medido en el-puesto 3d: con cuatro abiertos a
#   la vez, uno se quedaba colgado —ni diagnóstico ni error—. De uno en uno.
_TOKENIZADOR = threading.Lock()


def tokenizar(texto):
    """`duckdb.tokenize` con las posiciones en CARACTERES (las da en bytes)."""
    import duckdb
    try:
        with _TOKENIZADOR:
            ts = duckdb.tokenize(texto)
    except Exception:
        return []
    if len(texto) != len(texto.encode("utf-8")):
        m = []
        for i, ch in enumerate(texto):
            m.extend([i] * len(ch.encode("utf-8")))
        m.append(len(texto))
        ts = [(m[min(pos, len(m) - 1)], tipo) for pos, tipo in ts]
    return ts


def tokens(texto):
    ts = tokenizar(texto)
    out = []
    for i, (pos, tipo) in enumerate(ts):
        fin = ts[i + 1][0] if i + 1 < len(ts) else len(texto)
        out.append((pos, texto[pos:fin].strip(), str(tipo).split(".")[-1].lower()))
    return [t for t in out if t[1]]


def sentencias(texto):
    """[(inicio, fin)] de cada sentencia: los `;` que el tokenizador ve (los de una
    cadena o un comentario no cuentan)."""
    cortes = [pos for pos, tipo in tokenizar(texto) if str(tipo).endswith("operator") and texto[pos:pos + 1] == ";"]
    out, ini = [], 0
    for c in cortes:
        out.append((ini, c))
        ini = c + 1
    out.append((ini, len(texto)))
    return out


def la_del_cursor(texto, cursor):
    v0, v1 = max(0, cursor - VENTANA // 2), min(len(texto), cursor + VENTANA // 2)
    for ini, fin in sentencias(texto[v0:v1]):
        if ini <= cursor - v0 <= fin:
            return v0 + ini, v0 + fin
    return v0, v1


def alias_de(ts, cat):
    """alias -> nombre, y el nombre corto -> nombre (`<p>.<n> [as] <alias>`)."""
    al = {}
    txt = [t[1] for t in ts]
    for i, j, n, _ in nombres(txt):
        if n in cat.legibles:
            al[txt[j].strip('"').lower()] = n
            j = j + 1
            if j < len(txt) and txt[j].lower() == "as":
                j += 1
            if j < len(txt) and re.match(r"^[A-Za-z_]\w*$", txt[j]) and txt[j].lower() not in NO_ALIAS:
                al[txt[j].lower()] = n
    return al


def en_cadena_o_comentario(antes):
    # El tokenizador no ve como cadena una sin cerrar: se cuentan las comillas.
    sin_com = re.sub(r"--[^\n]*\n", "\n", antes)
    return sin_com.count("'") % 2 == 1 or re.search(r"--[^\n]*$", antes) is not None


def desplazamiento(texto, linea, col):
    ls = texto.split("\n")
    return sum(len(x) + 1 for x in ls[:linea]) + min(col, len(ls[linea]) if linea < len(ls) else 0)


# ═════════════════════════════════════════════════════════════════════════════
# Lo que el editor pide
# ═════════════════════════════════════════════════════════════════════════════
def completar_lista(texto, cursor, cat):
    """(items, incompleta): lo de `completar`, filtrado por lo que se esta
    escribiendo y con `TOPE_ITEMS` como mucho."""
    items = completar(texto, cursor, cat)
    m = re.search(r"[\w.]*$", texto[:cursor])
    escrito = (m.group(0) if m else "").lower()
    palabra = escrito.rsplit(".", 1)[-1]
    if palabra:
        # lo escrito puede ser el nombre entero (`standard_test.pro`) o su ultimo trozo
        items = [x for x in items if x[0].lower().startswith(escrito) or x[0].lower().rsplit(".", 1)[-1].startswith(palabra)
                 or palabra in x[0].lower()]
    return items[:TOPE_ITEMS], (len(items) > TOPE_ITEMS or bool(palabra))


def completar(texto, cursor, cat):
    """[(etiqueta, kind de LSP, detalle)] en `cursor`."""
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
        q = ts[-2][1].strip('"').lower() if len(ts) > 1 else ""
        # `base.schema.`: los nombres de ese schema
        if len(ts) > 3 and ts[-3][1] == ".":
            b = ts[-4][1].strip('"').lower()
            if b in cat.paquetes:
                return [(i["name"], 7, i.get("kind", "")) for n, i in sorted(cat.legibles.items())
                        if str(i["paquete"]).lower() == b and i["schema"].lower() == q]
        # `base.`: sus schemas (0038: el nombre es de tres partes)
        if q in cat.paquetes:
            return [(x, 9, "schema") for x in sorted(cat.schemas.get(q, ()))]
        if q in al:
            return [(c, 5, t or "") for c, t in cat.cols(al[q])]
        return []
    ultima = next((t[1].lower() for t in reversed(ts) if t[2] == "keyword"), "")
    if ultima in PALABRAS_DE_TABLA:
        return ([(cat.legibles[n]["completo"], 7, cat.legibles[n].get("kind", "")) for n in sorted(cat.legibles)]
                + [(p, 9, "base") for p in cat.paquetes])
    vistos, out = set(), []
    for n in sorted(set(al.values())):
        for c, t in cat.cols(n):
            if c not in vistos:
                vistos.add(c)
                out.append((c, 5, "%s · %s" % (t or "sin tipo", n)))
    return out + [(a, 6, al[a]) for a in sorted(al) if a not in al.values()]


def _posicion(texto, mensaje):
    """(linea, columna) 0-based del error dentro de `texto`: el `^` de DuckDB (su
    linea 1 lleva el `explain `), o el final si es «end of input»."""
    ls = mensaje.split("\n")
    for k, l in enumerate(ls):
        g = re.match(r"^LINE (\d+): (.*)$", l)
        if g and k + 1 < len(ls) and "^" in ls[k + 1]:
            n, trozo = int(g.group(1)), g.group(2)
            car = ls[k + 1].index("^") - len("LINE %d: " % n)
            lineas = (PREFIJO + texto).split("\n")
            linea = lineas[n - 1] if n - 1 < len(lineas) else ""
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


def _al_final(texto, linea, col):
    ls = texto.rstrip().split("\n")
    return linea >= len(ls) - 1 and col >= len(ls[-1])


def _rango(texto, ini, fin):
    l0, c0 = texto[:ini].count("\n"), ini - (texto.rfind("\n", 0, ini) + 1)
    l1, c1 = texto[:fin].count("\n"), fin - (texto.rfind("\n", 0, fin) + 1)
    return {"start": {"line": l0, "character": c0}, "end": {"line": l1, "character": c1}}


def lo_que_duckdb_entiende(s):
    """De una sentencia, lo que DuckDB puede comprobar: `(desde, texto)`, o `None`.

    El guion (0039) y la vista (ADR 0040 paso 5) tienen frases que DuckDB no
    conoce —`create dataset`, `create schema b.s`, `create standard database`,
    `create view … (col comment '…') with schema evolution as`, `drop view`—, y
    `explain` las marcaba como error aunque corren. Lo suyo lo dice ore-serve al
    correrlas; aquí se comprueba sólo lo que es de DuckDB: la consulta de detrás
    de `as` de un `create view` o un `create dataset … as`, en su sitio."""
    ts = tokens(s)
    w = [t[1].lower() for t in ts]
    if not w:
        return 0, s
    if w[0] == "drop" and w[1:2] == ["view"]:
        return None
    if w[0] != "create":
        return 0, s
    k = 2 if w[1:2] in (["standard"], ["foreign"]) else 1
    if w[k:k + 1] == ["database"]:
        return None
    if w[1:2] == ["schema"]:
        # `create schema b.s` es del árbol; `create schema tmp`, de DuckDB
        return None if "." in w[2:7] else (0, s)
    j = 1
    if w[j:j + 2] == ["or", "replace"]:
        j += 2
    if w[j:j + 1] in (["temp"], ["temporary"]):
        return 0, s
    if w[j:j + 1] not in (["view"], ["dataset"]):
        return 0, s
    hondo = 0
    for i in range(j + 1, len(ts)):
        if w[i] == "(":
            hondo += 1
        elif w[i] == ")":
            hondo -= 1
        elif w[i] == "as" and hondo == 0:
            if i + 1 < len(ts):
                return ts[i + 1][0], s[ts[i + 1][0]:]
            return None  # se está escribiendo
    # `create dataset b.s.d (col tipo, …)`, o una vista a medio escribir
    return None


def diagnosticar(texto, cat):
    out = []
    ts = tokens(texto)
    avisados = set()
    for i, j, n, dos in nombres([t[1] for t in ts]):
        rango = _rango(texto, ts[i][0], ts[j][0] + len(ts[j][1]))
        # una Table de otra fuente: sql() no la lee; se dice cual es su View
        if n in cat.ajenas and n not in cat.legibles:
            a = cat.ajenas[n]
            ind = next((r["ref"].replace("view:", "") for r in (a.get("relaciones") or [])
                        if r.get("tipo") == "produce" and r.get("ref", "").startswith("view:")), "")
            out.append({"range": rango, "severity": 1, "source": "ore",
                        "message": "`%s` es una Table de otra fuente: sql() no la lee%s" % (
                            a["completo"], (", lee su View `%s`" % ind) if ind else "")})
        # 0038: dos partes se leen en `default`, y se dice (una vez por nombre)
        if dos and (n in cat.legibles or n in cat.ajenas) and n not in avisados:
            avisados.add(n)
            i_ = cat.legibles.get(n) or cat.ajenas[n]
            out.append({"range": rango, "severity": 2, "source": "ore", "code": DOS_PARTES,
                        "message": "`%s.%s` tiene dos partes: se lee como `%s` · escribe las tres, `base.schema.nombre`" % (
                            i_["paquete"], i_["name"], i_["completo"])})
    if not texto.strip():
        return out
    for ini, fin in sentencias(texto):
        s = texto[ini:fin]
        if not s.strip():
            continue
        if len(s) > TOPE_BINDER:
            out.append({"range": _rango(texto, ini, ini), "severity": 3, "source": "ore",
                        "message": "sentencia de %d KB: no se comprueba mientras se escribe" % (len(s) // 1024)})
            continue
        parte = lo_que_duckdb_entiende(s)
        if parte is None:
            continue
        # lo de DuckDB, y desde dónde de la sentencia empieza
        d, s = parte
        ini += d
        with cat.candado:
            try:
                ts_s = tokens(s)
                cat.asegurar({n for _, _, n, _ in nombres([t[1] for t in ts_s])})
                # ⛔ UNA sentencia: con varias, DuckDB EJECUTA las que siguen.
                cat.con.execute(PREFIJO + s)
                continue
            except Exception as e:
                m = str(e)
        p = _posicion(s, m) or (0, 0)
        if "at end of input" in m and _al_final(s, *p):
            continue  # se esta escribiendo
        msg = m.split("\n")[0]
        sug = re.search(r"Candidate bindings: (.*)", m)
        if sug:
            msg += " · ¿%s?" % sug.group(1).split(",")[0].strip()
        falta = re.search(r"Table with name (\S+) does not exist", m)
        if falta:
            # con las tablas bajo demanda DuckDB no sabe sugerir: el indice si
            # (y se sugiere por su nombre ENTERO: `ventas.clientes` es `ventas.espana.clientes`)
            completos = {cat.legibles[n]["name"].lower(): cat.legibles[n]["completo"] for n in sorted(cat.legibles)}
            cerca = difflib.get_close_matches(falta.group(1).lower(), list(completos), n=1)
            if cerca:
                msg = msg.split("!")[0] + "! · ¿%s?" % completos[cerca[0]]
        a = ini + desplazamiento(s, *p)
        largo = re.match(r"[\w.]*", texto[a:]).end() or 1
        out.append({"range": _rango(texto, a, a + largo), "severity": 1, "source": "duckdb", "message": msg})
    return out


def explicar(texto, linea, col, cat):
    """El hover: markdown, o None."""
    ls = texto.split("\n")
    l = ls[linea] if linea < len(ls) else ""
    a = col
    while a > 0 and re.match(r"[\w.]", l[a - 1]):
        a -= 1
    b = col
    while b < len(l) and re.match(r"[\w.]", l[b]):
        b += 1
    palabra = l[a:b].lower().strip(".")
    if clave_de(palabra) in cat.legibles and palabra.count(".") in (1, 2):
        i = cat.legibles[clave_de(palabra)]
        p = i.get("puntero") or {}
        conductos = ", ".join("%s %s" % kv for kv in ((i.get("acceso") or {}).get("conductos") or {}).items())
        clas = ", ".join("%s:%s" % kv for kv in ((i.get("acceso") or {}).get("clasificacion") or {}).items())
        partes = ["**%s** · %s · dueño %s" % (i["completo"], i["kind"], i.get("owner") or "?"),
                  "%d columnas · %s filas · %s" % (len(i.get("expone", [])), p.get("filas", "?"), p.get("estado", "sin puntero"))]
        if i.get("description"):
            partes.insert(1, i["description"])
        if clas:
            partes.append("clasificación: " + clas)
        if conductos:
            partes.append("conductos: " + conductos)
        return "\n\n".join(partes)
    ini, fin = la_del_cursor(texto, desplazamiento(texto, linea, col))
    al = alias_de(tokens(texto[ini:fin]), cat)
    if "." in palabra:
        q, palabra = palabra.rsplit(".", 1)
        candidatos = [al[q]] if q in al else []
    else:
        candidatos = sorted(set(al.values()))
    for n in candidatos:
        for c, t in cat.cols(n):
            if (c or "").lower() == palabra:
                return "`%s` · %s · de %s" % (c, t or "sin tipo", n)
    return None


# ═════════════════════════════════════════════════════════════════════════════
# El servidor
# ═════════════════════════════════════════════════════════════════════════════
class Servidor:
    """Atiende mensajes de LSP ya analizados; contesta por `mandar(str)`.

    `cargar()` devuelve el indice del arbol (lo de `GET /assets`). Se pide al
    primer mensaje y otra vez al abrir un fichero si tiene mas de `FRESCO_S`.
    """

    def __init__(self, cargar, mandar, log=lambda *_: None):
        self.cargar, self._mandar, self.log = cargar, mandar, log
        self.cat, self.cuando = None, 0.0
        self.docs = {}
        self.relojes = {}
        self.candado = threading.Lock()
        self.cargando = threading.Lock()
        self.primero = threading.Event()
        # ⛔ SU hilo: lo que la correa da se encola y la correa sigue (medido: el
        #   indice cargandose en el hilo de la correa dejaba a pyright sin canal).
        import queue
        self.cola = queue.Queue()
        threading.Thread(target=self._trabajar, daemon=True).start()

    def _trabajar(self):
        while True:
            m = self.cola.get()
            try:
                self._atender_ya(m)
            except Exception as e:  # noqa: BLE001 — el hilo no muere por un mensaje
                self.log("servidor de SQL: %s" % e)

    def mandar(self, m):
        self._mandar(json.dumps(m))

    def _recargar(self):
        """Pedir el indice y montar el catalogo; uno a la vez."""
        if not self.cargando.acquire(blocking=False):
            return
        try:
            try:
                nuevo = Catalogo(self.cargar())
            except Exception as e:
                self.log("servidor de SQL: no pude leer el indice (%s)" % e)
                nuevo = Catalogo({}) if self.cat is None else None
            if nuevo is not None:
                viejo, self.cat, self.cuando = self.cat, nuevo, time.time()
                if viejo:
                    viejo.cerrar()
            else:
                self.cuando = time.time()  # no insistir en cada tecla
        finally:
            self.cargando.release()
            self.primero.set()

    def catalogo(self, forzar=False):
        """El de ahora. El primero se espera (en este hilo, no en la correa); los
        siguientes se refrescan en segundo plano y mientras vale el que habia."""
        if self.cat is None:
            self._recargar()
            self.primero.wait(120)
            if self.cat is None:
                self.cat = Catalogo({})
        elif forzar or time.time() - self.cuando > FRESCO_S:
            if forzar:
                self._recargar()
            else:
                threading.Thread(target=self._recargar, daemon=True).start()
        return self.cat

    def programar(self, uri):
        with self.candado:
            r = self.relojes.pop(uri, None)
            if r:
                r.cancel()
            version = self.docs.get(uri, ("", 0))[1]
            t = threading.Timer(DEMORA, self.publicar, args=(uri, version))
            t.daemon = True
            self.relojes[uri] = t
            t.start()

    def publicar(self, uri, version):
        texto, v = self.docs.get(uri, ("", -1))
        if v != version:
            return
        try:
            ds = diagnosticar(texto, self.catalogo())
        except Exception as e:
            ds = [{"range": _rango("", 0, 0), "severity": 2, "source": "ore", "message": "el servidor de SQL fallo: %s" % e}]
        if self.docs.get(uri, ("", -1))[1] == version:
            self.mandar({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                         "params": {"uri": uri, "version": version, "diagnostics": ds}})

    def atender(self, m):
        """Lo que la correa da: se encola, y la correa sigue."""
        self.cola.put(m)

    def _atender_ya(self, m):
        metodo, i, p = m.get("method"), m.get("id"), m.get("params") or {}
        uri = (p.get("textDocument") or {}).get("uri")
        if metodo in ("textDocument/completion", "textDocument/hover") and uri not in self.docs:
            # un agente que se reinicio: no se sabe su texto. Que lo reabran.
            self.mandar({"jsonrpc": "2.0", "id": i, "error": {"code": DESCONOCIDO, "message": "documento no abierto: %s" % uri,
                                                               "data": {"ore": "documento-desconocido"}}})
            return
        try:
            r = self._atender(metodo, p)
        except Exception as e:
            if i is not None and metodo is not None:
                self.mandar({"jsonrpc": "2.0", "id": i, "error": {"code": -32603, "message": str(e)}})
            return
        if i is not None and metodo is not None:
            self.mandar({"jsonrpc": "2.0", "id": i, "result": r})

    def _texto(self, p):
        return self.docs.get((p.get("textDocument") or {}).get("uri"), ("", 0))[0]

    def _atender(self, metodo, p):
        if metodo == "initialize":
            # el indice se pide ya, sin esperarlo: la respuesta sale al momento
            threading.Thread(target=self.catalogo, daemon=True).start()
            return {"capabilities": {"textDocumentSync": 1, "hoverProvider": True,
                                     "completionProvider": {"triggerCharacters": ["."]}},
                    "serverInfo": {"name": "ore-sql"}}
        if metodo == "textDocument/didOpen":
            d = p["textDocument"]
            self.docs[d["uri"]] = (d.get("text", ""), d.get("version", 1))
            self.programar(d["uri"])
        elif metodo == "textDocument/didChange":
            d = p["textDocument"]
            cambios = p.get("contentChanges") or []
            if cambios:
                self.docs[d["uri"]] = (cambios[-1].get("text", ""), d.get("version", 0))
                self.programar(d["uri"])
        elif metodo == "textDocument/didClose":
            uri = p["textDocument"]["uri"]
            self.docs.pop(uri, None)
            r = self.relojes.pop(uri, None)
            if r:
                r.cancel()
        elif metodo == "textDocument/completion":
            texto = self._texto(p)
            pos = p.get("position") or {}
            cur = desplazamiento(texto, pos.get("line", 0), pos.get("character", 0))
            items, incompleta = completar_lista(texto, cur, self.catalogo())
            return {"isIncomplete": incompleta, "items": [{"label": l, "kind": k, "detail": d} for l, k, d in items]}
        elif metodo == "textDocument/hover":
            pos = p.get("position") or {}
            h = explicar(self._texto(p), pos.get("line", 0), pos.get("character", 0), self.catalogo())
            return {"contents": {"kind": "markdown", "value": h}} if h else None
        elif metodo == "ore/indice":
            self.catalogo(forzar=True)
            for uri in list(self.docs):
                self.programar(uri)
        return None


# ═════════════════════════════════════════════════════════════════════════════
# Por stdio, para probarlo sin agente
# ═════════════════════════════════════════════════════════════════════════════
def main():
    fichero = sys.argv[sys.argv.index("--indice") + 1]
    salida = threading.Lock()

    def mandar(texto):
        b = texto.encode("utf-8")
        with salida:
            sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(b) + b)
            sys.stdout.buffer.flush()

    def cargar():
        return json.load(open(fichero, encoding="utf-8"))

    s = Servidor(cargar, mandar)
    f = sys.stdin.buffer
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
        if largo is None:
            continue
        try:
            m = json.loads(f.read(largo).decode("utf-8"))
        except ValueError:
            continue
        if m.get("method") == "ore/indice" and (m.get("params") or {}).get("fichero"):
            fichero = m["params"]["fichero"]
        if m.get("method") == "exit":
            return
        s.atender(m)


if __name__ == "__main__":
    main()
